// Module rationale: skill command keeps parse, inspect, registry provenance, and execution wiring together until the native skill UX settles.
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use runx_contracts::{JsonObject, JsonValue};
use runx_runtime::skill_front::{
    PreparedEntryProvenance, PreparedSkillRunApproval, PreparedSkillRunStatus,
};
use runx_runtime::{
    ManagedAgentPolicy, RUNX_DEVELOPMENT_AUTO_APPROVE_ENV,
    RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64_ENV, RUNX_RECEIPT_SIGN_ISSUER_TYPE_ENV,
    RUNX_RECEIPT_SIGN_KID_ENV, SkillCredentialContext, SkillRunRequest, WorkspaceEnv,
    development_auto_approve_requested, resolve_skill_credential_for_path,
};

mod credential;
mod inputs;
mod operator_context;
mod output;
mod parser;
mod provider_readiness;
mod resolver;

use credential::{
    inspect_context as inspect_credential_context, write_required as write_needs_credential,
};
use inputs::read_input_document;
use operator_context::write_operator_context;
use output::{SkillOutputResume, skill_result_exit_code, write_skill_output};
pub use parser::{parse_skill_plan, parse_skill_plan_with_workspace};
use provider_readiness::{
    append_text as append_provider_readiness_text, inspect as inspect_provider_readiness,
};
use resolver::{RegistryTrustState, ResolvedSkillRef, resolve_skill_ref_details};

#[derive(Debug, PartialEq)]
pub struct SkillPlan {
    pub action: SkillAction,
    pub skill_path: PathBuf,
    pub runner: Option<String>,
    pub receipt_dir: Option<PathBuf>,
    pub run_id: Option<String>,
    pub answers: Option<PathBuf>,
    pub registry: Option<String>,
    pub expected_digest: Option<String>,
    pub json: bool,
    pub non_interactive: bool,
    pub skip_operator_context: bool,
    pub full_operator_context: bool,
    pub approve_operator_context: Option<String>,
    pub inputs: BTreeMap<String, JsonValue>,
    pub input_document: Option<crate::document_input::DocumentInputSource>,
    /// Optional stored profile selector. Secret resolution happens only after
    /// the selected runner's manifest credential requirement is known.
    pub credential_profile: Option<String>,
    pub managed_agent: ManagedAgentPolicy,
}

#[derive(Debug, PartialEq)]
pub enum SkillAction {
    Inspect,
    Run,
}

// Function rationale: the top-level command path owns resolve/inspect/run/failure presentation in one explicit dispatch.
pub fn run_native_skill(plan: SkillPlan) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let workspace = match WorkspaceEnv::load_process(cwd) {
        Ok(workspace) => workspace,
        Err(error) => {
            return write_skill_failure(&error.to_string(), plan.json, "env_error", 1, None);
        }
    };
    run_native_skill_with_workspace(plan, &workspace)
}

