mod bundle;
mod paged;
mod supervisor;

use std::sync::Arc;
use std::time::Instant;

use runx_contracts::javascript_worker::MAX_WORKER_POOL_SIZE;
use runx_contracts::{JsonObject, JsonValue};

use self::bundle::validated_module;
use self::supervisor::JavaScriptWorkerSupervisor;
use crate::RuntimeError;
use crate::adapter::{InvocationOutput, InvocationStatus, SkillAdapter, SkillInvocation};
use crate::adapter_pipeline::AdapterProjection;

const WORKER_PATH_ENV: &str = "RUNX_JS_WORKER_PATH";

/// One explicit deterministic-JavaScript session. Clones share a bounded lazy
/// worker pool so warm sequential work reuses one process while concurrent
/// branches receive independent wall-time kill boundaries. Independent
/// adapters never share failure or lifecycle state.
#[derive(Clone)]
pub struct JavaScriptAdapter {
    supervisor: Arc<JavaScriptWorkerSupervisor>,
    max_concurrency: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JavaScriptSessionStats {
    pub spawned_process_count: u64,
    pub peak_in_flight: usize,
}

pub(crate) struct PreparedJavaScriptInvocation {
    entry_module: String,
    export_name: String,
    modules: std::collections::BTreeMap<String, String>,
    environment: std::collections::BTreeMap<String, String>,
    worker_path: Option<String>,
    limits: runx_contracts::javascript_worker::InvocationLimits,
}

impl PreparedJavaScriptInvocation {
    fn with_inputs(
        &self,
        inputs: &JsonObject,
    ) -> Result<supervisor::WorkerInvocation, RuntimeError> {
        let inputs = serde_json::to_value(inputs)
            .map_err(|source| RuntimeError::json("serializing JavaScript inputs", source))?;
        Ok(supervisor::WorkerInvocation {
            entry_module: self.entry_module.clone(),
            export_name: self.export_name.clone(),
            modules: self.modules.clone(),
            inputs,
            environment: self.environment.clone(),
            worker_path: self.worker_path.clone(),
            limits: self.limits,
        })
    }
}

impl JavaScriptAdapter {
    #[must_use]
    pub fn new_session() -> Self {
        Self::with_max_concurrency(1)
    }

    #[must_use]
    pub fn with_max_concurrency(max_concurrency: usize) -> Self {
        let max_concurrency = max_concurrency.clamp(1, MAX_WORKER_POOL_SIZE);
        Self {
            supervisor: Arc::new(JavaScriptWorkerSupervisor::new(max_concurrency)),
            max_concurrency,
        }
    }

    #[must_use]
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    #[must_use]
    pub fn spawned_process_count(&self) -> u64 {
        self.supervisor.spawn_count()
    }

    #[must_use]
    pub fn session_stats(&self) -> JavaScriptSessionStats {
        JavaScriptSessionStats {
            spawned_process_count: self.supervisor.spawn_count(),
            peak_in_flight: self.supervisor.peak_in_flight(),
        }
    }

    pub(crate) fn prepare_invocation(
        &self,
        request: &SkillInvocation,
    ) -> Result<PreparedJavaScriptInvocation, RuntimeError> {
        validate_pure_javascript_boundary(request)?;
        validated_module(request)
    }

    pub(crate) fn invoke_prepared(
        &self,
        prepared: &PreparedJavaScriptInvocation,
        inputs: &JsonObject,
    ) -> Result<InvocationOutput, RuntimeError> {
        let started = Instant::now();
        let outcome = self.supervisor.invoke(prepared.with_inputs(inputs)?)?;
        project_worker_outcome(started, outcome, prepared.limits)
    }

    pub(crate) fn invoke_with_artifacts(
        &self,
        request: SkillInvocation,
        local_artifacts: &crate::services::LocalArtifactService,
    ) -> Result<InvocationOutput, RuntimeError> {
        if request.source.pages.is_some() {
            return paged::invoke(self, request, local_artifacts);
        }
        self.invoke_once(request)
    }

