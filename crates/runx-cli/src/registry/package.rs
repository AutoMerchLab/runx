// rust-style-allow: large-file - remote publish packaging owns skill sidecar
// selection, temporary harness packaging, and regression fixtures until publish
// splits into separate reader, selector, and harness modules.
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use runx_contracts::{JsonObject, JsonValue};
use runx_runtime::registry::RegistryPublishHarnessReport;
use serde::Serialize;

use super::{RegistryCliError, internal_error};

// rust-style-allow: long-function - this is one package read transaction:
// resolve the local subject, read its profile, select consumed sidecars, and
// prepare the temporary harness package from the same inputs.
pub(super) fn read_skill_package(
    subject: &str,
    profile: Option<&Path>,
    env: &BTreeMap<String, String>,
    cwd: &Path,
    include_harness: bool,
) -> Result<SkillPackage, RegistryCliError> {
    let subject_path = runx_runtime::resolve_path_from_user_input(subject, env, cwd, true);
    let metadata = fs::metadata(&subject_path).map_err(|error| RegistryCliError {
        message: format!(
            "failed to read skill package {}: {error}",
            subject_path.display()
        ),
        exit_code: 1,
    })?;
    let markdown_path = if metadata.is_dir() {
        subject_path.join("SKILL.md")
    } else {
        subject_path.clone()
    };
    let markdown = fs::read_to_string(&markdown_path).map_err(|error| RegistryCliError {
        message: format!(
            "failed to read skill markdown {}: {error}",
            markdown_path.display()
        ),
        exit_code: 1,
    })?;
    let profile_path = profile
        .map(|path| super::resolve_path(path, env, cwd, true))
        .or_else(|| {
            let candidate = markdown_path.parent()?.join("X.yaml");
            candidate.exists().then_some(candidate)
        });
    let profile_document = match profile_path {
        Some(ref path) => Some(fs::read_to_string(path).map_err(|error| RegistryCliError {
            message: format!("failed to read skill profile {}: {error}", path.display()),
            exit_code: 1,
        })?),
        None => None,
    };
    let package_files = if include_harness {
        collect_publish_package_files(&markdown_path, profile_path.as_deref())?
    } else {
        Vec::new()
    };
    let harness_package_files = if include_harness {
        collect_publish_harness_package_files(
            &markdown_path,
            profile_path.as_deref(),
            &package_files,
        )?
    } else {
        Vec::new()
    };
    let skill_dir = markdown_path.parent().map(Path::to_path_buf);
    let harness_package = if include_harness {
        publish_harness_package(
            &markdown,
            profile_document.as_deref(),
            &harness_package_files,
            skill_dir.as_deref(),
        )?
    } else {
        PublishHarnessPackage {
            path: None,
            temp_dir: None,
            fixture_paths: Vec::new(),
        }
    };
    Ok(SkillPackage {
        markdown,
        profile_document,
        harness_path: harness_package.path,
        harness_temp_dir: harness_package.temp_dir,
        harness_fixture_paths: harness_package.fixture_paths,
        package_files,
    })
}

pub(super) fn run_publish_harness(
    harness_path: Option<&Path>,
    fixture_paths: &[PathBuf],
) -> Result<RegistryPublishHarnessReport, RegistryCliError> {
    let Some(harness_path) = harness_path else {
        return Ok(RegistryPublishHarnessReport::not_declared());
    };
    let receipt_dir = publish_harness_receipt_dir()?;
    let request = runx_runtime::InlineHarnessRequest {
        skill_path: harness_path.to_path_buf(),
        receipt_dir: Some(receipt_dir.clone()),
        env: Some(publish_harness_env()),
    };
    let report = crate::runtime::local_orchestrator().run_inline_harness(&request);
    let _ignored = fs::remove_dir_all(&receipt_dir);
    let report = report.map_err(|error| {
        internal_error(format!(
            "inline harness failed for {}: {error}",
            harness_path.display()
        ))
    })?;
    let report = publish_harness_report(report);
    if report.status == "not_declared" && !fixture_paths.is_empty() {
        return run_standalone_publish_harness(fixture_paths);
    }
    if report.failed() {
        return Err(internal_error(format!(
            "Harness failed for {}: {}",
            harness_path.display(),
            report.assertion_errors.join("; ")
        )));
    }
    Ok(report)
}