// Function rationale: the top-level command path owns resolve/inspect/run/failure presentation in one explicit dispatch.
pub fn run_native_skill_with_workspace(plan: SkillPlan, workspace: &WorkspaceEnv) -> ExitCode {
    let cwd = workspace.cwd().to_path_buf();
    let mut env = workspace.env().clone();
    let development_auto_approve = match development_auto_approve_requested(&env, &cwd) {
        Ok(requested) => requested && !production_receipt_signing_configured(&env),
        Err(error) => {
            return write_skill_failure(&error.to_string(), plan.json, "config_error", 1, None);
        }
    };
    if development_auto_approve {
        env.insert(
            RUNX_DEVELOPMENT_AUTO_APPROVE_ENV.to_owned(),
            "true".to_owned(),
        );
    } else {
        env.remove(RUNX_DEVELOPMENT_AUTO_APPROVE_ENV);
    }
    let resume_skill_ref = plan.skill_path.to_string_lossy().into_owned();
    let resolved = match resolve_skill_ref_details(
        &plan.skill_path,
        &cwd,
        resolver::SkillResolverOptions {
            env: &env,
            registry: plan.registry.as_deref(),
            expected_digest: plan.expected_digest.as_deref(),
        },
    ) {
        Ok(skill_path) => skill_path,
        Err(error) => {
            return write_skill_failure(&error.to_string(), plan.json, "skill_error", 1, None);
        }
    };
    let skill_path = resolved.runnable_path.clone();
    let credential = match resolve_skill_credential_for_path(
        &skill_path,
        plan.runner.as_deref(),
        plan.credential_profile.as_deref(),
        workspace,
    ) {
        Ok(credential) => credential,
        Err(error) => {
            return write_skill_failure(
                &error.to_string(),
                plan.json,
                "credential_error",
                1,
                registry_provenance(&resolved),
            );
        }
    };
    if plan.action == SkillAction::Inspect {
        return write_skill_inspection(
            &skill_path,
            plan.runner.as_deref(),
            plan.json,
            registry_provenance(&resolved),
            credential.as_ref(),
            &env,
            &cwd,
        );
    }
    if let Some(context) = credential.as_ref()
        && !context.resolution.is_ready()
    {
        return write_needs_credential(&context.request, plan.json);
    }
    let inputs = match plan.input_document.as_ref() {
        Some(source) => match read_input_document(source, &env, &cwd) {
            Ok(inputs) => inputs,
            Err(error) => {
                return write_skill_failure(
                    &error,
                    plan.json,
                    "input_error",
                    1,
                    registry_provenance(&resolved),
                );
            }
        },
        None => plan.inputs.clone(),
    };
    let resume = SkillOutputResume {
        skill_ref: Some(&resume_skill_ref),
        selected_runner: plan.runner.as_deref(),
        receipt_dir: plan.receipt_dir.as_deref(),
        answers_path: plan.answers.as_deref(),
    };
    let request = SkillRunRequest {
        skill_path,
        receipt_dir: plan.receipt_dir.clone(),
        run_id: plan.run_id.clone(),
        answers_path: plan.answers.clone(),
        inputs,
        env,
        cwd,
        managed_agent: plan.managed_agent.clone(),
        local_credential: credential
            .as_ref()
            .and_then(|context| context.resolution.descriptor().cloned()),
    };
    let orchestrator = match crate::runtime::local_orchestrator() {
        Ok(orchestrator) => orchestrator,
        Err(error) => {
            return write_skill_failure(
                &format!("failed to initialize runtime effects: {error}"),
                plan.json,
                "skill_error",
                1,
                registry_provenance(&resolved),
            );
        }
    };
    let result = if plan.skip_operator_context {
        match plan.runner.as_deref() {
            Some(runner) => orchestrator.run_skill_with_runner(&request, runner),
            None => orchestrator.run_skill(&request),
        }
    } else {
        let mut prepared = match orchestrator.prepare_skill(
            request,
            plan.runner.as_deref(),
            prepared_entry_provenance(&resolved),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return write_skill_failure(
                    &error.to_string(),
                    plan.json,
                    "skill_error",
                    1,
                    registry_provenance(&resolved),
                );
            }
        };
        if let Err(error) = write_operator_context(prepared.report(), plan.full_operator_context) {
            return write_skill_failure(
                &error,
                plan.json,
                "skill_error",
                1,
                registry_provenance(&resolved),
            );
        }
        if prepared.report().status == PreparedSkillRunStatus::Blocked {
            return write_skill_failure(
                prepared
                    .report()
                    .blocked_reason
                    .as_deref()
                    .unwrap_or("operator context preparation blocked"),
                plan.json,
                "operator_context_blocked",
                1,
                registry_provenance(&resolved),
            );
        }
        if !prepared.requires_operator_approval() && plan.approve_operator_context.is_none() {
            if let Err(error) = prepared.admit_safe() {
                return write_skill_failure(
                    &error.to_string(),
                    plan.json,
                    "operator_context_admission_error",
                    1,
                    registry_provenance(&resolved),
                );
            }
            orchestrator.run_prepared_skill(&prepared)
        } else {
            match authorize_operator_context(
                &plan,
                prepared.digest(),
                &resume_skill_ref,
                development_auto_approve,
            ) {
                OperatorAuthorization::Approved(mode) => {
                    let actor = if mode == "development_auto_approve" {
                        "local_development_override".to_owned()
                    } else {
                        workspace
                            .env()
                            .get("USER")
                            .cloned()
                            .unwrap_or_else(|| "local_operator".to_owned())
                    };
                    if mode == "development_auto_approve" {
                        let _ignored = writeln!(
                            io::stderr(),
                            "Development override: operator context auto-approved"
                        );
                    }
                    if let Err(error) = prepared.approve(PreparedSkillRunApproval::now(actor, mode))
                    {
                        return write_skill_failure(
                            &error.to_string(),
                            plan.json,
                            "operator_context_approval_error",
                            1,
                            registry_provenance(&resolved),
                        );
                    }
                    orchestrator.run_prepared_skill(&prepared)
                }
                OperatorAuthorization::NeedsApproval => {
                    return write_operator_approval_required(prepared.digest(), plan.json);
                }
                OperatorAuthorization::Denied { message, code } => {
                    return write_skill_failure(
                        &message,
                        plan.json,
                        code,
                        1,
                        registry_provenance(&resolved),
                    );
                }
            }
        }
    };
    match result {
        Ok(mut result) => {
            attach_registry_provenance(&mut result.output, &resolved);
            let exit_code = skill_result_exit_code(&result.output);
            write_skill_output(&result.output, plan.json, exit_code, resume)
        }
        Err(error) => write_skill_failure(
            &error.to_string(),
            plan.json,
            "skill_error",
            1,
            registry_provenance(&resolved),
        ),
    }
}

