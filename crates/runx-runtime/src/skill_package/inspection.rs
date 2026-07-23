use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

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
        if let Some(catalog) = manifest.catalog.as_ref() {
            output.insert("catalog".to_owned(), inspect_catalog(catalog));
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
        "execution_closure".to_owned(),
        inspect_execution_closure(loaded, runner)?,
    );
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

fn inspect_catalog(catalog: &CatalogMetadata) -> JsonValue {
    let mut output = JsonObject::from([
        (
            "kind".to_owned(),
            JsonValue::String(catalog.kind.as_str().to_owned()),
        ),
        (
            "audience".to_owned(),
            JsonValue::String(catalog.audience.as_str().to_owned()),
        ),
        (
            "visibility".to_owned(),
            JsonValue::String(catalog.visibility.as_str().to_owned()),
        ),
        (
            "role".to_owned(),
            JsonValue::String(catalog.role.as_str().to_owned()),
        ),
    ]);
    if let Some(canonical_skill) = &catalog.canonical_skill {
        output.insert(
            "canonical_skill".to_owned(),
            JsonValue::String(canonical_skill.clone()),
        );
    }
    if let Some(provider) = &catalog.provider {
        output.insert("provider".to_owned(), JsonValue::String(provider.clone()));
    }
    if let Some(runtime_path) = &catalog.runtime_path {
        output.insert(
            "runtime_path".to_owned(),
            JsonValue::String(runtime_path.clone()),
        );
    }
    JsonValue::Object(output)
}

#[derive(Default)]
struct ExecutionClosure {
    components: BTreeSet<String>,
    skill_edges: BTreeSet<String>,
    profiles: BTreeSet<String>,
    agent_acts: usize,
    declared_artifact: bool,
}

