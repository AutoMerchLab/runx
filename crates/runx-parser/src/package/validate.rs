use std::collections::{BTreeMap, BTreeSet};

use runx_contracts::sha256_prefixed;

use crate::{
    SkillRunnerManifest, ValidatedSkill,
    harness_fixture::{HarnessFixture, parse_harness_fixture},
    parse_runner_manifest_yaml, parse_skill_markdown, validate_runner_manifest, validate_skill,
};

use super::path::validate_source_paths;
use super::{SkillPackageError, SkillPackageSource, ValidatedSkillPackage};

mod modules;
mod references;
mod tools;

use modules::validate_modules;
use references::{collect_package_references, validate_context_skill_sources};
use tools::validate_package_tools;

pub fn validate_skill_package(
    source: SkillPackageSource,
) -> Result<ValidatedSkillPackage, SkillPackageError> {
    validate_source_paths(&source.files, source.symlinks.iter().cloned())?;
    let manual_markdown = required_text_file(&source, "SKILL.md")?.to_owned();
    let skill = validate_manual(&manual_markdown)?;
    let profiles = validate_profiles(&source)?;
    validate_package_identity(&skill, profiles.get("X.yaml"))?;
    let references = collect_package_references(&profiles, &source)?;
    let tools = validate_package_tools(&source)?;
    validate_context_skill_sources(&source, &references.context_refs)?;
    let mut execution_files = references.execution_files;
    for package_tool in tools.values() {
        execution_files.insert(package_tool.manifest_path.clone());
        execution_files.extend(package_tool.source_files.iter().cloned());
    }
    validate_execution_files(&source, &execution_files)?;
    let harness_files = validate_harness_support_files(&source, &profiles)?;
    let mut context_skill_refs = references
        .context_refs
        .into_iter()
        .map(|reference| reference.reference)
        .collect::<Vec<_>>();
    context_skill_refs.sort();
    context_skill_refs.dedup();
    let javascript_modules = validate_modules(&source, references.module_roots, &execution_files)?;
    let harness_fixtures = validate_harness_fixtures(&source)?;
    let mut consumed_files = BTreeSet::from(["SKILL.md".to_owned()]);
    consumed_files.extend(profiles.keys().cloned());
    consumed_files.extend(javascript_modules.keys().cloned());
    consumed_files.extend(execution_files.iter().cloned());
    consumed_files.extend(harness_files.iter().cloned());
    consumed_files.extend(validate_operator_reference_files(&source)?);
    consumed_files.extend(validate_nested_package_consumed_files(&source)?);
    let source_digests = source
        .files
        .iter()
        .map(|(path, contents)| (path.clone(), sha256_prefixed(contents)))
        .collect::<BTreeMap<_, _>>();
    let manual_digest = source_digests
        .get("SKILL.md")
        .cloned()
        .ok_or_else(|| SkillPackageError::invalid("SKILL.md", "manual digest is missing"))?;
    let package_digest = package_digest(&source.files);
    Ok(ValidatedSkillPackage {
        skill,
        profiles,
        manual_markdown,
        manual_digest,
        package_digest,
        source_digests,
        javascript_modules,
        tools,
        execution_files,
        harness_files,
        consumed_files,
        harness_fixtures,
        context_skill_refs,
        source,
    })
}

fn validate_nested_package_consumed_files(
    source: &SkillPackageSource,
) -> Result<BTreeSet<String>, SkillPackageError> {
    let mut consumed = BTreeSet::new();
    for prefix in immediate_nested_manual_prefixes(source) {
        let nested = validate_skill_package(nested_package_source(source, &prefix))
            .map_err(|error| error.with_path_prefix(&prefix))?;
        consumed.extend(
            nested
                .consumed_files
                .into_iter()
                .map(|path| format!("{prefix}/{path}")),
        );
    }
    Ok(consumed)
}

fn validate_operator_reference_files(
    source: &SkillPackageSource,
) -> Result<BTreeSet<String>, SkillPackageError> {
    source
        .files
        .iter()
        .filter(|(path, _)| {
            (path.starts_with("references/") || path.contains("/references/"))
                && path.ends_with(".md")
                && !has_nested_manual_boundary(path, source)
        })
        .map(|(path, contents)| {
            text_file(path, contents)?;
            Ok(path.clone())
        })
        .collect()
}

