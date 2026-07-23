// Module rationale: execution-closure inspection keeps package traversal,
// registry-edge classification, cycle detection, and summary projection in one
// canonical walk.
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use runx_contracts::{JsonObject, JsonValue};
use runx_parser::{GraphStep, SkillRunnerDefinition, SourceKind};

use super::super::LoadedSkillPackage;

#[derive(Default)]
struct ExecutionClosure {
    components: BTreeSet<String>,
    skill_edges: BTreeSet<String>,
    direct_external_skill_edges: BTreeSet<(String, String)>,
    profiles: BTreeSet<String>,
    agent_acts: usize,
    declared_artifact: bool,
}

pub(super) fn inspect_execution_closure(
    loaded: &LoadedSkillPackage,
    runner: &SkillRunnerDefinition,
) -> Result<JsonValue, String> {
    let mut closure = ExecutionClosure::default();
    let package_root = loaded.directory.canonicalize().map_err(|error| {
        format!(
            "canonicalizing inspected skill {}: {error}",
            loaded.directory.display()
        )
    })?;
    {
        let mut walk = ExecutionWalkState {
            package_root: &package_root,
            closure: &mut closure,
            visited: BTreeSet::new(),
        };
        walk_runner_execution(loaded, "X.yaml", runner, true, &mut walk)?;
    }
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
            "direct_external_skill_edges".to_owned(),
            JsonValue::Array(
                closure
                    .direct_external_skill_edges
                    .into_iter()
                    .map(|(skill, runner)| {
                        JsonValue::Object(JsonObject::from([
                            ("skill".to_owned(), JsonValue::String(skill)),
                            ("runner".to_owned(), JsonValue::String(runner)),
                        ]))
                    })
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

struct ExecutionWalkState<'a> {
    package_root: &'a Path,
    closure: &'a mut ExecutionClosure,
    visited: BTreeSet<String>,
}

fn walk_runner_execution(
    loaded: &LoadedSkillPackage,
    profile_path: &str,
    runner: &SkillRunnerDefinition,
    collect_direct_edges: bool,
    walk: &mut ExecutionWalkState<'_>,
) -> Result<(), String> {
    let identity_directory = loaded.directory.canonicalize().map_err(|error| {
        format!(
            "canonicalizing inspected skill {}: {error}",
            loaded.directory.display()
        )
    })?;
    let identity = format!("{}#{}", identity_directory.display(), runner.name);
    if !walk.visited.insert(identity) {
        return Ok(());
    }
    walk.closure
        .profiles
        .insert(format!("{profile_path}#{}", runner.name));
    walk_source_execution(
        loaded,
        profile_path,
        &runner.source,
        runner.artifacts.is_some(),
        collect_direct_edges,
        walk,
    )?;
    Ok(())
}

fn walk_source_execution(
    loaded: &LoadedSkillPackage,
    profile_path: &str,
    source: &runx_parser::SkillSource,
    declared_artifact: bool,
    collect_direct_edges: bool,
    walk: &mut ExecutionWalkState<'_>,
) -> Result<(), String> {
    match source.source_type {
        SourceKind::Graph => {
            let graph = source
                .graph
                .as_ref()
                .ok_or_else(|| "graph source omitted its validated graph".to_owned())?;
            for step in &graph.steps {
                if let Some(tool) = &step.tool {
                    walk.closure.components.insert(format!("tool:{tool}"));
                }
                if let Some(resolved) = resolve_step_skill(loaded, profile_path, step)? {
                    walk.closure.skill_edges.insert(resolved.edge.clone());
                    if collect_direct_edges {
                        record_direct_external_skill_edge(
                            &resolved,
                            step,
                            walk.package_root,
                            &mut walk.closure.direct_external_skill_edges,
                        )?;
                    }
                    if let Some(nested) = resolved.nested {
                        walk_runner_execution(
                            &nested.loaded,
                            &nested.profile_path,
                            &nested.runner,
                            false,
                            walk,
                        )?;
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
                        collect_direct_edges,
                        walk,
                    )?;
                }
            }
        }
        SourceKind::Agent | SourceKind::AgentStep => {
            walk.closure.agent_acts = walk.closure.agent_acts.saturating_add(1);
            walk.closure.declared_artifact |= declared_artifact;
        }
        SourceKind::JavaScript => {
            walk.closure.components.insert("javascript".to_owned());
        }
        SourceKind::CliTool => {
            let component = source.command.as_deref().map_or_else(
                || "cli-tool".to_owned(),
                |command| format!("cli-tool:{command}"),
            );
            walk.closure.components.insert(component);
        }
        SourceKind::Mcp => {
            let component = source
                .tool
                .as_deref()
                .map_or_else(|| "mcp".to_owned(), |tool| format!("mcp:{tool}"));
            walk.closure.components.insert(component);
        }
        SourceKind::A2a => {
            walk.closure.components.insert("a2a".to_owned());
        }
        SourceKind::ExternalAdapter => {
            walk.closure
                .components
                .insert("external-adapter".to_owned());
        }
        SourceKind::ThreadOutboxProvider => {
            walk.closure
                .components
                .insert("thread-outbox-provider".to_owned());
        }
    }
    Ok(())
}

struct ResolvedStepSkill {
    edge: String,
    static_external_name: Option<String>,
    nested: Option<ResolvedNestedSkill>,
}

struct ResolvedNestedSkill {
    loaded: LoadedSkillPackage,
    profile_path: String,
    runner: SkillRunnerDefinition,
}

fn record_direct_external_skill_edge(
    resolved: &ResolvedStepSkill,
    step: &GraphStep,
    package_root: &Path,
    edges: &mut BTreeSet<(String, String)>,
) -> Result<(), String> {
    let runner = step.runner.as_deref().unwrap_or("default").to_owned();
    if let Some(nested) = &resolved.nested {
        let directory = nested.loaded.directory.canonicalize().map_err(|error| {
            format!(
                "canonicalizing directly composed skill {}: {error}",
                nested.loaded.directory.display()
            )
        })?;
        if !directory.starts_with(package_root) {
            edges.insert((
                nested.loaded.package.skill.name.clone(),
                nested.runner.name.clone(),
            ));
        }
    } else if let Some(skill) = &resolved.static_external_name {
        edges.insert((skill.clone(), runner));
    }
    Ok(())
}

fn resolve_step_skill(
    loaded: &LoadedSkillPackage,
    profile_path: &str,
    step: &GraphStep,
) -> Result<Option<ResolvedStepSkill>, String> {
    let Some(reference) = step.skill.as_deref() else {
        return Ok(None);
    };
    let runner_name = step.runner.as_deref().unwrap_or("default");
    let Some(nested) = load_local_referenced_skill(loaded, reference)? else {
        return Ok(Some(ResolvedStepSkill {
            edge: format!("{reference}#{runner_name}"),
            static_external_name: registry_skill_name(reference),
            nested: None,
        }));
    };
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
    Ok(Some(ResolvedStepSkill {
        edge: format!("{}#{}", nested.package.skill.name, nested_runner.name),
        static_external_name: None,
        nested: Some(ResolvedNestedSkill {
            loaded: nested,
            profile_path: nested_profile,
            runner: nested_runner,
        }),
    }))
}

fn registry_skill_name(reference: &str) -> Option<String> {
    if !is_external_or_dynamic_skill_reference(reference) || reference.starts_with('$') {
        return None;
    }
    crate::registry::parse_registry_ref(reference)
        .skill_id
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn load_local_referenced_skill(
    loaded: &LoadedSkillPackage,
    reference: &str,
) -> Result<Option<LoadedSkillPackage>, String> {
    if is_external_or_dynamic_skill_reference(reference) {
        return Ok(None);
    }
    super::super::load_validated_skill_package(&loaded.directory.join(reference))
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
