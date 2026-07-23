//! Deterministic quote and reservation construction for the native payment
//! tools. Model judgment may select a payment intent; it never authors the
//! authority term, subset proof, idempotency binding, or capability reference.

use runx_contracts::schema::IsoDateTime;
use runx_contracts::{
    AuthorityCapability, AuthorityEffectCredentialForm, AuthorityEffectLimit,
    AuthorityResourceFamily, AuthorityTerm, AuthorityVerb, Decision, DecisionChoice,
    DecisionInputs, DecisionJustification, Intent, JsonNumber, JsonObject, JsonValue, Reference,
    ReferenceType, sha256_hex,
};
use runx_core::policy::{AttenuationRequest, mint_attenuated};
use runx_runtime::{EffectToolRequest, RuntimeError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::authority::PaymentBoundsComparator;
use crate::refunds::{
    RefundAdmissionDecision, RefundAdmissionInput, RefundRefusalCode, RefundRequest,
    RefundableCharge, admit_refund,
};

mod capabilities;
mod charge;
mod invoice;
mod refund;
mod refund_proof;
mod spend;

use capabilities::{
    CHARGE_CHALLENGE_TOOL, CHARGE_PLAN_TOOL, CHARGE_PRICE_TOOL, CHARGE_VERIFICATION_REQUEST_TOOL,
    INVOICE_PLAN_TOOL, QUOTE_TOOL, REFUND_PLAN_TOOL, RESERVE_TOOL,
};
use charge::{charge_challenge, charge_plan, charge_price, charge_verification_request};
use invoice::invoice_plan;
use refund::refund_plan;
use spend::{payment_limit, quote, reserve};

const PAYMENT_FAMILY: &str = "payment";

pub(crate) fn capabilities() -> &'static [&'static dyn runx_runtime::CapabilityContract] {
    capabilities::CAPABILITIES
}

#[derive(Debug, Error)]
enum PaymentPlanningError {
    #[error("{0}")]
    Invalid(String),
    #[error("payment authority attenuation failed: {0}")]
    Attenuation(String),
    #[error("payment planning serialization failed: {0}")]
    Serialization(String),
}

pub(crate) fn invoke(request: EffectToolRequest<'_>) -> Option<Result<JsonValue, RuntimeError>> {
    let result = match request.tool_ref {
        QUOTE_TOOL => quote(request),
        RESERVE_TOOL => reserve(request),
        CHARGE_PRICE_TOOL => charge_price(request),
        CHARGE_CHALLENGE_TOOL => charge_challenge(request),
        CHARGE_VERIFICATION_REQUEST_TOOL => charge_verification_request(request),
        CHARGE_PLAN_TOOL => charge_plan(request),
        REFUND_PLAN_TOOL => refund_plan(request),
        INVOICE_PLAN_TOOL => invoice_plan(request),
        _ => return None,
    };
    Some(result.map_err(|error| RuntimeError::SkillFailed {
        skill_name: request.tool_ref.to_owned(),
        message: error.to_string(),
    }))
}

fn is_opaque_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.chars().any(char::is_whitespace)
        && !value.to_ascii_lowercase().starts_with("sk-")
        && !value.to_ascii_lowercase().starts_with("bearer:")
        && !value.to_ascii_lowercase().contains("-----begin")
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn finding(code: impl Into<String>, message: impl Into<String>) -> JsonValue {
    JsonValue::Object(JsonObject::from([
        ("code".to_owned(), JsonValue::String(code.into())),
        ("message".to_owned(), JsonValue::String(message.into())),
    ]))
}

fn packet_findings(packet: &JsonObject) -> Vec<JsonValue> {
    packet
        .get("open_questions")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
}

fn admit_opaque(
    value: Option<&JsonValue>,
    field: &str,
    max_len: usize,
    required: bool,
    findings: &mut Vec<JsonValue>,
) -> Option<String> {
    let Some(raw) = value else {
        if required {
            findings.push(finding(
                format!("{field}.missing"),
                format!("{field} is required"),
            ));
        }
        return None;
    };
    let Some(value) = raw
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        findings.push(finding(
            format!("{field}.invalid"),
            format!("{field} must be a non-empty string"),
        ));
        return None;
    };
    let lower = value.to_ascii_lowercase();
    if value.len() > max_len
        || value.chars().any(char::is_whitespace)
        || lower.starts_with("sk-")
        || lower.starts_with("bearer:")
        || lower.contains("-----begin")
    {
        findings.push(finding(
            format!("{field}.unsafe"),
            format!("{field} must be an opaque non-secret reference"),
        ));
        return None;
    }
    Some(value.to_owned())
}