fn run_standalone_publish_harness(
    fixture_paths: &[PathBuf],
) -> Result<RegistryPublishHarnessReport, RegistryCliError> {
    let harness_env = publish_harness_env();
    let mut case_names = Vec::new();
    let mut receipt_ids = Vec::new();
    let mut graph_case_count = 0usize;
    for fixture_path in fixture_paths {
        let output = runx_runtime::run_harness_fixture_with_env(fixture_path, harness_env.clone())
            .map_err(|error| {
                internal_error(format!(
                    "standalone publish harness failed for {}: {error}",
                    fixture_path.display()
                ))
            })?;
        if matches!(output.fixture.kind, runx_runtime::HarnessFixtureKind::Graph) {
            graph_case_count += 1;
        }
        case_names.push(output.fixture.name);
        receipt_ids.push(output.receipt.id.to_string());
    }
    Ok(RegistryPublishHarnessReport {
        status: "passed".to_owned(),
        case_count: fixture_paths.len(),
        assertion_error_count: 0,
        assertion_errors: Vec::new(),
        case_names,
        receipt_ids,
        graph_case_count,
    })
}

#[derive(Clone, Debug)]
pub(super) struct SkillPackage {
    pub(super) markdown: String,
    pub(super) profile_document: Option<String>,
    pub(super) harness_path: Option<PathBuf>,
    pub(super) harness_temp_dir: Option<PathBuf>,
    pub(super) harness_fixture_paths: Vec<PathBuf>,
    pub(super) package_files: Vec<HostedSkillPackageFile>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct HostedSkillPackageFile {
    pub(super) path: String,
    pub(super) content: String,
}

struct PublishHarnessPackage {
    path: Option<PathBuf>,
    temp_dir: Option<PathBuf>,
    fixture_paths: Vec<PathBuf>,
}

const MAX_REMOTE_PUBLISH_FILE_BYTES: u64 = 512 * 1024;
const MAX_REMOTE_PUBLISH_TOTAL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_REMOTE_PUBLISH_FILE_COUNT: usize = 128;
fn publish_harness_env() -> BTreeMap<String, String> {
    let mut env = runx_runtime::RuntimeOptions::safe_process_env();
    strip_hosted_publish_env(&mut env);
    crate::runtime::ensure_local_development_receipt_signing_env(&mut env);
    env
}

fn strip_hosted_publish_env(env: &mut BTreeMap<String, String>) {
    env.retain(|key, _| !key.starts_with("RUNX_HOSTED_"));
}

fn publish_harness_package(
    markdown: &str,
    profile_document: Option<&str>,
    package_files: &[HostedSkillPackageFile],
    source_skill_dir: Option<&Path>,
) -> Result<PublishHarnessPackage, RegistryCliError> {
    let Some(profile_document) = profile_document else {
        return Ok(PublishHarnessPackage {
            path: None,
            temp_dir: None,
            fixture_paths: Vec::new(),
        });
    };
    let temp_dir = unique_temp_dir("runx-publish-profile-harness")?;
    fs::write(temp_dir.join("SKILL.md"), markdown).map_err(|error| {
        internal_error(format!(
            "failed to write publish harness skill fixture {}: {error}",
            temp_dir.join("SKILL.md").display()
        ))
    })?;
    fs::write(temp_dir.join("X.yaml"), profile_document).map_err(|error| {
        internal_error(format!(
            "failed to write publish harness profile fixture {}: {error}",
            temp_dir.join("X.yaml").display()
        ))
    })?;
    for file in package_files {
        let destination = temp_dir.join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                internal_error(format!(
                    "failed to create publish harness package directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        write_publish_harness_file(&destination, &file.content).map_err(|error| {
            internal_error(format!(
                "failed to write publish harness package file {}: {error}",
                destination.display()
            ))
        })?;
    }
    let fixture_paths = copy_standalone_publish_fixtures(source_skill_dir, &temp_dir)?;
    Ok(PublishHarnessPackage {
        path: Some(temp_dir.clone()),
        temp_dir: Some(temp_dir),
        fixture_paths,
    })
}

fn copy_standalone_publish_fixtures(
    source_skill_dir: Option<&Path>,
    temp_dir: &Path,
) -> Result<Vec<PathBuf>, RegistryCliError> {
    let Some(source_skill_dir) = source_skill_dir else {
        return Ok(Vec::new());
    };
    let source_fixtures_dir = source_skill_dir.join("fixtures");
    let Ok(entries) = fs::read_dir(&source_fixtures_dir) else {
        return Ok(Vec::new());
    };
    let destination_fixtures_dir = temp_dir.join("fixtures");
    fs::create_dir_all(&destination_fixtures_dir).map_err(|error| {
        internal_error(format!(
            "failed to create publish harness fixtures directory {}: {error}",
            destination_fixtures_dir.display()
        ))
    })?;
    let mut fixture_paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            internal_error(format!(
                "failed to read publish harness fixture entry in {}: {error}",
                source_fixtures_dir.display()
            ))
        })?;
        let source_path = entry.path();
        if !is_standalone_publish_fixture(&source_path) {
            continue;
        }
        let destination = destination_fixtures_dir.join(entry.file_name());
        fs::copy(&source_path, &destination).map_err(|error| {
            internal_error(format!(
                "failed to copy publish harness fixture {}: {error}",
                source_path.display()
            ))
        })?;
        fixture_paths.push(destination);
    }
    fixture_paths.sort();
    Ok(fixture_paths)
}

