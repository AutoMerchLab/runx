use super::{SkillRunError, invalid};
#[cfg(feature = "cli-tool")]
use super::{contract_json_value, identifier_segment, seal_skill_output, sealed_output};

use std::collections::BTreeMap;
use std::path::Path;

use runx_contracts::{JsonObject, JsonValue};
use runx_parser::{SkillRunnerDefinition, SkillRunnerManifest};
#[cfg(feature = "cli-tool")]
use sha2::{Digest, Sha256};

use crate::adapter::SkillInvocation;
#[cfg(feature = "cli-tool")]
use crate::adapter::{SkillAdapter, SkillOutput};
#[cfg(feature = "cli-tool")]
use crate::adapters::cli_tool::CliToolAdapter;
#[cfg(feature = "cli-tool")]
use crate::adapters::javascript::JavaScriptAdapter;
use crate::execution::orchestrator::SkillRunRequest;
#[cfg(feature = "cli-tool")]
use crate::output_contract::{attach_verified_metadata, verified_runner_metadata_with_artifacts};
#[cfg(feature = "cli-tool")]
use crate::receipts::StepSealClosure;
use crate::services::{ReceiptServices, WorkspaceEnv};
#[cfg(feature = "cli-tool")]
use runx_contracts::ClosureDisposition;

#[cfg(test)]
mod credential_tests;

pub(crate) fn selected_runner<'a>(
    manifest: &'a SkillRunnerManifest,
    requested: Option<&str>,
) -> Result<&'a SkillRunnerDefinition, SkillRunError> {
    if let Some(name) = requested {
        return manifest
            .runners
            .get(name)
            .ok_or_else(|| invalid(format!("runner {name} is not declared in the manifest")));
    }
    let defaults = manifest
        .runners
        .values()
        .filter(|runner| runner.default)
        .collect::<Vec<_>>();
    match defaults.as_slice() {
        [runner] => Ok(*runner),
        [] if manifest.runners.len() == 1 => manifest
            .runners
            .values()
            .next()
            .ok_or_else(|| invalid("runner manifest declares no runners")),
        [] => Err(invalid("runner manifest has no default runner")),
        _ => Err(invalid("runner manifest declares multiple default runners")),
    }
}

pub(super) fn runner_invocation(
    skill_dir: &Path,
    runner: &SkillRunnerDefinition,
    inputs: &BTreeMap<String, JsonValue>,
    env: &BTreeMap<String, String>,
    local_credential: Option<&crate::execution::orchestrator::LocalCredentialDescriptor>,
) -> Result<SkillInvocation, SkillRunError> {
    if !matches!(
        runner.source.source_type.as_str(),
        "agent" | "agent-task" | "cli-tool" | "javascript" | "graph"
    ) {
        return Err(invalid(format!(
            "runx skill native execution only supports agent, agent-task, graph, cli-tool, and javascript runners, got {}",
            runner.source.source_type
        )));
    }
    let credential_delivery = credential_delivery_from_invocation(env, local_credential)?;
    Ok(SkillInvocation {
        skill_name: runner.name.clone(),
        source: runner.source.clone(),
        artifacts: runner.artifacts.clone(),
        allowed_tools: runner.allowed_tools.clone(),
        inputs: inputs.clone().into_iter().collect(),
        resolved_inputs: JsonObject::new(),
        current_context: Vec::new(),
        skill_directory: skill_dir.to_path_buf(),
        env: env.clone(),
        credential_delivery,
    })
}

pub(super) fn credential_delivery_from_invocation(
    env: &BTreeMap<String, String>,
    local_credential: Option<&crate::execution::orchestrator::LocalCredentialDescriptor>,
) -> Result<crate::credentials::CredentialDelivery, SkillRunError> {
    let hosted_handles = env
        .get(crate::credentials::RUNX_HOSTED_CREDENTIAL_HANDLES_JSON_ENV)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty());
    if let Some(descriptor) = local_credential {
        return crate::credentials::CredentialDelivery::from_local_descriptor(
            descriptor.provider.clone(),
            descriptor.auth_mode.clone(),
            descriptor.env_var.clone(),
            descriptor.material_ref.clone(),
            descriptor.scopes.clone(),
            descriptor.secret.clone(),
        )
        .and_then(|delivery| delivery.bind_audience(descriptor.audience.as_deref()))
        .map_err(|error| invalid(format!("local credential provision failed: {error}")));
    }
    if let Some(raw) = hosted_handles {
        return crate::credentials::CredentialDelivery::from_hosted_handles_json(raw).map_err(
            |error| {
                invalid(format!(
                    "hosted credential handle admission failed: {error}"
                ))
            },
        );
    }
    Ok(crate::credentials::CredentialDelivery::none())
}

