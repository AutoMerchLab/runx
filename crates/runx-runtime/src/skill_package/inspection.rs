use std::path::Path;

use runx_contracts::{JsonObject, JsonValue};
use runx_parser::{
    CatalogMetadata, GraphStep, SkillInput, SkillRunnerDefinition, SourceKind,
    ValidatedSkillPackage,
};

use super::LoadedSkillPackage;

/// Project one already-validated package into the stable operator inspection
/// envelope. No source document is reparsed here.
pub fn inspect_skill_package(
    skill_path: &Path,
    selected_runner: Option<&str>,
) -> Result<JsonValue, String> {
    let loaded =
        super::load_validated_skill_package(skill_path).map_err(|error| error.to_string())?;
    inspect_loaded_skill_package(&loaded, selected_runner)
}

pub(crate) fn inspect_loaded_skill_package(
    loaded: &LoadedSkillPackage,
    selected_runner: Option<&str>,
) -> Result<JsonValue, String> {
    let mut output = base_inspection(loaded);
    let manifest = loaded.manifest();
    let runner = match (manifest, selected_runner) {
        (Some(manifest), Some(name)) => Some(
            manifest
                .runners
                .get(name)
                .ok_or_else(|| format!("skill has no runner '{name}'"))?,
        ),
        (Some(manifest), None) => manifest
            .runners
            .values()
            .find(|runner| runner.default)
            .or_else(|| {
                (manifest.runners.len() == 1)
                    .then(|| manifest.runners.values().next())
                    .flatten()
            }),
        (None, Some(name)) => return Err(format!("skill has no runner '{name}'")),
        (None, None) => None,
    };
    if let Some(runner) = runner {
        append_runner_inspection(&mut output, loaded, runner)?;
    }
    Ok(JsonValue::Object(output))
}