fn admit_opaque_array(
    value: Option<&JsonValue>,
    field: &str,
    max_items: usize,
    max_len: usize,
    findings: &mut Vec<JsonValue>,
) -> Vec<String> {
    let Some(values) = value.and_then(JsonValue::as_array) else {
        findings.push(finding(
            format!("{field}.invalid"),
            format!("{field} must be an array"),
        ));
        return Vec::new();
    };
    if values.len() > max_items {
        findings.push(finding(
            format!("{field}.limit"),
            format!("{field} exceeds {max_items} entries"),
        ));
        return Vec::new();
    }
    let mut admitted = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if let Some(value) = admit_opaque(
            Some(value),
            &format!("{field}[{index}]"),
            max_len,
            true,
            findings,
        ) && !admitted.contains(&value)
        {
            admitted.push(value);
        }
    }
    admitted
}

fn looks_like_iso_datetime(value: &str) -> bool {
    let Some(value) = value.strip_suffix('Z') else {
        return false;
    };
    let mut parts = value.split('.');
    let Some(base) = parts.next() else {
        return false;
    };
    let fraction = parts.next();
    if parts.next().is_some()
        || fraction.is_some_and(|fraction| {
            fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return false;
    }
    base.len() == 19
        && base.as_bytes()[4] == b'-'
        && base.as_bytes()[7] == b'-'
        && base.as_bytes()[10] == b'T'
        && base.as_bytes()[13] == b':'
        && base.as_bytes()[16] == b':'
        && base
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit())
}

fn required_object<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a JsonObject, PaymentPlanningError> {
    object
        .get(field)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| invalid(format!("{field} must be an object")))
}

fn required_array<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a [JsonValue], PaymentPlanningError> {
    object
        .get(field)
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| invalid(format!("{field} must be an array")))
}

fn required_string<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a str, PaymentPlanningError> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("{field} must be a non-empty string")))
}

fn required_u64(object: &JsonObject, field: &str) -> Result<u64, PaymentPlanningError> {
    optional_u64(object, field)?
        .ok_or_else(|| invalid(format!("{field} must be a non-negative integer")))
}

fn optional_u64(object: &JsonObject, field: &str) -> Result<Option<u64>, PaymentPlanningError> {
    match object.get(field) {
        None => Ok(None),
        Some(JsonValue::Number(JsonNumber::U64(value))) => Ok(Some(*value)),
        Some(JsonValue::Number(JsonNumber::I64(value))) if *value >= 0 => {
            Ok(u64::try_from(*value).ok())
        }
        _ => Err(invalid(format!("{field} must be a non-negative integer"))),
    }
}

fn single_string<'a>(
    values: &'a [JsonValue],
    field: &str,
) -> Result<&'a str, PaymentPlanningError> {
    if values.len() != 1 {
        return Err(invalid(format!("{field} must contain exactly one rail")));
    }
    values[0]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("{field}[0] must be a non-empty string")))
}

fn string_array<'a>(
    value: &'a JsonValue,
    field: &str,
) -> Result<Vec<&'a str>, PaymentPlanningError> {
    let values = value
        .as_array()
        .ok_or_else(|| invalid(format!("{field} must be an array")))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid(format!("{field}[{index}] must be a string")))
        })
        .collect()
}

fn required_typed<T: DeserializeOwned>(
    object: &JsonObject,
    field: &str,
) -> Result<T, PaymentPlanningError> {
    let value = object
        .get(field)
        .ok_or_else(|| invalid(format!("{field} is required")))?;
    serde_json::from_value(
        serde_json::to_value(value)
            .map_err(|error| PaymentPlanningError::Serialization(error.to_string()))?,
    )
    .map_err(|error| invalid(format!("{field} is invalid: {error}")))
}