enum OperatorAuthorization {
    Approved(&'static str),
    NeedsApproval,
    Denied { message: String, code: &'static str },
}

fn authorize_operator_context(
    plan: &SkillPlan,
    digest: &str,
    skill_ref: &str,
    development_auto_approve: bool,
) -> OperatorAuthorization {
    if let Some(approved) = plan.approve_operator_context.as_deref() {
        if approved == digest {
            return OperatorAuthorization::Approved("explicit_digest");
        }
        return OperatorAuthorization::Denied {
            message: format!(
                "operator context approval is stale for {skill_ref}: prepared {digest}; supplied {approved}. Review and approve the new digest"
            ),
            code: "operator_context_approval_mismatch",
        };
    }
    if development_auto_approve {
        return OperatorAuthorization::Approved("development_auto_approve");
    }
    if plan.non_interactive || !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return OperatorAuthorization::NeedsApproval;
    }
    let _ignored = write!(io::stderr(), "Run this prepared skill? [y/N] ");
    let _ignored = io::stderr().flush();
    let mut answer = String::new();
    match io::stdin().read_line(&mut answer) {
        Ok(_) if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") => {
            OperatorAuthorization::Approved("interactive_terminal")
        }
        Ok(_) => OperatorAuthorization::Denied {
            message: "operator context approval denied".to_owned(),
            code: "operator_context_approval_denied",
        },
        Err(error) => OperatorAuthorization::Denied {
            message: format!("failed to read operator context approval: {error}"),
            code: "operator_context_approval_error",
        },
    }
}

fn production_receipt_signing_configured(env: &BTreeMap<String, String>) -> bool {
    [
        RUNX_RECEIPT_SIGN_KID_ENV,
        RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64_ENV,
        RUNX_RECEIPT_SIGN_ISSUER_TYPE_ENV,
    ]
    .iter()
    .any(|name| env.get(*name).is_some_and(|value| !value.trim().is_empty()))
}

