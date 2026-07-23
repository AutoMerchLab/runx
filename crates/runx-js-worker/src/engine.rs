use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use boa_engine::builtins::promise::PromiseState;
use boa_engine::context::time::FixedClock;
use boa_engine::job::{Job, JobExecutor};
use boa_engine::module::MapModuleLoader;
use boa_engine::{Context, JsError, JsNativeError, JsValue, Module, Source, js_string};
use thiserror::Error;

use crate::protocol::{InvocationLimits, WorkerFailureCode};

const VIRTUAL_ROOT: &str = "/runx";
const LOOP_ITERATION_LIMIT: u64 = 10_000_000;
const RECURSION_LIMIT: usize = 1_024;
const BACKTRACE_LIMIT: usize = 32;
const ERROR_MESSAGE_BYTES: usize = 4_096;

#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct EngineError {
    pub(crate) code: WorkerFailureCode,
    pub(crate) message: String,
}

impl EngineError {
    fn new(code: WorkerFailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: bounded_message(message.into()),
        }
    }
}

pub(crate) fn evaluate(
    entry_module: &str,
    export_name: &str,
    modules: &BTreeMap<String, String>,
    inputs: serde_json::Value,
    limits: InvocationLimits,
) -> Result<serde_json::Value, EngineError> {
    let limits = limits
        .validate()
        .map_err(|error| EngineError::new(WorkerFailureCode::InvalidRequest, error.to_string()))?;
    validate_bundle(entry_module, export_name, modules, limits)?;
    validate_input(&inputs, limits.input_bytes)?;

    let loader = Rc::new(MapModuleLoader::new());
    let jobs = Rc::new(BoundedJobExecutor::new(limits.queued_jobs));
    let mut context = Context::builder()
        .clock(Rc::new(FixedClock::from_millis(0)))
        .job_executor(jobs.clone())
        .module_loader(loader.clone())
        .build()
        .map_err(|error| engine_failure("creating JavaScript context", error))?;
    configure_context(&mut context, limits.stack_bytes)?;

    let parsed = parse_modules(modules, &loader, &mut context)?;
    let entry = parsed.get(entry_module).ok_or_else(|| {
        EngineError::new(
            WorkerFailureCode::ModuleRejected,
            format!("entry module {entry_module:?} is absent from the validated bundle"),
        )
    })?;
    settle_module(entry, &jobs, &mut context)?;
    let exported = entry
        .namespace(&mut context)
        .get(js_string!(export_name), &mut context)
        .map_err(|error| engine_failure("resolving JavaScript export", error))?;
    let callable = exported.as_callable().ok_or_else(|| {
        EngineError::new(
            WorkerFailureCode::ExecutionFailed,
            format!("JavaScript module does not export callable {export_name:?}"),
        )
    })?;
    let input = JsValue::from_json(&inputs, &mut context)
        .map_err(|error| engine_failure("materializing JavaScript input", error))?;
    let result = callable
        .call(&JsValue::undefined(), &[input], &mut context)
        .map_err(|error| engine_failure("calling JavaScript export", error))?;
    let result = settle_result(result, &jobs, &mut context)?;
    let output = result
        .to_json(&mut context)
        .map_err(|error| engine_failure("converting JavaScript result to JSON", error))?
        .ok_or_else(|| {
            EngineError::new(
                WorkerFailureCode::OutputRejected,
                "JavaScript result is not JSON-compatible",
            )
        })?;
    let output_bytes = serde_json::to_vec(&output)
        .map_err(|error| EngineError::new(WorkerFailureCode::OutputRejected, error.to_string()))?;
    if output_bytes.len() > limits.output_bytes {
        return Err(EngineError::new(
            WorkerFailureCode::ResourceLimit,
            format!(
                "JavaScript output is {} bytes; limit is {} bytes",
                output_bytes.len(),
                limits.output_bytes
            ),
        ));
    }
    Ok(output)
}

fn validate_input(inputs: &serde_json::Value, maximum: usize) -> Result<(), EngineError> {
    let bytes = serde_json::to_vec(inputs)
        .map_err(|error| EngineError::new(WorkerFailureCode::InvalidRequest, error.to_string()))?;
    if bytes.len() <= maximum {
        return Ok(());
    }
    Err(EngineError::new(
        WorkerFailureCode::ResourceLimit,
        format!(
            "JavaScript input is {} bytes; limit is {maximum} bytes",
            bytes.len()
        ),
    ))
}

