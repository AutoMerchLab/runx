//! Step output projection helpers. Translate the invocation's typed claim and
//! declared run-outputs / artifact-emits into the typed step projection that
//! downstream graph state machines and receipt sealers consume.

use runx_contracts::{JsonObject, JsonValue};
use runx_parser::{GraphStep, SkillArtifactContract};

use crate::RuntimeError;
use crate::adapter::InvocationOutput;
use crate::execution::output_projection::{
    StepOutputProjection, data_envelope, declared_claim_value, project_step_output,
};

/// Project a step's output from its producing runner contract.
///
/// The addressable surface is sourced from the contract, never from the step
/// kind: declared `run.outputs` plus the effective artifact packets. The
/// effective artifact contract is the step's own inline `artifacts` when present,
/// otherwise `extra_artifacts` (the invoked sub-skill / tool runner contract).
pub(super) fn build_step_output_projection(
    step: &GraphStep,
    output: &InvocationOutput,
    extra_outputs: Option<&JsonObject>,
    extra_artifacts: Option<&SkillArtifactContract>,
) -> Result<StepOutputProjection, RuntimeError> {
    let mut projection = project_step_output(output);
    // A failed invocation produced diagnostics, not its declared success
    // contract. Preserve that failure for sealing instead of replacing it with
    // a secondary "declared output was not returned" projection error.
    if !output.succeeded() {
        return Ok(projection);
    }
    let empty_claim = JsonObject::new();
    let claim = output.value.as_object().unwrap_or(&empty_claim);
    expose_declared_run_outputs(step, extra_outputs, claim, &mut projection.outputs)?;
    expose_effective_artifacts(step, extra_artifacts, claim, &mut projection.outputs)?;
    Ok(projection)
}

/// Return only the step outputs declared by its runner or artifact contract.
/// Effect supervisors must inspect this same addressable surface that downstream
/// graph steps consume, never the adapter's transport-level stdout shape.
pub(super) fn contract_output_claim(projection: &StepOutputProjection) -> &JsonObject {
    &projection.outputs
}

/// Resolve the effective artifact contract for a step and expose its packets. The
/// step's own inline contract wins; otherwise the producing runner's contract is
/// used. Both have already crossed the parser-owned typed boundary.
fn expose_effective_artifacts(
    step: &GraphStep,
    extra_artifacts: Option<&SkillArtifactContract>,
    claim: &JsonObject,
    outputs: &mut JsonObject,
) -> Result<(), RuntimeError> {
    if claim.is_empty() {
        return Ok(());
    }
    if let Some(artifacts) = step.artifacts.as_ref().or(extra_artifacts) {
        let named_emits = artifacts
            .named_emits
            .as_ref()
            .map(|emits| emits.keys().cloned().collect::<Vec<_>>());
        return expose_artifact_packets(
            artifacts.wrap_as.as_deref(),
            named_emits.as_deref(),
            claim,
            outputs,
        );
    }
    Ok(())
}

/// `wrap_as` exposes the whole claim as one `{ data: ... }` packet (idempotent via
/// `data_envelope`), and each `named_emits` key exposes that claim field as its own
/// `{ data: ... }` packet.
fn expose_artifact_packets(
    wrap_as: Option<&str>,
    named_emits: Option<&[String]>,
    claim: &JsonObject,
    outputs: &mut JsonObject,
) -> Result<(), RuntimeError> {
    if let Some(wrap_as) = wrap_as {
        let value = declared_claim_value(claim, wrap_as).map_or_else(
            || data_envelope(JsonValue::Object(claim.clone())),
            data_envelope,
        );
        outputs.insert(wrap_as.to_owned(), value);
    }
    if let Some(named_emits) = named_emits {
        for name in named_emits {
            let Some(value) = declared_claim_value(claim, name) else {
                continue;
            };
            outputs.insert(name.clone(), data_envelope(value));
        }
    }
    Ok(())
}

fn expose_declared_run_outputs(
    step: &GraphStep,
    extra_outputs: Option<&JsonObject>,
    claim: &JsonObject,
    outputs: &mut JsonObject,
) -> Result<(), RuntimeError> {
    let declared_outputs = step
        .run
        .as_ref()
        .and_then(|run| run.source())
        .and_then(|source| source.outputs.as_ref())
        .or(extra_outputs);
    let Some(declared_outputs) = declared_outputs else {
        return Ok(());
    };
    for name in declared_outputs.keys() {
        let Some(value) = declared_claim_value(claim, name) else {
            return Err(RuntimeError::InvalidRunStep {
                step_id: step.id.clone(),
                reason: format!("declared run output {name:?} was not returned by the step"),
            });
        };
        outputs.insert(name.clone(), value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use runx_contracts::{JsonObject, JsonValue};
    use runx_parser::{GraphStep, parse_graph_yaml, validate_graph};

    use super::build_step_output_projection;
    use crate::RuntimeError;
    use crate::adapter::InvocationOutput;

    #[test]
    fn failed_invocation_preserves_its_diagnostic_instead_of_enforcing_success_outputs() {
        let step = declared_output_step();
        let output = InvocationOutput::runtime_failure(
            JsonValue::Object(JsonObject::from([(
                "provider_error".to_owned(),
                JsonValue::String("credits depleted".to_owned()),
            )])),
            "credits depleted",
            4,
            JsonObject::new(),
        );

        let projection = build_step_output_projection(&step, &output, None, None)
            .expect("a failed invocation does not owe its success contract");

        assert!(projection.outputs.is_empty());
        assert_eq!(
            output.failure_message().as_deref(),
            Some("credits depleted")
        );
    }

    #[test]
    fn successful_invocation_still_owes_every_declared_output() {
        let step = declared_output_step();
        let output = InvocationOutput::runtime_success(JsonValue::Null, 4, JsonObject::new());

        let error = match build_step_output_projection(&step, &output, None, None) {
            Err(error) => error,
            Ok(_) => panic!("success without the declared output must fail"),
        };

        assert!(matches!(
            error,
            RuntimeError::InvalidRunStep { reason, .. }
                if reason.contains("declared run output \"result\" was not returned")
        ));
    }

    fn declared_output_step() -> GraphStep {
        let graph = validate_graph(
            parse_graph_yaml(
                r#"
name: output-contract
steps:
  - id: produce
    run:
      type: javascript
      module: produce.mjs
      export: run
      outputs:
        result: object
"#,
            )
            .expect("graph YAML"),
        )
        .expect("valid graph");
        graph.steps.into_iter().next().expect("step")
    }
}
