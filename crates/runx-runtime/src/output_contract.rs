//! Runner-output and packet-contract verification shared by every execution
//! source. Agent, JavaScript, CLI, native-tool, and nested-skill results must
//! cross the same typed boundary before a successful receipt can be sealed.

use std::collections::BTreeMap;
use std::path::Path;

use runx_contracts::{
    JsonObject, JsonValue, Output, OutputField, output_contract_digest, validate_output_value,
};
use runx_parser::SkillArtifactContract;

use crate::RuntimeError;
use crate::adapter::{CONTRACT_VERIFICATION_METADATA, SkillOutput};

pub(crate) fn verified_runner_metadata_with_artifacts(
    skill_name: &str,
    payload: &JsonValue,
    raw_output: Option<&JsonObject>,
    artifacts: Option<&SkillArtifactContract>,
    skill_directory: &Path,
    env: &BTreeMap<String, String>,
) -> Result<JsonObject, RuntimeError> {
    let output = raw_output.map(parse_output_contract).transpose()?;
    verified_output_metadata_with_artifacts(
        skill_name,
        payload,
        output.as_ref(),
        artifacts,
        skill_directory,
        env,
    )
}

pub(crate) fn verified_output_metadata_with_artifacts(
    skill_name: &str,
    payload: &JsonValue,
    output: Option<&BTreeMap<String, OutputField>>,
    artifacts: Option<&SkillArtifactContract>,
    skill_directory: &Path,
    env: &BTreeMap<String, String>,
) -> Result<JsonObject, RuntimeError> {
    let has_packet_contract = artifact_packet_contracts(artifacts);
    if output.is_none() && !has_packet_contract {
        return Ok(JsonObject::new());
    }

    let mut verification = JsonObject::new();
    if let Some(output) = output {
        validate_output_value(Some(output), payload).map_err(|error| {
            RuntimeError::SkillFailed {
                skill_name: skill_name.to_owned(),
                message: format!("runner output contract violation at {error}"),
            }
        })?;
        let digest = output_contract_digest(Some(output))
            .map_err(|source| RuntimeError::json("hashing runner output contract", source))?;
        verification.insert(
            "output_contract_sha256".to_owned(),
            JsonValue::String(digest),
        );
    }

    let packet_schemas = crate::packet_validation::verify_declared_packets(
        payload,
        artifacts,
        skill_directory,
        env,
    )?;
    if !packet_schemas.is_empty() {
        verification.insert(
            "packet_schemas".to_owned(),
            JsonValue::Object(packet_schemas),
        );
    }

    if verification.is_empty() {
        return Ok(JsonObject::new());
    }
    Ok([(
        CONTRACT_VERIFICATION_METADATA.to_owned(),
        JsonValue::Object(verification),
    )]
    .into_iter()
    .collect())
}

pub(crate) fn attach_verified_metadata(
    output: &mut SkillOutput,
    mut metadata: JsonObject,
) -> Result<(), RuntimeError> {
    let Some(verification) = metadata.remove(CONTRACT_VERIFICATION_METADATA) else {
        return Ok(());
    };
    if !metadata.is_empty() {
        return Err(RuntimeError::ReceiptInvalid {
            message: "runner contract verification produced undeclared metadata".to_owned(),
        });
    }
    if output
        .metadata
        .insert(CONTRACT_VERIFICATION_METADATA.to_owned(), verification)
        .is_some()
    {
        return Err(RuntimeError::ReceiptInvalid {
            message: "runner output supplied duplicate contract verification metadata".to_owned(),
        });
    }
    Ok(())
}

fn parse_output_contract(raw: &JsonObject) -> Result<BTreeMap<String, OutputField>, RuntimeError> {
    let value = serde_json::to_value(JsonValue::Object(raw.clone()))
        .map_err(|source| RuntimeError::json("serializing runner output contract", source))?;
    let Output(output) = serde_json::from_value(value)
        .map_err(|source| RuntimeError::json("parsing runner output contract", source))?;
    Ok(output)
}

fn artifact_packet_contracts(artifacts: Option<&SkillArtifactContract>) -> bool {
    artifacts.is_some_and(|artifacts| {
        artifacts.packet.is_some()
            || artifacts
                .packets
                .as_ref()
                .is_some_and(|packets| !packets.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_declared_contract_produces_no_receipt_metadata() -> Result<(), RuntimeError> {
        assert!(
            verified_output_metadata_with_artifacts(
                "plain",
                &JsonValue::String("plain output".to_owned()),
                None,
                None,
                Path::new("."),
                &BTreeMap::new(),
            )?
            .is_empty()
        );
        Ok(())
    }
}