fn typed_value(value: &impl Serialize) -> Result<JsonValue, PaymentPlanningError> {
    serde_json::from_value(
        serde_json::to_value(value)
            .map_err(|error| PaymentPlanningError::Serialization(error.to_string()))?,
    )
    .map_err(|error| PaymentPlanningError::Serialization(error.to_string()))
}

fn object_value(value: impl Serialize) -> Result<JsonValue, PaymentPlanningError> {
    let value = typed_value(&value)?;
    match value {
        JsonValue::Object(_) => Ok(value),
        _ => Err(PaymentPlanningError::Serialization(
            "expected object value".to_owned(),
        )),
    }
}

fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>, PaymentPlanningError> {
    serde_json::to_vec(value)
        .map_err(|error| PaymentPlanningError::Serialization(error.to_string()))
}

fn is_wildcard(value: &str) -> bool {
    matches!(value.trim(), "" | "*" | "any")
}

fn invalid(message: impl Into<String>) -> PaymentPlanningError {
    PaymentPlanningError::Invalid(message.into())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::wildcard_imports)]
mod tests {
    use super::*;
    use runx_contracts::AuthorityBounds;

    #[test]
    fn payment_capability_registry_uses_the_runtime_typed_plane() {
        let capabilities = super::capabilities();
        assert_eq!(capabilities.len(), 8);
        for capability in capabilities {
            assert!(capability.input_schema().is_ok());
            assert!(capability.catalog_inputs().is_ok());
            assert!(capability.output_schema().is_object());
        }
        let registry = runx_runtime::RuntimeEffectRegistry::with_effect(
            crate::PaymentRuntimeEffect::new(crate::DeterministicPaymentFinalitySupervisor),
        );
        assert!(
            registry.is_ok(),
            "payment capabilities must register: {registry:?}"
        );
    }

    #[test]
    fn native_quote_and_reserve_are_derived_from_parent_authority() {
        let parent = parent_authority();
        let inputs = JsonObject::from([
            (
                "payment_signal".to_owned(),
                object_value(serde_json::json!({
                    "challenge_id": "ch_1",
                    "amount_minor": 125,
                    "currency": "USD",
                    "rail": "mock",
                    "counterparty": "merchant:demo",
                    "operation": "search.paid",
                    "realm": "test",
                }))
                .expect("signal"),
            ),
            (
                "parent_payment_authority".to_owned(),
                typed_value(&parent).expect("parent"),
            ),
            (
                "idempotency_seed".to_owned(),
                JsonValue::String("demo-search-001".to_owned()),
            ),
        ]);
        let quote = quote(tool_request(QUOTE_TOOL, &inputs)).expect("quote");
        let quote = quote.as_object().expect("quote object");
        let reserve_inputs = JsonObject::from([
            (
                "payment_quote_packet".to_owned(),
                JsonValue::Object(quote.clone()),
            ),
            (
                "parent_payment_authority".to_owned(),
                typed_value(&parent).expect("parent"),
            ),
            (
                "idempotency_seed".to_owned(),
                JsonValue::String("demo-search-001".to_owned()),
            ),
            (
                "target_harness_ref".to_owned(),
                typed_value(&Reference::with_uri(
                    ReferenceType::Harness,
                    "runx:harness:spend",
                ))
                .expect("harness"),
            ),
            (
                "target_act_id".to_owned(),
                JsonValue::String("act_fulfill".to_owned()),
            ),
        ]);
        let reserved = reserve(tool_request(RESERVE_TOOL, &reserve_inputs)).expect("reserve");
        let reserved = reserved.as_object().expect("reservation object");
        let authority = reserved
            .get("reserved_payment_authority")
            .and_then(JsonValue::as_object)
            .expect("reserved authority");
        let child: AuthorityTerm = required_typed(authority, "child_authority").expect("child");
        assert!(crate::is_payment_authority_subset(&child, &parent));
    }

