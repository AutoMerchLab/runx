use std::collections::BTreeSet;

use runx_contracts::{JsonObject, JsonValue};
use runx_core::policy::{CwdPolicy, parse_rfc3339_moment};

mod assessment;
mod capability;
mod output;
mod render;

pub(super) use capability::CAPABILITIES;
use capability::SandboxPlanInput;
use output::SandboxPlanOutput;

use assessment::{
    assess_declaration, build_declaration, declaration_json, inspect_identity,
    inspect_source_evidence,
};
use render::render_profile;

use super::capability::decode_typed_output;
use super::{NativeInvocation, invalid_input};
use crate::RuntimeError;

const TOOL: &str = "sandbox.plan";
const MAX_ITEMS: usize = 100;

fn prepare(
    invocation: &NativeInvocation<'_, SandboxPlanInput>,
) -> Result<SandboxPlanOutput, RuntimeError> {
    decode_typed_output(TOOL, build(invocation.inputs)?)
}

fn build(inputs: &SandboxPlanInput) -> Result<JsonValue, RuntimeError> {
    let workload = &inputs.workload;
    let requirements = optional_object(workload, "requirements")?;
    let baseline = match &inputs.baseline {
        Some(baseline) => baseline,
        None => empty_object(),
    };
    let mut findings = Vec::new();

    let identity = inspect_identity(workload, &mut findings);
    let evidence = inspect_source_evidence(
        &inputs.as_of,
        inputs.max_age_days,
        requirements,
        &mut findings,
    )?;
    let (declaration, unsupported) = build_declaration(requirements, baseline, &mut findings)?;
    let assessment = assess_declaration(&declaration, &unsupported, &mut findings);
    let declaration = declaration_json(&declaration);

    Ok(render_profile(
        identity,
        evidence,
        declaration,
        assessment,
        findings,
    ))
}

fn apply_baseline(
    baseline: &JsonObject,
    network: bool,
    writable_paths: &[String],
    env_allowlist: Option<&[String]>,
    require_enforcement: bool,
    findings: &mut Vec<JsonValue>,
) -> Result<(), RuntimeError> {
    if baseline.get("network_allowed").and_then(JsonValue::as_bool) == Some(false) && network {
        finding(
            findings,
            "sandbox.baseline.network",
            "baseline forbids network access",
        );
    }
    if baseline
        .get("require_enforcement")
        .and_then(JsonValue::as_bool)
        == Some(true)
        && !require_enforcement
    {
        finding(
            findings,
            "sandbox.baseline.enforcement",
            "baseline requires sandbox enforcement",
        );
    }
    if let Some(allowed) = optional_strings(baseline, "allowed_writable_paths")? {
        subset(
            writable_paths,
            &allowed,
            findings,
            "sandbox.baseline.writable_path",
        );
    }
    if let Some(allowed) = optional_strings(baseline, "allowed_env")? {
        subset(
            env_allowlist.unwrap_or(&[]),
            &allowed,
            findings,
            "sandbox.baseline.env",
        );
    }
    Ok(())
}

fn subset(values: &[String], allowed: &[String], findings: &mut Vec<JsonValue>, code: &str) {
    let allowed = allowed.iter().collect::<BTreeSet<_>>();
    for value in values {
        if !allowed.contains(value) {
            finding(findings, code, format!("baseline does not admit {value}"));
        }
    }
}

fn parse_cwd_policy(value: Option<&str>, findings: &mut Vec<JsonValue>) -> Option<CwdPolicy> {
    match value.unwrap_or("skill-directory") {
        "skill-directory" => Some(CwdPolicy::SkillDirectory),
        "workspace" => Some(CwdPolicy::Workspace),
        "custom" => Some(CwdPolicy::Custom),
        other => {
            finding(
                findings,
                "sandbox.cwd_policy.invalid",
                format!("unsupported cwd_policy {other}"),
            );
            None
        }
    }
}

fn optional_object<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a JsonObject, RuntimeError> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(empty_object()),
        Some(value) => value
            .as_object()
            .ok_or_else(|| invalid_input(TOOL, format!("{field} must be an object"))),
    }
}

fn empty_object() -> &'static JsonObject {
    static EMPTY: std::sync::OnceLock<JsonObject> = std::sync::OnceLock::new();
    EMPTY.get_or_init(JsonObject::new)
}

fn optional_text<'a>(object: &'a JsonObject, field: &str) -> Option<&'a str> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn boolean(object: &JsonObject, field: &str, default: bool) -> Result<bool, RuntimeError> {
    match object.get(field) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid_input(TOOL, format!("{field} must be boolean"))),
    }
}

fn strings(object: &JsonObject, field: &str) -> Result<Vec<String>, RuntimeError> {
    optional_strings(object, field).map(Option::unwrap_or_default)
}

fn optional_strings(object: &JsonObject, field: &str) -> Result<Option<Vec<String>>, RuntimeError> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| invalid_input(TOOL, format!("{field} must be an array")))?;
    if values.len() > MAX_ITEMS {
        return Err(invalid_input(
            TOOL,
            format!("{field} exceeds {MAX_ITEMS} items"),
        ));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        let value = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                invalid_input(TOOL, format!("{field} must contain non-empty strings"))
            })?;
        unique.insert(value.to_owned());
    }
    Ok(Some(unique.into_iter().collect()))
}

fn required_time(object: &JsonObject, field: &str, findings: &mut Vec<JsonValue>) -> Option<i64> {
    let Some(value) = optional_text(object, field) else {
        finding(
            findings,
            "sandbox.time.missing",
            format!("{field} is required"),
        );
        return None;
    };
    match parse_rfc3339_moment(value) {
        Some((days, seconds, _)) => days.checked_mul(86_400)?.checked_add(seconds),
        None => {
            finding(
                findings,
                "sandbox.time.invalid",
                format!("{field} must be RFC3339"),
            );
            None
        }
    }
}

fn finding(findings: &mut Vec<JsonValue>, code: &str, message: impl Into<String>) {
    findings.push(JsonValue::Object(JsonObject::from([
        ("code".to_owned(), JsonValue::String(code.to_owned())),
        ("message".to_owned(), JsonValue::String(message.into())),
    ])));
}

fn non_empty(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
        JsonValue::String(value) => !value.trim().is_empty(),
        JsonValue::Bool(value) => *value,
        JsonValue::Number(_) => true,
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests;