fn write_operator_approval_required(digest: &str, json: bool) -> ExitCode {
    let approval_flag = format!("--approve-operator-context {digest}");
    if json {
        let value = JsonValue::Object(JsonObject::from([
            (
                "schema".to_owned(),
                JsonValue::String("runx.operator_context_approval.v1".to_owned()),
            ),
            (
                "status".to_owned(),
                JsonValue::String("needs_operator_approval".to_owned()),
            ),
            ("digest".to_owned(), JsonValue::String(digest.to_owned())),
            (
                "approval_flag".to_owned(),
                JsonValue::String(approval_flag.clone()),
            ),
        ]));
        return write_skill_output(
            &value,
            true,
            ExitCode::from(2),
            SkillOutputResume {
                skill_ref: None,
                selected_runner: None,
                receipt_dir: None,
                answers_path: None,
            },
        );
    }
    let _ignored = writeln!(io::stdout(), "Approval required");
    let _ignored = writeln!(io::stdout(), "Rerun the same command with:");
    let _ignored = writeln!(io::stdout(), "  {approval_flag}");
    ExitCode::from(2)
}

fn prepared_entry_provenance(resolved: &ResolvedSkillRef) -> PreparedEntryProvenance {
    PreparedEntryProvenance {
        kind: match resolved.kind {
            resolver::SkillRefKind::ExplicitPath => "explicit_path",
            resolver::SkillRefKind::ExportedShim => "exported_shim",
            resolver::SkillRefKind::WorkspaceLocal => "workspace_local",
            resolver::SkillRefKind::Installed => "installed",
            resolver::SkillRefKind::Official => "official",
            resolver::SkillRefKind::Registry => "registry",
        }
        .to_owned(),
        reference: resolved.skill_id.clone(),
        source: resolved
            .registry_source
            .clone()
            .unwrap_or_else(|| "local-path".to_owned()),
        source_label: resolved
            .registry_source_fingerprint
            .clone()
            .unwrap_or_else(|| resolved.runnable_path.to_string_lossy().into_owned()),
        skill_id: resolved.skill_id.clone(),
        version: resolved.version.clone(),
        digest: resolved.digest.clone(),
        package_digest: None,
        trust_tier: resolved.trust_tier.clone(),
    }
}

fn write_skill_inspection(
    skill_path: &Path,
    runner: Option<&str>,
    json: bool,
    provenance: Option<JsonObject>,
    credential: Option<&SkillCredentialContext>,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> ExitCode {
    match inspect_skill(skill_path, runner, provenance, credential, env, cwd) {
        Ok(value) if json => crate::cli_io::write_stdout_code(
            &format!(
                "{}\n",
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned())
            ),
            0,
        ),
        Ok(value) => write_inspection_text(&value),
        Err(message) => write_skill_failure(&message, json, "skill_error", 1, None),
    }
}

