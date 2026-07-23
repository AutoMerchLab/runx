use runx_contracts::{
    AuthorityEffectLimit, AuthorityTerm, AuthorityVerb, ClosureDisposition, EffectFinalityPhase,
    ProofKind, Receipt, Reference, ReferenceType,
};
use runx_runtime::VerifiedReceiptStore;

use super::{PaymentPlanningError, RefundableCharge, invalid, json_bytes, sha256_hex};

const PAYMENT_FAMILY: &str = "payment";
const REFUND_OPERATION: &str = "refund";
const MONEY_MOVEMENT_URI_PREFIX: &str = "runx:money_movement:";
const RECEIPT_URI_PREFIX: &str = "runx:receipt:";

pub(super) struct VerifiedRefundProof {
    pub charge: RefundableCharge,
    pub history_receipt_refs: Vec<String>,
    pub proof_digest: String,
}

pub(super) fn resolve_refund_proof(
    request: runx_runtime::EffectToolRequest<'_>,
    original_receipt_ref: &str,
) -> Result<VerifiedRefundProof, PaymentPlanningError> {
    let store = VerifiedReceiptStore::resolve(request.env, request.skill_directory)
        .map_err(|error| invalid(format!("refund receipt store is unavailable: {error}")))?;
    let original = store.read_exact(original_receipt_ref).map_err(|error| {
        invalid(format!(
            "original payment receipt proof is invalid: {error}"
        ))
    })?;
    let original_projection = payment_projection(&original, AuthorityVerb::Commit)?;
    if original_projection.operation == REFUND_OPERATION {
        return Err(invalid(
            "original payment receipt must describe a charge, not another refund",
        ));
    }
    let history = verified_refund_history(&store, &original, &original_projection)?;
    let proof_digest = refund_proof_digest(&original, &original_projection, &history.entries)?;
    Ok(VerifiedRefundProof {
        charge: RefundableCharge {
            money_movement_id: original_projection.money_movement_id,
            rail: original_projection.rail,
            phase: EffectFinalityPhase::Sealed,
            amount_minor: original_projection.amount_minor,
            refunded_minor: history.refunded_minor,
            currency: original_projection.currency,
            payer_ref: original_projection.counterparty,
            proof_ref: original_projection.proof_ref,
        },
        history_receipt_refs: history
            .entries
            .into_iter()
            .map(|(receipt_ref, _, _)| receipt_ref)
            .collect(),
        proof_digest,
    })
}

struct VerifiedRefundHistory {
    refunded_minor: u64,
    entries: Vec<(String, String, u64)>,
}

