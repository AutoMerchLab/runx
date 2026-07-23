use super::{
    PackageHarnessReport, SkillRunError, SkillRunOverrides, execute_skill_run_with_overrides,
};

use std::collections::BTreeMap;
use std::path::Path;

use runx_contracts::{JsonObject, JsonValue, Receipt};
use runx_parser::harness_fixture::HarnessExpectedStatus;
use runx_parser::{HarnessCallerFixture, RunnerHarnessCase, SkillRunnerManifest};

use crate::RuntimeError;
use crate::effects::RuntimeEffectRegistry;
use crate::execution::harness::runner::HarnessReplayError;
use crate::execution::harness::{assert_json_expectation, assert_receipt_expectation, status_name};
use crate::execution::orchestrator::SkillRunRequest;

use super::runner_manifest::selected_runner;

mod package;

pub(crate) use package::run_package_harness_with_effects;

/// Run a validated skill's declared inline cases through the same execution
/// path as `runx skill`. Package admission and harness workspace preparation
/// happen once in the owning package entry point.
fn run_loaded_inline_harness_with_effects(
    loaded: &crate::LoadedSkillPackage,
    case_receipt_root: Option<&Path>,
    output_receipt_dir: Option<&Path>,
    env: Option<&BTreeMap<String, String>>,
    effects: &RuntimeEffectRegistry,
) -> Result<PackageHarnessReport, SkillRunError> {
    let manifest = loaded.manifest().cloned().ok_or_else(|| {
        SkillRunError::Invalid(format!(
            "skill package {} does not declare X.yaml runners",
            loaded.directory.display()
        ))
    })?;
    let Some(harness) = manifest.harness.as_ref() else {
        return Ok(PackageHarnessReport::not_declared());
    };
    if harness.cases.is_empty() {
        return Ok(PackageHarnessReport::not_declared());
    }

    let cwd = std::env::current_dir()
        .map_err(|source| RuntimeError::io("resolving cwd for inline harness", source))?;
    let context = InlineHarnessContext {
        skill_dir: &loaded.directory,
        case_receipt_root,
        output_receipt_dir,
        env,
        effects,
        manifest: &manifest,
        cwd: &cwd,
    };
    Ok(run_inline_harness_cases(context, &harness.cases))
}

#[derive(Clone, Copy)]
struct InlineHarnessContext<'a> {
    skill_dir: &'a Path,
    case_receipt_root: Option<&'a Path>,
    output_receipt_dir: Option<&'a Path>,
    env: Option<&'a BTreeMap<String, String>>,
    effects: &'a RuntimeEffectRegistry,
    manifest: &'a SkillRunnerManifest,
    cwd: &'a Path,
}

fn run_inline_harness_cases(
    context: InlineHarnessContext<'_>,
    cases: &[RunnerHarnessCase],
) -> PackageHarnessReport {
    let mut assertion_errors = Vec::new();
    let mut case_names = Vec::with_capacity(cases.len());
    let mut receipt_ids = Vec::new();
    let mut graph_case_count = 0;
    for (index, case) in cases.iter().enumerate() {
        case_names.push(case.name.clone());
        let case_receipt_dir = context
            .case_receipt_root
            .map(|root| root.join(index.to_string()));
        let outcome = run_inline_harness_case(context, case_receipt_dir.as_deref(), case);
        if outcome.is_graph {
            graph_case_count += 1;
        }
        if let Some(receipt_id) = outcome.receipt_id {
            receipt_ids.push(receipt_id);
        }
        if let Some(error) = outcome.assertion_error {
            assertion_errors.push(error);
        }
    }

    let status = if assertion_errors.is_empty() {
        "passed"
    } else {
        "failed"
    };
    PackageHarnessReport {
        assertion_error_count: assertion_errors.len(),
        status,
        case_count: cases.len(),
        assertion_errors,
        case_names,
        receipt_ids,
        graph_case_count,
    }
}

struct InlineHarnessCaseOutcome {
    is_graph: bool,
    receipt_id: Option<String>,
    assertion_error: Option<String>,
}

fn run_inline_harness_case(
    context: InlineHarnessContext<'_>,
    receipt_dir: Option<&Path>,
    case: &RunnerHarnessCase,
) -> InlineHarnessCaseOutcome {
    let runner = match selected_runner(context.manifest, case.runner.as_deref()) {
        Ok(runner) => runner,
        Err(error) => return inline_harness_case_error(&case.name, error),
    };
    let is_graph = runner.source.source_type == runx_parser::SourceKind::Graph;

    // Enforce the required-input contract the real `runx skill` prepare stage
    // applies. The harness executes directly, so without this a missing required
    // input would seal an empty run instead of blocking, masking the failure.
    let missing = crate::input_contract::missing_required(&runner.inputs, &case.inputs);
    if !missing.is_empty() {
        return InlineHarnessCaseOutcome {
            is_graph,
            receipt_id: None,
            assertion_error: inline_harness_status_error(case, HarnessExpectedStatus::Failure),
        };
    }

    let request = inline_harness_case_request(
        context.skill_dir,
        receipt_dir,
        context.env,
        case,
        context.cwd,
    );
    let overrides = SkillRunOverrides {
        runner: case.runner.clone(),
        seeded_answers: seeded_answers_from_caller(&case.caller),
    };
    execute_inline_harness_case(
        &request,
        receipt_dir,
        context.output_receipt_dir,
        case,
        is_graph,
        &overrides,
        context.effects,
    )
}