    #[test]
    fn native_quote_rejects_parent_reference_without_real_authority() {
        let inputs = JsonObject::from([
            (
                "payment_signal".to_owned(),
                object_value(serde_json::json!({
                    "amount_minor": 125,
                    "currency": "USD",
                    "rail": "mock",
                    "counterparty": "merchant:demo",
                    "operation": "search.paid",
                }))
                .expect("signal"),
            ),
            (
                "parent_payment_authority".to_owned(),
                object_value(serde_json::json!({"authority_ref": "authority:payment:test"}))
                    .expect("reference"),
            ),
            (
                "idempotency_seed".to_owned(),
                JsonValue::String("demo-search-001".to_owned()),
            ),
        ]);
        let error = quote(tool_request(QUOTE_TOOL, &inputs)).expect_err("must reject");
        assert!(
            error
                .to_string()
                .contains("parent_payment_authority is invalid")
        );
    }

    #[test]
    fn native_quote_enforces_the_callers_rail_allowlist() {
        let parent = parent_authority();
        let inputs = JsonObject::from([
            (
                "payment_signal".to_owned(),
                object_value(serde_json::json!({
                    "amount_minor": 125,
                    "currency": "USD",
                    "rail": "mock",
                    "counterparty": "merchant:demo",
                    "operation": "search.paid",
                    "realm": "test",
                }))
                .expect("signal"),
            ),
            (
                "parent_payment_authority".to_owned(),
                typed_value(&parent).expect("parent"),
            ),
            (
                "rail_preferences".to_owned(),
                JsonValue::Array(vec![JsonValue::String("x402".to_owned())]),
            ),
            (
                "idempotency_seed".to_owned(),
                JsonValue::String("wrong-rail-001".to_owned()),
            ),
        ]);

        let error = quote(tool_request(QUOTE_TOOL, &inputs)).expect_err("must reject rail drift");
        assert!(
            error
                .to_string()
                .contains("outside the caller rail preferences")
        );
    }

    #[test]
    fn native_charge_plan_is_deterministic_and_never_claims_forwarding() {
        let credential = object_value(serde_json::json!({
            "family": "mpp",
            "credential_ref": "credential:mpp:paid-search-001",
        }))
        .expect("credential");
        let plan = charge_plan_for("mpp", credential.clone());
        let replay = charge_plan_for("mpp", credential);
        assert_eq!(plan.get("plan_digest"), replay.get("plan_digest"));
        assert_eq!(
            plan.get("decision").and_then(JsonValue::as_str),
            Some("ready_for_provider_verification")
        );
        assert_eq!(
            plan.get("forwarding_status").and_then(JsonValue::as_str),
            Some("not_forwarded")
        );
        assert_eq!(
            plan.get("receipt_status").and_then(JsonValue::as_str),
            Some("not_sealed")
        );
    }

    #[test]
    fn native_charge_plan_blocks_raw_credentials() {
        let credential = object_value(serde_json::json!({
            "family": "stripe",
            "credential_ref": "credential:stripe:paid-search-002",
            "secret_key": "sk_test_private",
        }))
        .expect("credential");
        let plan = charge_plan_for("stripe", credential);
        assert_eq!(
            plan.get("decision").and_then(JsonValue::as_str),
            Some("blocked")
        );
        assert!(
            plan.get("findings")
                .and_then(JsonValue::as_array)
                .is_some_and(|findings| findings.iter().any(|finding| {
                    finding
                        .as_object()
                        .and_then(|finding| finding.get("code"))
                        .and_then(JsonValue::as_str)
                        == Some("credential.raw_fields")
                }))
        );
    }

    #[test]
    fn native_invoice_plan_emits_an_executable_spend_handoff() {
        let parent = invoice_parent_authority();
        let inputs = invoice_inputs(&parent, "mock");
        let output = invoice_plan(tool_request(INVOICE_PLAN_TOOL, &inputs)).expect("plan");
        let plan = output
            .as_object()
            .and_then(|output| output.get("settlement_plan"))
            .and_then(JsonValue::as_object)
            .expect("settlement plan");
        assert_eq!(
            plan.get("decision").and_then(JsonValue::as_str),
            Some("ready_for_spend")
        );
        let downstream = plan
            .get("downstream")
            .and_then(JsonValue::as_object)
            .expect("downstream");
        assert_eq!(
            downstream.get("runner").and_then(JsonValue::as_str),
            Some("mock")
        );
        assert!(
            downstream
                .get("inputs")
                .and_then(JsonValue::as_object)
                .is_some_and(|inputs| inputs.contains_key("parent_payment_authority"))
        );
    }