    fn invoke_once(&self, request: SkillInvocation) -> Result<InvocationOutput, RuntimeError> {
        let prepared = self.prepare_invocation(&request)?;
        self.invoke_prepared(&prepared, &request.inputs)
    }
}

impl Default for JavaScriptAdapter {
    fn default() -> Self {
        Self::new_session()
    }
}

impl std::fmt::Debug for JavaScriptAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JavaScriptAdapter")
            .field("max_concurrency", &self.max_concurrency)
            .field("spawned_process_count", &self.spawned_process_count())
            .finish_non_exhaustive()
    }
}

impl SkillAdapter for JavaScriptAdapter {
    fn adapter_type(&self) -> &'static str {
        "javascript"
    }

    fn invoke(&self, request: SkillInvocation) -> Result<InvocationOutput, RuntimeError> {
        self.invoke_with_artifacts(request, &crate::services::LocalArtifactService::default())
    }

    fn isolated_fanout_adapter(
        &self,
        source: &runx_parser::SkillSource,
    ) -> Option<Box<dyn SkillAdapter + Send + Sync>> {
        (source.source_type == runx_parser::SourceKind::JavaScript)
            .then(|| Box::new(self.clone()) as Box<dyn SkillAdapter + Send + Sync>)
    }
}

fn project_worker_outcome(
    started: Instant,
    outcome: supervisor::WorkerInvocationOutcome,
    limits: runx_contracts::javascript_worker::InvocationLimits,
) -> Result<InvocationOutput, RuntimeError> {
    match outcome.result {
        supervisor::WorkerInvocationResult::Success(output) => {
            let value = serde_json::from_value(output).map_err(|source| {
                RuntimeError::json("converting JavaScript worker output", source)
            })?;
            Ok(
                AdapterProjection::from_duration_ms(elapsed_ms(started)).runtime_output(
                    InvocationStatus::Success,
                    value,
                    None,
                    javascript_metadata("completed", outcome.isolation, limits),
                ),
            )
        }
        supervisor::WorkerInvocationResult::Failure {
            code,
            limit,
            message,
            ..
        } => Ok(
            AdapterProjection::from_duration_ms(elapsed_ms(started)).runtime_output(
                InvocationStatus::Failure,
                JsonValue::Null,
                Some(message),
                javascript_failure_metadata(&code, limit, outcome.isolation, limits),
            ),
        ),
    }
}

fn validate_pure_javascript_boundary(request: &SkillInvocation) -> Result<(), RuntimeError> {
    let source = &request.source;
    let forbidden = source.command.is_some()
        || !source.args.is_empty()
        || source.cwd.is_some()
        || source.input_mode.is_some()
        || source.sandbox.is_some()
        || source.server.is_some()
        || source.tool.is_some()
        || source.arguments.is_some()
        || source.agent_card_url.is_some()
        || source.agent_identity.is_some()
        || source.agent.is_some()
        || source.task.is_some()
        || source.graph.is_some();
    if source.source_type != runx_parser::SourceKind::JavaScript || forbidden {
        return Err(RuntimeError::SandboxViolation {
            message: "javascript sources may declare only module, export, timeout_seconds, environment, outputs, and act metadata; the runtime owns every containment control"
                .to_owned(),
        });
    }
    if !request.credential_delivery.secret_env().is_empty() {
        return Err(RuntimeError::SandboxViolation {
            message: "javascript sources cannot receive credentials; route provider access through a typed native capability"
                .to_owned(),
        });
    }
    Ok(())
}

