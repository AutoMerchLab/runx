use runx_contracts::JsonValue;
use runx_core::policy::{
    CwdPolicy, SandboxDeclaration, SandboxProfile, normalize_sandbox_declaration,
};

use crate::ValidationError;

use super::{FIELDS, SkillSandbox};

pub(super) fn validate_sandbox(
    value: Option<&JsonValue>,
) -> Result<Option<SkillSandbox>, ValidationError> {
    let Some(record) = value else {
        return Ok(None);
    };
    let record = FIELDS.required_object(Some(record), "sandbox")?;
    let profile = required_sandbox_profile(record.get("profile"), "sandbox.profile")?;
    let cwd_policy = optional_cwd_policy(record.get("cwd_policy"))?;
    FIELDS.reject_unknown_fields(
        record,
        "sandbox",
        &[
            "approvedEscalation",
            "cwd_policy",
            "network",
            "profile",
            "require_enforcement",
            "writable_paths",
        ],
    )?;
    let network = FIELDS.optional_bool(record.get("network"), "sandbox.network")?;
    let writable_paths = FIELDS
        .optional_string_array(record.get("writable_paths"), "sandbox.writable_paths")?
        .unwrap_or_default();
    let require_enforcement = FIELDS.optional_bool(
        record.get("require_enforcement"),
        "sandbox.require_enforcement",
    )?;
    let declaration = sandbox_declaration(
        &profile,
        cwd_policy.as_deref(),
        network,
        Some(writable_paths.clone()),
        require_enforcement,
    )?;
    let normalized = normalize_sandbox_declaration(Some(&declaration));
    Ok(Some(SkillSandbox {
        profile: normalized.profile,
        cwd_policy: Some(normalized.cwd_policy),
        network: Some(normalized.network),
        writable_paths: normalized.writable_paths,
        require_enforcement,
        // TS currently preserves approvedEscalation only inside raw.
        approved_escalation: None,
        raw: record.clone(),
    }))
}

fn required_sandbox_profile(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<String, ValidationError> {
    let profile = FIELDS.required_string(value, field)?;
    if matches!(
        profile.as_str(),
        "readonly" | "workspace-write" | "network" | "unrestricted-local-dev"
    ) {
        return Ok(profile);
    }
    Err(FIELDS.validation_error(format!(
        "{field} must be readonly, workspace-write, network, or unrestricted-local-dev."
    )))
}

fn optional_cwd_policy(value: Option<&JsonValue>) -> Result<Option<String>, ValidationError> {
    let Some(value) = FIELDS.optional_string(value, "sandbox.cwd_policy")? else {
        return Ok(None);
    };
    if matches!(value.as_str(), "skill-directory" | "workspace" | "custom") {
        return Ok(Some(value));
    }
    Err(FIELDS
        .validation_error("sandbox.cwd_policy must be skill-directory, workspace, or custom."))
}

fn sandbox_declaration(
    profile: &str,
    cwd_policy: Option<&str>,
    network: Option<bool>,
    writable_paths: Option<Vec<String>>,
    require_enforcement: Option<bool>,
) -> Result<SandboxDeclaration, ValidationError> {
    Ok(SandboxDeclaration {
        profile: match profile {
            "readonly" => SandboxProfile::Readonly,
            "workspace-write" => SandboxProfile::WorkspaceWrite,
            "network" => SandboxProfile::Network,
            "unrestricted-local-dev" => SandboxProfile::UnrestrictedLocalDev,
            _ => return Err(FIELDS.validation_error("sandbox.profile is invalid.")),
        },
        cwd_policy: match cwd_policy {
            None => None,
            Some("skill-directory") => Some(CwdPolicy::SkillDirectory),
            Some("workspace") => Some(CwdPolicy::Workspace),
            Some("custom") => Some(CwdPolicy::Custom),
            Some(_) => return Err(FIELDS.validation_error("sandbox.cwd_policy is invalid.")),
        },
        env_allowlist: None,
        network,
        writable_paths,
        require_enforcement,
    })
}