fn execute_inline_harness_case(
    request: &SkillRunRequest,
    receipt_dir: Option<&Path>,
    output_receipt_dir: Option<&Path>,
    case: &RunnerHarnessCase,
    is_graph: bool,
    overrides: &SkillRunOverrides,
    effects: &RuntimeEffectRegistry,
) -> InlineHarnessCaseOutcome {
    match execute_skill_run_with_overrides(request, overrides, effects) {
        Ok(output) => {
            let receipt_id = receipt_id_from_output(&output);
            if receipt_id.is_some()
                && let (Some(receipt_dir), Some(output_receipt_dir)) =
                    (receipt_dir, output_receipt_dir)
                && let Err(error) =
                    persist_inline_case_receipts(request, receipt_dir, output_receipt_dir)
            {
                return InlineHarnessCaseOutcome {
                    is_graph,
                    receipt_id: None,
                    assertion_error: Some(format!(
                        "{}: failed to persist harness receipts: {error}",
                        case.name
                    )),
                };
            }
            InlineHarnessCaseOutcome {
                is_graph,
                receipt_id,
                assertion_error: inline_harness_expectation_error(request, case, &output),
            }
        }
        Err(error) => InlineHarnessCaseOutcome {
            is_graph,
            receipt_id: None,
            assertion_error: inline_harness_execution_error(case, &error),
        },
    }
}

fn persist_inline_case_receipts(
    request: &SkillRunRequest,
    case_receipt_dir: &Path,
    output_receipt_dir: &Path,
) -> Result<(), String> {
    let receipts = crate::services::ReceiptServices::from_env_or_local_development(&request.env)
        .map_err(|error| error.to_string())?;
    let produced = receipts
        .list_local_receipts(case_receipt_dir)
        .map_err(|error| error.to_string())?;
    receipts
        .write_local_receipts(&produced, output_receipt_dir)
        .map_err(|error| error.to_string())
}

fn inline_harness_case_request(
    skill_dir: &Path,
    receipt_dir: Option<&Path>,
    env: Option<&BTreeMap<String, String>>,
    case: &RunnerHarnessCase,
    cwd: &Path,
) -> SkillRunRequest {
    let mut env: BTreeMap<String, String> = env
        .cloned()
        .unwrap_or_else(crate::services::process_env_snapshot);
    env.extend(case.env.clone());
    SkillRunRequest {
        skill_path: skill_dir.to_path_buf(),
        receipt_dir: receipt_dir.map(Path::to_path_buf),
        run_id: None,
        answers_path: None,
        inputs: case.inputs.clone(),
        env,
        cwd: cwd.to_path_buf(),
        managed_agent: crate::execution::orchestrator::ManagedAgentPolicy::HostDriven,
        local_credential: None,
    }
}

fn inline_harness_case_error(
    case_name: &str,
    error: impl std::fmt::Display,
) -> InlineHarnessCaseOutcome {
    InlineHarnessCaseOutcome {
        is_graph: false,
        receipt_id: None,
        assertion_error: Some(format!("{case_name}: {error}")),
    }
}