fn javascript_metadata(
    state: &str,
    isolation: JsonObject,
    limits: runx_contracts::javascript_worker::InvocationLimits,
) -> JsonObject {
    [
        (
            "javascript_runtime".to_owned(),
            JsonValue::String("runx-js-worker".to_owned()),
        ),
        (
            "javascript_state".to_owned(),
            JsonValue::String(state.to_owned()),
        ),
        (
            "javascript_isolation".to_owned(),
            JsonValue::Object(isolation),
        ),
        (
            crate::adapter::EXECUTION_LIMITS_METADATA.to_owned(),
            JsonValue::Object(JsonObject::from([(
                "javascript_wall".to_owned(),
                JsonValue::Object(JsonObject::from([
                    (
                        "configured".to_owned(),
                        JsonValue::Number(runx_contracts::JsonNumber::U64(
                            limits.wall_milliseconds,
                        )),
                    ),
                    (
                        "maximum".to_owned(),
                        JsonValue::Number(runx_contracts::JsonNumber::U64(
                            runx_contracts::javascript_worker::MAX_WALL_MILLISECONDS,
                        )),
                    ),
                    (
                        "unit".to_owned(),
                        JsonValue::String("milliseconds".to_owned()),
                    ),
                ])),
            )])),
        ),
    ]
    .into_iter()
    .collect()
}

fn javascript_failure_metadata(
    code: &runx_contracts::javascript_worker::WorkerFailureCode,
    limit: Option<runx_contracts::javascript_worker::WorkerLimit>,
    isolation: JsonObject,
    limits: runx_contracts::javascript_worker::InvocationLimits,
) -> JsonObject {
    let mut metadata = javascript_metadata("failed", isolation, limits);
    metadata.insert(
        "javascript_failure_code".to_owned(),
        JsonValue::String(code.as_str().to_owned()),
    );
    if let Some(limit) = limit {
        if let Some(JsonValue::Object(execution_limits)) =
            metadata.get_mut(crate::adapter::EXECUTION_LIMITS_METADATA)
        {
            execution_limits.insert(
                "hit".to_owned(),
                JsonValue::Object(javascript_limit_hit(limit, limits)),
            );
        }
    }
    metadata
}

fn javascript_limit_hit(
    limit: runx_contracts::javascript_worker::WorkerLimit,
    limits: runx_contracts::javascript_worker::InvocationLimits,
) -> JsonObject {
    use runx_contracts::javascript_worker::{InvocationLimits, WorkerLimit};

    let maximum = InvocationLimits::default();
    let (configured, ceiling, unit, manifest_field) = match limit {
        WorkerLimit::SourceBytes => (
            usize_as_u64(limits.source_bytes),
            usize_as_u64(maximum.source_bytes),
            "bytes",
            None,
        ),
        WorkerLimit::InputBytes => (
            usize_as_u64(limits.input_bytes),
            usize_as_u64(maximum.input_bytes),
            "bytes",
            None,
        ),
        WorkerLimit::OutputBytes => (
            usize_as_u64(limits.output_bytes),
            usize_as_u64(maximum.output_bytes),
            "bytes",
            None,
        ),
        WorkerLimit::WallMilliseconds => (
            limits.wall_milliseconds,
            runx_contracts::javascript_worker::MAX_WALL_MILLISECONDS,
            "milliseconds",
            Some("source.timeout_seconds"),
        ),
        WorkerLimit::QueuedJobs => (
            u64::from(limits.queued_jobs),
            u64::from(maximum.queued_jobs),
            "jobs",
            None,
        ),
    };
    let mut hit = JsonObject::from([
        (
            "id".to_owned(),
            JsonValue::String(format!("javascript.{}", limit.as_str())),
        ),
        (
            "configured".to_owned(),
            JsonValue::Number(runx_contracts::JsonNumber::U64(configured)),
        ),
        (
            "maximum".to_owned(),
            JsonValue::Number(runx_contracts::JsonNumber::U64(ceiling)),
        ),
        ("unit".to_owned(), JsonValue::String(unit.to_owned())),
    ]);
    if let Some(field) = manifest_field {
        hit.insert(
            "manifest_field".to_owned(),
            JsonValue::String(field.to_owned()),
        );
    }
    hit
}

fn usize_as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