#[cfg(feature = "cli-tool")]
pub(super) fn execute_process_skill_run(
    request: &SkillRunRequest,
    workspace: &WorkspaceEnv,
    receipts: &ReceiptServices,
    manifest: &SkillRunnerManifest,
    runner: &SkillRunnerDefinition,
    invocation: SkillInvocation,
) -> Result<JsonValue, SkillRunError> {
    if request.answers_path.is_some() {
        return Err(invalid(
            "process-backed runners do not support continuation answers",
        ));
    }
    let run_id = request
        .run_id
        .clone()
        .unwrap_or_else(|| process_run_id(runner, &request.inputs));
    let ProcessAdapterOutput {
        output,
        payload,
        source_type,
    } = invoke_process_adapter(runner, invocation)?;
    let disposition = if output.succeeded() {
        ClosureDisposition::Closed
    } else {
        ClosureDisposition::Failed
    };
    let receipt = seal_skill_output(
        &run_id,
        runner,
        &output,
        StepSealClosure {
            reason_code: format!("process_{}", disposition.label()),
            summary: format!("{} {} completed", source_type.as_str(), runner.name),
            disposition,
        },
        receipts.signature_config(),
        workspace.env(),
    )?;
    write_skill_receipt(request, workspace, receipts, &receipt)?;
    Ok(JsonValue::Object(sealed_output(
        manifest,
        &run_id,
        &output,
        &payload,
        &receipt,
        contract_json_value(&receipt)?,
    )))
}

#[cfg(feature = "cli-tool")]
struct ProcessAdapterOutput {
    output: SkillOutput,
    payload: JsonValue,
    source_type: runx_parser::SourceKind,
}

#[cfg(feature = "cli-tool")]
fn invoke_process_adapter(
    runner: &SkillRunnerDefinition,
    invocation: SkillInvocation,
) -> Result<ProcessAdapterOutput, SkillRunError> {
    let credential_observation = invocation.credential_delivery.public_observation().cloned();
    let skill_directory = invocation.skill_directory.clone();
    let invocation_env = invocation.env.clone();
    let source_type = invocation.source.source_type;
    let mut output = match source_type {
        runx_parser::SourceKind::CliTool => CliToolAdapter.invoke(invocation)?,
        runx_parser::SourceKind::JavaScript => JavaScriptAdapter::default().invoke(invocation)?,
        _ => {
            return Err(invalid(format!(
                "process runner does not support source type {source_type}"
            )));
        }
    };
    if let Some(observation) = &credential_observation {
        output.record_credential_observation(observation)?;
    }
    let payload = parse_output_payload(&output.stdout);
    if output.succeeded() {
        let metadata = verified_runner_metadata_with_artifacts(
            &runner.name,
            &payload,
            runner.source.outputs.as_ref(),
            runner.artifacts.as_ref(),
            &skill_directory,
            &invocation_env,
        )?;
        attach_verified_metadata(&mut output, metadata)?;
    }
    Ok(ProcessAdapterOutput {
        output,
        payload,
        source_type,
    })
}

#[cfg(not(feature = "cli-tool"))]
pub(super) fn execute_process_skill_run(
    _request: &SkillRunRequest,
    _workspace: &WorkspaceEnv,
    _receipts: &ReceiptServices,
    _manifest: &SkillRunnerManifest,
    _runner: &SkillRunnerDefinition,
    _invocation: SkillInvocation,
) -> Result<JsonValue, SkillRunError> {
    Err(invalid(
        "runx skill cli-tool execution is unavailable because runx-runtime was built without the cli-tool feature",
    ))
}

pub(super) fn write_skill_receipt(
    request: &SkillRunRequest,
    workspace: &WorkspaceEnv,
    receipts: &ReceiptServices,
    receipt: &runx_contracts::Receipt,
) -> Result<(), SkillRunError> {
    let receipt_path = receipts.resolve_path(workspace, request.receipt_dir.as_deref(), None);
    receipts
        .write_local_receipt(receipt, &receipt_path)
        .map_err(Into::into)
}

#[cfg(feature = "cli-tool")]
fn process_run_id(runner: &SkillRunnerDefinition, inputs: &BTreeMap<String, JsonValue>) -> String {
    let input_bytes = serde_json::to_vec(inputs).unwrap_or_default();
    let digest = Sha256::digest(input_bytes);
    format!(
        "run_{}_{}",
        identifier_segment(&runner.name),
        hex_prefix(&digest, 12)
    )
}

#[cfg(feature = "cli-tool")]
fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    let full = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    full.chars().take(chars).collect()
}

#[cfg(feature = "cli-tool")]
fn parse_output_payload(stdout: &str) -> JsonValue {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return JsonValue::String(String::new());
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| JsonValue::String(trimmed.to_owned()))
}