    #[test]
    fn native_invoice_plan_blocks_unsupported_rails_and_raw_payee_fields() {
        let mut parent = invoice_parent_authority();
        parent.bounds.effect_limits[0].channels = vec!["ach".into()];
        let mut inputs = invoice_inputs(&parent, "ach");
        inputs.insert(
            "payee".to_owned(),
            object_value(serde_json::json!({
                "name": "Acme Hosting",
                "party_ref": "merchant:demo",
                "account_number": "123456789",
            }))
            .expect("payee"),
        );
        let output = invoice_plan(tool_request(INVOICE_PLAN_TOOL, &inputs)).expect("plan");
        let plan = output
            .as_object()
            .and_then(|output| output.get("settlement_plan"))
            .and_then(JsonValue::as_object)
            .expect("settlement plan");
        assert_eq!(
            plan.get("decision").and_then(JsonValue::as_str),
            Some("blocked")
        );
        assert_eq!(plan.get("downstream"), Some(&JsonValue::Null));
        let findings = plan
            .get("findings")
            .and_then(JsonValue::as_array)
            .expect("findings");
        assert!(findings.iter().any(|finding| {
            finding
                .as_object()
                .and_then(|finding| finding.get("code"))
                .and_then(JsonValue::as_str)
                == Some("rail.unsupported")
        }));
        assert!(findings.iter().any(|finding| {
            finding
                .as_object()
                .and_then(|finding| finding.get("code"))
                .and_then(JsonValue::as_str)
                == Some("payee.raw_fields")
        }));
    }

    fn charge_plan_for(family: &str, credential: JsonValue) -> JsonObject {
        let price_inputs = JsonObject::from([
            (
                "mcp_tool_call".to_owned(),
                object_value(serde_json::json!({
                    "tool": "search.paid",
                    "arguments": { "query": "runx" },
                }))
                .expect("tool call"),
            ),
            (
                "provider_policy".to_owned(),
                object_value(serde_json::json!({
                    "policy_ref": "policy:provider-demo",
                    "price_minor": 125,
                    "currency": "USD",
                    "accepted_settlement_families": [family],
                    "counterparty": "provider:demo",
                    "realm": "test",
                }))
                .expect("provider policy"),
            ),
        ]);
        let price = charge_price(tool_request(CHARGE_PRICE_TOOL, &price_inputs))
            .expect("price")
            .as_object()
            .cloned()
            .expect("price packet");
        let challenge_inputs = JsonObject::from([
            (
                "charge_price_packet".to_owned(),
                JsonValue::Object(price.clone()),
            ),
            (
                "idempotency_seed".to_owned(),
                JsonValue::String("paid-search-001".to_owned()),
            ),
        ]);
        let challenge = charge_challenge(tool_request(CHARGE_CHALLENGE_TOOL, &challenge_inputs))
            .expect("challenge")
            .as_object()
            .cloned()
            .expect("challenge packet");
        let verification_inputs = JsonObject::from([
            (
                "charge_price_packet".to_owned(),
                JsonValue::Object(price.clone()),
            ),
            (
                "charge_challenge_packet".to_owned(),
                JsonValue::Object(challenge.clone()),
            ),
            ("returned_credential".to_owned(), credential),
            (
                "verify_capability_ref".to_owned(),
                JsonValue::String("capability:charge-verify:paid-search-001".to_owned()),
            ),
            (
                "settlement_family".to_owned(),
                JsonValue::String(family.to_owned()),
            ),
            (
                "idempotency".to_owned(),
                challenge.get("idempotency").cloned().expect("idempotency"),
            ),
        ]);
        let verification = charge_verification_request(tool_request(
            CHARGE_VERIFICATION_REQUEST_TOOL,
            &verification_inputs,
        ))
        .expect("verification")
        .as_object()
        .cloned()
        .expect("verification packet");
        let plan_inputs = JsonObject::from([
            ("charge_price_packet".to_owned(), JsonValue::Object(price)),
            (
                "charge_challenge_packet".to_owned(),
                JsonValue::Object(challenge),
            ),
            (
                "charge_verification_request".to_owned(),
                JsonValue::Object(verification),
            ),
        ]);
        charge_plan(tool_request(CHARGE_PLAN_TOOL, &plan_inputs))
            .expect("plan")
            .as_object()
            .and_then(|output| output.get("charge_plan"))
            .and_then(JsonValue::as_object)
            .cloned()
            .expect("charge plan")
    }

