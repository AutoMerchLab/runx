// Module rationale: execution-closure inspection keeps package traversal,
// registry-edge classification, cycle detection, and summary projection in one
// canonical walk.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use runx_contracts::{JsonValue, sha256_prefixed};
use runx_parser::{GraphStep, SourceKind};
use serde::Serialize;

use super::super::LoadedSkillPackage;
use super::SkillInspectionError;

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
    env: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, JsonValue>, SkillInspectionError> {
    let runner_names = loaded
        .manifest()
        .map(|manifest| manifest.runners.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut inspector = ExecutionClosureInspector::new(loaded, env)?;
    runner_names
        .into_iter()
        .map(|runner_name| {
            let closure = inspector.inspect_root_runner(&runner_name)?;
            Ok((runner_name, closure))
        })
        .collect()
}

struct ExecutionClosureInspector<'a> {
    root: Arc<LoadedSkillPackage>,
    root_directory: PathBuf,
    package_root: PathBuf,
    env: Option<&'a BTreeMap<String, String>>,
}

impl<'a> ExecutionClosureInspector<'a> {
    fn new(
        root: Arc<LoadedSkillPackage>,
        env: Option<&'a BTreeMap<String, String>>,
    ) -> Result<Self, SkillInspectionError> {
        let root_directory = canonical_directory(&root.directory, "inspected skill")?;
        let package_root = canonical_directory(&root.package_root, "inspected package")?;
        Ok(Self {
            root,
            root_directory,
            package_root,
            env,
        })
    }

    fn inspect_root_runner(
        &mut self,
        runner_name: &str,
    ) -> Result<JsonValue, SkillInspectionError> {
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
    ) -> Result<(), SkillInspectionError> {
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
            .ok_or_else(|| SkillInspectionError::SubSkillNamedRunnerMissing {
                path: loaded.directory.clone(),
                runner: runner_name.clone(),
            })?;
        walk.closure
            .profiles
            .insert(format!("{profile_path}#{runner_name}"));
        self.walk_source(
            loaded.clone(),
            &runner.source,
            runner.artifacts.is_some(),
            edge_depth,
            walk,
        )
    }

    fn walk_source(
        &mut self,
        loaded: Arc<LoadedSkillPackage>,
        source: &runx_parser::SkillSource,
        declared_artifact: bool,
        edge_depth: EdgeDepth,
        walk: &mut ExecutionWalkState<'_>,
    ) -> Result<(), SkillInspectionError> {
        match source.source_type {
            SourceKind::Graph => {
                let graph = source
                    .graph
                    .as_ref()
                    .ok_or(SkillInspectionError::GraphMissing)?;
                for step in &graph.steps {
                    if let Some(tool) = &step.tool {
                        walk.closure.components.insert(format!("tool:{tool}"));
                    }
                    if let Some(resolved) = self.resolve_step_skill(loaded.clone(), step)? {
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
        step: &GraphStep,
    ) -> Result<Option<ResolvedStepSkill>, SkillInspectionError> {
        let Some(reference) = step.skill.as_deref() else {
            return Ok(None);
        };
        let requested_runner = step.runner.as_deref().unwrap_or("default");
        if reference.starts_with('$') || (is_registry_step_ref(reference) && self.env.is_none()) {
            return Ok(Some(ResolvedStepSkill {
                edge: format!("{reference}#{requested_runner}"),
                static_external_name: registry_skill_name(reference),
                nested: None,
            }));
        }
        let empty_env = BTreeMap::new();
        let env = self.env.unwrap_or(&empty_env);
        let loaded_step = crate::execution::graph::load_step_skill_package(
            &loaded.directory,
            step,
            crate::execution::graph::StepSkillLoadOptions { env },
        )?;
        let nested = Arc::new(loaded_step.package);
        let canonical_directory = canonical_directory(&nested.directory, "referenced sub-skill")?;
        let manifest =
            nested
                .manifest()
                .ok_or_else(|| SkillInspectionError::SubSkillManifestMissing {
                    path: nested.directory.clone(),
                })?;
        let nested_runner =
            crate::execution::graph::select_step_runner(manifest, step.runner.as_deref())?
                .name
                .clone();
        let nested_profile = nested
            .profile_path
            .clone()
            .unwrap_or_else(|| "X.yaml".to_owned());
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

fn canonical_directory(path: &Path, label: &'static str) -> Result<PathBuf, SkillInspectionError> {
    path.canonicalize()
        .map_err(|source| SkillInspectionError::Canonicalize {
            label,
            path: path.to_path_buf(),
            source,
        })
}

fn serialize_closure(closure: ClosureAccumulator) -> Result<JsonValue, SkillInspectionError> {
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
    let serialized = serde_json::to_vec(&output).map_err(|source| SkillInspectionError::Json {
        context: "serializing execution closure",
        source,
    })?;
    serde_json::from_slice(&serialized).map_err(|source| SkillInspectionError::Json {
        context: "projecting execution closure",
        source,
    })
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
    if !is_registry_step_ref(reference) {
        return None;
    }
    crate::registry::parse_registry_ref(reference)
        .skill_id
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn is_registry_step_ref(reference: &str) -> bool {
    reference.starts_with("registry:")
        || reference.starts_with("runx-registry:")
        || reference.starts_with("runx://skill/")
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
