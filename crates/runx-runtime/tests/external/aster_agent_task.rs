use std::path::{Path, PathBuf};

use runx_contracts::{ClosureDisposition, JsonObject, JsonValue};
use runx_receipts::validate_receipt;
use runx_runtime::{
    HarnessExpectedStatus, HarnessReplayOutput, InvocationOutput, RuntimeOptions, SkillAdapter,
    SkillInvocation, load_harness_fixture, run_harness_fixture_with_adapter,
};

const FIXTURE_CREATED_AT: &str = "2026-05-18T00:00:00Z";
const RETIRED_VERIFICATION_REPORT_FIELD: &str = concat!("verification", "_", "report");

#[test]
fn aster_agent_task_replays_current_rust_bridge_terminal_report()
-> Result<(), Box<dyn std::error::Error>> {
    let output = run_case()?;

    assert_eq!(output.status, HarnessExpectedStatus::Sealed);
    assert_eq!(output.receipt.seal.disposition, ClosureDisposition::Closed);
    validate_receipt(&output.receipt)
        .map_err(|verification| format!("{:?}", verification.findings))?;

    let payload = skill_payload(&output)?;
    assert_no_retired_bridge_fields(&payload, "report")?;
    assert_eq!(string_field_value(&payload, "schema")?, "runx.skill_run.v1");
    assert_eq!(string_field_value(&payload, "status")?, "sealed");
    assert_eq!(
        string_field_value(&payload, "receipt_id")?,
        "hrn_rcpt_aster_issue_triage_14"
    );
    assert_eq!(
        string_field_value(&payload, "run_id")?,
        "run_aster_issue_triage_14"
    );

    let result = object_field_value(&payload, "result")?;
    assert_eq!(
        string_field(result, "summary")?,
        "Aster issue-triage bridge completed."
    );
    assert!(payload.as_object().is_some_and(|object| {
        !object.contains_key("receipt") && !object.contains_key("execution")
    }));

    let skill_output = output
        .skill_output
        .as_ref()
        .ok_or("agent-task fixture did not produce skill output")?;
    assert_eq!(
        string_field(&skill_output.metadata, "agent_request_id")?,
        "agent_task.aster-rust-bridge.output"
    );

    Ok(())
}

#[test]
fn aster_external_fixture_records_grounded_bridge_sources() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = load_harness_fixture(case_path())?;

    assert_eq!(
        fixture.metadata.get("external_project"),
        Some(&JsonValue::String("aster".to_owned()))
    );
    assert_eq!(
        fixture.metadata.get("bridge_contract"),
        Some(&JsonValue::String("runx.skill_run.v1".to_owned()))
    );
    assert_eq!(
        fixture.metadata.get("receipt_contract"),
        Some(&JsonValue::String("runx.receipt.v1".to_owned()))
    );

    let inputs = JsonValue::Object(fixture.inputs);
    let bridge = object_field_value(&inputs, "bridge")?;
    let accepted_command = array_field(bridge, "accepted_command")?;
    assert_eq!(
        accepted_command.first(),
        Some(&JsonValue::String("skill".to_owned()))
    );
    assert!(
        accepted_command
            .iter()
            .any(|value| value == &JsonValue::String("<runx-root>/skills/issue-triage".to_owned()))
    );
    assert!(
        !accepted_command
            .iter()
            .any(|value| value == &JsonValue::String("--runner".to_owned()))
    );

    Ok(())
}

fn run_case() -> Result<HarnessReplayOutput, runx_runtime::HarnessReplayError> {
    run_harness_fixture_with_adapter(
        case_path(),
        NoopAdapter,
        RuntimeOptions {
            created_at: FIXTURE_CREATED_AT.to_owned(),
            ..RuntimeOptions::local_development(std::env::vars().collect())
        },
    )
}

fn skill_payload(output: &HarnessReplayOutput) -> Result<JsonValue, Box<dyn std::error::Error>> {
    let skill_output = output
        .skill_output
        .as_ref()
        .ok_or("agent-task fixture did not produce skill output")?;
    Ok(skill_output.value.clone())
}

fn object_field_value<'a>(
    value: &'a JsonValue,
    field: &str,
) -> Result<&'a JsonObject, Box<dyn std::error::Error>> {
    let JsonValue::Object(object) = value else {
        return Err("value is not an object".into());
    };
    object_field(object, field)
}

fn object_field<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a JsonObject, Box<dyn std::error::Error>> {
    match object.get(field) {
        Some(JsonValue::Object(value)) => Ok(value),
        Some(_) => Err(format!("{field} is not an object").into()),
        None => Err(format!("{field} is missing").into()),
    }
}

fn array_field<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a Vec<JsonValue>, Box<dyn std::error::Error>> {
    match object.get(field) {
        Some(JsonValue::Array(value)) => Ok(value),
        Some(_) => Err(format!("{field} is not an array").into()),
        None => Err(format!("{field} is missing").into()),
    }
}

fn string_field_value<'a>(
    value: &'a JsonValue,
    field: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    let JsonValue::Object(object) = value else {
        return Err("value is not an object".into());
    };
    string_field(object, field)
}

fn string_field<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    match object.get(field) {
        Some(JsonValue::String(value)) => Ok(value),
        Some(_) => Err(format!("{field} is not a string").into()),
        None => Err(format!("{field} is missing").into()),
    }
}

fn assert_no_retired_bridge_fields(
    value: &JsonValue,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match value {
        JsonValue::Object(object) => {
            for (key, child) in object {
                if is_retired_bridge_field(key) {
                    return Err(format!("retired bridge field {path}.{key}").into());
                }
                assert_no_retired_bridge_fields(child, &format!("{path}.{key}"))?;
            }
        }
        JsonValue::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                assert_no_retired_bridge_fields(child, &format!("{path}.{index}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_retired_bridge_field(field: &str) -> bool {
    matches!(
        field,
        "runId"
            | "receiptId"
            | "outcome"
            | "effect"
            | "issue_to_pr_outcome"
            | RETIRED_VERIFICATION_REPORT_FIELD
            | "verificationReport"
            | "target_effect"
            | "targetEffect"
    )
}

fn case_path() -> PathBuf {
    repo_root().join("fixtures/external/aster/agent-task/rust-bridge-sealed-skill.yaml")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct NoopAdapter;

impl SkillAdapter for NoopAdapter {
    fn adapter_type(&self) -> &'static str {
        "noop"
    }

    fn invoke(
        &self,
        _request: SkillInvocation,
    ) -> Result<InvocationOutput, runx_runtime::RuntimeError> {
        Ok(InvocationOutput::runtime_success(
            JsonValue::Null,
            0,
            Default::default(),
        ))
    }
}