fn inspect_execution_closure(
    loaded: &LoadedSkillPackage,
    runner: &SkillRunnerDefinition,
) -> Result<JsonValue, String> {
    let mut closure = ExecutionClosure::default();
    let mut visited = BTreeSet::new();
    walk_runner_execution(loaded, "X.yaml", runner, &mut closure, &mut visited)?;
    let components = closure.components.into_iter().collect::<Vec<_>>();
    let summary = execution_summary(&components, closure.agent_acts, closure.declared_artifact);
    let agent_acts = u64::try_from(closure.agent_acts).unwrap_or(u64::MAX);
    Ok(JsonValue::Object(JsonObject::from([
        ("summary".to_owned(), JsonValue::String(summary)),
        (
            "components".to_owned(),
            JsonValue::Array(components.into_iter().map(JsonValue::String).collect()),
        ),
        (
            "skill_edges".to_owned(),
            JsonValue::Array(
                closure
                    .skill_edges
                    .into_iter()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
        (
            "agent_acts".to_owned(),
            JsonValue::Number(runx_contracts::JsonNumber::U64(agent_acts)),
        ),
        (
            "declared_artifact".to_owned(),
            JsonValue::Bool(closure.declared_artifact),
        ),
        (
            "profiles".to_owned(),
            JsonValue::Array(
                closure
                    .profiles
                    .into_iter()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
    ])))
}

fn walk_runner_execution(
    loaded: &LoadedSkillPackage,
    profile_path: &str,
    runner: &SkillRunnerDefinition,
    closure: &mut ExecutionClosure,
    visited: &mut BTreeSet<String>,
) -> Result<(), String> {
    let identity_directory = loaded.directory.canonicalize().map_err(|error| {
        format!(
            "canonicalizing inspected skill {}: {error}",
            loaded.directory.display()
        )
    })?;
    let identity = format!("{}#{}", identity_directory.display(), runner.name);
    if !visited.insert(identity) {
        return Ok(());
    }
    closure
        .profiles
        .insert(format!("{profile_path}#{}", runner.name));
    walk_source_execution(
        loaded,
        profile_path,
        &runner.source,
        runner.artifacts.is_some(),
        closure,
        visited,
    )?;
    Ok(())
}

fn walk_source_execution(
    loaded: &LoadedSkillPackage,
    profile_path: &str,
    source: &runx_parser::SkillSource,
    declared_artifact: bool,
    closure: &mut ExecutionClosure,
    visited: &mut BTreeSet<String>,
) -> Result<(), String> {
    match source.source_type {
        SourceKind::Graph => {
            let graph = source
                .graph
                .as_ref()
                .ok_or_else(|| "graph source omitted its validated graph".to_owned())?;
            for step in &graph.steps {
                if let Some(tool) = &step.tool {
                    closure.components.insert(format!("tool:{tool}"));
                }
                if let Some(reference) = &step.skill {
                    if let Some(nested) = load_local_referenced_skill(loaded, reference)? {
                        let nested_profile = nested_profile_path(profile_path, reference)?;
                        let nested_runner = select_inspection_runner(
                            nested.manifest().ok_or_else(|| {
                                format!(
                                    "sub-skill {} has no executable manifest",
                                    nested.directory.display()
                                )
                            })?,
                            step.runner.as_deref(),
                        )
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "sub-skill {} has no selected runner for step {}",
                                nested.directory.display(),
                                step.id
                            )
                        })?;
                        closure.skill_edges.insert(format!(
                            "{}#{}",
                            nested.package.skill.name, nested_runner.name
                        ));
                        walk_runner_execution(
                            &nested,
                            &nested_profile,
                            &nested_runner,
                            closure,
                            visited,
                        )?;
                    } else {
                        closure.skill_edges.insert(format!(
                            "{reference}#{}",
                            step.runner.as_deref().unwrap_or("default")
                        ));
                    }
                }
                if let Some(run) = &step.run
                    && let Some(run_source) = run.source()
                {
                    walk_source_execution(
                        loaded,
                        profile_path,
                        run_source,
                        step.artifacts.is_some(),
                        closure,
                        visited,
                    )?;
                }
            }
        }
        SourceKind::Agent | SourceKind::AgentStep => {
            closure.agent_acts = closure.agent_acts.saturating_add(1);
            closure.declared_artifact |= declared_artifact;
        }
        SourceKind::JavaScript => {
            closure.components.insert("javascript".to_owned());
        }
        SourceKind::CliTool => {
            let component = source.command.as_deref().map_or_else(
                || "cli-tool".to_owned(),
                |command| format!("cli-tool:{command}"),
            );
            closure.components.insert(component);
        }
        SourceKind::Mcp => {
            let component = source
                .tool
                .as_deref()
                .map_or_else(|| "mcp".to_owned(), |tool| format!("mcp:{tool}"));
            closure.components.insert(component);
        }
        SourceKind::A2a => {
            closure.components.insert("a2a".to_owned());
        }
        SourceKind::ExternalAdapter => {
            closure.components.insert("external-adapter".to_owned());
        }
        SourceKind::ThreadOutboxProvider => {
            closure
                .components
                .insert("thread-outbox-provider".to_owned());
        }
    }
    Ok(())
}

fn load_local_referenced_skill(
    loaded: &LoadedSkillPackage,
    reference: &str,
) -> Result<Option<LoadedSkillPackage>, String> {
    if is_external_or_dynamic_skill_reference(reference) {
        return Ok(None);
    }
    super::load_validated_skill_package(&loaded.directory.join(reference))
        .map(Some)
        .map_err(|error| {
            format!(
                "loading referenced sub-skill {reference} from {}: {error}",
                loaded.directory.display()
            )
        })
}

fn is_external_or_dynamic_skill_reference(reference: &str) -> bool {
    reference.starts_with('$')
        || reference.starts_with("registry:")
        || reference.starts_with("runx-registry:")
        || reference.starts_with("runx://skill/")
}

fn nested_profile_path(current_profile: &str, reference: &str) -> Result<String, String> {
    let current_dir = Path::new(current_profile)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    normalize_relative_path(current_dir.join(reference).join("X.yaml")).ok_or_else(|| {
        format!("sub-skill reference {reference} escapes the inspected execution closure")
    })
}

fn normalize_relative_path(path: PathBuf) -> Option<String> {
    let mut normalized: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.last().is_some_and(|segment| segment != "..") {
                    normalized.pop();
                } else {
                    normalized.push("..".to_owned());
                }
            }
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    Some(normalized.join("/"))
}

