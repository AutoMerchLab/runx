use std::collections::BTreeMap;
use std::fs;

use runx_parser::{SkillSource, SourceKind};

use super::*;
use crate::credentials::CredentialDelivery;

#[test]
fn rejects_credentials_before_loading_a_module() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut request = invocation(directory.path());
    request.credential_delivery = CredentialDelivery::from_local_descriptor(
        "example",
        "token",
        "EXAMPLE_TOKEN",
        "local:test",
        vec!["example:read".to_owned()],
        "secret",
    )?;
    let error = JavaScriptAdapter::default()
        .invoke(request)
        .err()
        .map(|error| error.to_string());
    assert!(error.is_some_and(|message| message.contains("cannot receive credentials")));
    Ok(())
}

#[test]
fn rejects_author_selected_sandbox_controls() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut request = invocation(directory.path());
    request.source.sandbox = Some(runx_parser::SkillSandbox {
        profile: runx_core::policy::SandboxProfile::Readonly,
        cwd_policy: None,
        env_allowlist: None,
        network: Some(false),
        writable_paths: Vec::new(),
        require_enforcement: Some(true),
        approved_escalation: None,
        raw: JsonObject::new(),
    });
    let error = JavaScriptAdapter::default()
        .invoke(request)
        .err()
        .map(|error| error.to_string());
    assert!(error.is_some_and(|message| message.contains("runtime owns")));
    Ok(())
}

fn invocation(skill_directory: &std::path::Path) -> SkillInvocation {
    let _ = fs::create_dir_all(skill_directory);
    SkillInvocation {
        skill_name: "javascript-test".to_owned(),
        artifacts: None,
        allowed_tools: None,
        source: SkillSource {
            source_type: SourceKind::JavaScript,
            command: None,
            module: Some("domain.mjs".to_owned()),
            javascript_export: None,
            pages: None,
            args: Vec::new(),
            cwd: None,
            timeout_seconds: None,
            input_mode: None,
            sandbox: None,
            server: None,
            tool: None,
            arguments: None,
            agent_card_url: None,
            agent_identity: None,
            agent: None,
            task: None,
            outputs: None,
            graph: None,
            external_adapter: None,
            thread_outbox_provider: None,
            act: None,
            raw: JsonObject::new(),
        },
        inputs: JsonObject::new(),
        resolved_inputs: JsonObject::new(),
        current_context: Vec::new(),
        skill_directory: skill_directory.to_path_buf(),
        env: BTreeMap::new(),
        credential_delivery: CredentialDelivery::none(),
    }
}
