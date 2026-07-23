//! Step output projection helpers. Translate the skill's stdout claim and
//! declared run-outputs / artifact-emits into the typed step projection that
//! downstream graph state machines and receipt sealers consume.

use runx_contracts::{JsonObject, JsonValue};
use runx_parser::{GraphStep, SkillArtifactContract};

use crate::RuntimeError;
use crate::adapter::SkillOutput;
use crate::execution::output_projection::{
    BASE_OUTPUT_FIELDS, StepOutputProjection, data_envelope, project_step_output,
};

/// Project a step's output from its producing runner contract.
///
/// The addressable surface is sourced from the contract, never from the step
/// kind: declared `run.outputs` plus the effective artifact packets. The
/// effective artifact contract is the step's own inline `artifacts` when present,
/// otherwise `extra_artifacts` (the invoked sub-skill / tool runner contract).
/// Base/diagnostic keys (`raw`/`skill_claim`/`stdout`/`stderr`/
/// `status`) are inserted by `project_step_output` for receipts and replay but are
/// never part of the addressable contract.
pub(super) fn build_step_output_projection(
    step: &GraphStep,
    output: &SkillOutput,
    extra_artifacts: Option<&SkillArtifactContract>,
) -> Result<StepOutputProjection, RuntimeError> {
    let mut projection = project_step_output(output);
    expose_declared_run_outputs(step, &projection.claim, &mut projection.outputs)?;
    expose_effective_artifacts(
        step,
        extra_artifacts,
        &projection.claim,
        &mut projection.outputs,
    )?;
    Ok(projection)
}

/// Return only the step outputs declared by its runner or artifact contract.
/// Effect supervisors must inspect this same addressable surface that downstream
/// graph steps consume, never the adapter's transport-level stdout shape.
pub(super) fn contract_output_claim(projection: &StepOutputProjection) -> JsonObject {
    projection
        .outputs
        .iter()
        .filter(|(name, _)| !BASE_OUTPUT_FIELDS.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
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
            step,
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
    step: &GraphStep,
    wrap_as: Option<&str>,
    named_emits: Option<&[String]>,
    claim: &JsonObject,
    outputs: &mut JsonObject,
) -> Result<(), RuntimeError> {
    if let Some(wrap_as) = wrap_as {
        reject_reserved_step_output_name(step, wrap_as, "artifact output")?;
        let value = declared_claim_value(claim, wrap_as).map_or_else(
            || data_envelope(JsonValue::Object(claim.clone())),
            data_envelope,
        );
        outputs.insert(wrap_as.to_owned(), value);
    }
    if let Some(named_emits) = named_emits {
        for name in named_emits {
            reject_reserved_step_output_name(step, name, "artifact output")?;
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
    claim: &JsonObject,
    outputs: &mut JsonObject,
) -> Result<(), RuntimeError> {
    let Some(run) = &step.run else {
        return Ok(());
    };
    let Some(declared_outputs) = run.source().and_then(|source| source.outputs.as_ref()) else {
        return Ok(());
    };
    for name in declared_outputs.keys() {
        reject_reserved_step_output_name(step, name, "declared run output")?;
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

fn declared_claim_value(claim: &JsonObject, name: &str) -> Option<JsonValue> {
    claim.get(name).cloned().or_else(|| {
        ["output", "outputs", "payload"]
            .iter()
            .find_map(|envelope| {
                let JsonValue::Object(object) = claim.get(*envelope)? else {
                    return None;
                };
                object.get(name).cloned()
            })
    })
}

fn reject_reserved_step_output_name(
    step: &GraphStep,
    name: &str,
    output_kind: &str,
) -> Result<(), RuntimeError> {
    if BASE_OUTPUT_FIELDS.contains(&name) {
        return Err(RuntimeError::InvalidRunStep {
            step_id: step.id.clone(),
            reason: format!("{output_kind} name {name:?} is reserved"),
        });
    }
    Ok(())
}