fn select_inspection_runner<'a>(
    manifest: &'a runx_parser::SkillRunnerManifest,
    selected: Option<&str>,
) -> Option<&'a SkillRunnerDefinition> {
    if let Some(selected) = selected {
        return manifest.runners.get(selected);
    }
    manifest
        .runners
        .values()
        .find(|runner| runner.default)
        .or_else(|| {
            (manifest.runners.len() == 1)
                .then(|| manifest.runners.values().next())
                .flatten()
        })
}

fn execution_summary(components: &[String], agent_acts: usize, declared_artifact: bool) -> String {
    let agent_summary = match (agent_acts, declared_artifact) {
        (0, _) => None,
        (1, true) => Some("1 agent act -> declared artifact".to_owned()),
        (count, true) => Some(format!("{count} agent acts -> declared artifact")),
        (1, false) => Some("1 agent act".to_owned()),
        (count, false) => Some(format!("{count} agent acts")),
    };
    match (components.is_empty(), agent_summary) {
        (true, Some(agent)) => agent,
        (false, Some(agent)) => format!("{}; {agent}", components.join(", ")),
        (false, None) => components.join(", "),
        (true, None) => "none".to_owned(),
    }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::fs;

    use runx_contracts::JsonValue;

    use super::inspect_skill_package;

    const ROOT_MANUAL: &str =
        "---\nname: root\ndescription: Root inspection fixture.\n---\n\n# Root\n";
    const CHILD_MANUAL: &str =
        "---\nname: child\ndescription: Child inspection fixture.\n---\n\n# Child\n";
    const ROOT_MANIFEST: &str = r#"
skill: root
version: "0.1.0"
runners:
  inspect:
    default: true
    type: graph
    graph:
      name: root
      steps:
        - id: child
          skill: child
"#;
    const CHILD_MANIFEST: &str = r#"
skill: child
version: "0.1.0"
runners:
  read:
    default: true
    type: graph
    graph:
      name: child
      steps:
        - id: digest
          tool: data.digest
          inputs:
            value: inspected
"#;

    #[test]
    fn execution_closure_uses_validated_names_and_transitive_native_tools()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir().expect("temporary skill catalog");
        let root = temp.path().join("root");
        let child = root.join("child");
        fs::create_dir_all(&child).expect("child skill directory");
        fs::write(root.join("SKILL.md"), ROOT_MANUAL).expect("root manual");
        fs::write(root.join("X.yaml"), ROOT_MANIFEST).expect("root manifest");
        fs::write(child.join("SKILL.md"), CHILD_MANUAL).expect("child manual");
        fs::write(child.join("X.yaml"), CHILD_MANIFEST).expect("child manifest");

        let inspected = inspect_skill_package(&root, None).expect("valid inspection");
        let JsonValue::Object(inspected) = inspected else {
            return Err("inspection should be an object".into());
        };
        let closure = inspected
            .get("execution_closure")
            .and_then(JsonValue::as_object)
            .expect("execution closure");
        assert_eq!(
            closure.get("summary").and_then(JsonValue::as_str),
            Some("tool:data.digest")
        );
        assert_eq!(
            closure.get("skill_edges"),
            Some(&JsonValue::Array(vec![JsonValue::String(
                "child#read".to_owned()
            )]))
        );
        assert_eq!(
            closure.get("profiles"),
            Some(&JsonValue::Array(vec![
                JsonValue::String("X.yaml#inspect".to_owned()),
                JsonValue::String("child/X.yaml#read".to_owned()),
            ]))
        );
        Ok(())
    }
}
