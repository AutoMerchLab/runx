use super::{
    CwdPolicy, RequiredSandboxDeclaration, SandboxAdmissionDecision, SandboxAdmissionOptions,
    SandboxDeclaration, SandboxProfile,
};

#[must_use]
pub fn normalize_sandbox_declaration(
    sandbox: Option<&SandboxDeclaration>,
) -> RequiredSandboxDeclaration {
    let Some(sandbox) = sandbox else {
        return RequiredSandboxDeclaration {
            profile: SandboxProfile::Readonly,
            cwd_policy: CwdPolicy::SkillDirectory,
            env_allowlist: None,
            network: false,
            writable_paths: Vec::new(),
            require_enforcement: true,
        };
    };

    RequiredSandboxDeclaration {
        profile: sandbox.profile.clone(),
        cwd_policy: sandbox
            .cwd_policy
            .clone()
            .unwrap_or(CwdPolicy::SkillDirectory),
        env_allowlist: sandbox.env_allowlist.clone(),
        network: sandbox
            .network
            .unwrap_or(matches!(sandbox.profile, SandboxProfile::Network)),
        writable_paths: sandbox.writable_paths.clone().unwrap_or_default(),
        require_enforcement: sandbox.require_enforcement.unwrap_or(!matches!(
            sandbox.profile,
            SandboxProfile::UnrestrictedLocalDev
        )),
    }
}

#[must_use]
pub fn sandbox_requires_approval(sandbox: Option<&SandboxDeclaration>) -> bool {
    matches!(
        normalize_sandbox_declaration(sandbox).profile,
        SandboxProfile::UnrestrictedLocalDev
    )
}

#[must_use]
pub fn is_reserved_runx_sandbox_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("RUNX_RECEIPT_SIGN_")
        || upper.starts_with("RUNX_INTERNAL_PREPARED_")
        || matches!(
            upper.as_str(),
            "RUNX_AGENT_API_KEY"
                | "RUNX_HOSTED_API_ADMIN_TOKEN"
                | "RUNX_HOSTED_CREDENTIAL_HANDLES_JSON"
                | "RUNX_HOSTED_REGISTRY_PUBLISH_TOKEN"
                | "RUNX_PAYMENT_ADMISSION_SIGNING_KEY"
                | "RUNX_PROVIDER_ADMISSION_SIGNING_KEY"
                | "RUNX_PUBLIC_API_TOKEN"
                | "RUNX_REGISTRY_MANIFEST_TRUST_KEY_BASE64"
        )
}

#[must_use]
pub fn admit_sandbox(
    sandbox: Option<&SandboxDeclaration>,
    options: &SandboxAdmissionOptions,
) -> SandboxAdmissionDecision {
    let declaration = normalize_sandbox_declaration(sandbox);
    let mut reasons = Vec::new();

    collect_profile_violations(&declaration, &mut reasons);

    if !reasons.is_empty() {
        return SandboxAdmissionDecision::Deny { reasons };
    }

    if requires_unapproved_escalation(&declaration, options) {
        return SandboxAdmissionDecision::ApprovalRequired {
            reasons: vec![
                "unrestricted-local-dev sandbox requires explicit caller approval".to_owned(),
            ],
        };
    }

    SandboxAdmissionDecision::Allow {
        reasons: vec![format!(
            "sandbox profile '{}' admitted",
            sandbox_profile_name(&declaration.profile)
        )],
    }
}

fn collect_profile_violations(declaration: &RequiredSandboxDeclaration, reasons: &mut Vec<String>) {
    collect_reserved_env_allowlist_violations(declaration, reasons);

    if matches!(declaration.profile, SandboxProfile::Readonly) {
        if !declaration.writable_paths.is_empty() {
            reasons.push("readonly sandbox cannot declare writable paths".to_owned());
        }
        if declaration.network {
            reasons.push("readonly sandbox cannot declare network access".to_owned());
        }
    }

    if matches!(declaration.profile, SandboxProfile::WorkspaceWrite) {
        collect_unsafe_writable_paths(declaration, reasons);
    }

    if matches!(declaration.profile, SandboxProfile::Network)
        && !declaration.writable_paths.is_empty()
    {
        reasons.push("network sandbox cannot declare writable paths; use unrestricted-local-dev for combined local write and network access".to_owned());
    }
}

fn collect_reserved_env_allowlist_violations(
    declaration: &RequiredSandboxDeclaration,
    reasons: &mut Vec<String>,
) {
    let Some(env_allowlist) = declaration.env_allowlist.as_ref() else {
        return;
    };
    let denied = env_allowlist
        .iter()
        .filter(|name| is_reserved_runx_sandbox_env_name(name))
        .cloned()
        .collect::<Vec<_>>();
    if denied.is_empty() {
        return;
    }
    reasons.push(format!(
        "sandbox env_allowlist contains reserved runx environment variable(s): {}",
        denied.join(", ")
    ));
}