fn receipt_id_from_output(output: &JsonValue) -> Option<String> {
    output
        .as_object()
        .and_then(|object| object.get("receipt_id"))
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn inline_harness_expectation_error(
    request: &SkillRunRequest,
    case: &RunnerHarnessCase,
    output: &JsonValue,
) -> Option<String> {
    assert_inline_harness_expectations(request, case, output)
        .err()
        .map(|error| format!("{}: {error}", case.name))
}

fn assert_inline_harness_expectations(
    request: &SkillRunRequest,
    case: &RunnerHarnessCase,
    output: &JsonValue,
) -> Result<(), HarnessReplayError> {
    let actual_status = inline_harness_actual_status(output);
    if let Some(expected_status) = &case.expect.status
        && *expected_status != actual_status
    {
        return Err(HarnessReplayError::Mismatch {
            field: "expect.status".to_owned(),
            expected: status_name(expected_status).to_owned(),
            actual: status_name(&actual_status).to_owned(),
        });
    }

    if let Some(expected_receipt) = &case.expect.receipt {
        let actual_receipt = output
            .as_object()
            .and_then(|object| object.get("receipt"))
            .cloned()
            .ok_or_else(|| HarnessReplayError::InvalidReplayMetadata {
                field: "receipt".to_owned(),
                message: "skill run output omitted its receipt".to_owned(),
            })?;
        let actual_receipt = serde_json::to_value(actual_receipt).map_err(|error| {
            HarnessReplayError::InvalidReplayMetadata {
                field: "receipt".to_owned(),
                message: error.to_string(),
            }
        })?;
        let actual_receipt =
            serde_json::from_value::<Receipt>(actual_receipt).map_err(|error| {
                HarnessReplayError::InvalidReplayMetadata {
                    field: "receipt".to_owned(),
                    message: error.to_string(),
                }
            })?;
        let receipts = crate::services::ReceiptServices::from_env_or_local_development(
            &request.env,
        )
        .map_err(|error| {
            HarnessReplayError::Runtime(RuntimeError::ReceiptInvalid {
                message: error.to_string(),
            })
        })?;
        assert_receipt_expectation(
            expected_receipt,
            &actual_receipt,
            receipts.signature_config().signature_policy(),
        )?;
    }

    let payload = output
        .as_object()
        .and_then(|object| object.get("payload"))
        .unwrap_or(&JsonValue::Null);
    if !case.expect.steps.is_empty() {
        let actual_steps = payload
            .as_object()
            .and_then(|object| object.get("steps"))
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(|step| {
                step.as_object()
                    .and_then(|object| object.get("step_id"))
                    .and_then(JsonValue::as_str)
            })
            .collect::<Vec<_>>();
        if case
            .expect
            .steps
            .iter()
            .map(String::as_str)
            .ne(actual_steps.iter().copied())
        {
            return Err(HarnessReplayError::Mismatch {
                field: "expect.steps".to_owned(),
                expected: case.expect.steps.join(","),
                actual: actual_steps.join(","),
            });
        }
    }
    if let Some(expectation) = &case.expect.output {
        assert_json_expectation(expectation, payload, "expect.output")?;
    }
    for (step_id, expectation) in &case.expect.step_outputs {
        let actual = payload
            .as_object()
            .and_then(|object| object.get("step_outputs"))
            .and_then(JsonValue::as_object)
            .and_then(|step_outputs| step_outputs.get(step_id))
            .unwrap_or(&JsonValue::Null);
        assert_json_expectation(
            expectation,
            actual,
            &format!("expect.step_outputs.{step_id}"),
        )?;
    }
    Ok(())
}

fn inline_harness_status_error(
    case: &RunnerHarnessCase,
    actual: HarnessExpectedStatus,
) -> Option<String> {
    let expected = case.expect.status.as_ref()?;
    (actual != *expected).then(|| {
        format!(
            "{}: expected status {}, got {}",
            case.name,
            status_name(expected),
            status_name(&actual)
        )
    })
}

fn inline_harness_execution_error(
    case: &RunnerHarnessCase,
    error: &impl std::fmt::Display,
) -> Option<String> {
    match case.expect.status.as_ref() {
        Some(HarnessExpectedStatus::Failure) => None,
        Some(expected) => Some(format!(
            "{}: expected status {}, execution failed: {error}",
            case.name,
            status_name(expected)
        )),
        None => Some(format!("{}: {error}", case.name)),
    }
}

// Merge a harness case's caller answers + approvals into one map keyed by
// resolution request id, the shape the seeded agent/graph answer lookup expects.
// Approvals are recorded as booleans under their gate id.
fn seeded_answers_from_caller(caller: &HarnessCallerFixture) -> Option<JsonObject> {
    let mut merged = caller.answers.clone().unwrap_or_default();
    if let Some(approvals) = &caller.approvals {
        for (gate, approved) in approvals {
            merged
                .entry(gate.clone())
                .or_insert_with(|| JsonValue::Bool(*approved));
        }
    }
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

// Map an `execute_skill_run` output onto the harness status vocabulary
// (sealed/failure/needs_agent/policy_denied). A pending run is needs_agent; a
// terminal run is derived from its closure disposition so the mapping matches
// the standalone harness `status_from_disposition`.
fn inline_harness_actual_status(output: &JsonValue) -> HarnessExpectedStatus {
    let Some(object) = output.as_object() else {
        return HarnessExpectedStatus::Sealed;
    };
    if object.get("status").and_then(JsonValue::as_str) == Some("needs_agent") {
        return HarnessExpectedStatus::NeedsAgent;
    }
    let disposition = object
        .get("closure")
        .and_then(JsonValue::as_object)
        .and_then(|closure| closure.get("disposition"))
        .and_then(JsonValue::as_str);
    match disposition {
        Some("deferred") => HarnessExpectedStatus::NeedsAgent,
        Some("blocked") => HarnessExpectedStatus::PolicyDenied,
        Some("declined" | "failed" | "killed" | "timed_out" | "superseded") => {
            HarnessExpectedStatus::Failure
        }
        _ => HarnessExpectedStatus::Sealed,
    }
}
