use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ExecutionGraph, SkillExternalAdapterManifest, SkillRunnerManifest, SkillSource, SourceKind,
};

use super::super::path::normalize_context_ref;
use super::super::{SkillPackageError, SkillPackageSource};
use super::contract::{text_file, validate_manual};

pub(super) fn collect_package_references(
    profiles: &BTreeMap<String, SkillRunnerManifest>,
    package: &SkillPackageSource,
) -> Result<PackageReferences, SkillPackageError> {
    let mut references = PackageReferences::default();
    for (profile_path, manifest) in profiles {
        let profile_dir = profile_directory(profile_path);
        for (name, runner) in &manifest.runners {
            collect_source_references(
                &format!("{profile_path}.runners.{name}.source"),
                profile_dir,
                &runner.source,
                package,
                &mut references,
            )?;
        }
    }
    Ok(references)
}

#[derive(Default)]
pub(super) struct PackageReferences {
    pub(super) module_roots: BTreeSet<String>,
    pub(super) execution_files: BTreeSet<String>,
    pub(super) context_refs: Vec<ContextReference>,
}

#[derive(Clone)]
pub(super) struct ContextReference {
    field: String,
    profile_dir: String,
    pub(super) reference: String,
}

fn collect_source_references(
    field: &str,
    profile_dir: &str,
    source: &SkillSource,
    package: &SkillPackageSource,
    references: &mut PackageReferences,
) -> Result<(), SkillPackageError> {
    if let Some(module) = &source.module {
        references
            .module_roots
            .insert(package_relative(profile_dir, module));
    }
    if source.source_type == SourceKind::CliTool {
        collect_process_execution_files(
            profile_dir,
            source.command.iter().chain(&source.args),
            &mut references.execution_files,
        );
    }
    if let Some(server) = &source.server {
        collect_process_execution_files(
            profile_dir,
            std::iter::once(&server.command).chain(&server.args),
            &mut references.execution_files,
        );
    }
    if let Some(declaration) = source.external_adapter.as_ref() {
        collect_external_adapter_files(
            field,
            profile_dir,
            declaration,
            package,
            &mut references.execution_files,
        )?;
    }
    if let Some(graph) = &source.graph {
        collect_graph_references(field, profile_dir, graph, package, references)?;
    }
    Ok(())
}

fn collect_external_adapter_files(
    field: &str,
    profile_dir: &str,
    declaration: &SkillExternalAdapterManifest,
    package: &SkillPackageSource,
    files: &mut BTreeSet<String>,
) -> Result<(), SkillPackageError> {
    let (manifest, script_dir) = match declaration {
        SkillExternalAdapterManifest::Inline(manifest) => {
            (manifest.as_ref().clone(), profile_dir.to_owned())
        }
        SkillExternalAdapterManifest::Path(path) => {
            let manifest_path = package_relative(profile_dir, path);
            let bytes = package.files.get(&manifest_path).ok_or_else(|| {
                SkillPackageError::invalid(
                    field,
                    format!("external-adapter manifest {manifest_path:?} is missing"),
                )
            })?;
            let manifest = serde_json::from_slice::<runx_contracts::ExternalAdapterManifest>(bytes)
                .map_err(|error| {
                    SkillPackageError::invalid(
                        &manifest_path,
                        format!("external-adapter manifest is invalid: {error}"),
                    )
                })?;
            files.insert(manifest_path.clone());
            (manifest, package_directory(&manifest_path).to_owned())
        }
    };
    for value in manifest
        .transport
        .command
        .iter()
        .map(AsRef::as_ref)
        .chain(manifest.transport.args.iter().flatten().map(String::as_str))
    {
        if let Some(path) = process_script_path(&script_dir, value) {
            files.insert(path);
        }
    }
    Ok(())
}

fn collect_process_execution_files<'a>(
    profile_dir: &str,
    values: impl IntoIterator<Item = &'a String>,
    files: &mut BTreeSet<String>,
) {
    for value in values {
        if let Some(path) = process_script_path(profile_dir, value) {
            files.insert(path);
        }
    }
}

fn process_script_path(profile_dir: &str, value: &str) -> Option<String> {
    let path = value
        .trim()
        .strip_prefix("./")
        .unwrap_or_else(|| value.trim());
    let valid = (path.ends_with(".js") || path.ends_with(".mjs"))
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."));
    valid.then(|| package_relative(profile_dir, path))
}

fn collect_graph_references(
    field: &str,
    profile_dir: &str,
    graph: &ExecutionGraph,
    package: &SkillPackageSource,
    references: &mut PackageReferences,
) -> Result<(), SkillPackageError> {
    for (index, step) in graph.steps.iter().enumerate() {
        let step_field = format!("{field}.graph.steps[{index}]");
        collect_step_context_refs(
            &step_field,
            profile_dir,
            &step.context_skills,
            &mut references.context_refs,
        )?;
        let Some(run) = &step.run else {
            continue;
        };
        let Some(nested) = run.source() else {
            continue;
        };
        collect_source_references(
            &format!("{step_field}.run"),
            profile_dir,
            nested,
            package,
            references,
        )?;
    }
    Ok(())
}

fn collect_step_context_refs(
    step_field: &str,
    profile_dir: &str,
    references: &[String],
    context_refs: &mut Vec<ContextReference>,
) -> Result<(), SkillPackageError> {
    let mut seen = BTreeSet::new();
    for reference in references {
        if !seen.insert(reference) {
            return Err(SkillPackageError::invalid(
                format!("{step_field}.context_skills"),
                format!("context skill ref {reference:?} is declared more than once"),
            ));
        }
        context_refs.push(ContextReference {
            field: format!("{step_field}.context_skills"),
            profile_dir: profile_dir.to_owned(),
            reference: reference.clone(),
        });
    }
    Ok(())
}

pub(super) fn validate_context_skill_sources(
    source: &SkillPackageSource,
    references: &[ContextReference],
) -> Result<(), SkillPackageError> {
    for reference in references {
        let Some(manual_path) =
            normalize_context_ref(&reference.profile_dir, &reference.reference)?
        else {
            continue;
        };
        let markdown = source.files.get(&manual_path).ok_or_else(|| {
            SkillPackageError::invalid(
                &reference.field,
                format!(
                    "context skill ref {:?} does not resolve to {manual_path}",
                    reference.reference
                ),
            )
        })?;
        validate_manual(text_file(&manual_path, markdown)?).map_err(|error| {
            SkillPackageError::invalid(
                manual_path,
                format!("context skill manual is invalid: {error}"),
            )
        })?;
    }
    Ok(())
}

fn profile_directory(path: &str) -> &str {
    path.strip_suffix("/X.yaml").unwrap_or("")
}

fn package_directory(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(directory, _)| directory)
}

fn package_relative(directory: &str, path: &str) -> String {
    if directory.is_empty() {
        path.to_owned()
    } else {
        format!("{directory}/{path}")
    }
}
