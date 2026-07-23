// Module rationale: the provider-charge protocol keeps price, challenge, credential verification, and the non-forwarding plan together for one auditable state transition.

use super::{
    EffectToolRequest, JsonObject, JsonValue, PAYMENT_FAMILY, PaymentPlanningError, admit_opaque,
    admit_opaque_array, finding, invalid, json_bytes, looks_like_iso_datetime, object_value,
    optional_u64, packet_findings, required_object, required_string, sha256_hex,
};

// Function rationale: price admission binds one provider policy, requested authority, and evidence packet in a single fail-closed projection.
pub(super) fn charge_price(
    request: EffectToolRequest<'_>,
) -> Result<JsonValue, PaymentPlanningError> {
    let call = required_object(request.inputs, "mcp_tool_call")?;
    let policy = required_object(request.inputs, "provider_policy")?;
    let mut findings = Vec::new();
    let operation = admit_opaque(
        call.get("tool"),
        "mcp_tool_call.tool",
        256,
        true,
        &mut findings,
    );
    let arguments = call.get("arguments").and_then(JsonValue::as_object);
    if arguments.is_none() {
        findings.push(finding(
            "mcp_tool_call.arguments",
            "mcp_tool_call.arguments must be an object",
        ));
    }
    let amount_minor = match optional_u64(policy, "price_minor")? {
        Some(amount) if amount > 0 => Some(amount),
        _ => {
            findings.push(finding(
                "provider_policy.price_minor",
                "provider_policy.price_minor must be a positive safe integer",
            ));
            None
        }
    };
    let currency = admit_opaque(
        policy.get("currency"),
        "provider_policy.currency",
        3,
        true,
        &mut findings,
    );
    if currency.as_deref().is_some_and(|value| {
        value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_uppercase())
    }) {
        findings.push(finding(
            "provider_policy.currency",
            "provider_policy.currency must be an uppercase ISO 4217 code",
        ));
    }
    let settlement_families = admit_opaque_array(
        policy.get("accepted_settlement_families"),
        "provider_policy.accepted_settlement_families",
        10,
        64,
        &mut findings,
    );
    if settlement_families.is_empty() {
        findings.push(finding(
            "provider_policy.accepted_settlement_families",
            "at least one settlement family is required",
        ));
    }
    let counterparty = admit_opaque(
        policy.get("counterparty"),
        "provider_policy.counterparty",
        256,
        true,
        &mut findings,
    );
    let realm = admit_opaque(
        policy.get("realm"),
        "provider_policy.realm",
        64,
        false,
        &mut findings,
    )
    .unwrap_or_else(|| "local".to_owned());
    let expires_at = policy
        .get("expires_at")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if expires_at
        .as_deref()
        .is_some_and(|value| !looks_like_iso_datetime(value))
    {
        findings.push(finding(
            "provider_policy.expires_at",
            "provider_policy.expires_at must be an ISO-8601 UTC timestamp",
        ));
    }
    let policy_ref = admit_opaque(
        policy.get("policy_ref"),
        "provider_policy.policy_ref",
        256,
        false,
        &mut findings,
    );
    let price_core = serde_json::json!({
        "operation": operation.clone(),
        "amount_minor": amount_minor,
        "currency": currency.clone(),
        "settlement_families": settlement_families.clone(),
        "counterparty": counterparty.clone(),
        "realm": realm.clone(),
        "expires_at": expires_at.clone(),
    });
    let price_id = format!("charge-price:{}", sha256_hex(&json_bytes(&price_core)?));
    let policy_source = policy_ref.unwrap_or(format!(
        "policy:sha256:{}",
        sha256_hex(&json_bytes(policy)?)
    ));
    let tool_source = format!("tool-call:sha256:{}", sha256_hex(&json_bytes(call)?));
    let arguments_digest = format!(
        "sha256:{}",
        sha256_hex(&json_bytes(&arguments.cloned().unwrap_or_default())?)
    );
    let ready = findings.is_empty();
    let charge_price = object_value(serde_json::json!({
        "decision": if ready { "ready" } else { "blocked" },
        "price_id": price_id,
        "operation": operation.clone(),
        "amount_minor": amount_minor,
        "currency": currency.clone(),
        "settlement_families": settlement_families.clone(),
        "counterparty": counterparty.clone(),
        "realm": realm.clone(),
        "expires_at": expires_at,
    }))?;
    let requested_payment_authority = object_value(serde_json::json!({
        "resource_family": "effect",
        "verbs": ["verify"],
        "bounds": {
            "effect_limits": [{
                "family": PAYMENT_FAMILY,
                "unit": currency.clone(),
                "max_per_call_units": amount_minor,
                "channels": settlement_families,
                "realm": realm.clone(),
                "peer": counterparty,
                "operation": operation,
                "idempotency_required": true,
                "receipt_before_success": true,
            }],
        },
    }))?;
    Ok(JsonValue::Object(JsonObject::from([
        ("charge_price".to_owned(), charge_price),
        (
            "requested_payment_authority".to_owned(),
            requested_payment_authority,
        ),
        (
            "price_evidence".to_owned(),
            object_value(serde_json::json!({
                "source_refs": [policy_source, tool_source],
                "arguments_digest": arguments_digest,
            }))?,
        ),
        (
            "policy_metadata".to_owned(),
            object_value(serde_json::json!({
                "provider_realm": realm,
                "direction": "provider_charge",
            }))?,
        ),
        ("open_questions".to_owned(), JsonValue::Array(findings)),
    ])))
}

