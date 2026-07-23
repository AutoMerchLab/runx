mod execution_closure;
mod runner;

use std::path::Path;

use runx_contracts::{JsonObject, JsonValue};
use runx_parser::{CatalogMetadata, SkillRunnerDefinition, SourceKind};

use super::LoadedSkillPackage;
use execution_closure::inspect_execution_closure;
use runner::{catalog_capabilities, fixture_examples, inspect_runner};

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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::fs;

    use runx_contracts::{JsonObject, JsonValue};

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
            closure.get("direct_external_skill_edges"),
            Some(&JsonValue::Array(Vec::new()))
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

    #[test]
    fn execution_closure_distinguishes_direct_external_skills_from_private_stages()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir().expect("temporary skill catalog");
        let root = temp.path().join("root");
        let internal = root.join("internal");
        let external = temp.path().join("research");
        fs::create_dir_all(&internal).expect("internal skill directory");
        fs::create_dir_all(&external).expect("external skill directory");
        fs::write(root.join("SKILL.md"), ROOT_MANUAL).expect("root manual");
        fs::write(
            root.join("X.yaml"),
            r#"
skill: root
runners:
  inspect:
    default: true
    type: graph
    graph:
      name: root
      steps:
        - id: internal
          skill: internal
        - id: research
          skill: ../research
          runner: brief
"#,
        )
        .expect("root manifest");
        for (directory, name, runner) in [
            (&internal, "internal", "read"),
            (&external, "research", "brief"),
        ] {
            fs::write(
                directory.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: Inspection fixture.\n---\n\n# {name}\n"),
            )
            .expect("child manual");
            fs::write(
                directory.join("X.yaml"),
                format!(
                    "skill: {name}\nrunners:\n  {runner}:\n    default: true\n    type: graph\n    graph:\n      name: {name}\n      steps:\n        - id: digest\n          tool: data.digest\n          inputs:\n            value: inspected\n"
                ),
            )
            .expect("child manifest");
        }

        let inspected = inspect_skill_package(&root, None).expect("valid inspection");
        let closure = inspected
            .as_object()
            .and_then(|value| value.get("execution_closure"))
            .and_then(JsonValue::as_object)
            .expect("execution closure");
        assert_eq!(
            closure.get("direct_external_skill_edges"),
            Some(&JsonValue::Array(vec![JsonValue::Object(
                JsonObject::from([
                    ("runner".to_owned(), JsonValue::String("brief".to_owned())),
                    ("skill".to_owned(), JsonValue::String("research".to_owned())),
                ])
            )]))
        );
        Ok(())
    }
}
