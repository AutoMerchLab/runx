use std::path::Path;
use std::sync::{Arc, Mutex};

use runx_contracts::{JsonObject, JsonValue};
use runx_runtime::{
    InvocationStatus, Runtime, RuntimeError, RuntimeOptions, SkillAdapter, SkillInvocation,
    SkillOutput,
};

#[derive(Clone, Default)]
struct RecordingAdapter {
    calls: Arc<Mutex<Vec<(String, JsonObject)>>>,
}

impl SkillAdapter for RecordingAdapter {
    fn adapter_type(&self) -> &'static str {
        "context-regression"
    }

    fn invoke(&self, request: SkillInvocation) -> Result<SkillOutput, RuntimeError> {
        self.calls
            .lock()
            .map_err(|_| RuntimeError::ReceiptInvalid {
                message: "context regression adapter lock poisoned".to_owned(),
            })?
            .push((request.skill_name.clone(), request.inputs.clone()));
        Ok(SkillOutput {
            status: InvocationStatus::Success,
            stdout: serde_json::to_string(&JsonValue::Object(request.inputs)).map_err(
                |source| RuntimeError::ReceiptInvalid {
                    message: format!("serializing context regression output: {source}"),
                },
            )?,
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 0,
            metadata: JsonObject::new(),
        })
    }
}

#[test]
fn graph_context_materialization_reaches_the_target_invocation()
-> Result<(), Box<dyn std::error::Error>> {
    let adapter = RecordingAdapter::default();
    let runtime = Runtime::new(adapter.clone(), RuntimeOptions::local_development());
    let graph_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/graphs/sequential/graph.yaml");

    let run = runtime.run_graph_file(&graph_path)?;

    assert_eq!(run.steps.len(), 2);
    let calls = adapter
        .calls
        .lock()
        .map_err(|_| "context regression adapter lock poisoned")?;
    assert_eq!(calls.len(), 2, "producer and consumer must both run");
    let (_, consumer_inputs) = calls.last().ok_or("consumer skill was not invoked")?;
    assert_eq!(
        consumer_inputs.get("message"),
        Some(&JsonValue::String("hello from graph".to_owned()))
    );
    Ok(())
}