// Function rationale: challenge construction keeps the price, replay key, authority, and effect-required signal visibly bound.
pub(super) fn charge_challenge(
    request: EffectToolRequest<'_>,
) -> Result<JsonValue, PaymentPlanningError> {
    let packet = required_object(request.inputs, "charge_price_packet")?;
    let price = required_object(packet, "charge_price")?;
    let authority = required_object(packet, "requested_payment_authority")?;
    let mut findings = packet_findings(packet);
    if price.get("decision").and_then(JsonValue::as_str) != Some("ready") {
        findings.push(finding("price.blocked", "charge price is not ready"));
    }
    let seed = required_string(request.inputs, "idempotency_seed")?;
    let price_id = required_string(price, "price_id")?;
    let challenge_id = format!(
        "charge-challenge:{}",
        sha256_hex(&json_bytes(&serde_json::json!({
            "price_id": price_id,
            "idempotency_seed": seed,
        }))?)
    );
    let families = price
        .get("settlement_families")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let rail = families
        .first()
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let ready = findings.is_empty();
    Ok(JsonValue::Object(JsonObject::from([
        (
            "effect_required_signal".to_owned(),
            object_value(serde_json::json!({
                "signal_type": "effect_required",
                "challenge_id": challenge_id.clone(),
                "amount_minor": price.get("amount_minor"),
                "currency": price.get("currency"),
                "rail": rail,
                "counterparty": price.get("counterparty"),
                "operation": price.get("operation"),
            }))?,
        ),
        (
            "charge_challenge".to_owned(),
            object_value(serde_json::json!({
                "decision": if ready { "ready" } else { "blocked" },
                "challenge_id": challenge_id,
                "price_id": price_id,
                "required_authority": authority,
                "receipt_before_forward_required": true,
            }))?,
        ),
        (
            "idempotency".to_owned(),
            object_value(serde_json::json!({
                "key": format!("charge:{seed}"),
                "replay_policy": "recover_or_refuse_duplicate",
            }))?,
        ),
        (
            "accepted_settlement_families".to_owned(),
            JsonValue::Array(families),
        ),
        ("open_questions".to_owned(), JsonValue::Array(findings)),
    ])))
}