fn base_inspection(loaded: &LoadedSkillPackage) -> JsonObject {
    let package = &loaded.package;
    let mut output = JsonObject::from([
        (
            "schema".to_owned(),
            JsonValue::String("runx.skill.inspect.v1".to_owned()),
        ),
        ("status".to_owned(), JsonValue::String("ok".to_owned())),
        (
            "name".to_owned(),
            JsonValue::String(package.skill.name.clone()),
        ),
        (
            "skill_path".to_owned(),
            JsonValue::String(loaded.directory.to_string_lossy().into_owned()),
        ),
        (
            "manual_digest".to_owned(),
            JsonValue::String(package.manual_digest.clone()),
        ),
        (
            "package_digest".to_owned(),
            JsonValue::String(package.package_digest.clone()),
        ),
    ]);
    if let Some(description) = &package.skill.description {
        output.insert(
            "description".to_owned(),
            JsonValue::String(description.clone()),
        );
    }
    if let Some(manifest) = loaded.manifest() {
        if let Some(version) = &manifest.version {
            output.insert("version".to_owned(), JsonValue::String(version.clone()));
        }
        if let Some(capabilities) = manifest.catalog.as_ref().and_then(catalog_capabilities) {
            output.insert("capabilities".to_owned(), capabilities);
        }
        output.insert(
            "runners".to_owned(),
            JsonValue::Array(
                manifest
                    .runners
                    .keys()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    } else {
        output.insert("runners".to_owned(), JsonValue::Array(Vec::new()));
    }
    output
}

fn append_runner_inspection(
    output: &mut JsonObject,
    loaded: &LoadedSkillPackage,
    runner: &SkillRunnerDefinition,
) -> Result<(), String> {
    output.insert("runner".to_owned(), inspect_runner(runner)?);
    output.insert(
        "readiness".to_owned(),
        JsonValue::Object(JsonObject::from([(
            "status".to_owned(),
            JsonValue::String("ready".to_owned()),
        )])),
    );
    output.insert(
        "examples".to_owned(),
        JsonValue::Array(fixture_examples(
            &loaded.package,
            loaded.manifest(),
            &runner.name,
        )),
    );
    output.insert(
        "resume".to_owned(),
        JsonValue::Object(JsonObject::from([
            (
                "may_pause".to_owned(),
                JsonValue::Bool(matches!(
                    runner.source.source_type,
                    SourceKind::Agent | SourceKind::AgentStep | SourceKind::Graph
                )),
            ),
            (
                "command".to_owned(),
                JsonValue::String("runx resume <run-id> answers.json".to_owned()),
            ),
        ])),
    );
    Ok(())
}

fn inspect_runner(runner: &SkillRunnerDefinition) -> Result<JsonValue, String> {
    let mut output = JsonObject::from([
        ("name".to_owned(), JsonValue::String(runner.name.clone())),
        (
            "type".to_owned(),
            JsonValue::String(runner.source.source_type.as_str().to_owned()),
        ),
        (
            "inputs".to_owned(),
            JsonValue::Array(
                runner
                    .inputs
                    .iter()
                    .map(|(name, input)| inspect_input(name, input))
                    .collect(),
            ),
        ),
        (
            "outputs".to_owned(),
            JsonValue::Array(
                runner
                    .source
                    .outputs
                    .iter()
                    .flat_map(|outputs| outputs.iter())
                    .map(|(name, declaration)| inspect_output(name, declaration))
                    .collect(),
            ),
        ),
    ]);
    insert_runner_contract_metadata(&mut output, runner)?;
    let provider_requirements = inspect_provider_requirements(runner);
    if !provider_requirements.is_empty() {
        output.insert(
            "provider_requirements".to_owned(),
            JsonValue::Array(provider_requirements),
        );
    }
    Ok(JsonValue::Object(output))
}

fn insert_runner_contract_metadata(
    output: &mut JsonObject,
    runner: &SkillRunnerDefinition,
) -> Result<(), String> {
    if let Some(artifacts) = &runner.artifacts {
        output.insert(
            "artifacts".to_owned(),
            serde_json::to_value(artifacts)
                .and_then(serde_json::from_value)
                .map_err(|error| format!("serializing runner artifacts: {error}"))?,
        );
    }
    if let Some(allowed_tools) = &runner.allowed_tools {
        output.insert(
            "allowed_tools".to_owned(),
            JsonValue::Array(
                allowed_tools
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }
    if !runner.scopes.is_empty() {
        output.insert(
            "scopes".to_owned(),
            JsonValue::Array(
                runner
                    .scopes
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }
    if let Some(mutating) = runner.mutating {
        output.insert("mutating".to_owned(), JsonValue::Bool(mutating));
    }
    Ok(())
}

fn inspect_provider_requirements(runner: &SkillRunnerDefinition) -> Vec<JsonValue> {
    runner
        .source
        .graph
        .iter()
        .flat_map(|graph| graph.steps.iter())
        .filter_map(inspect_provider_requirement)
        .collect()
}

fn inspect_provider_requirement(step: &GraphStep) -> Option<JsonValue> {
    let tool = step.tool.as_deref()?;
    let access = match tool {
        "provider.read" => "read",
        "provider.mutate" => "mutate",
        _ => return None,
    };
    let provider = step.inputs.get("expected_provider")?.as_str()?;
    if provider.trim().is_empty() || provider.starts_with('$') {
        return None;
    }
    let mut requirement = JsonObject::from([
        ("step_id".to_owned(), JsonValue::String(step.id.clone())),
        (
            "provider".to_owned(),
            JsonValue::String(provider.to_owned()),
        ),
        ("access".to_owned(), JsonValue::String(access.to_owned())),
        (
            "scopes".to_owned(),
            JsonValue::Array(step.scopes.iter().cloned().map(JsonValue::String).collect()),
        ),
    ]);
    if let Some(operation) = step.inputs.get("operation").and_then(JsonValue::as_str) {
        requirement.insert(
            "operation".to_owned(),
            JsonValue::String(operation.to_owned()),
        );
    }
    Some(JsonValue::Object(requirement))
}

fn catalog_capabilities(catalog: &CatalogMetadata) -> Option<JsonValue> {
    Some(JsonValue::Object(JsonObject::from([
        (
            "execution".to_owned(),
            JsonValue::String(catalog.execution?.as_str().to_owned()),
        ),
        (
            "completion".to_owned(),
            JsonValue::String(catalog.completion?.as_str().to_owned()),
        ),
        (
            "requires_adapter".to_owned(),
            JsonValue::Bool(catalog.requires_adapter?),
        ),
        (
            "approval".to_owned(),
            JsonValue::String(catalog.approval?.as_str().to_owned()),
        ),
    ])))
}

fn inspect_input(name: &str, input: &SkillInput) -> JsonValue {
    let mut output = JsonObject::from([
        ("name".to_owned(), JsonValue::String(name.to_owned())),
        (
            "type".to_owned(),
            JsonValue::String(input.input_type.clone()),
        ),
        ("required".to_owned(), JsonValue::Bool(input.required)),
    ]);
    if let Some(description) = &input.description {
        output.insert(
            "description".to_owned(),
            JsonValue::String(description.clone()),
        );
    }
    JsonValue::Object(output)
}

fn inspect_output(name: &str, declaration: &JsonValue) -> JsonValue {
    let mut output = JsonObject::from([("name".to_owned(), JsonValue::String(name.to_owned()))]);
    match declaration {
        JsonValue::String(kind) => {
            output.insert("type".to_owned(), JsonValue::String(kind.clone()));
        }
        JsonValue::Object(details) => {
            if let Some(kind) = details.get("type").and_then(JsonValue::as_str) {
                output.insert("type".to_owned(), JsonValue::String(kind.to_owned()));
            }
            if let Some(required) = details.get("required").and_then(JsonValue::as_bool) {
                output.insert("required".to_owned(), JsonValue::Bool(required));
            }
        }
        _ => {}
    }
    JsonValue::Object(output)
}

fn fixture_examples(
    package: &ValidatedSkillPackage,
    manifest: Option<&runx_parser::SkillRunnerManifest>,
    runner: &str,
) -> Vec<JsonValue> {
    let mut examples = manifest
        .and_then(|manifest| manifest.harness.as_ref())
        .into_iter()
        .flat_map(|harness| harness.cases.iter())
        .filter(|case| case.runner.as_deref().is_none_or(|name| name == runner))
        .map(|case| JsonValue::String(case.name.clone()))
        .chain(
            package
                .source
                .files
                .keys()
                .filter(|path| path.starts_with("fixtures/") && path.ends_with(".yaml"))
                .cloned()
                .map(JsonValue::String),
        )
        .collect::<Vec<_>>();
    examples.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    examples.dedup();
    examples
}
