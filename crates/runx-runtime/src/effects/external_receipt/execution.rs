use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use runx_contracts::{ExternalReceiptVerification, JsonObject, JsonValue, sha256_prefixed};

use super::EXTERNAL_RECEIPT_VERIFY_TOOL;
use super::contract::{ExternalReceiptVerifyInput, ExternalReceiptVerifyOutput};
use crate::effects::EffectToolRequest;
use crate::process::{ProcessSpec, run_process};
use crate::process_invocation::process_base_environment;
use crate::{CapabilityOutput, RuntimeError};

const SCAFLD_BIN_ENV: &str = "RUNX_SCAFLD_BIN";
const OUTPUT_LIMIT_BYTES: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn verify_external_receipt(
    request: EffectToolRequest<'_>,
) -> Result<JsonValue, RuntimeError> {
    if request.admission.is_none() {
        return Err(invalid("external receipt verification was not admitted"));
    }
    let input = JsonValue::Object(request.inputs.clone())
        .deserialize_into::<ExternalReceiptVerifyInput>()
        .map_err(|error| invalid(format!("invalid input: {error}")))?;
    validate_target(&input.target)?;
    validate_digest(&input.contract_digest)?;
    let repo_root = resolve_repo_root(&input.repo_root, request)?;
    let receipt_path = resolve_receipt_path(&repo_root, &input.receipt_path)?;
    let receipt_bytes =
        fs::read(&receipt_path).map_err(|error| invalid(format!("reading receipt: {error}")))?;
    let receipt_digest = sha256_prefixed(&receipt_bytes);
    let receipt: JsonValue = serde_json::from_slice(&receipt_bytes)
        .map_err(|error| invalid(format!("parsing receipt: {error}")))?;
    let binding = validate_receipt_binding(&receipt, &input)?;

    let verifier = request
        .env
        .get(SCAFLD_BIN_ENV)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("scafld");
    let environment = process_base_environment(request.env)?;
    let outcome = run_process(
        ProcessSpec::new(
            "external scafld receipt verification",
            verifier,
            OUTPUT_LIMIT_BYTES,
        )
        .args(vec![
            "verify".to_owned(),
            receipt_path.to_string_lossy().into_owned(),
            "--target".to_owned(),
            input.target.clone(),
            "--root".to_owned(),
            repo_root.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .cwd(&repo_root)
        .env(environment)
        .timeout(Some(TIMEOUT)),
    )
    .map_err(|error| invalid(format!("running canonical verifier: {error}")))?;
    if outcome.timed_out || outcome.stdout.truncated || outcome.stderr.truncated {
        return Err(invalid("canonical verifier exceeded runtime bounds"));
    }
    if !outcome.status.success() {
        return Err(invalid("canonical verifier rejected the receipt"));
    }
    let verifier_output: JsonValue = serde_json::from_slice(&outcome.stdout.bytes)
        .map_err(|error| invalid(format!("canonical verifier returned invalid JSON: {error}")))?;
    let verifier_ok = verifier_output
        .as_object()
        .and_then(|value| value.get("ok"))
        .and_then(JsonValue::as_bool)
        == Some(true);
    if !verifier_ok {
        return Err(invalid("canonical verifier did not report success"));
    }

    encode_output(ExternalReceiptVerifyOutput {
        external_receipt_verification: ExternalReceiptVerification {
            schema: "runx.external_receipt.verification.v1".to_owned(),
            verifier: "scafld".to_owned(),
            verified: true,
            task_id: binding.task_id,
            verdict: binding.verdict,
            target: input.target,
            contract_digest: input.contract_digest,
            receipt_ref: format!("runx:external_receipt:{receipt_digest}"),
            receipt_digest,
            verified_at: request.observed_at.to_owned(),
        },
    })
}

struct ReceiptBinding {
    task_id: String,
    verdict: String,
}

fn validate_receipt_binding(
    receipt: &JsonValue,
    input: &ExternalReceiptVerifyInput,
) -> Result<ReceiptBinding, RuntimeError> {
    let body = receipt
        .as_object()
        .and_then(|value| value.get("body"))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| invalid("receipt omits its signed body"))?;
    let task_id = required_body_string(body, "task_id")?;
    let verdict = required_body_string(body, "verdict")?;
    if verdict != "pass" {
        return Err(invalid("receipt verdict is not pass"));
    }
    if required_body_string(body, "head_commit")? != input.target {
        return Err(invalid("receipt head_commit does not match target"));
    }
    let expected_contract = input.contract_digest.trim_start_matches("sha256:");
    if required_body_string(body, "spec_fingerprint")? != expected_contract {
        return Err(invalid(
            "receipt spec_fingerprint does not match contract_digest",
        ));
    }
    let blockers = body
        .get("open_blockers")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| invalid("receipt omits open_blockers"))?;
    if !blockers.is_empty() {
        return Err(invalid("receipt has open blockers"));
    }
    Ok(ReceiptBinding { task_id, verdict })
}

fn required_body_string(body: &JsonObject, field: &str) -> Result<String, RuntimeError> {
    body.get(field)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("receipt body omits {field}")))
}

fn resolve_repo_root(
    requested: &str,
    request: EffectToolRequest<'_>,
) -> Result<PathBuf, RuntimeError> {
    crate::services::resolve_scoped_root(
        requested,
        "workspace",
        request.env,
        request.skill_directory,
    )
    .map_err(|error| invalid(error.to_string()))
}

fn resolve_receipt_path(repo_root: &Path, requested: &str) -> Result<PathBuf, RuntimeError> {
    let requested = PathBuf::from(requested);
    let joined = if requested.is_absolute() {
        requested
    } else {
        repo_root.join(requested)
    };
    let canonical = joined
        .canonicalize()
        .map_err(|error| invalid(format!("resolving receipt path: {error}")))?;
    if !canonical.starts_with(repo_root) || !canonical.is_file() {
        return Err(invalid(
            "receipt_path must identify a file within repo_root",
        ));
    }
    Ok(canonical)
}

fn validate_target(value: &str) -> Result<(), RuntimeError> {
    if value.is_empty()
        || value.starts_with('-')
        || value.len() > 1024
        || value.contains('\0')
        || value.chars().any(char::is_whitespace)
    {
        return Err(invalid("target must be a bounded Git ref or commit id"));
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), RuntimeError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(invalid("contract_digest must be a sha256 digest"));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("contract_digest must be a sha256 digest"));
    }
    Ok(())
}

fn encode_output<T: CapabilityOutput>(output: T) -> Result<JsonValue, RuntimeError> {
    serde_json::to_value(output)
        .and_then(serde_json::from_value)
        .map_err(|error| RuntimeError::json("serializing external receipt verification", error))
}

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::SkillFailed {
        skill_name: EXTERNAL_RECEIPT_VERIFY_TOOL.to_owned(),
        message: message.into(),
    }
}