    fn invoice_inputs(parent: &AuthorityTerm, rail: &str) -> JsonObject {
        JsonObject::from([
            (
                "invoice_ref".to_owned(),
                JsonValue::String("INV-2026-0412".to_owned()),
            ),
            (
                "amount_minor".to_owned(),
                JsonValue::Number(JsonNumber::U64(125)),
            ),
            ("currency".to_owned(), JsonValue::String("USD".to_owned())),
            (
                "payee".to_owned(),
                object_value(serde_json::json!({
                    "name": "Acme Hosting",
                    "party_ref": "merchant:demo",
                    "settlement_ref": "payee-account:acme-hosting:test",
                }))
                .expect("payee"),
            ),
            ("rail".to_owned(), JsonValue::String(rail.to_owned())),
            (
                "rail_profile_ref".to_owned(),
                JsonValue::String(format!("rail-profile:{rail}:test")),
            ),
            (
                "parent_payment_authority".to_owned(),
                typed_value(parent).expect("parent"),
            ),
            (
                "idempotency_seed".to_owned(),
                JsonValue::String("invoice-0412".to_owned()),
            ),
            ("realm".to_owned(), JsonValue::String("test".to_owned())),
        ])
    }

    fn invoice_parent_authority() -> AuthorityTerm {
        let mut parent = parent_authority();
        parent.bounds.effect_limits[0].operation = Some("invoice.settle".into());
        parent
    }

    fn tool_request<'a>(tool_ref: &'a str, inputs: &'a JsonObject) -> EffectToolRequest<'a> {
        let env = Box::leak(Box::new(std::collections::BTreeMap::new()));
        let credentials = Box::leak(Box::new(runx_runtime::CredentialDelivery::none()));
        EffectToolRequest {
            tool_ref,
            observed_at: "2026-05-18T00:00:00Z",
            inputs,
            env,
            skill_directory: std::path::Path::new("."),
            credential_delivery: credentials,
            admission: None,
        }
    }

    fn parent_authority() -> AuthorityTerm {
        AuthorityTerm {
            term_id: "authority-term:payment:test".into(),
            principal_ref: Reference::with_uri(ReferenceType::Principal, "principal:operator:test"),
            resource_ref: Reference::with_uri(ReferenceType::Surface, "merchant:demo"),
            resource_family: AuthorityResourceFamily::Effect,
            verbs: vec![AuthorityVerb::Prepare, AuthorityVerb::Commit],
            bounds: AuthorityBounds {
                effect_limits: vec![AuthorityEffectLimit {
                    family: PAYMENT_FAMILY.into(),
                    unit: "USD".into(),
                    max_per_call_units: Some(1_000),
                    max_per_run_units: Some(5_000),
                    max_per_period_units: None,
                    period: None,
                    channels: vec!["mock".into()],
                    realm: Some("test".into()),
                    peer: Some("merchant:demo".into()),
                    operation: Some("search.paid".into()),
                    preflight_ttl_ms: None,
                    approval_threshold_units: None,
                    authorization_form: Some(AuthorityEffectCredentialForm::SingleUseCapability),
                    preflight_required: false,
                    commitment_required: false,
                    idempotency_required: true,
                    recovery_required: true,
                    receipt_before_success: true,
                    single_use_capability: true,
                }],
                ..AuthorityBounds::default()
            },
            conditions: Vec::new(),
            approvals: Vec::new(),
            capabilities: vec![AuthorityCapability::EffectSingleUseCapability],
            expires_at: Some("2026-05-22T00:00:00Z".into()),
            issued_by_ref: Reference::with_uri(ReferenceType::Host, "authority:payment:test"),
            credential_ref: None,
        }
    }
}