fn immediate_nested_manual_prefixes(source: &SkillPackageSource) -> Vec<String> {
    source
        .files
        .keys()
        .filter_map(|path| path.strip_suffix("/SKILL.md"))
        .filter(|prefix| {
            let segments = prefix.split('/').collect::<Vec<_>>();
            (1..segments.len()).all(|length| {
                !source
                    .files
                    .contains_key(&format!("{}/SKILL.md", segments[..length].join("/")))
            })
        })
        .map(str::to_owned)
        .collect()
}

fn nested_package_source(source: &SkillPackageSource, prefix: &str) -> SkillPackageSource {
    let prefix = format!("{prefix}/");
    SkillPackageSource {
        files: source
            .files
            .iter()
            .filter_map(|(path, contents)| {
                path.strip_prefix(&prefix)
                    .map(|relative| (relative.to_owned(), contents.clone()))
            })
            .collect(),
        symlinks: source
            .symlinks
            .iter()
            .filter_map(|path| path.strip_prefix(&prefix).map(str::to_owned))
            .collect(),
    }
}

fn validate_harness_support_files(
    source: &SkillPackageSource,
    profiles: &BTreeMap<String, SkillRunnerManifest>,
) -> Result<BTreeSet<String>, SkillPackageError> {
    let mut files = BTreeSet::new();
    for (profile_path, manifest) in profiles {
        let Some(harness) = &manifest.harness else {
            continue;
        };
        let profile_directory = profile_path
            .rsplit_once('/')
            .map_or("", |(directory, _)| directory);
        for (index, declared) in harness.files.iter().enumerate() {
            let field = format!("{profile_path}.harness.files[{index}]");
            if declared.trim() != declared
                || declared.is_empty()
                || declared.starts_with('/')
                || declared.contains('\\')
                || declared
                    .split('/')
                    .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
                || !declared.starts_with("fixtures/")
            {
                return Err(SkillPackageError::invalid(
                    field,
                    "harness files must be normalized profile-relative paths under fixtures/",
                ));
            }
            let resolved = if profile_directory.is_empty() {
                declared.clone()
            } else {
                format!("{profile_directory}/{declared}")
            };
            if !source.files.contains_key(&resolved) {
                return Err(SkillPackageError::invalid(
                    field,
                    format!("declared harness file {resolved:?} is missing from the package"),
                ));
            }
            files.insert(resolved);
        }
    }
    Ok(files)
}

fn validate_execution_files(
    source: &SkillPackageSource,
    paths: &std::collections::BTreeSet<String>,
) -> Result<(), SkillPackageError> {
    for path in paths {
        if !source.files.contains_key(path) {
            return Err(SkillPackageError::invalid(
                path,
                "declared execution sidecar is missing from the skill package",
            ));
        }
    }
    Ok(())
}

fn validate_harness_fixtures(
    source: &SkillPackageSource,
) -> Result<BTreeMap<String, HarnessFixture>, SkillPackageError> {
    source
        .files
        .iter()
        .filter(|(path, _)| {
            path.starts_with("fixtures/")
                && (path.ends_with(".yaml") || path.ends_with(".yml"))
                && !has_nested_manual_boundary(path, source)
        })
        .map(|(path, contents)| {
            let fixture = parse_harness_fixture(text_file(path, contents)?).map_err(|error| {
                SkillPackageError::invalid(path, format!("invalid harness fixture: {error}"))
            })?;
            Ok((path.clone(), fixture))
        })
        .collect()
}

fn required_text_file<'a>(
    source: &'a SkillPackageSource,
    path: &str,
) -> Result<&'a str, SkillPackageError> {
    let bytes = source
        .files
        .get(path)
        .ok_or_else(|| SkillPackageError::invalid(path, "required package file is missing"))?;
    text_file(path, bytes)
}

fn text_file<'a>(path: &str, bytes: &'a [u8]) -> Result<&'a str, SkillPackageError> {
    std::str::from_utf8(bytes).map_err(|error| {
        SkillPackageError::invalid(path, format!("parser input must be UTF-8: {error}"))
    })
}

fn validate_manual(markdown: &str) -> Result<ValidatedSkill, SkillPackageError> {
    let parsed = parse_skill_markdown(markdown).map_err(|source| SkillPackageError::Parse {
        path: "SKILL.md".to_owned(),
        source,
    })?;
    validate_manual_ownership(&parsed.frontmatter)?;
    validate_skill(parsed).map_err(|source| SkillPackageError::Validation {
        path: "SKILL.md".to_owned(),
        source,
    })
}

