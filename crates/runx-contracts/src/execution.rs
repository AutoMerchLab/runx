//! Shared execution declarations and governed outcome contracts.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::RunxSchema;
use crate::{JsonNumber, JsonObject, JsonValue};

/// One declared input shared by skills, local tools, inspection, and runtime
/// materialization. Keeping this at the contract boundary prevents each
/// parser or catalog surface from inventing its own type/default semantics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct InputDefinition {
    #[serde(rename = "type")]
    pub input_type: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<bool>,
}

impl InputDefinition {
    /// Whether a concrete JSON value satisfies this parser-validated input
    /// type. Manifest validation and runtime materialization use this same
    /// predicate.
    #[must_use]
    pub fn accepts_value(&self, value: &JsonValue) -> bool {
        match self.input_type.as_str() {
            "json" => true,
            "string" => matches!(value, JsonValue::String(_)),
            "number" => matches!(value, JsonValue::Number(_)),
            "integer" => matches!(
                value,
                JsonValue::Number(JsonNumber::I64(_) | JsonNumber::U64(_))
            ),
            "boolean" => matches!(value, JsonValue::Bool(_)),
            "object" => matches!(value, JsonValue::Object(_)),
            "array" => matches!(value, JsonValue::Array(_)),
            _ => false,
        }
    }
}

/// How a successful execution result is exposed to downstream graph context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactContract {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emits: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub named_emits: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packets: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_as: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    pub max_attempts: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct IdempotencyPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedDisposition {
    Completed,
    NeedsAgent,
    PolicyDenied,
    ApprovalRequired,
    Observing,
    Escalated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeState {
    Pending,
    Complete,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonObject>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptSurfaceRef {
    #[serde(rename = "type")]
    pub surface_type: String,
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputContextCapture {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<JsonValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSemantics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<GovernedDisposition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_state: Option<OutcomeState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ReceiptOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_context: Option<InputContextCapture>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_refs: Option<Vec<ReceiptSurfaceRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_refs: Option<Vec<ReceiptSurfaceRef>>,
}
