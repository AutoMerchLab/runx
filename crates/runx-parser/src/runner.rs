use std::collections::BTreeMap;

use runx_contracts::{JsonObject, JsonValue};
use serde::{Deserialize, Serialize};

use crate::skill::{
    CatalogMetadata, RunnerHarnessManifest, SkillRunnerDefinition, validate_catalog_metadata,
    validate_harness_manifest, validate_runner_definition,
};
use crate::{
    ParseError, ValidationError, assert_execution_profile_yaml_subset,
    json_fields::{self, JsonFieldReader},
};

const FIELDS: JsonFieldReader = JsonFieldReader::new("runner_manifest");
const MANIFEST_FIELDS: &[&str] = &[
    "skill", "version", "runx", "policy", "emits", "catalog", "runners", "harness",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawRunnerManifestIr {
    pub document: JsonObject,
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillRunnerManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runx: Option<runx_contracts::JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<runx_contracts::JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emits: Option<runx_contracts::JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<CatalogMetadata>,
    pub runners: BTreeMap<String, SkillRunnerDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<RunnerHarnessManifest>,
    pub raw: RawRunnerManifestIr,
}

pub fn parse_runner_manifest_yaml(yaml: &str) -> Result<RawRunnerManifestIr, ParseError> {
    assert_execution_profile_yaml_subset("runner_manifest", yaml)?;
    let parsed: JsonValue =
        serde_norway::from_str(yaml).map_err(|error| ParseError::InvalidYaml {
            field: "runner_manifest".to_owned(),
            message: error.to_string(),
        })?;
    let JsonValue::Object(document) = parsed else {
        return Err(ParseError::InvalidDocument {
            field: "runner_manifest".to_owned(),
            message: "Runner manifest YAML must parse to an object.".to_owned(),
        });
    };
    Ok(RawRunnerManifestIr {
        document,
        raw: yaml.to_owned(),
    })
}

pub fn validate_runner_manifest(
    raw: RawRunnerManifestIr,
) -> Result<SkillRunnerManifest, ValidationError> {
    FIELDS.reject_unknown_fields(&raw.document, "runner_manifest", MANIFEST_FIELDS)?;
    let runners_record = FIELDS.required_object(raw.document.get("runners"), "runners")?;
    let mut runners = BTreeMap::new();
    for (name, value) in runners_record {
        let JsonValue::Object(runner) = value else {
            return Err(FIELDS.validation_error(format!("runners.{name} must be an object.")));
        };
        runners.insert(
            name.clone(),
            validate_runner_definition(name, runner.clone())?,
        );
    }

    let harness = validate_harness_manifest(
        FIELDS.optional_object(raw.document.get("harness"), "harness")?,
        "harness",
    )?;
    validate_harness_runners(&harness, &runners)?;

    let catalog = validate_catalog_metadata(
        FIELDS.optional_object(raw.document.get("catalog"), "catalog")?,
        "catalog",
    )?;
    Ok(SkillRunnerManifest {
        skill: FIELDS.optional_string(raw.document.get("skill"), "skill")?,
        version: FIELDS.optional_string(raw.document.get("version"), "version")?,
        runx: FIELDS.optional_object(raw.document.get("runx"), "runx")?,
        policy: raw.document.get("policy").cloned(),
        emits: raw.document.get("emits").cloned(),
        catalog,
        runners,
        harness,
        raw,
    })
}

pub fn resolve_post_run_reflect_policy(
    runx: Option<&JsonObject>,
    field: &str,
) -> Result<String, ValidationError> {
    let post_run = FIELDS.optional_object(
        json_fields::field_value(runx, "post_run"),
        &format!("{field}.post_run"),
    )?;
    let reflect = FIELDS
        .optional_string(
            json_fields::field_value(post_run.as_ref(), "reflect"),
            &format!("{field}.post_run.reflect"),
        )?
        .unwrap_or_else(|| "never".to_owned());
    if matches!(reflect.as_str(), "auto" | "always" | "never") {
        return Ok(reflect);
    }
    Err(FIELDS.validation_error(format!(
        "{field}.post_run.reflect must be auto, always, or never."
    )))
}

fn validate_harness_runners(
    harness: &Option<RunnerHarnessManifest>,
    runners: &BTreeMap<String, SkillRunnerDefinition>,
) -> Result<(), ValidationError> {
    for entry in harness.iter().flat_map(|harness| harness.cases.iter()) {
        if let Some(runner) = &entry.runner {
            if !runners.contains_key(runner) {
                return Err(FIELDS.validation_error(format!(
                    "harness.cases runner {runner} is not declared in runners."
                )));
            }
        }
    }
    Ok(())
}