fn validate_manual_ownership(
    frontmatter: &runx_contracts::JsonObject,
) -> Result<(), SkillPackageError> {
    const MANIFEST_FIELDS: &[&str] = &[
        "allowed_tools",
        "artifacts",
        "auth",
        "credentials",
        "execution",
        "harness",
        "idempotency",
        "inputs",
        "mutating",
        "outputs",
        "retry",
        "risk",
        "runners",
        "runtime",
        "source",
    ];
    if let Some(field) = MANIFEST_FIELDS
        .iter()
        .find(|field| frontmatter.contains_key(**field))
    {
        return Err(SkillPackageError::invalid(
            format!("SKILL.md.{field}"),
            "execution metadata belongs in X.yaml; SKILL.md is the operator manual",
        ));
    }
    if let Some(runx_contracts::JsonValue::Object(runx)) = frontmatter.get("runx")
        && let Some(field) = runx
            .keys()
            .find(|field| !matches!(field.as_str(), "category" | "tags"))
    {
        return Err(SkillPackageError::invalid(
            format!("SKILL.md.runx.{field}"),
            "execution metadata belongs in X.yaml; SKILL.md.runx may contain only catalog category and tags",
        ));
    }
    Ok(())
}

fn validate_profiles(
    source: &SkillPackageSource,
) -> Result<BTreeMap<String, SkillRunnerManifest>, SkillPackageError> {
    owned_profile_paths(source)
        .into_iter()
        .map(|path| {
            let contents = source
                .files
                .get(&path)
                .ok_or_else(|| SkillPackageError::invalid(&path, "profile source is missing"))?;
            let manifest = validate_manifest(&path, text_file(&path, contents)?)?;
            Ok((path, manifest))
        })
        .collect()
}

fn owned_profile_paths(source: &SkillPackageSource) -> Vec<String> {
    source
        .files
        .keys()
        .filter(|path| path.as_str() == "X.yaml" || path.ends_with("/X.yaml"))
        .filter(|path| !has_nested_manual_boundary(path, source))
        .cloned()
        .collect()
}

pub(super) fn has_nested_manual_boundary(path: &str, source: &SkillPackageSource) -> bool {
    let Some((directory, _)) = path.rsplit_once('/') else {
        return false;
    };
    let mut prefix = String::new();
    for segment in directory.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        if source.files.contains_key(&format!("{prefix}/SKILL.md")) {
            return true;
        }
    }
    false
}

fn validate_manifest(path: &str, contents: &str) -> Result<SkillRunnerManifest, SkillPackageError> {
    let parsed =
        parse_runner_manifest_yaml(contents).map_err(|source| SkillPackageError::Parse {
            path: path.to_owned(),
            source,
        })?;
    let manifest =
        validate_runner_manifest(parsed).map_err(|source| SkillPackageError::Validation {
            path: path.to_owned(),
            source,
        })?;
    let defaults = manifest
        .runners
        .values()
        .filter(|runner| runner.default)
        .count();
    if defaults > 1 {
        return Err(SkillPackageError::invalid(
            format!("{path}.runners"),
            "runner selection is ambiguous: declare at most one default runner",
        ));
    }
    Ok(manifest)
}

fn validate_package_identity(
    skill: &ValidatedSkill,
    manifest: Option<&SkillRunnerManifest>,
) -> Result<(), SkillPackageError> {
    let Some(manifest_name) = manifest.and_then(|manifest| manifest.skill.as_deref()) else {
        return Ok(());
    };
    if manifest_name == skill.name {
        return Ok(());
    }
    Err(SkillPackageError::invalid(
        "X.yaml.skill",
        format!(
            "manifest skill {manifest_name:?} does not match SKILL.md name {:?}",
            skill.name
        ),
    ))
}

fn package_digest(files: &BTreeMap<String, Vec<u8>>) -> String {
    let capacity = files
        .iter()
        .map(|(path, contents)| path.len().saturating_add(contents.len()).saturating_add(16))
        .sum();
    let mut canonical = Vec::with_capacity(capacity);
    canonical.extend_from_slice(b"runx.skill-package.v1\0");
    for (path, contents) in files {
        append_digest_field(&mut canonical, path.as_bytes());
        append_digest_field(&mut canonical, contents);
    }
    sha256_prefixed(&canonical)
}

fn append_digest_field(target: &mut Vec<u8>, value: &[u8]) {
    target.extend_from_slice(&(value.len() as u64).to_be_bytes());
    target.extend_from_slice(value);
}