fn collect_unsafe_writable_paths(
    declaration: &RequiredSandboxDeclaration,
    reasons: &mut Vec<String>,
) {
    let unsafe_paths = declaration
        .writable_paths
        .iter()
        .filter(|path| is_unsafe_writable_path(path))
        .cloned()
        .collect::<Vec<_>>();

    if !unsafe_paths.is_empty() {
        reasons.push(format!(
            "workspace-write sandbox has unsafe writable path(s): {}",
            unsafe_paths.join(", ")
        ));
    }
}

fn requires_unapproved_escalation(
    declaration: &RequiredSandboxDeclaration,
    options: &SandboxAdmissionOptions,
) -> bool {
    matches!(declaration.profile, SandboxProfile::UnrestrictedLocalDev)
        && !options.approved_escalation.unwrap_or(false)
        && !options.skip_escalation.unwrap_or(false)
}

fn is_unsafe_writable_path(value: &str) -> bool {
    value.is_empty() || value.split(['/', '\\']).any(|segment| segment == "..")
}

fn sandbox_profile_name(profile: &SandboxProfile) -> &'static str {
    match profile {
        SandboxProfile::Readonly => "readonly",
        SandboxProfile::WorkspaceWrite => "workspace-write",
        SandboxProfile::Network => "network",
        SandboxProfile::UnrestrictedLocalDev => "unrestricted-local-dev",
    }
}

#[cfg(test)]
mod tests {
    use super::{admit_sandbox, is_reserved_runx_sandbox_env_name, normalize_sandbox_declaration};
    use crate::policy::{
        SandboxAdmissionDecision, SandboxAdmissionOptions, SandboxDeclaration, SandboxProfile,
    };

    #[test]
    fn normalize_defaults_match_typescript() {
        let declaration = normalize_sandbox_declaration(None);

        assert_eq!(declaration.profile, SandboxProfile::Readonly);
        assert!(!declaration.network);
        assert!(declaration.writable_paths.is_empty());
    }

    #[test]
    fn unrestricted_local_dev_requires_approval() {
        let sandbox = SandboxDeclaration {
            profile: SandboxProfile::UnrestrictedLocalDev,
            cwd_policy: None,
            env_allowlist: None,
            network: None,
            writable_paths: None,
            require_enforcement: None,
        };

        assert_eq!(
            admit_sandbox(Some(&sandbox), &SandboxAdmissionOptions::default()),
            SandboxAdmissionDecision::ApprovalRequired {
                reasons: vec![
                    "unrestricted-local-dev sandbox requires explicit caller approval".to_owned()
                ]
            }
        );
    }

    #[test]
    fn reserved_sandbox_env_names_cover_exact_runtime_owned_material() {
        assert!(is_reserved_runx_sandbox_env_name(
            "RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64"
        ));
        assert!(is_reserved_runx_sandbox_env_name("RUNX_AGENT_API_KEY"));
        assert!(!is_reserved_runx_sandbox_env_name("RUNX_GIT_ASKPASS_TOKEN"));
        assert!(is_reserved_runx_sandbox_env_name(
            "RUNX_PROVIDER_ADMISSION_SIGNING_KEY"
        ));
        assert!(!is_reserved_runx_sandbox_env_name("RUNX_CWD"));
        assert!(!is_reserved_runx_sandbox_env_name("RUNX_MCP_SCOPE"));
        assert!(is_reserved_runx_sandbox_env_name(
            "RUNX_REGISTRY_MANIFEST_TRUST_KEY_BASE64"
        ));
        assert!(!is_reserved_runx_sandbox_env_name("PATH"));
    }

    #[test]
    fn sandbox_admission_denies_reserved_env_allowlist_names() {
        let sandbox = SandboxDeclaration {
            profile: SandboxProfile::Readonly,
            cwd_policy: None,
            env_allowlist: Some(vec![
                "PATH".to_owned(),
                "RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64".to_owned(),
            ]),
            network: None,
            writable_paths: None,
            require_enforcement: None,
        };

        assert_eq!(
            admit_sandbox(Some(&sandbox), &SandboxAdmissionOptions::default()),
            SandboxAdmissionDecision::Deny {
                reasons: vec![
                    "sandbox env_allowlist contains reserved runx environment variable(s): RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64".to_owned()
                ]
            }
        );
    }
}
