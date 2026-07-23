use runx_contracts::{JsonObject, JsonValue};
use runx_core::policy::{
    SandboxAdmissionDecision, SandboxAdmissionOptions, SandboxDeclaration, SandboxProfile,
    admit_sandbox, normalize_sandbox_declaration,
};

use super::{
    apply_baseline, boolean, finding, non_empty, optional_strings, optional_text, parse_cwd_policy,
    required_time, strings, valid_sha256,
};
use crate::RuntimeError;

pub(super) struct WorkloadIdentity {
    pub(super) ref_form: &'static str,
    pub(super) image_digest: String,
    pub(super) skill_ref: String,
    pub(super) class: String,
}

pub(super) struct SourceEvidence {
    pub(super) source_ref: String,
    pub(super) source_digest: String,
    pub(super) observed_at: String,
}

pub(super) struct SandboxAssessment {
    pub(super) decision: &'static str,
    pub(super) admission_status: &'static str,
    pub(super) admission_reasons: Vec<String>,
    pub(super) residual_level: &'static str,
}

pub(super) fn inspect_identity(
    workload: &JsonObject,
    findings: &mut Vec<JsonValue>,
) -> WorkloadIdentity {
    let image_digest = optional_text(workload, "image_digest");
    let skill_ref = optional_text(workload, "skill_ref");
    if image_digest.is_some() == skill_ref.is_some() {
        finding(
            findings,
            "sandbox.workload.identity",
            "workload requires exactly one image_digest or skill_ref",
        );
    }
    if image_digest.is_some_and(|value| !valid_sha256(value)) {
        finding(
            findings,
            "sandbox.workload.digest",
            "image_digest must be a lowercase sha256 digest",
        );
    }
    WorkloadIdentity {
        ref_form: if image_digest.is_some() {
            "image_digest"
        } else if skill_ref.is_some() {
            "skill_ref"
        } else {
            ""
        },
        image_digest: image_digest.unwrap_or_default().to_owned(),
        skill_ref: skill_ref.unwrap_or_default().to_owned(),
        class: optional_text(workload, "class")
            .unwrap_or_default()
            .to_owned(),
    }
}

pub(super) fn inspect_source_evidence(
    as_of: &str,
    max_age_days: u64,
    requirements: &JsonObject,
    findings: &mut Vec<JsonValue>,
) -> Result<SourceEvidence, RuntimeError> {
    let as_of = parse_time(as_of, "as_of", findings);
    let max_age_days = max_age_days as f64;
    if !(0.0 < max_age_days && max_age_days <= 3650.0) {
        finding(
            findings,
            "sandbox.requirements.max_age",
            "max_age_days must be greater than zero and at most 3650",
        );
    }
    let source_ref = optional_text(requirements, "source_ref").unwrap_or_default();
    let source_digest = optional_text(requirements, "source_digest").unwrap_or_default();
    let observed_at = required_time(requirements, "observed_at", findings);
    if source_ref.is_empty() || !valid_sha256(source_digest) {
        finding(
            findings,
            "sandbox.requirements.provenance",
            "requirements require source_ref and source_digest",
        );
    }
    if let (Some(as_of), Some(observed_at)) = (as_of, observed_at) {
        let age_days = (as_of - observed_at) as f64 / 86_400.0;
        if age_days < 0.0 || age_days > max_age_days {
            finding(
                findings,
                "sandbox.requirements.stale",
                "requirements are stale or future-dated",
            );
        }
    }
    Ok(SourceEvidence {
        source_ref: source_ref.to_owned(),
        source_digest: source_digest.to_owned(),
        observed_at: optional_text(requirements, "observed_at")
            .unwrap_or_default()
            .to_owned(),
    })
}

fn parse_time(value: &str, field: &str, findings: &mut Vec<JsonValue>) -> Option<i64> {
    let value = value.trim();
    let Some((days, seconds, _)) = runx_core::policy::parse_rfc3339_moment(value) else {
        finding(
            findings,
            "sandbox.time.invalid",
            format!("{field} must be RFC3339"),
        );
        return None;
    };
    days.checked_mul(86_400)?.checked_add(seconds)
}