fn is_standalone_publish_fixture(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "yaml" | "yml"))
}

fn write_publish_harness_file(path: &Path, content: &str) -> Result<(), std::io::Error> {
    fs::write(path, content)?;
    mark_executable_if_script(path, content)
}

#[cfg(unix)]
fn mark_executable_if_script(path: &Path, content: &str) -> Result<(), std::io::Error> {
    if !content.starts_with("#!") {
        return Ok(());
    }
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn mark_executable_if_script(_path: &Path, _content: &str) -> Result<(), std::io::Error> {
    Ok(())
}

fn collect_publish_package_files(
    markdown_path: &Path,
    profile_path: Option<&Path>,
) -> Result<Vec<HostedSkillPackageFile>, RegistryCliError> {
    if markdown_path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md") {
        return Ok(Vec::new());
    }
    let Some(package_dir) = markdown_path.parent() else {
        return Ok(Vec::new());
    };
    let package_dir = fs::canonicalize(package_dir).map_err(|error| {
        internal_error(format!(
            "failed to canonicalize skill package directory {}: {error}",
            package_dir.display()
        ))
    })?;
    let profile_path = profile_path.and_then(|path| fs::canonicalize(path).ok());
    let markdown_path = fs::canonicalize(markdown_path).map_err(|error| {
        internal_error(format!(
            "failed to canonicalize skill markdown {}: {error}",
            markdown_path.display()
        ))
    })?;
    let consumed_root_scripts = consumed_root_scripts_from_profile(profile_path.as_ref())?;
    collect_allowed_publish_package_files(
        &package_dir,
        &markdown_path,
        profile_path.as_ref(),
        &consumed_root_scripts,
    )
}

fn collect_publish_harness_package_files(
    markdown_path: &Path,
    profile_path: Option<&Path>,
    package_files: &[HostedSkillPackageFile],
) -> Result<Vec<HostedSkillPackageFile>, RegistryCliError> {
    let mut files = package_files
        .iter()
        .cloned()
        .map(|file| (file.path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let Some(profile_path) = profile_path else {
        return Ok(files.into_values().collect());
    };
    let Some(package_dir) = markdown_path.parent() else {
        return Ok(files.into_values().collect());
    };
    let package_dir = fs::canonicalize(package_dir).map_err(|error| {
        internal_error(format!(
            "failed to canonicalize skill package directory {}: {error}",
            package_dir.display()
        ))
    })?;
    let dependencies = consumed_harness_dependency_files_from_profile(profile_path)?;
    let mut total_bytes = files
        .values()
        .map(|file| file.content.len() as u64)
        .sum::<u64>();
    for declared_relative in dependencies {
        for relative in resolve_publish_harness_dependency_paths(&package_dir, &declared_relative)?
        {
            if files.contains_key(&relative) {
                continue;
            }
            copy_publish_harness_dependency(&package_dir, &relative, &mut files, &mut total_bytes)?;
        }
    }
    Ok(files.into_values().collect())
}

fn resolve_publish_harness_dependency_paths(
    package_dir: &Path,
    declared_relative: &str,
) -> Result<Vec<String>, RegistryCliError> {
    if package_dir.join(declared_relative).exists() {
        return Ok(vec![declared_relative.to_owned()]);
    }
    let graph_dir = package_dir.join("graph");
    let mut matches = Vec::new();
    let Ok(entries) = fs::read_dir(&graph_dir) else {
        return Ok(vec![declared_relative.to_owned()]);
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            internal_error(format!(
                "failed to read publish harness graph dependency entry in {}: {error}",
                graph_dir.display()
            ))
        })?;
        let stage_dir = entry.path();
        if stage_dir.join(declared_relative).exists() {
            let stage_name = entry.file_name().to_string_lossy().to_string();
            matches.push(format!("graph/{stage_name}/{declared_relative}"));
        }
    }
    if matches.len() == 1 {
        return Ok(matches);
    }
    Ok(vec![declared_relative.to_owned()])
}

