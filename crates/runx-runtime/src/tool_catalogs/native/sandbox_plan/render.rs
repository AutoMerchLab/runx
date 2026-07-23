use runx_contracts::{JsonObject, JsonValue};

use super::assessment::{SandboxAssessment, SourceEvidence, WorkloadIdentity};

// Function rationale: this is the declarative projection of
// one stable hardening packet; assessment and policy logic live upstream.
pub(super) fn render_profile(
    identity: WorkloadIdentity,
    evidence: SourceEvidence,
    declaration: JsonObject,
    assessment: SandboxAssessment,
    findings: Vec<JsonValue>,
) -> JsonValue {
    JsonValue::Object(JsonObject::from([(
        "hardening_profile".to_owned(),
        JsonValue::Object(JsonObject::from([
            (
                "schema".to_owned(),
                JsonValue::String("runx.hardening.v1".to_owned()),
            ),
            (
                "decision".to_owned(),
                JsonValue::String(assessment.decision.to_owned()),
            ),
            (
                "workload".to_owned(),
                JsonValue::Object(JsonObject::from([
                    (
                        "ref_form".to_owned(),
                        JsonValue::String(identity.ref_form.to_owned()),
                    ),
                    (
                        "image_digest".to_owned(),
                        JsonValue::String(identity.image_digest),
                    ),
                    ("skill_ref".to_owned(), JsonValue::String(identity.skill_ref)),
                    ("class".to_owned(), JsonValue::String(identity.class)),
                ])),
            ),
            ("declaration".to_owned(), JsonValue::Object(declaration)),
            (
                "source_evidence".to_owned(),
                JsonValue::Object(JsonObject::from([
                    (
                        "source_ref".to_owned(),
                        JsonValue::String(evidence.source_ref),
                    ),
                    (
                        "source_digest".to_owned(),
                        JsonValue::String(evidence.source_digest),
                    ),
                    (
                        "observed_at".to_owned(),
                        JsonValue::String(evidence.observed_at),
                    ),
                    (
                        "provenance".to_owned(),
                        JsonValue::String("caller_supplied_source_digest".to_owned()),
                    ),
                ])),
            ),
            (
                "admission".to_owned(),
                JsonValue::Object(JsonObject::from([
                    (
                        "status".to_owned(),
                        JsonValue::String(assessment.admission_status.to_owned()),
                    ),
                    (
                        "reasons".to_owned(),
                        JsonValue::Array(
                            assessment
                                .admission_reasons
                                .into_iter()
                                .map(JsonValue::String)
                                .collect(),
                        ),
                    ),
                ])),
            ),
            (
                "unsupported_controls".to_owned(),
                JsonValue::Array(
                    [
                        "seccomp syscall policy",
                        "host egress allowlist",
                        "Linux capability policy",
                        "CPU and memory limits",
                    ]
                    .into_iter()
                    .map(|control| JsonValue::String(control.to_owned()))
                    .collect(),
                ),
            ),
            (
                "residual_risk".to_owned(),
                JsonValue::Object(JsonObject::from([
                    (
                        "level".to_owned(),
                        JsonValue::String(assessment.residual_level.to_owned()),
                    ),
                    (
                        "reason".to_owned(),
                        JsonValue::String("The Runx sandbox declaration governs filesystem, environment, and coarse network access; syscall, host-level egress, Linux capabilities, and resource quotas require a separate runtime boundary.".to_owned()),
                    ),
                ])),
            ),
            ("applied".to_owned(), JsonValue::Bool(false)),
            (
                "validation".to_owned(),
                JsonValue::Object(JsonObject::from([
                    (
                        "status".to_owned(),
                        JsonValue::String(
                            if assessment.decision == "ready" {
                                "pass"
                            } else {
                                "fail"
                            }
                            .to_owned(),
                        ),
                    ),
                    ("findings".to_owned(), JsonValue::Array(findings)),
                ])),
            ),
        ])),
    )]))
}
