use std::collections::BTreeMap;

use runx_contracts::{JsonObject, JsonValue};
use serde::{Deserialize, Serialize};

use crate::ValidationError;

use super::FIELDS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessCallerFixture {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approvals: Option<BTreeMap<String, bool>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptExpectation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessExpectation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ReceiptExpectation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunnerHarnessCase {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    pub inputs: JsonObject,
    pub env: BTreeMap<String, String>,
    pub caller: HarnessCallerFixture,
    pub expect: HarnessExpectation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunnerHarnessManifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    pub cases: Vec<RunnerHarnessCase>,
}

pub(crate) fn validate_harness_manifest(
    value: Option<JsonObject>,
    field: &str,
) -> Result<Option<RunnerHarnessManifest>, ValidationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let files = FIELDS
        .optional_string_array(value.get("files"), &format!("{field}.files"))?
        .unwrap_or_default();
    let cases = FIELDS
        .required_plain_array(value.get("cases"), &format!("{field}.cases"))?
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            validate_harness_case(
                FIELDS.required_object(Some(entry), &format!("{field}.cases[{index}]"))?,
                &format!("{field}.cases[{index}]"),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(RunnerHarnessManifest { files, cases }))
}

fn validate_harness_case(
    value: &JsonObject,
    field: &str,
) -> Result<RunnerHarnessCase, ValidationError> {
    Ok(RunnerHarnessCase {
        name: FIELDS.required_string(value.get("name"), &format!("{field}.name"))?,
        runner: FIELDS
            .optional_non_empty_string(value.get("runner"), &format!("{field}.runner"))?,
        inputs: FIELDS
            .optional_object(value.get("inputs"), &format!("{field}.inputs"))?
            .unwrap_or_default(),
        env: validate_string_object(
            FIELDS
                .optional_object(value.get("env"), &format!("{field}.env"))?
                .unwrap_or_default(),
            &format!("{field}.env"),
        )?,
        caller: validate_harness_caller(
            FIELDS
                .optional_object(value.get("caller"), &format!("{field}.caller"))?
                .unwrap_or_default(),
            &format!("{field}.caller"),
        )?,
        expect: validate_harness_expectation(
            FIELDS.required_object(value.get("expect"), &format!("{field}.expect"))?,
            &format!("{field}.expect"),
        )?,
    })
}

fn validate_string_object(
    value: JsonObject,
    field: &str,
) -> Result<BTreeMap<String, String>, ValidationError> {
    value
        .into_iter()
        .map(|(key, value)| match value {
            JsonValue::String(value) => Ok((key, value)),
            _ => Err(FIELDS.validation_error(format!("{field}.{key} must be a string."))),
        })
        .collect()
}

fn validate_harness_caller(
    value: JsonObject,
    field: &str,
) -> Result<HarnessCallerFixture, ValidationError> {
    Ok(HarnessCallerFixture {
        answers: FIELDS.optional_object(value.get("answers"), &format!("{field}.answers"))?,
        approvals: Some(validate_bool_object(
            FIELDS
                .optional_object(value.get("approvals"), &format!("{field}.approvals"))?
                .unwrap_or_default(),
            &format!("{field}.approvals"),
        )?),
    })
}

fn validate_bool_object(
    value: JsonObject,
    field: &str,
) -> Result<BTreeMap<String, bool>, ValidationError> {
    value
        .into_iter()
        .map(|(key, value)| match value {
            JsonValue::Bool(value) => Ok((key, value)),
            _ => Err(FIELDS.validation_error(format!("{field}.{key} must be a boolean."))),
        })
        .collect()
}

fn validate_harness_expectation(
    value: &JsonObject,
    field: &str,
) -> Result<HarnessExpectation, ValidationError> {
    Ok(HarnessExpectation {
        status: optional_harness_status(value.get("status"), &format!("{field}.status"))?,
        receipt: validate_receipt_expectation(
            FIELDS.optional_object(value.get("receipt"), &format!("{field}.receipt"))?,
            &format!("{field}.receipt"),
        )?,
        steps: FIELDS.optional_string_array(value.get("steps"), &format!("{field}.steps"))?,
    })
}

fn validate_receipt_expectation(
    value: Option<JsonObject>,
    field: &str,
) -> Result<Option<ReceiptExpectation>, ValidationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    Ok(Some(ReceiptExpectation {
        kind: optional_receipt_kind(value.get("kind"), &format!("{field}.kind"))?,
        status: optional_receipt_status(value.get("status"), &format!("{field}.status"))?,
        skill_name: FIELDS
            .optional_string(value.get("skill_name"), &format!("{field}.skill_name"))?,
        source_type: FIELDS
            .optional_string(value.get("source_type"), &format!("{field}.source_type"))?,
        graph_name: FIELDS
            .optional_string(value.get("graph_name"), &format!("{field}.graph_name"))?,
        owner: FIELDS.optional_string(value.get("owner"), &format!("{field}.owner"))?,
    }))
}

fn optional_harness_status(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<String>, ValidationError> {
    validate_enum(
        value,
        field,
        &[
            "sealed",
            "failure",
            "needs_agent",
            "policy_denied",
            "escalated",
        ],
    )
}

fn optional_receipt_status(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<String>, ValidationError> {
    validate_enum(value, field, &["sealed", "failure"])
}

fn optional_receipt_kind(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<String>, ValidationError> {
    validate_enum(value, field, &["receipt"])
}

fn validate_enum(
    value: Option<&JsonValue>,
    field: &str,
    allowed: &[&str],
) -> Result<Option<String>, ValidationError> {
    let Some(value) = FIELDS.optional_string(value, field)? else {
        return Ok(None);
    };
    if allowed.iter().any(|allowed| *allowed == value) {
        return Ok(Some(value));
    }
    Err(FIELDS.validation_error(format!("{field} must be {}.", allowed.join(", "))))
}