// rust-style-allow: long-function - publish packaging keeps file safety, byte limits, and canonical path checks in one audit-friendly collector.
fn copy_publish_harness_dependency(
    package_dir: &Path,
    relative: &str,
    files: &mut BTreeMap<String, HostedSkillPackageFile>,
    total_bytes: &mut u64,
) -> Result<(), RegistryCliError> {
    if files.contains_key(relative) {
        return Ok(());
    }
    if should_reject_remote_publish_file(relative) {
        return Err(internal_error(format!(
            "publish harness dependency {relative} looks like a secret or local credential"
        )));
    }
    let candidate = package_dir.join(relative);
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        internal_error(format!(
            "failed to inspect publish harness dependency {}: {error}",
            candidate.display()
        ))
    })?;
    if !metadata.file_type().is_file() {
        return Err(internal_error(format!(
            "publish harness dependency {} is not a regular file",
            candidate.display()
        )));
    }
    if metadata.len() > MAX_REMOTE_PUBLISH_FILE_BYTES {
        return Err(internal_error(format!(
            "publish harness dependency {} exceeds {} bytes",
            candidate.display(),
            MAX_REMOTE_PUBLISH_FILE_BYTES
        )));
    }
    *total_bytes += metadata.len();
    if *total_bytes > MAX_REMOTE_PUBLISH_TOTAL_BYTES {
        return Err(internal_error(format!(
            "publish harness dependencies exceed {} total bytes",
            MAX_REMOTE_PUBLISH_TOTAL_BYTES
        )));
    }
    if files.len() >= MAX_REMOTE_PUBLISH_FILE_COUNT {
        return Err(internal_error(format!(
            "publish harness package cannot contain more than {MAX_REMOTE_PUBLISH_FILE_COUNT} files"
        )));
    }
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        internal_error(format!(
            "failed to canonicalize publish harness dependency {}: {error}",
            candidate.display()
        ))
    })?;
    if !canonical.starts_with(package_dir) {
        return Err(internal_error(format!(
            "publish harness dependency {} escapes the skill package",
            candidate.display()
        )));
    }
    let content = fs::read_to_string(&canonical).map_err(|error| {
        internal_error(format!(
            "publish harness dependency {} must be UTF-8 text: {error}",
            canonical.display()
        ))
    })?;
    files.insert(
        relative.to_owned(),
        HostedSkillPackageFile {
            path: relative.to_owned(),
            content,
        },
    );
    Ok(())
}

fn collect_allowed_publish_package_files(
    package_dir: &Path,
    markdown_path: &Path,
    profile_path: Option<&PathBuf>,
    consumed_root_scripts: &BTreeSet<String>,
) -> Result<Vec<HostedSkillPackageFile>, RegistryCliError> {
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    collect_allowed_publish_package_files_from_dir(
        package_dir,
        package_dir,
        markdown_path,
        profile_path,
        consumed_root_scripts,
        &mut files,
        &mut total_bytes,
    )?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

// rust-style-allow: long-function - the recursive selector keeps traversal,
// size caps, secret-name rejection, and UTF-8 materialization in one auditable
// package boundary.
fn collect_allowed_publish_package_files_from_dir(
    package_dir: &Path,
    current_dir: &Path,
    markdown_path: &Path,
    profile_path: Option<&PathBuf>,
    consumed_root_scripts: &BTreeSet<String>,
    files: &mut Vec<HostedSkillPackageFile>,
    total_bytes: &mut u64,
) -> Result<(), RegistryCliError> {
    for entry in fs::read_dir(current_dir).map_err(|error| {
        internal_error(format!(
            "failed to read remote publish package directory {}: {error}",
            current_dir.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            internal_error(format!(
                "failed to read remote publish package entry in {}: {error}",
                current_dir.display()
            ))
        })?;
        let candidate = entry.path();
        let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
            internal_error(format!(
                "failed to inspect remote publish package entry {}: {error}",
                candidate.display()
            ))
        })?;
        let relative = publish_relative_path(package_dir, &candidate)?;
        if metadata.file_type().is_dir() {
            if should_descend_remote_publish_dir(&relative) {
                collect_allowed_publish_package_files_from_dir(
                    package_dir,
                    &candidate,
                    markdown_path,
                    profile_path,
                    consumed_root_scripts,
                    files,
                    total_bytes,
                )?;
            }
            continue;
        }
        if !is_allowed_remote_publish_package_file(&relative, consumed_root_scripts) {
            continue;
        }
        if !metadata.file_type().is_file() {
            return Err(internal_error(format!(
                "remote publish package file {} is not a regular file",
                candidate.display()
            )));
        }
        let canonical = fs::canonicalize(&candidate).map_err(|error| {
            internal_error(format!(
                "failed to canonicalize remote publish package file {}: {error}",
                candidate.display()
            ))
        })?;
        if canonical == markdown_path || profile_path == Some(&canonical) {
            continue;
        }
        if metadata.len() > MAX_REMOTE_PUBLISH_FILE_BYTES {
            return Err(internal_error(format!(
                "remote publish package file {} exceeds {} bytes",
                canonical.display(),
                MAX_REMOTE_PUBLISH_FILE_BYTES
            )));
        }
        *total_bytes += metadata.len();
        if *total_bytes > MAX_REMOTE_PUBLISH_TOTAL_BYTES {
            return Err(internal_error(format!(
                "remote publish package files exceed {} total bytes",
                MAX_REMOTE_PUBLISH_TOTAL_BYTES
            )));
        }
        if files.len() >= MAX_REMOTE_PUBLISH_FILE_COUNT {
            return Err(internal_error(format!(
                "remote publish package cannot contain more than {MAX_REMOTE_PUBLISH_FILE_COUNT} package files"
            )));
        }
        if should_reject_remote_publish_file(&relative) {
            return Err(internal_error(format!(
                "remote publish package file {relative} looks like a secret or local credential; remove it before publishing"
            )));
        }
        let content = fs::read_to_string(&canonical).map_err(|error| {
            internal_error(format!(
                "remote publish package file {} must be UTF-8 text: {error}",
                canonical.display()
            ))
        })?;
        files.push(HostedSkillPackageFile {
            path: relative,
            content,
        });
    }
    Ok(())
}