// Function rationale: credential admission rejects raw fields and binds the selected family, capability, challenge, and idempotency in one reviewable check.
pub(super) fn charge_verification_request(
    request: EffectToolRequest<'_>,
) -> Result<JsonValue, PaymentPlanningError> {
    let price_packet = required_object(request.inputs, "charge_price_packet")?;
    let challenge_packet = required_object(request.inputs, "charge_challenge_packet")?;
    let price = required_object(price_packet, "charge_price")?;
    let challenge = required_object(challenge_packet, "charge_challenge")?;
    let credential = required_object(request.inputs, "returned_credential")?;
    let mut findings = packet_findings(price_packet);
    findings.extend(packet_findings(challenge_packet));
    let family = admit_opaque(
        request.inputs.get("settlement_family"),
        "settlement_family",
        64,
        true,
        &mut findings,
    );
    let credential_family = admit_opaque(
        credential.get("family"),
        "returned_credential.family",
        64,
        true,
        &mut findings,
    );
    let credential_ref = admit_opaque(
        credential.get("credential_ref"),
        "returned_credential.credential_ref",
        512,
        true,
        &mut findings,
    );
    let capability_ref = admit_opaque(
        request.inputs.get("verify_capability_ref"),
        "verify_capability_ref",
        512,
        true,
        &mut findings,
    );
    let extras = credential
        .keys()
        .filter(|field| !matches!(field.as_str(), "family" | "credential_ref"))
        .cloned()
        .collect::<Vec<_>>();
    if !extras.is_empty() {
        findings.push(finding(
            "credential.raw_fields",
            format!(
                "returned_credential contains unsupported fields: {}",
                extras.join(", ")
            ),
        ));
    }
    let admitted_families = challenge_packet
        .get("accepted_settlement_families")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if family.as_deref().is_some_and(|selected| {
        !admitted_families
            .iter()
            .any(|candidate| candidate.as_str() == Some(selected))
    }) {
        findings.push(finding(
            "family.not_admitted",
            "settlement family is not admitted by the challenge",
        ));
    }
    if family != credential_family {
        findings.push(finding(
            "family.mismatch",
            "credential family does not match selected settlement family",
        ));
    }
    if challenge.get("decision").and_then(JsonValue::as_str) != Some("ready") {
        findings.push(finding(
            "challenge.blocked",
            "charge challenge is not ready",
        ));
    }
    let idempotency = required_object(request.inputs, "idempotency")?;
    let core = serde_json::json!({
        "price_id": price.get("price_id").and_then(JsonValue::as_str).unwrap_or_default(),
        "challenge_id": challenge.get("challenge_id").and_then(JsonValue::as_str).unwrap_or_default(),
        "settlement_family": family.clone(),
        "credential_ref": credential_ref.clone(),
        "verify_capability_ref": capability_ref,
        "idempotency": idempotency,
    });
    let request_digest = format!("sha256:{}", sha256_hex(&json_bytes(&core)?));
    let ready = findings.is_empty();
    let mut verification_request = object_value(core)?
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("verification request must be an object"))?;
    verification_request.insert(
        "decision".to_owned(),
        JsonValue::String(
            if ready {
                "ready_for_provider_adapter"
            } else {
                "blocked"
            }
            .to_owned(),
        ),
    );
    verification_request.insert(
        "request_digest".to_owned(),
        JsonValue::String(request_digest),
    );
    Ok(JsonValue::Object(JsonObject::from([
        (
            "verification_request".to_owned(),
            JsonValue::Object(verification_request),
        ),
        (
            "credential_binding".to_owned(),
            object_value(serde_json::json!({
                "family": credential_family,
                "credential_ref": credential_ref,
            }))?,
        ),
        (
            "provider_status".to_owned(),
            JsonValue::String("not_called".to_owned()),
        ),
        (
            "receipt_status".to_owned(),
            JsonValue::String("not_sealed".to_owned()),
        ),
        (
            "forwarding_status".to_owned(),
            JsonValue::String("not_forwarded".to_owned()),
        ),
        ("open_questions".to_owned(), JsonValue::Array(findings)),
    ])))
}

// Function rationale: the final charge projection deliberately keeps every non-final provider and receipt state explicit.
pub(super) fn charge_plan(
    request: EffectToolRequest<'_>,
) -> Result<JsonValue, PaymentPlanningError> {
    let price = required_object(request.inputs, "charge_price_packet")?;
    let challenge = required_object(request.inputs, "charge_challenge_packet")?;
    let verification = required_object(request.inputs, "charge_verification_request")?;
    let mut findings = packet_findings(price);
    findings.extend(packet_findings(challenge));
    findings.extend(packet_findings(verification));
    let verification_request = required_object(verification, "verification_request")?;
    let ready = verification_request
        .get("decision")
        .and_then(JsonValue::as_str)
        == Some("ready_for_provider_adapter")
        && findings.is_empty();
    let core = serde_json::json!({
        "charge_price": price.get("charge_price").and_then(JsonValue::as_object),
        "charge_challenge": challenge.get("charge_challenge").and_then(JsonValue::as_object),
        "idempotency": challenge.get("idempotency").and_then(JsonValue::as_object),
        "verification_request": verification_request,
        "credential_binding": verification.get("credential_binding").and_then(JsonValue::as_object),
    });
    let plan_digest = format!("sha256:{}", sha256_hex(&json_bytes(&core)?));
    let mut plan = object_value(core)?
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("charge plan must be an object"))?;
    for (key, value) in [
        (
            "decision",
            if ready {
                "ready_for_provider_verification"
            } else {
                "blocked"
            },
        ),
        ("provider_status", "not_called"),
        ("receipt_status", "not_sealed"),
        ("forwarding_status", "not_forwarded"),
        ("approval_status", "not_requested"),
    ] {
        plan.insert(key.to_owned(), JsonValue::String(value.to_owned()));
    }
    plan.insert(
        "schema".to_owned(),
        JsonValue::String("runx.payment.charge_plan.v1".to_owned()),
    );
    plan.insert(
        "runtime_forwarding_enabled".to_owned(),
        JsonValue::Bool(false),
    );
    plan.insert("findings".to_owned(), JsonValue::Array(findings));
    plan.insert("plan_digest".to_owned(), JsonValue::String(plan_digest));
    plan.insert(
        "next_action".to_owned(),
        JsonValue::String(
            if ready {
                "route through the selected settlement-family verifier; seal its receipt before forwarding"
            } else {
                "resolve the recorded pricing, challenge, or credential gaps"
            }
            .to_owned(),
        ),
    );
    Ok(JsonValue::Object(JsonObject::from([(
        "charge_plan".to_owned(),
        JsonValue::Object(plan),
    )])))
}