fn verified_refund_history(
    store: &VerifiedReceiptStore,
    original: &Receipt,
    original_projection: &PaymentReceiptProjection,
) -> Result<VerifiedRefundHistory, PaymentPlanningError> {
    let mut refunded_minor = 0_u64;
    let mut entries = Vec::new();
    for receipt in store
        .list()
        .map_err(|error| invalid(format!("refund receipt history is invalid: {error}")))?
    {
        if receipt.id == original.id || !references_original_receipt(&receipt, original.id.as_str())
        {
            continue;
        }
        let refund = payment_projection(&receipt, AuthorityVerb::Reverse)?;
        if refund.operation != REFUND_OPERATION {
            continue;
        }
        if refund.rail != original_projection.rail
            || refund.currency != original_projection.currency
            || refund.counterparty != original_projection.counterparty
        {
            return Err(invalid(format!(
                "refund receipt {} does not match the original payment identity",
                receipt.id
            )));
        }
        refunded_minor = refunded_minor
            .checked_add(refund.amount_minor)
            .ok_or_else(|| invalid("verified refund history amount overflowed"))?;
        entries.push((
            receipt.id.to_string(),
            receipt.digest.to_string(),
            refund.amount_minor,
        ));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(VerifiedRefundHistory {
        refunded_minor,
        entries,
    })
}

fn refund_proof_digest(
    original: &Receipt,
    projection: &PaymentReceiptProjection,
    history: &[(String, String, u64)],
) -> Result<String, PaymentPlanningError> {
    let proof_value = serde_json::json!({
        "original_receipt_ref": original.id,
        "original_receipt_digest": original.digest,
        "money_movement_id": projection.money_movement_id,
        "provider_proof_ref": projection.proof_ref,
        "history": history,
    });
    Ok(format!("sha256:{}", sha256_hex(&json_bytes(&proof_value)?)))
}

struct PaymentReceiptProjection {
    rail: String,
    operation: String,
    amount_minor: u64,
    currency: String,
    counterparty: String,
    proof_ref: String,
    money_movement_id: String,
}

fn payment_projection(
    receipt: &Receipt,
    verb: AuthorityVerb,
) -> Result<PaymentReceiptProjection, PaymentPlanningError> {
    if receipt.seal.disposition != ClosureDisposition::Closed {
        return Err(invalid(format!(
            "payment receipt {} is not closed",
            receipt.id
        )));
    }
    let limit = validated_payment_limit(receipt, verb)?;
    let amount_minor = limit.max_per_call_units.ok_or_else(|| {
        invalid(format!(
            "payment receipt {} has no per-call amount bound",
            receipt.id
        ))
    })?;
    let [rail] = limit.channels.as_slice() else {
        return Err(invalid(format!(
            "payment receipt {} must bind exactly one rail",
            receipt.id
        )));
    };
    let operation = required_limit_binding(receipt, limit.operation.as_deref(), "operation")?;
    let counterparty = required_limit_binding(receipt, limit.peer.as_deref(), "counterparty")?;
    let (proof_ref, money_movement_id) = payment_proof_identity(receipt)?;
    Ok(PaymentReceiptProjection {
        rail: rail.as_str().to_owned(),
        operation,
        amount_minor,
        currency: limit.unit.as_str().to_owned(),
        counterparty,
        proof_ref,
        money_movement_id,
    })
}

fn validated_payment_limit(
    receipt: &Receipt,
    verb: AuthorityVerb,
) -> Result<&AuthorityEffectLimit, PaymentPlanningError> {
    let mut candidates = receipt
        .authority
        .terms
        .iter()
        .filter(|term| term.verbs.contains(&verb))
        .filter_map(|term| payment_limit(term).map(|limit| (term, limit)))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(_, limit)| {
        (
            limit.max_per_call_units.unwrap_or(u64::MAX),
            limit.channels.len(),
        )
    });
    let (_, limit) = candidates.first().copied().ok_or_else(|| {
        invalid(format!(
            "payment receipt {} has no bounded {verb:?} authority",
            receipt.id
        ))
    })?;
    if !limit.receipt_before_success
        || !limit.idempotency_required
        || !limit.recovery_required
        || !limit.single_use_capability
    {
        return Err(invalid(format!(
            "payment receipt {} lacks required finality controls",
            receipt.id
        )));
    }
    Ok(limit)
}

fn required_limit_binding(
    receipt: &Receipt,
    value: Option<&str>,
    binding: &str,
) -> Result<String, PaymentPlanningError> {
    value.map(str::to_owned).ok_or_else(|| {
        invalid(format!(
            "payment receipt {} has no {binding} binding",
            receipt.id
        ))
    })
}

fn payment_proof_identity(receipt: &Receipt) -> Result<(String, String), PaymentPlanningError> {
    let verification_refs = receipt_verification_refs(receipt);
    let proof_ref = single_reference_uri(
        &verification_refs,
        |reference| reference.proof_kind == Some(ProofKind::EffectEvidence),
        "provider effect proof",
        receipt,
    )?;
    let movement_ref = single_reference_uri(
        &verification_refs,
        |reference| {
            reference
                .uri
                .as_str()
                .starts_with(MONEY_MOVEMENT_URI_PREFIX)
        },
        "money movement",
        receipt,
    )?;
    let money_movement_id = movement_ref
        .strip_prefix(MONEY_MOVEMENT_URI_PREFIX)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("payment receipt money movement reference is malformed"))?;
    Ok((proof_ref, money_movement_id.to_owned()))
}

fn payment_limit(term: &AuthorityTerm) -> Option<&AuthorityEffectLimit> {
    term.bounds
        .effect_limits
        .iter()
        .find(|limit| limit.family.as_str() == PAYMENT_FAMILY)
}

fn receipt_verification_refs(receipt: &Receipt) -> Vec<&Reference> {
    receipt
        .acts
        .iter()
        .flat_map(|act| &act.criterion_bindings)
        .flat_map(|binding| &binding.verification_refs)
        .collect()
}

fn single_reference_uri(
    references: &[&Reference],
    predicate: impl Fn(&Reference) -> bool,
    label: &str,
    receipt: &Receipt,
) -> Result<String, PaymentPlanningError> {
    let values = references
        .iter()
        .copied()
        .filter(|reference| predicate(reference))
        .map(|reference| reference.uri.as_str())
        .collect::<Vec<_>>();
    let [value] = values.as_slice() else {
        return Err(invalid(format!(
            "payment receipt {} must contain exactly one {label} reference",
            receipt.id
        )));
    };
    Ok((*value).to_owned())
}