// Function rationale: inspection assembles one public JSON contract from SKILL.md, X.yaml, fixtures, and selected runner metadata.
fn inspect_skill(
    skill_path: &Path,
    selected_runner: Option<&str>,
    provenance: Option<JsonObject>,
    credential: Option<&SkillCredentialContext>,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<JsonValue, String> {
    let mut output = runx_runtime::inspect_skill_package(skill_path, selected_runner)?;
    let JsonValue::Object(object) = &mut output else {
        return Err("native skill inspection returned a non-object".to_owned());
    };
    if let Some(provenance) = provenance {
        object.insert(
            "registry_provenance".to_owned(),
            JsonValue::Object(provenance),
        );
    }
    if object.get("runner").is_some()
        && let Some(provider) = inspect_provider_readiness(object, env, cwd)
    {
        let status = provider
            .as_object()
            .and_then(|value| value.get("status"))
            .and_then(JsonValue::as_str)
            .unwrap_or("provider_readiness_unknown")
            .to_owned();
        object.insert("provider".to_owned(), provider);
        object.insert(
            "readiness".to_owned(),
            JsonValue::Object(JsonObject::from([(
                "status".to_owned(),
                JsonValue::String(status),
            )])),
        );
    }
    if object.get("runner").is_some()
        && let Some(credential) = credential
    {
        let credential = inspect_credential_context(credential);
        let ready = credential
            .as_object()
            .and_then(|value| value.get("status"))
            .and_then(JsonValue::as_str)
            == Some("ready");
        object.insert("credential".to_owned(), credential);
        if !ready {
            object.insert(
                "readiness".to_owned(),
                JsonValue::Object(JsonObject::from([(
                    "status".to_owned(),
                    JsonValue::String("needs_credential".to_owned()),
                )])),
            );
        }
    }
    Ok(output)
}

// Function rationale: text rendering mirrors the inspect JSON shape and is kept adjacent to avoid presentation drift.
fn write_inspection_text(value: &JsonValue) -> ExitCode {
    let Some(object) = value.as_object() else {
        return crate::cli_io::write_stdout_code("{}\n", 0);
    };
    let mut out = String::new();
    out.push_str(&format!(
        "skill: {}\n",
        object_string(object, "name").unwrap_or("<unnamed>")
    ));
    if let Some(description) = object_string(object, "description") {
        out.push_str(&format!("description: {description}\n"));
    }
    if let Some(version) = object_string(object, "version") {
        out.push_str(&format!("version: {version}\n"));
    }
    if let Some(runner) = object.get("runner").and_then(JsonValue::as_object) {
        out.push_str(&format!(
            "runner: {}\n",
            object_string(runner, "name").unwrap_or("<unknown>")
        ));
        if let Some(kind) = object_string(runner, "type") {
            out.push_str(&format!("type: {kind}\n"));
        }
        if let Some(readiness) = object.get("readiness").and_then(JsonValue::as_object)
            && let Some(status) = object_string(readiness, "status")
        {
            out.push_str(&format!("readiness: {status}\n"));
        }
        append_provider_readiness_text(&mut out, object);
        if let Some(credential) = object.get("credential").and_then(JsonValue::as_object) {
            out.push_str(&format!(
                "credential: {} ({})\n",
                object_string(credential, "provider").unwrap_or("<unknown>"),
                object_string(credential, "status").unwrap_or("unknown")
            ));
        }
        if let Some(capabilities) = object.get("capabilities").and_then(JsonValue::as_object) {
            for key in ["execution", "completion", "requires_adapter", "approval"] {
                if let Some(value) = capabilities.get(key) {
                    out.push_str(&format!("{key}: {}\n", display_json_scalar(value)));
                }
            }
        }
        if let Some(inputs) = runner.get("inputs").and_then(JsonValue::as_array)
            && !inputs.is_empty()
        {
            out.push_str("inputs:\n");
            for input in inputs {
                if let Some(input) = input.as_object() {
                    let name = object_string(input, "name").unwrap_or("<unknown>");
                    let kind = object_string(input, "type").unwrap_or("json");
                    let required = input
                        .get("required")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false);
                    let marker = if required { "required" } else { "optional" };
                    out.push_str(&format!("  - {name}: {kind} ({marker})\n"));
                }
            }
        }
        if let Some(outputs) = runner.get("outputs").and_then(JsonValue::as_array)
            && !outputs.is_empty()
        {
            out.push_str("outputs:\n");
            for output in outputs {
                if let Some(output) = output.as_object() {
                    let name = object_string(output, "name").unwrap_or("<unknown>");
                    let kind = object_string(output, "type").unwrap_or("json");
                    out.push_str(&format!("  - {name}: {kind}\n"));
                }
            }
        }
        if let Some(examples) = object.get("examples").and_then(JsonValue::as_array)
            && !examples.is_empty()
        {
            out.push_str("examples:\n");
            for example in examples {
                if let Some(example) = example.as_str() {
                    out.push_str(&format!("  - {example}\n"));
                }
            }
        }
        if let Some(resume) = object.get("resume").and_then(JsonValue::as_object)
            && resume
                .get("may_pause")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        {
            out.push_str(&format!(
                "resume: {}\n",
                object_string(resume, "command").unwrap_or("runx resume <run-id> answers.json")
            ));
        }
        out.push_str("run: runx skill <skill> [runner]\n");
    } else if let Some(runners) = object.get("runners").and_then(JsonValue::as_array) {
        out.push_str("runners:\n");
        for runner in runners {
            if let Some(runner) = runner.as_str() {
                out.push_str(&format!("  - {runner}\n"));
            }
        }
        out.push_str("next: runx skill <skill> <runner>\n");
    }
    crate::cli_io::write_stdout_code(&out, 0)
}

