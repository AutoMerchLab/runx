// Module rationale: execution-closure inspection keeps package traversal,
// registry-edge classification, cycle detection, and summary projection in one
// canonical walk.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use runx_contracts::{JsonValue, sha256_prefixed};
use runx_parser::{GraphStep, SourceKind};
use serde::Serialize;

use super::super::LoadedSkillPackage;

#[derive(Default)]
struct ClosureAccumulator {
    components: BTreeSet<String>,
    skill_edges: BTreeSet<String>,
    direct_external_skill_edges: BTreeSet<DirectExternalSkillEdge>,
    unresolved_skill_edges: BTreeSet<String>,
    package_bindings: BTreeSet<ExecutionPackageBinding>,
    profiles: BTreeSet<String>,
    agent_acts: usize,
    declared_artifact: bool,
}

#[derive(Serialize)]
struct ExecutionClosure {
    closure_digest: String,
    runtime_release: String,
    fully_bound: bool,
    summary: String,
    components: Vec<String>,
    skill_edges: Vec<String>,
    direct_external_skill_edges: Vec<DirectExternalSkillEdge>,
    unresolved_skill_edges: Vec<String>,
    package_bindings: Vec<ExecutionPackageBinding>,
    agent_acts: u64,
    declared_artifact: bool,
    profiles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct ExecutionPackageBinding {
    skill: String,
    runner: String,
    package_digest: String,
    source_path: String,
    source_files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct DirectExternalSkillEdge {
    skill: String,
    runner: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgeDepth {
    Direct,
    Nested,
}

impl EdgeDepth {
    const fn records_direct_edges(self) -> bool {
        matches!(self, Self::Direct)
    }
}

pub(super) fn inspect_execution_closures(
    loaded: Arc<LoadedSkillPackage>,
) -> Result<BTreeMap<String, JsonValue>, String> {
    let runner_names = loaded
        .manifest()
        .map(|manifest| manifest.runners.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut inspector = ExecutionClosureInspector::new(loaded)?;
    runner_names
        .into_iter()
        .map(|runner_name| {
            let closure = inspector.inspect_root_runner(&runner_name)?;
            Ok((runner_name, closure))
        })
        .collect()
}

struct ExecutionClosureInspector {
    root: Arc<LoadedSkillPackage>,
    root_directory: PathBuf,
    package_root: PathBuf,
    loaded_by_directory: BTreeMap<PathBuf, Arc<LoadedSkillPackage>>,
}

impl ExecutionClosureInspector {
    fn new(root: Arc<LoadedSkillPackage>) -> Result<Self, String> {
        let root_directory = canonical_directory(&root.directory, "inspected skill")?;
        let package_root = canonical_directory(&root.package_root, "inspected package")?;
        let loaded_by_directory = BTreeMap::from([(root_directory.clone(), root.clone())]);
        Ok(Self {
            root,
            root_directory,
            package_root,
            loaded_by_directory,
        })
    }

    fn inspect_root_runner(&mut self, runner_name: &str) -> Result<JsonValue, String> {
        let mut closure = ClosureAccumulator::default();
        let mut visited = BTreeSet::new();
        let mut walk = ExecutionWalkState {
            closure: &mut closure,
            visited: &mut visited,
        };
        let profile_path = self
            .root
            .profile_path
            .as_deref()
            .unwrap_or("X.yaml")
            .to_owned();
        self.walk_runner(
            self.root.clone(),
            self.root_directory.clone(),
            profile_path,
            runner_name.to_owned(),
            EdgeDepth::Direct,
            &mut walk,
        )?;
        serialize_closure(closure)
    }

    fn walk_runner(
        &mut self,
        loaded: Arc<LoadedSkillPackage>,
        skill_directory: PathBuf,
        profile_path: String,
        runner_name: String,
        edge_depth: EdgeDepth,
        walk: &mut ExecutionWalkState<'_>,
    ) -> Result<(), String> {
        if !walk.visited.insert((skill_directory, runner_name.clone())) {
            return Ok(());
        }
        let package_root = canonical_directory(&loaded.package_root, "bound skill package")?;
        walk.closure
            .package_bindings
            .insert(ExecutionPackageBinding {
                skill: loaded.package.skill.name.clone(),
                runner: runner_name.clone(),
                package_digest: loaded.package.package_digest.clone(),
                source_path: package_root.to_string_lossy().into_owned(),
                source_files: loaded.package.source.files.keys().cloned().collect(),
            });
        let runner = loaded
            .manifest()
            .and_then(|manifest| manifest.runners.get(&runner_name))
            .ok_or_else(|| {
                format!(
                    "sub-skill {} has no selected runner {runner_name}",
                    loaded.directory.display()
                )
            })?;
        walk.closure
            .profiles
            .insert(format!("{profile_path}#{runner_name}"));
        self.walk_source(
            loaded.clone(),
            &profile_path,
            &runner.source,
            runner.artifacts.is_some(),
            edge_depth,
            walk,
        )
    }

    fn walk_source(
        &mut self,
        loaded: Arc<LoadedSkillPackage>,
        profile_path: &str,
        source: &runx_parser::SkillSource,
        declared_artifact: bool,
        edge_depth: EdgeDepth,
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
                    if let Some(resolved) =
                        self.resolve_step_skill(loaded.clone(), profile_path, step)?
                    {
                        let ResolvedStepSkill {
                            edge,
                            static_external_name,
                            nested,
                        } = resolved;
                        walk.closure.skill_edges.insert(edge.clone());
                        if nested.is_none() {
                            walk.closure.unresolved_skill_edges.insert(edge);
                        }
                        if edge_depth.records_direct_edges() {
                            record_direct_external_skill_edge(
                                static_external_name,
                                nested.as_ref(),
                                step,
                                &self.package_root,
                                &mut walk.closure.direct_external_skill_edges,
                            );
                        }
                        if let Some(nested) = nested {
                            self.walk_runner(
                                nested.loaded,
                                nested.canonical_directory,
                                nested.profile_path,
                                nested.runner_name,
                                EdgeDepth::Nested,
                                walk,
                            )?;
                        }
                    }
                    if let Some(run_source) = step.run.as_ref().and_then(|run| run.source()) {
                        self.walk_source(
                            loaded.clone(),
                            profile_path,
                            run_source,
                            step.artifacts.is_some(),
                            edge_depth,
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

    fn resolve_step_skill(
        &mut self,
        loaded: Arc<LoadedSkillPackage>,
        profile_path: &str,
        step: &GraphStep,
    ) -> Result<Option<ResolvedStepSkill>, String> {
        let Some(reference) = step.skill.as_deref() else {
            return Ok(None);
        };
        let requested_runner = step.runner.as_deref().unwrap_or("default");
        let Some((canonical_directory, nested)) =
            self.load_local_referenced_skill(&loaded, reference)?
        else {
            return Ok(Some(ResolvedStepSkill {
                edge: format!("{reference}#{requested_runner}"),
                static_external_name: registry_skill_name(reference),
                nested: None,
            }));
        };
        let nested_profile = nested_profile_path(profile_path, reference)?;
        let nested_runner = select_inspection_runner_name(
            nested.manifest().ok_or_else(|| {
                format!(
                    "sub-skill {} has no executable manifest",
                    nested.directory.display()
                )
            })?,
            step.runner.as_deref(),
        )
        .ok_or_else(|| {
            format!(
                "sub-skill {} has no selected runner for step {}",
                nested.directory.display(),
                step.id
            )
        })?;
        Ok(Some(ResolvedStepSkill {
            edge: format!("{}#{nested_runner}", nested.package.skill.name),
            static_external_name: None,
            nested: Some(ResolvedNestedSkill {
                loaded: nested,
                canonical_directory,
                profile_path: nested_profile,
                runner_name: nested_runner,
            }),
        }))
    }

    fn load_local_referenced_skill(
        &mut self,
        loaded: &LoadedSkillPackage,
        reference: &str,
    ) -> Result<Option<(PathBuf, Arc<LoadedSkillPackage>)>, String> {
        if is_external_or_dynamic_skill_reference(reference) {
            return Ok(None);
        }
        let candidate = loaded.directory.join(reference);
        let directory = super::super::resolve_skill_package_directory(&candidate)
            .map_err(|error| local_skill_load_error(loaded, reference, error))?;
        let canonical_directory = canonical_directory(&directory, "referenced sub-skill")?;
        if let Some(cached) = self.loaded_by_directory.get(&canonical_directory) {
            return Ok(Some((canonical_directory, cached.clone())));
        }
        let nested = Arc::new(
            super::super::load_validated_skill_package(&canonical_directory)
                .map_err(|error| local_skill_load_error(loaded, reference, error))?,
        );
        self.loaded_by_directory
            .insert(canonical_directory.clone(), nested.clone());
        Ok(Some((canonical_directory, nested)))
    }
}

struct ExecutionWalkState<'a> {
    closure: &'a mut ClosureAccumulator,
    visited: &'a mut BTreeSet<(PathBuf, String)>,
}

struct ResolvedStepSkill {
    edge: String,
    static_external_name: Option<String>,
    nested: Option<ResolvedNestedSkill>,
}

struct ResolvedNestedSkill {
    loaded: Arc<LoadedSkillPackage>,
    canonical_directory: PathBuf,
    profile_path: String,
    runner_name: String,
}

fn record_direct_external_skill_edge(
    static_external_name: Option<String>,
    nested: Option<&ResolvedNestedSkill>,
    step: &GraphStep,
    package_root: &Path,
    edges: &mut BTreeSet<DirectExternalSkillEdge>,
) {
    if let Some(nested) = nested {
        if !nested.canonical_directory.starts_with(package_root) {
            edges.insert(DirectExternalSkillEdge {
                skill: nested.loaded.package.skill.name.clone(),
                runner: nested.runner_name.clone(),
            });
        }
    } else if let Some(skill) = static_external_name {
        edges.insert(DirectExternalSkillEdge {
            skill,
            runner: step.runner.as_deref().unwrap_or("default").to_owned(),
        });
    }
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("canonicalizing {label} {}: {error}", path.display()))
}

fn local_skill_load_error(
    loaded: &LoadedSkillPackage,
    reference: &str,
    error: impl std::fmt::Display,
) -> String {
    format!(
        "loading referenced sub-skill {reference} from {}: {error}",
        loaded.directory.display()
    )
}

fn serialize_closure(closure: ClosureAccumulator) -> Result<JsonValue, String> {
    let components = closure.components.into_iter().collect::<Vec<_>>();
    let package_bindings = closure.package_bindings.into_iter().collect::<Vec<_>>();
    let unresolved_skill_edges = closure
        .unresolved_skill_edges
        .into_iter()
        .collect::<Vec<_>>();
    let output = ExecutionClosure {
        closure_digest: execution_closure_digest(&package_bindings, &unresolved_skill_edges),
        runtime_release: crate::EXECUTION_RUNTIME_RELEASE.to_owned(),
        fully_bound: unresolved_skill_edges.is_empty(),
        summary: execution_summary(&components, closure.agent_acts, closure.declared_artifact),
        components,
        skill_edges: closure.skill_edges.into_iter().collect(),
        direct_external_skill_edges: closure.direct_external_skill_edges.into_iter().collect(),
        unresolved_skill_edges,
        package_bindings,
        agent_acts: u64::try_from(closure.agent_acts).unwrap_or(u64::MAX),
        declared_artifact: closure.declared_artifact,
        profiles: closure.profiles.into_iter().collect(),
    };
    let serialized = serde_json::to_vec(&output)
        .map_err(|error| format!("serializing execution closure: {error}"))?;
    serde_json::from_slice(&serialized)
        .map_err(|error| format!("projecting execution closure: {error}"))
}

fn execution_closure_digest(
    package_bindings: &[ExecutionPackageBinding],
    unresolved_skill_edges: &[String],
) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"runx.execution-closure.v1\0");
    append_digest_field(&mut canonical, crate::EXECUTION_RUNTIME_RELEASE.as_bytes());
    for binding in package_bindings {
        append_digest_field(&mut canonical, binding.skill.as_bytes());
        append_digest_field(&mut canonical, binding.runner.as_bytes());
        append_digest_field(&mut canonical, binding.package_digest.as_bytes());
    }
    for edge in unresolved_skill_edges {
        append_digest_field(&mut canonical, b"unresolved");
        append_digest_field(&mut canonical, edge.as_bytes());
    }
    sha256_prefixed(&canonical)
}

fn append_digest_field(target: &mut Vec<u8>, value: &[u8]) {
    target.extend_from_slice(&(value.len() as u64).to_be_bytes());
    target.extend_from_slice(value);
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

fn select_inspection_runner_name(
    manifest: &runx_parser::SkillRunnerManifest,
    selected: Option<&str>,
) -> Option<String> {
    if let Some(selected) = selected {
        return manifest
            .runners
            .contains_key(selected)
            .then(|| selected.to_owned());
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
        .map(|runner| runner.name.clone())
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