fn validate_bundle(
    entry_module: &str,
    export_name: &str,
    modules: &BTreeMap<String, String>,
    limits: InvocationLimits,
) -> Result<(), EngineError> {
    validate_module_path(entry_module)?;
    validate_export_name(export_name)?;
    if !modules.contains_key(entry_module) {
        return Err(EngineError::new(
            WorkerFailureCode::ModuleRejected,
            "entry module is not present in the supplied bundle",
        ));
    }
    let input_bytes = serde_json::to_vec(modules)
        .map_err(|error| EngineError::new(WorkerFailureCode::InvalidRequest, error.to_string()))?;
    if input_bytes.len() > limits.source_bytes {
        return Err(EngineError::new(
            WorkerFailureCode::ResourceLimit,
            format!(
                "JavaScript module bundle is {} bytes; limit is {} bytes",
                input_bytes.len(),
                limits.source_bytes
            ),
        ));
    }
    for (path, source) in modules {
        validate_module_path(path)?;
        let imports = runx_parser::javascript_module_imports(path, source).map_err(|error| {
            EngineError::new(WorkerFailureCode::ModuleRejected, error.to_string())
        })?;
        for specifier in imports {
            let resolved = runx_parser::resolve_javascript_module_import(path, &specifier)
                .map_err(|error| {
                    EngineError::new(WorkerFailureCode::ModuleRejected, error.to_string())
                })?;
            if !modules.contains_key(&resolved) {
                return Err(EngineError::new(
                    WorkerFailureCode::ModuleRejected,
                    format!(
                        "JavaScript import {specifier:?} from {path:?} resolves outside the supplied bundle"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_module_path(path: &str) -> Result<(), EngineError> {
    let valid_extension = path.ends_with(".js") || path.ends_with(".mjs");
    let valid_segments = !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."));
    if valid_extension && valid_segments {
        return Ok(());
    }
    Err(EngineError::new(
        WorkerFailureCode::ModuleRejected,
        format!("JavaScript module path {path:?} is not a normalized relative .js/.mjs path"),
    ))
}

fn validate_export_name(name: &str) -> Result<(), EngineError> {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || matches!(character, '_' | '$'));
    if valid_start
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
    {
        return Ok(());
    }
    Err(EngineError::new(
        WorkerFailureCode::InvalidRequest,
        format!("JavaScript export name {name:?} is not an identifier"),
    ))
}

fn configure_context(context: &mut Context, stack_bytes: usize) -> Result<(), EngineError> {
    let mut runtime_limits = context.runtime_limits();
    runtime_limits.set_loop_iteration_limit(LOOP_ITERATION_LIMIT);
    runtime_limits.set_stack_size_limit(stack_bytes / std::mem::size_of::<JsValue>());
    runtime_limits.set_recursion_limit(RECURSION_LIMIT);
    runtime_limits.set_backtrace_limit(BACKTRACE_LIMIT);
    context.set_runtime_limits(runtime_limits);
    context
        .eval(Source::from_bytes(
            r#"
                Object.defineProperty(Math, "random", {
                    value() { throw new TypeError("Math.random is unavailable in deterministic modules"); },
                    configurable: false,
                    enumerable: false,
                    writable: false
                });
            "#,
        ))
        .map(|_| ())
        .map_err(|error| engine_failure("installing deterministic globals", error))
}

fn parse_modules(
    modules: &BTreeMap<String, String>,
    loader: &MapModuleLoader,
    context: &mut Context,
) -> Result<BTreeMap<String, Module>, EngineError> {
    let mut parsed = BTreeMap::new();
    for (path, source) in modules {
        let virtual_path = virtual_path(path);
        let module = Module::parse(
            Source::from_bytes(source.as_bytes()).with_path(&virtual_path),
            None,
            context,
        )
        .map_err(|error| engine_failure(format!("parsing JavaScript module {path:?}"), error))?;
        loader.insert(virtual_path.to_string_lossy(), module.clone());
        parsed.insert(path.clone(), module);
    }
    Ok(parsed)
}

fn settle_module(
    module: &Module,
    jobs: &BoundedJobExecutor,
    context: &mut Context,
) -> Result<(), EngineError> {
    let promise = module.load_link_evaluate(context);
    context
        .run_jobs()
        .map_err(|error| engine_failure("evaluating JavaScript module", error))?;
    jobs.check()?;
    match promise.state() {
        PromiseState::Fulfilled(_) => Ok(()),
        PromiseState::Rejected(error) => Err(engine_failure(
            "evaluating JavaScript module",
            JsError::from_opaque(error),
        )),
        PromiseState::Pending => Err(EngineError::new(
            WorkerFailureCode::ExecutionFailed,
            "JavaScript module evaluation left a pending promise",
        )),
    }
}

fn settle_result(
    result: JsValue,
    jobs: &BoundedJobExecutor,
    context: &mut Context,
) -> Result<JsValue, EngineError> {
    let Some(object) = result.as_object() else {
        return Ok(result);
    };
    let Ok(promise) = boa_engine::object::builtins::JsPromise::from_object(object.clone()) else {
        return Ok(result);
    };
    context
        .run_jobs()
        .map_err(|error| engine_failure("settling JavaScript result", error))?;
    jobs.check()?;
    match promise.state() {
        PromiseState::Fulfilled(value) => Ok(value),
        PromiseState::Rejected(error) => Err(engine_failure(
            "settling JavaScript result",
            JsError::from_opaque(error),
        )),
        PromiseState::Pending => Err(EngineError::new(
            WorkerFailureCode::ExecutionFailed,
            "JavaScript export returned a promise that did not settle in the immediate job queue",
        )),
    }
}

fn virtual_path(path: &str) -> PathBuf {
    Path::new(VIRTUAL_ROOT).join(path)
}

fn engine_failure(context: impl AsRef<str>, error: impl std::fmt::Display) -> EngineError {
    EngineError::new(
        WorkerFailureCode::ExecutionFailed,
        format!("{}: {error}", context.as_ref()),
    )
}

fn bounded_message(mut message: String) -> String {
    if message.len() <= ERROR_MESSAGE_BYTES {
        return message;
    }
    let mut end = ERROR_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    message.truncate(end);
    message.push('…');
    message
}

struct BoundedJobExecutor {
    promise_jobs: RefCell<VecDeque<boa_engine::job::PromiseJob>>,
    async_jobs: RefCell<VecDeque<boa_engine::job::NativeAsyncJob>>,
    generic_jobs: RefCell<VecDeque<boa_engine::job::GenericJob>>,
    count: Cell<u32>,
    maximum: u32,
    overflowed: Cell<bool>,
}

impl BoundedJobExecutor {
    fn new(maximum: u32) -> Self {
        Self {
            promise_jobs: RefCell::new(VecDeque::new()),
            async_jobs: RefCell::new(VecDeque::new()),
            generic_jobs: RefCell::new(VecDeque::new()),
            count: Cell::new(0),
            maximum,
            overflowed: Cell::new(false),
        }
    }

    fn check(&self) -> Result<(), EngineError> {
        if self.overflowed.get() {
            return Err(EngineError::new(
                WorkerFailureCode::ResourceLimit,
                format!("JavaScript queued more than {} jobs", self.maximum),
            ));
        }
        Ok(())
    }

    fn admit(&self) -> bool {
        let next = self.count.get().saturating_add(1);
        self.count.set(next);
        if next > self.maximum {
            self.overflowed.set(true);
            return false;
        }
        true
    }
}

impl JobExecutor for BoundedJobExecutor {
    fn enqueue_job(self: Rc<Self>, job: Job, _context: &mut Context) {
        if !self.admit() {
            return;
        }
        match job {
            Job::PromiseJob(job) => self.promise_jobs.borrow_mut().push_back(job),
            Job::AsyncJob(job) => self.async_jobs.borrow_mut().push_back(job),
            Job::GenericJob(job) => self.generic_jobs.borrow_mut().push_back(job),
            Job::TimeoutJob(_) => self.overflowed.set(true),
            _ => self.overflowed.set(true),
        }
    }

    fn run_jobs(self: Rc<Self>, context: &mut Context) -> boa_engine::JsResult<()> {
        loop {
            let asynchronous = self.async_jobs.borrow_mut().pop_front();
            if let Some(job) = asynchronous {
                let context_cell = RefCell::new(&mut *context);
                futures_lite::future::block_on(job.call(&context_cell))?;
                continue;
            }
            let promise = self.promise_jobs.borrow_mut().pop_front();
            if let Some(job) = promise {
                job.call(context)?;
                continue;
            }
            let generic = self.generic_jobs.borrow_mut().pop_front();
            if let Some(job) = generic {
                job.call(context)?;
                continue;
            }
            break;
        }
        if self.overflowed.get() {
            return Err(JsNativeError::range()
                .with_message("deterministic JavaScript job limit exceeded")
                .into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules(source: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("main.mjs".to_owned(), source.to_owned())])
    }

    #[test]
    fn evaluates_default_export_with_fixed_time() -> Result<(), EngineError> {
        let output = evaluate(
            "main.mjs",
            "default",
            &modules("export default ({ value }) => ({ value, now: Date.now() });"),
            serde_json::json!({"value": "runx"}),
            InvocationLimits::default(),
        )?;
        assert_eq!(output, serde_json::json!({"value": "runx", "now": 0}));
        Ok(())
    }

    #[test]
    fn resolves_relative_modules_from_memory() -> Result<(), EngineError> {
        let bundle = BTreeMap::from([
            (
                "domain/main.mjs".to_owned(),
                "import { value } from './value.mjs'; export default () => ({ value });".to_owned(),
            ),
            (
                "domain/value.mjs".to_owned(),
                "export const value = 42;".to_owned(),
            ),
        ]);
        let output = evaluate(
            "domain/main.mjs",
            "default",
            &bundle,
            serde_json::json!({}),
            InvocationLimits::default(),
        )?;
        assert_eq!(output, serde_json::json!({"value": 42}));
        Ok(())
    }

    #[test]
    fn rejects_host_randomness() {
        let error = evaluate(
            "main.mjs",
            "default",
            &modules("export default () => Math.random();"),
            serde_json::json!({}),
            InvocationLimits::default(),
        )
        .err()
        .map(|error| error.to_string());
        assert!(error.is_some_and(|message| message.contains("Math.random")));
    }
}
