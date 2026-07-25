use super::{
    SkillRunError, SkillSealContext, SkillSourceAdapter, identifier_segment, invalid, sealed_output,
};

use std::collections::BTreeMap;
use std::path::Path;

use runx_contracts::{JsonObject, JsonValue};
use runx_parser::{SkillRunnerDefinition, SkillRunnerManifest};
use sha2::{Digest, Sha256};

use crate::adapter::{InvocationOutput, SkillAdapter, SkillInvocation};
use crate::execution::orchestrator::SkillRunRequest;
use crate::output_contract::{attach_verified_metadata, verified_runner_metadata_with_artifacts};
use crate::receipts::StepSealClosure;
use crate::services::{ReceiptServices, WorkspaceEnv};
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
    manifest: &SkillRunnerManifest,
    runner: &SkillRunnerDefinition,
    inputs: &BTreeMap<String, JsonValue>,
    env: &BTreeMap<String, String>,
    local_credential: Option<&crate::execution::orchestrator::LocalCredentialDescriptor>,
) -> Result<SkillInvocation, SkillRunError> {
    let credential_delivery = credential_delivery_from_invocation(env, local_credential)?;
    Ok(SkillInvocation {
        skill_name: runner.name.clone(),
        step_id: None,
        source: runner.source.clone(),
        requirements: manifest.execution_requirements(runner),
        artifacts: runner.artifacts.clone(),
        allowed_tools: runner.allowed_tools.clone(),
        inputs: inputs.clone().into_iter().collect(),
        resolved_inputs: JsonObject::new(),
        current_context: Vec::new(),
        provenance: Vec::new(),
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

pub(super) fn execute_adapter_skill_run(
    request: &SkillRunRequest,
    workspace: &WorkspaceEnv,
    receipts: &ReceiptServices,
    manifest: &SkillRunnerManifest,
    runner: &SkillRunnerDefinition,
    invocation: SkillInvocation,
) -> Result<JsonValue, SkillRunError> {
    if request.answers_path.is_some() {
        return Err(invalid(
            "native adapter runners do not support continuation answers",
        ));
    }
    let run_id = request
        .run_id
        .clone()
        .unwrap_or_else(|| adapter_run_id(runner, &request.inputs));
    let AdapterOutput {
        output,
        payload,
        source_type,
    } = invoke_source_adapter(runner, invocation)?;
    let disposition = if output.succeeded() {
        ClosureDisposition::Closed
    } else {
        ClosureDisposition::Failed
    };
    let reason_family = match source_type {
        runx_parser::SourceKind::CliTool | runx_parser::SourceKind::JavaScript => "process",
        _ => "adapter",
    };
    let receipt = SkillSealContext::from_services(&run_id, runner, receipts, workspace)
        .seal_output(
            &output,
            None,
            StepSealClosure {
                reason_code: format!("{reason_family}_{}", disposition.label()),
                summary: format!("{} {} completed", source_type.as_str(), runner.name),
                disposition,
            },
            None,
        )?;
    write_skill_receipt(request, workspace, receipts, &receipt)?;
    Ok(JsonValue::Object(sealed_output(
        manifest, &run_id, &output, &payload, None, None, &receipt,
    )))
}

struct AdapterOutput {
    output: InvocationOutput,
    payload: JsonValue,
    source_type: runx_parser::SourceKind,
}

fn invoke_source_adapter(
    runner: &SkillRunnerDefinition,
    invocation: SkillInvocation,
) -> Result<AdapterOutput, SkillRunError> {
    let skill_directory = invocation.skill_directory.clone();
    let invocation_env = invocation.env.clone();
    let source_type = invocation.source.source_type;
    let mut output = SkillSourceAdapter::default().invoke(invocation)?;
    let payload = output.value.clone();
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
    Ok(AdapterOutput {
        output,
        payload,
        source_type,
    })
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

fn adapter_run_id(runner: &SkillRunnerDefinition, inputs: &BTreeMap<String, JsonValue>) -> String {
    let input_bytes = serde_json::to_vec(inputs).unwrap_or_default();
    let digest = Sha256::digest(input_bytes);
    format!(
        "run_{}_{}",
        identifier_segment(&runner.name),
        hex_prefix(&digest, 12)
    )
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    let full = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    full.chars().take(chars).collect()
}
