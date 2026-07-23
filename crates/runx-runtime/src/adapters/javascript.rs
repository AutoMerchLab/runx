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
use crate::adapter::{
    FanoutExecutionMode, InvocationStatus, SkillAdapter, SkillInvocation, SkillOutput,
};
use crate::adapter_pipeline::{AdapterCapture, AdapterProjection};

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
    limits: runx_contracts::javascript_worker::InvocationLimits,
}

impl PreparedJavaScriptInvocation {
    fn with_inputs(
        &self,
        inputs: &JsonObject,
    ) -> Result<supervisor::WorkerInvocation, RuntimeError> {
        let inputs = serde_json::to_value(inputs)
            .map_err(|source| RuntimeError::json("serializing JavaScript inputs", source))?;
        let input_bytes = serde_json::to_vec(&inputs)
            .map_err(|source| RuntimeError::json("measuring JavaScript inputs", source))?;
        if input_bytes.len() > self.limits.input_bytes {
            return Err(RuntimeError::JavaScriptWorker {
                message: format!(
                    "JavaScript input is {} bytes; limit is {} bytes",
                    input_bytes.len(),
                    self.limits.input_bytes
                ),
            });
        }
        Ok(supervisor::WorkerInvocation {
            entry_module: self.entry_module.clone(),
            export_name: self.export_name.clone(),
            modules: self.modules.clone(),
            inputs,
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
    ) -> Result<SkillOutput, RuntimeError> {
        let started = Instant::now();
        let outcome = self.supervisor.invoke(prepared.with_inputs(inputs)?)?;
        project_worker_outcome(started, outcome)
    }

    pub(crate) fn invoke_with_artifacts(
        &self,
        request: SkillInvocation,
        local_artifacts: &crate::services::LocalArtifactService,
    ) -> Result<SkillOutput, RuntimeError> {
        if request.source.pages.is_some() {
            return paged::invoke(self, request, local_artifacts);
        }
        self.invoke_once(request)
    }

    fn invoke_once(&self, request: SkillInvocation) -> Result<SkillOutput, RuntimeError> {
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

    fn invoke(&self, request: SkillInvocation) -> Result<SkillOutput, RuntimeError> {
        self.invoke_with_artifacts(request, &crate::services::LocalArtifactService::default())
    }

    fn fanout_execution_mode(&self, _source: &runx_parser::SkillSource) -> FanoutExecutionMode {
        FanoutExecutionMode::IsolatedParallel
    }

    fn clone_for_fanout(&self) -> Option<Box<dyn SkillAdapter + Send + Sync>> {
        Some(Box::new(self.clone()))
    }
}

fn project_worker_outcome(
    started: Instant,
    outcome: supervisor::WorkerInvocationOutcome,
) -> Result<SkillOutput, RuntimeError> {
    match outcome.result {
        supervisor::WorkerInvocationResult::Success(output) => {
            let stdout = serde_json::to_string(&output)
                .map_err(|source| RuntimeError::json("serializing JavaScript output", source))?;
            Ok(
                AdapterProjection::from_duration_ms(elapsed_ms(started)).output(
                    InvocationStatus::Success,
                    AdapterCapture::new(stdout, String::new()),
                    Some(0),
                    javascript_metadata("completed", outcome.isolation),
                ),
            )
        }
        supervisor::WorkerInvocationResult::Failure { code, message, .. } => Ok(
            AdapterProjection::from_duration_ms(elapsed_ms(started)).output(
                InvocationStatus::Failure,
                AdapterCapture::new(String::new(), message),
                Some(1),
                javascript_failure_metadata(&code, outcome.isolation),
            ),
        ),
    }
}

fn validate_pure_javascript_boundary(request: &SkillInvocation) -> Result<(), RuntimeError> {
    let source = &request.source;
    let forbidden = source.command.is_some()
        || !source.args.is_empty()
        || source.cwd.is_some()
        || source.timeout_seconds.is_some()
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
            message: "javascript sources may declare only module, export, outputs, and act metadata; the runtime owns every containment control"
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

fn javascript_metadata(state: &str, isolation: JsonObject) -> JsonObject {
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
    ]
    .into_iter()
    .collect()
}

fn javascript_failure_metadata(
    code: &runx_contracts::javascript_worker::WorkerFailureCode,
    isolation: JsonObject,
) -> JsonObject {
    let mut metadata = javascript_metadata("failed", isolation);
    metadata.insert(
        "javascript_failure_code".to_owned(),
        JsonValue::String(code.as_str().to_owned()),
    );
    metadata
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