pub(super) fn build_declaration(
    requirements: &JsonObject,
    baseline: &JsonObject,
    findings: &mut Vec<JsonValue>,
) -> Result<(SandboxDeclaration, Vec<&'static str>), RuntimeError> {
    let network = boolean(requirements, "network", false)?;
    let writable_paths = strings(requirements, "writable_paths")?;
    if writable_paths
        .iter()
        .any(|path| std::path::Path::new(path).is_absolute())
    {
        finding(
            findings,
            "sandbox.writable_path.absolute",
            "planned writable paths must be workspace-relative",
        );
    }
    let env_allowlist = optional_strings(requirements, "env_allowlist")?;
    let require_enforcement = boolean(requirements, "require_enforcement", true)?;
    let cwd_policy = parse_cwd_policy(optional_text(requirements, "cwd_policy"), findings);
    let unsupported = unsupported_controls(requirements);
    if !unsupported.is_empty() {
        finding(
            findings,
            "sandbox.control.unsupported",
            format!(
                "Runx sandbox declarations do not express: {}",
                unsupported.join(", ")
            ),
        );
    }
    apply_baseline(
        baseline,
        network,
        &writable_paths,
        env_allowlist.as_deref(),
        require_enforcement,
        findings,
    )?;
    let profile = if network {
        SandboxProfile::Network
    } else if writable_paths.is_empty() {
        SandboxProfile::Readonly
    } else {
        SandboxProfile::WorkspaceWrite
    };
    Ok((
        SandboxDeclaration {
            profile,
            cwd_policy,
            env_allowlist,
            network: Some(network),
            writable_paths: Some(writable_paths),
            require_enforcement: Some(require_enforcement),
        },
        unsupported,
    ))
}

fn unsupported_controls(requirements: &JsonObject) -> Vec<&'static str> {
    [
        "allowed_syscalls",
        "allowed_egress_hosts",
        "required_capabilities",
    ]
    .into_iter()
    .filter(|field| requirements.get(*field).is_some_and(non_empty))
    .collect()
}

pub(super) fn assess_declaration(
    declaration: &SandboxDeclaration,
    unsupported: &[&str],
    findings: &mut Vec<JsonValue>,
) -> SandboxAssessment {
    let admission = admit_sandbox(Some(declaration), &SandboxAdmissionOptions::default());
    let (admission_status, admission_reasons) = match admission {
        SandboxAdmissionDecision::Allow { reasons } => ("allow", reasons),
        SandboxAdmissionDecision::ApprovalRequired { reasons } => ("approval_required", reasons),
        SandboxAdmissionDecision::Deny { reasons } => ("deny", reasons),
    };
    if admission_status != "allow" {
        for reason in &admission_reasons {
            finding(findings, "sandbox.admission.denied", reason);
        }
    }
    let writable = declaration
        .writable_paths
        .as_deref()
        .is_some_and(|paths| !paths.is_empty());
    let unsupported_shape =
        declaration.network == Some(true) && writable || !unsupported.is_empty();
    let baseline_violation = findings.iter().any(|finding| {
        finding
            .as_object()
            .and_then(|finding| finding.get("code"))
            .and_then(JsonValue::as_str)
            .is_some_and(|code| code.starts_with("sandbox.baseline."))
    });
    let decision = if findings.is_empty() && admission_status == "allow" {
        "ready"
    } else if unsupported_shape {
        "unsupported_runtime_shape"
    } else if admission_status == "deny" || baseline_violation {
        "refused"
    } else {
        "needs_more_evidence"
    };
    SandboxAssessment {
        decision,
        admission_status,
        admission_reasons,
        residual_level: if declaration.network == Some(true) || writable {
            "medium"
        } else {
            "low"
        },
    }
}

pub(super) fn declaration_json(declaration: &SandboxDeclaration) -> JsonObject {
    let normalized = normalize_sandbox_declaration(Some(declaration));
    JsonObject::from([
        (
            "profile".to_owned(),
            JsonValue::String(normalized.profile.as_str().to_owned()),
        ),
        (
            "cwd_policy".to_owned(),
            JsonValue::String(normalized.cwd_policy.as_str().to_owned()),
        ),
        (
            "env_allowlist".to_owned(),
            normalized.env_allowlist.map_or(JsonValue::Null, |values| {
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect())
            }),
        ),
        ("network".to_owned(), JsonValue::Bool(normalized.network)),
        (
            "writable_paths".to_owned(),
            JsonValue::Array(
                normalized
                    .writable_paths
                    .into_iter()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
        (
            "require_enforcement".to_owned(),
            JsonValue::Bool(normalized.require_enforcement),
        ),
    ])
}

// Function rationale: this is a declarative projection of
// one stable hardening packet; all policy decisions are computed before it.