fn publish_relative_path(package_dir: &Path, candidate: &Path) -> Result<String, RegistryCliError> {
    let relative = candidate.strip_prefix(package_dir).map_err(|error| {
        internal_error(format!(
            "failed to relativize remote publish package entry {}: {error}",
            candidate.display()
        ))
    })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn should_descend_remote_publish_dir(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    !matches!(
        name,
        ".git"
            | ".runx"
            | ".scafld"
            | ".ssh"
            | "assets"
            | "dist"
            | "fixtures"
            | "node_modules"
            | "src"
            | "target"
    )
}

fn is_allowed_remote_publish_package_file(
    relative: &str,
    consumed_root_scripts: &BTreeSet<String>,
) -> bool {
    if relative.is_empty()
        || relative
            .split('/')
            .any(|segment| segment.is_empty() || segment.starts_with('.'))
    {
        return false;
    }
    if relative.split('/').any(|segment| {
        matches!(
            segment,
            "assets" | "dist" | "fixtures" | "node_modules" | "src" | "target"
        )
    }) {
        return false;
    }
    let file_name = relative.rsplit('/').next().unwrap_or(relative);
    if !relative.contains('/') && (file_name.ends_with(".mjs") || file_name.ends_with(".js")) {
        return consumed_root_scripts.contains(relative);
    }
    if relative.contains("/references/") || relative.starts_with("references/") {
        return file_name.ends_with(".md");
    }
    matches!(file_name, "SKILL.md" | "X.yaml" | "manifest.json")
        || matches!(
            file_name,
            "run.mjs" | "run.js" | "harness.mjs" | "harness.js"
        )
}

fn consumed_root_scripts_from_profile(
    profile_path: Option<&PathBuf>,
) -> Result<BTreeSet<String>, RegistryCliError> {
    let Some(profile_path) = profile_path else {
        return Ok(BTreeSet::new());
    };
    let document = fs::read_to_string(profile_path).map_err(|error| {
        internal_error(format!(
            "failed to read profile while selecting publish package files {}: {error}",
            profile_path.display()
        ))
    })?;
    let manifest = runx_runtime::validate_runner_manifest(
        runx_runtime::parse_runner_manifest_yaml(&document).map_err(|error| {
            internal_error(format!(
                "failed to parse profile while selecting publish package files {}: {error}",
                profile_path.display()
            ))
        })?,
    )
    .map_err(|error| {
        internal_error(format!(
            "failed to validate profile while selecting publish package files {}: {error}",
            profile_path.display()
        ))
    })?;
    let mut scripts = BTreeSet::new();
    for runner in manifest.runners.values() {
        collect_root_scripts_from_source(&runner.source, &mut scripts);
    }
    Ok(scripts)
}

fn consumed_harness_dependency_files_from_profile(
    profile_path: &Path,
) -> Result<BTreeSet<String>, RegistryCliError> {
    let document = fs::read_to_string(profile_path).map_err(|error| {
        internal_error(format!(
            "failed to read profile while selecting publish harness files {}: {error}",
            profile_path.display()
        ))
    })?;
    let manifest = runx_runtime::validate_runner_manifest(
        runx_runtime::parse_runner_manifest_yaml(&document).map_err(|error| {
            internal_error(format!(
                "failed to parse profile while selecting publish harness files {}: {error}",
                profile_path.display()
            ))
        })?,
    )
    .map_err(|error| {
        internal_error(format!(
            "failed to validate profile while selecting publish harness files {}: {error}",
            profile_path.display()
        ))
    })?;
    let mut files = BTreeSet::new();
    if let Some(harness) = manifest.harness {
        for case in harness.cases {
            for value in case.inputs.values() {
                collect_harness_dependency_from_value(value, &mut files);
            }
            for value in case.env.values() {
                collect_harness_dependency_from_string(value, &mut files);
            }
            if let Some(answers) = case.caller.answers {
                for value in answers.values() {
                    collect_harness_dependency_from_value(value, &mut files);
                }
            }
        }
    }
    Ok(files)
}

fn collect_harness_dependency_from_value(value: &JsonValue, files: &mut BTreeSet<String>) {
    match value {
        JsonValue::String(value) => collect_harness_dependency_from_string(value, files),
        JsonValue::Array(values) => {
            for value in values {
                collect_harness_dependency_from_value(value, files);
            }
        }
        JsonValue::Object(values) => {
            for value in values.values() {
                collect_harness_dependency_from_value(value, files);
            }
        }
        JsonValue::Bool(_) | JsonValue::Null | JsonValue::Number(_) => {}
    }
}

fn collect_harness_dependency_from_string(value: &str, files: &mut BTreeSet<String>) {
    if let Some(path) = normalize_harness_dependency_file(value) {
        files.insert(path);
    }
}

fn normalize_harness_dependency_file(value: &str) -> Option<String> {
    let path = value
        .trim()
        .strip_prefix("./")
        .unwrap_or_else(|| value.trim());
    if !(path.ends_with(".mjs") || path.ends_with(".js")) {
        return None;
    }
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    Some(path.to_owned())
}

fn collect_root_scripts_from_source(
    source: &runx_runtime::SkillSource,
    scripts: &mut BTreeSet<String>,
) {
    collect_root_scripts_from_command(&source.command, &source.args, scripts);
    if let Some(graph) = &source.graph {
        for step in &graph.steps {
            if let Some(run) = &step.run {
                collect_root_scripts_from_run_object(run, scripts);
            }
        }
    }
}

fn collect_root_scripts_from_run_object(run: &JsonObject, scripts: &mut BTreeSet<String>) {
    let command = json_string(run.get("command"));
    let args = run
        .get("args")
        .and_then(|value| match value {
            JsonValue::Array(values) => Some(
                values
                    .iter()
                    .filter_map(|value| match value {
                        JsonValue::String(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    collect_root_scripts_from_command(&command, &args, scripts);
}

fn collect_root_scripts_from_command(
    command: &Option<String>,
    args: &[String],
    scripts: &mut BTreeSet<String>,
) {
    if let Some(command) = command
        && let Some(script) = normalize_consumed_root_script(command)
    {
        scripts.insert(script);
    }
    for arg in args {
        if let Some(script) = normalize_consumed_root_script(arg) {
            scripts.insert(script);
        }
    }
}

fn normalize_consumed_root_script(value: &str) -> Option<String> {
    let script = value
        .trim()
        .strip_prefix("./")
        .unwrap_or_else(|| value.trim());
    if script.contains('/') || script.is_empty() {
        return None;
    }
    (script.ends_with(".mjs") || script.ends_with(".js")).then(|| script.to_owned())
}

fn json_string(value: Option<&JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn should_reject_remote_publish_file(relative: &str) -> bool {
    let Some(file_name) = relative.rsplit('/').next() else {
        return true;
    };
    let lower = file_name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || matches!(
            lower.as_str(),
            ".npmrc"
                | ".pypirc"
                | ".netrc"
                | "credentials.json"
                | "credential.json"
                | "secrets.json"
                | "secret.json"
                | "id_rsa"
                | "id_ed25519"
        )
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
}

fn publish_harness_receipt_dir() -> Result<PathBuf, RegistryCliError> {
    unique_temp_dir("runx-publish-harness")
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, RegistryCliError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| internal_error(error.to_string()))?
        .as_nanos();
    let path = env::temp_dir().join(format!("{prefix}-{}-{nanos}", process::id()));
    fs::create_dir_all(&path).map_err(|error| {
        internal_error(format!(
            "failed to create temporary directory {}: {error}",
            path.display()
        ))
    })?;
    Ok(path)
}

fn publish_harness_report(
    report: runx_runtime::InlineHarnessReport,
) -> RegistryPublishHarnessReport {
    RegistryPublishHarnessReport {
        status: report.status.to_owned(),
        case_count: report.case_count,
        assertion_error_count: report.assertion_error_count,
        assertion_errors: report.assertion_errors,
        case_names: report.case_names,
        receipt_ids: report.receipt_ids,
        graph_case_count: report.graph_case_count,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_publish_harness_package_files, collect_publish_package_files,
        should_reject_remote_publish_file, strip_hosted_publish_env, unique_temp_dir,
    };
    use std::fs;

    use runx_runtime::{
        RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64_ENV, RUNX_RECEIPT_SIGN_ISSUER_TYPE_ENV,
        RUNX_RECEIPT_SIGN_KID_ENV,
    };

    #[test]
    fn publish_harness_supplies_local_signing_env_for_fresh_users() {
        let mut env = std::collections::BTreeMap::new();

        crate::runtime::ensure_local_development_receipt_signing_env(&mut env);

        assert_eq!(
            env.get(RUNX_RECEIPT_SIGN_KID_ENV).map(String::as_str),
            Some(crate::runtime::LOCAL_DEVELOPMENT_RECEIPT_SIGNING_KID)
        );
        assert_eq!(
            env.get(RUNX_RECEIPT_SIGN_ISSUER_TYPE_ENV)
                .map(String::as_str),
            Some(crate::runtime::LOCAL_DEVELOPMENT_RECEIPT_SIGNING_ISSUER_TYPE)
        );
        assert!(
            env.get(RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64_ENV)
                .is_some_and(|value| !value.trim().is_empty())
        );
    }

    #[test]
    fn publish_harness_does_not_mask_partial_signing_env() {
        let mut env = std::collections::BTreeMap::from([(
            RUNX_RECEIPT_SIGN_KID_ENV.to_owned(),
            "explicit-kid".to_owned(),
        )]);

        crate::runtime::ensure_local_development_receipt_signing_env(&mut env);

        assert_eq!(
            env.get(RUNX_RECEIPT_SIGN_KID_ENV).map(String::as_str),
            Some("explicit-kid")
        );
        assert!(!env.contains_key(RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64_ENV));
        assert!(!env.contains_key(RUNX_RECEIPT_SIGN_ISSUER_TYPE_ENV));
    }

    #[test]
    fn publish_harness_strips_hosted_registry_credentials() {
        let mut env = std::collections::BTreeMap::from([
            ("PATH".to_owned(), "/bin".to_owned()),
            (
                "RUNX_HOSTED_REGISTRY_PUBLISH_TOKEN".to_owned(),
                "secret".to_owned(),
            ),
            (
                "RUNX_HOSTED_PUBLIC_HARNESS_OWNERS".to_owned(),
                "[\"runx\"]".to_owned(),
            ),
        ]);

        strip_hosted_publish_env(&mut env);

        assert_eq!(env.get("PATH").map(String::as_str), Some("/bin"));
        assert!(!env.contains_key("RUNX_HOSTED_REGISTRY_PUBLISH_TOKEN"));
        assert!(!env.contains_key("RUNX_HOSTED_PUBLIC_HARNESS_OWNERS"));
    }

    #[test]
    fn remote_publish_rejects_common_secret_file_names() {
        for path in [
            ".env",
            ".env.local",
            ".npmrc",
            "credentials.json",
            "nested/secrets.json",
            "private.pem",
            "tls/client.key",
            "id_ed25519",
        ] {
            assert!(
                should_reject_remote_publish_file(path),
                "{path} should not be publishable as a skill package sidecar"
            );
        }
    }

    #[test]
    fn remote_publish_allows_normal_skill_sidecars() {
        for path in ["run.mjs", "run.js", "harness.mjs", "harness.js"] {
            assert!(
                !should_reject_remote_publish_file(path),
                "{path} should remain publishable as a skill package sidecar"
            );
        }
    }

    #[test]
    fn remote_publish_package_includes_consumed_skill_material()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = unique_temp_dir("runx-publish-consumed-material-test")?;
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: sidecars\n---\n# Sidecars\n",
        )?;
        fs::write(
            dir.join("X.yaml"),
            r#"skill: sidecars
runners:
  main:
    default: true
    type: graph
    graph:
      name: sidecars
      steps:
        - id: inspect
          run:
            type: cli-tool
            command: node
            args:
              - ./inspect_repo.mjs
"#,
        )?;
        fs::write(dir.join("run.mjs"), "console.log('run')\n")?;
        fs::write(dir.join("harness.mjs"), "console.log('harness')\n")?;
        fs::write(dir.join("inspect_repo.mjs"), "console.log('root runner')\n")?;
        fs::write(dir.join("notes.txt"), "not packaged\n")?;

        fs::create_dir_all(dir.join("context/review-rubric"))?;
        fs::write(
            dir.join("context/review-rubric/SKILL.md"),
            "---\nname: review-rubric\n---\n# Review\n",
        )?;
        fs::write(
            dir.join("context/review-rubric/X.yaml"),
            "skill: review-rubric\ncatalog:\n  role: context\n",
        )?;

        fs::create_dir_all(dir.join("references"))?;
        fs::write(dir.join("references/operator.md"), "# Operator\n")?;

        fs::create_dir_all(dir.join("tools/frantic/post"))?;
        fs::write(
            dir.join("tools/frantic/post/manifest.json"),
            "{\"schema\":\"runx.tool.manifest.v1\",\"name\":\"frantic.post\"}\n",
        )?;
        fs::write(
            dir.join("tools/frantic/post/run.mjs"),
            "console.log('tool')\n",
        )?;
        fs::create_dir_all(dir.join("tools/frantic/post/src"))?;
        fs::write(
            dir.join("tools/frantic/post/src/index.ts"),
            "console.log('not packaged')\n",
        )?;

        fs::create_dir_all(dir.join("graph/quote"))?;
        fs::write(
            dir.join("graph/quote/SKILL.md"),
            "---\nname: quote-stage\n---\n# Quote\n",
        )?;
        fs::write(dir.join("graph/quote/X.yaml"), "skill: quote-stage\n")?;
        fs::write(dir.join("graph/quote/run.mjs"), "console.log('stage')\n")?;

        fs::create_dir_all(dir.join("push-outbox"))?;
        fs::write(
            dir.join("push-outbox/SKILL.md"),
            "---\nname: push-outbox\n---\n# Push\n",
        )?;
        fs::write(dir.join("push-outbox/manifest.json"), "{}\n")?;

        fs::write(dir.join(".env"), "SECRET=not-packaged\n")?;
        fs::create_dir_all(dir.join("fixtures"))?;
        fs::write(dir.join("fixtures/happy-path.yaml"), "case: happy\n")?;

        let files =
            collect_publish_package_files(&dir.join("SKILL.md"), Some(&dir.join("X.yaml")))?;
        let paths = files.into_iter().map(|file| file.path).collect::<Vec<_>>();

        assert!(paths.contains(&"inspect_repo.mjs".to_owned()));
        assert!(!paths.contains(&"run.mjs".to_owned()));
        assert!(!paths.contains(&"harness.mjs".to_owned()));
        assert!(paths.contains(&"context/review-rubric/SKILL.md".to_owned()));
        assert!(paths.contains(&"context/review-rubric/X.yaml".to_owned()));
        assert!(paths.contains(&"references/operator.md".to_owned()));
        assert!(paths.contains(&"tools/frantic/post/manifest.json".to_owned()));
        assert!(paths.contains(&"tools/frantic/post/run.mjs".to_owned()));
        assert!(paths.contains(&"graph/quote/SKILL.md".to_owned()));
        assert!(paths.contains(&"graph/quote/X.yaml".to_owned()));
        assert!(paths.contains(&"graph/quote/run.mjs".to_owned()));
        assert!(paths.contains(&"push-outbox/SKILL.md".to_owned()));
        assert!(paths.contains(&"push-outbox/manifest.json".to_owned()));
        assert!(!paths.contains(&"notes.txt".to_owned()));
        assert!(!paths.contains(&".env".to_owned()));
        assert!(!paths.contains(&"tools/frantic/post/src/index.ts".to_owned()));
        assert!(!paths.contains(&"fixtures/happy-path.yaml".to_owned()));

        let _ignored = fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn publish_harness_package_includes_explicit_harness_dependencies_only()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = unique_temp_dir("runx-publish-harness-dependency-test")?;
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: harness-deps\n---\n# Harness deps\n",
        )?;
        fs::write(
            dir.join("X.yaml"),
            r#"skill: harness-deps
harness:
  cases:
    - name: fixture-helper
      runner: main
      inputs:
        helper: ./fixtures/helper.mjs
        ignored_text: ./fixtures/not-a-script.txt
      expect:
        status: sealed
runners:
  main:
    default: true
    type: cli-tool
    command: node
    args:
      - ./run.mjs
    input_mode: stdin
"#,
        )?;
        fs::write(dir.join("run.mjs"), "console.log('run')\n")?;
        fs::create_dir_all(dir.join("fixtures"))?;
        fs::write(dir.join("fixtures/helper.mjs"), "console.log('helper')\n")?;
        fs::write(dir.join("fixtures/not-a-script.txt"), "not copied\n")?;

        let package_files =
            collect_publish_package_files(&dir.join("SKILL.md"), Some(&dir.join("X.yaml")))?;
        let package_paths = package_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert!(!package_paths.contains(&"fixtures/helper.mjs"));

        let harness_files = collect_publish_harness_package_files(
            &dir.join("SKILL.md"),
            Some(&dir.join("X.yaml")),
            &package_files,
        )?;
        let harness_paths = harness_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert!(harness_paths.contains(&"fixtures/helper.mjs"));
        assert!(!harness_paths.contains(&"fixtures/not-a-script.txt"));

        let _ignored = fs::remove_dir_all(dir);
        Ok(())
    }
}