fn references_original_receipt(receipt: &Receipt, original_receipt_ref: &str) -> bool {
    let expected_uri = format!("{RECEIPT_URI_PREFIX}{original_receipt_ref}");
    receipt.acts.iter().any(|act| {
        act.source_refs
            .iter()
            .chain(&act.target_refs)
            .chain(&act.artifact_refs)
            .chain(
                act.criterion_bindings
                    .iter()
                    .flat_map(|binding| &binding.verification_refs),
            )
            .any(|reference| {
                reference.reference_type == ReferenceType::Receipt
                    && reference.uri.as_str() == expected_uri
            })
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;

    use runx_contracts::{
        AuthorityBounds, AuthorityCapability, AuthorityEffectCredentialForm, AuthorityEffectLimit,
        AuthorityResourceFamily, AuthorityTerm, JsonNumber, JsonObject, JsonValue,
    };
    use runx_receipts::{canonical_receipt_body_digest, content_addressed_receipt_id};
    use runx_runtime::{
        CredentialDelivery, EffectToolRequest, InvocationStatus, LocalReceiptStore,
        RUNX_RECEIPT_DIR_ENV, SkillOutput,
    };

    use super::*;
    use crate::planning::refund::refund_plan;

    #[test]
    fn refund_plan_receipt_proof() {
        let temp = tempfile::tempdir().expect("temporary workspace");
        let receipt_dir = temp.path().join("receipts");
        let store = LocalReceiptStore::new(&receipt_dir);
        let original = payment_receipt("charge", AuthorityVerb::Commit, "charge", 125, None);
        store.write_receipt(&original).expect("original receipt");
        let prior_refund = payment_receipt(
            "prior-refund",
            AuthorityVerb::Reverse,
            REFUND_OPERATION,
            25,
            Some(original.id.as_str()),
        );
        store
            .write_receipt(&prior_refund)
            .expect("prior refund receipt");

        let parent = refund_authority(125);
        let inputs = refund_inputs(original.id.as_str(), &parent, 100);
        let env = BTreeMap::from([(
            RUNX_RECEIPT_DIR_ENV.to_owned(),
            receipt_dir.to_string_lossy().into_owned(),
        )]);
        let credentials = CredentialDelivery::none();
        let request = EffectToolRequest {
            tool_ref: "payment.refund_plan",
            observed_at: "2026-07-20T00:00:00Z",
            inputs: &inputs,
            env: &env,
            skill_directory: temp.path(),
            credential_delivery: &credentials,
            admission: None,
        };

        let first = refund_plan(request).expect("verified refund plan");
        let second = refund_plan(request).expect("stable verified refund plan");
        let first_plan = first
            .as_object()
            .and_then(|output| output.get("refund_plan"))
            .and_then(JsonValue::as_object)
            .expect("refund plan packet");
        let second_plan = second
            .as_object()
            .and_then(|output| output.get("refund_plan"))
            .and_then(JsonValue::as_object)
            .expect("refund plan packet");

        assert_eq!(
            first_plan.get("decision").and_then(JsonValue::as_str),
            Some("ready_for_refund_adapter")
        );
        assert_eq!(
            first_plan
                .get("original_charge")
                .and_then(JsonValue::as_object)
                .and_then(|charge| charge.get("refunded_minor"))
                .and_then(json_u64),
            Some(25_u64)
        );
        assert_eq!(
            first_plan
                .get("idempotency")
                .and_then(JsonValue::as_object)
                .and_then(|value| value.get("key")),
            second_plan
                .get("idempotency")
                .and_then(JsonValue::as_object)
                .and_then(|value| value.get("key"))
        );
        assert!(!inputs.contains_key("original_receipt"));
        assert!(!inputs.contains_key("idempotency_seed"));

        let over_refund = refund_inputs(original.id.as_str(), &parent, 101);
        let blocked = refund_plan(EffectToolRequest {
            inputs: &over_refund,
            ..request
        })
        .expect("over-refund plan");
        assert_eq!(
            blocked
                .as_object()
                .and_then(|output| output.get("refund_plan"))
                .and_then(JsonValue::as_object)
                .and_then(|plan| plan.get("decision"))
                .and_then(JsonValue::as_str),
            Some("blocked")
        );
    }

    fn payment_receipt(
        step_id: &str,
        verb: AuthorityVerb,
        operation: &str,
        amount_minor: u64,
        original_receipt_ref: Option<&str>,
    ) -> Receipt {
        let output = SkillOutput {
            status: InvocationStatus::Success,
            stdout: "{}".to_owned(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 1,
            metadata: JsonObject::new(),
        };
        let mut receipt = runx_runtime::receipts::step_receipt(
            "refund-proof-test",
            step_id,
            1,
            &output,
            "2026-07-20T00:00:00Z",
        )
        .expect("base receipt");
        receipt.authority.terms = vec![payment_authority(verb, operation, amount_minor)];
        receipt.authority.grant_refs = vec![Reference::with_uri(
            ReferenceType::Grant,
            format!("runx:grant:payment:{step_id}"),
        )];
        let mut references = vec![
            Reference {
                reference_type: ReferenceType::Verification,
                uri: format!("proof:{step_id}").into(),
                provider: Some("mock".into()),
                locator: None,
                label: Some("payment rail supervisor proof".into()),
                observed_at: None,
                proof_kind: Some(ProofKind::EffectEvidence),
            },
            Reference {
                reference_type: ReferenceType::Target,
                uri: format!("{MONEY_MOVEMENT_URI_PREFIX}{step_id}").into(),
                provider: Some("mock".into()),
                locator: None,
                label: Some("verified payment movement".into()),
                observed_at: None,
                proof_kind: Some(ProofKind::EffectFinality),
            },
        ];
        if let Some(original_receipt_ref) = original_receipt_ref {
            references.push(Reference {
                reference_type: ReferenceType::Receipt,
                uri: format!("{RECEIPT_URI_PREFIX}{original_receipt_ref}").into(),
                provider: None,
                locator: None,
                label: Some("original payment receipt".into()),
                observed_at: None,
                proof_kind: Some(ProofKind::EffectFinality),
            });
        }
        receipt.acts[0].criterion_bindings[0].verification_refs = references.clone();
        receipt.seal.criteria[0].verification_refs = references;
        reseal(receipt)
    }

    fn reseal(mut receipt: Receipt) -> Receipt {
        receipt.id = "pending".into();
        receipt.digest = "sha256:pending".into();
        receipt.signature.value = "sig:pending".into();
        receipt.id = content_addressed_receipt_id(&receipt)
            .expect("content-addressed receipt id")
            .into();
        let digest = canonical_receipt_body_digest(&receipt).expect("receipt body digest");
        receipt.digest = digest.clone().into();
        receipt.signature.value = format!("sig:{digest}").into();
        receipt
    }

    fn refund_inputs(
        original_receipt_ref: &str,
        parent: &AuthorityTerm,
        amount_minor: u64,
    ) -> JsonObject {
        JsonObject::from([
            (
                "original_receipt_ref".to_owned(),
                JsonValue::String(original_receipt_ref.to_owned()),
            ),
            (
                "refund_request".to_owned(),
                serde_json::from_value(serde_json::json!({
                    "amount_minor": amount_minor,
                    "reason": "operator_refund",
                    "requested_counterparty": "payer:demo",
                }))
                .expect("refund request"),
            ),
            (
                "settlement_family".to_owned(),
                JsonValue::String("mock".to_owned()),
            ),
            (
                "parent_payment_authority".to_owned(),
                serde_json::from_value(serde_json::to_value(parent).expect("parent value"))
                    .expect("parent JSON"),
            ),
        ])
    }

    fn json_u64(value: &JsonValue) -> Option<u64> {
        match value {
            JsonValue::Number(JsonNumber::U64(value)) => Some(*value),
            JsonValue::Number(JsonNumber::I64(value)) => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    fn payment_authority(verb: AuthorityVerb, operation: &str, amount_minor: u64) -> AuthorityTerm {
        authority("payment-receipt", verb, operation, amount_minor)
    }

    fn refund_authority(amount_minor: u64) -> AuthorityTerm {
        authority(
            "refund-parent",
            AuthorityVerb::Reverse,
            REFUND_OPERATION,
            amount_minor,
        )
    }

    fn authority(
        term_id: &str,
        verb: AuthorityVerb,
        operation: &str,
        amount_minor: u64,
    ) -> AuthorityTerm {
        AuthorityTerm {
            term_id: term_id.into(),
            principal_ref: Reference::with_uri(ReferenceType::Principal, "runx:principal:operator"),
            resource_ref: Reference::with_uri(ReferenceType::Target, "payer:demo"),
            resource_family: AuthorityResourceFamily::Effect,
            verbs: vec![verb],
            bounds: AuthorityBounds {
                effect_limits: vec![AuthorityEffectLimit {
                    family: PAYMENT_FAMILY.into(),
                    unit: "USD".into(),
                    max_per_call_units: Some(amount_minor),
                    max_per_run_units: Some(500),
                    max_per_period_units: None,
                    period: None,
                    channels: vec!["mock".into()],
                    realm: Some("test".into()),
                    peer: Some("payer:demo".into()),
                    operation: Some(operation.into()),
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
            expires_at: None,
            issued_by_ref: Reference::with_uri(ReferenceType::Grant, "runx:grant:payment"),
            credential_ref: None,
        }
    }
}