fn display_json_scalar(value: &JsonValue) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned()))
}

fn object_string<'a>(object: &'a JsonObject, key: &str) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}

fn attach_registry_provenance(output: &mut JsonValue, resolved: &ResolvedSkillRef) {
    let Some(provenance) = registry_provenance(resolved) else {
        return;
    };
    let JsonValue::Object(object) = output else {
        return;
    };
    object.insert(
        "registry_provenance".to_owned(),
        JsonValue::Object(provenance),
    );
}

fn registry_provenance(resolved: &ResolvedSkillRef) -> Option<JsonObject> {
    let skill_id = resolved.skill_id.as_ref()?;
    let mut provenance = JsonObject::new();
    provenance.insert("skill_id".to_owned(), JsonValue::String(skill_id.clone()));
    insert_optional(&mut provenance, "version", resolved.version.as_ref());
    insert_optional(&mut provenance, "digest", resolved.digest.as_ref());
    insert_optional(
        &mut provenance,
        "profile_digest",
        resolved.profile_digest.as_ref(),
    );
    insert_optional(
        &mut provenance,
        "registry_source",
        resolved.registry_source.as_ref(),
    );
    insert_optional(
        &mut provenance,
        "registry_source_fingerprint",
        resolved.registry_source_fingerprint.as_ref(),
    );
    insert_optional(&mut provenance, "trust_tier", resolved.trust_tier.as_ref());
    insert_optional(
        &mut provenance,
        "registry_key_id",
        resolved.registry_key_id.as_ref(),
    );
    if matches!(
        resolved.trust_state.as_ref(),
        Some(RegistryTrustState::Trusted)
    ) {
        provenance.insert(
            "trust_state".to_owned(),
            JsonValue::String("trusted".to_owned()),
        );
    }
    Some(provenance)
}

fn insert_optional(object: &mut JsonObject, key: &str, value: Option<&String>) {
    if let Some(value) = value {
        object.insert(key.to_owned(), JsonValue::String(value.clone()));
    }
}

fn write_skill_failure(
    message: &str,
    json: bool,
    code: &str,
    exit_code: u8,
    provenance: Option<JsonObject>,
) -> ExitCode {
    if json {
        let output = skill_json_failure_output(message, code, provenance);
        return crate::cli_io::write_stdout_code(&output, exit_code);
    }
    let _ignored = writeln!(io::stderr(), "runx: {message}");
    ExitCode::from(exit_code)
}

fn skill_json_failure_output(message: &str, code: &str, provenance: Option<JsonObject>) -> String {
    let mut error = JsonObject::new();
    error.insert("message".to_owned(), JsonValue::String(message.to_owned()));
    error.insert("code".to_owned(), JsonValue::String(code.to_owned()));
    let mut output = JsonObject::new();
    output.insert("status".to_owned(), JsonValue::String("failure".to_owned()));
    output.insert("error".to_owned(), JsonValue::Object(error));
    if let Some(provenance) = provenance {
        output.insert(
            "registry_provenance".to_owned(),
            JsonValue::Object(provenance),
        );
    }
    serde_json::to_string_pretty(&JsonValue::Object(output))
        .map(|json| format!("{json}\n"))
        .unwrap_or_else(|_| crate::router::json_failure_output(message, code))
}
