use std::fs;
use std::path::{Path, PathBuf};

use runx_parser::{
    SkillRunnerManifest, ValidatedSkill, parse_runner_manifest_yaml, parse_skill_markdown,
    validate_runner_manifest, validate_skill,
};

use super::RunxExportLoadError;

pub(super) fn discover_skill_paths(root: &Path) -> Result<Vec<PathBuf>, RunxExportLoadError> {
    let mut paths = Vec::new();
    if root.join("SKILL.md").exists() {
        paths.push(canonicalize(root, "canonicalizing root skill")?);
    }
    discover_skill_paths_below(&root.join("skills"), &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn discover_skill_paths_below(
    directory: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), RunxExportLoadError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error("reading", directory, source)),
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| io_error("reading", directory, source))?;
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("reading file type", &entry.path(), source))?;
        if file_type.is_dir() && !is_ignored_directory(&entry.file_name().to_string_lossy()) {
            directories.push(entry.path());
        }
    }
    directories.sort();
    for candidate in directories {
        if candidate.join("SKILL.md").exists() {
            paths.push(canonicalize(&candidate, "canonicalizing skill directory")?);
        }
        discover_skill_paths_below(&candidate, paths)?;
    }
    Ok(())
}

pub(super) fn read_validated_skill(
    skill_dir: &Path,
) -> Result<ValidatedSkill, RunxExportLoadError> {
    let path = skill_dir.join("SKILL.md");
    let source = read_to_string(&path)?;
    let raw = parse_skill_markdown(&source).map_err(|error| {
        RunxExportLoadError::Parse(format!("parsing {}: {error}", display_path(&path)))
    })?;
    validate_skill(raw).map_err(|error| {
        RunxExportLoadError::Parse(format!("validating {}: {error}", display_path(&path)))
    })
}

pub(super) fn read_optional_runner_manifest(
    skill_dir: &Path,
) -> Result<Option<SkillRunnerManifest>, RunxExportLoadError> {
    let path = skill_dir.join("X.yaml");
    if !path.exists() {
        return Ok(None);
    }
    let source = read_to_string(&path)?;
    let raw = parse_runner_manifest_yaml(&source).map_err(|error| {
        RunxExportLoadError::Parse(format!("parsing {}: {error}", display_path(&path)))
    })?;
    validate_runner_manifest(raw).map(Some).map_err(|error| {
        RunxExportLoadError::Parse(format!("validating {}: {error}", display_path(&path)))
    })
}

fn is_ignored_directory(name: &str) -> bool {
    matches!(name, ".git" | "node_modules" | "target")
}

fn canonicalize(path: &Path, context: &str) -> Result<PathBuf, RunxExportLoadError> {
    fs::canonicalize(path).map_err(|source| RunxExportLoadError::Io {
        context: format!("{context} {}", display_path(path)),
        source,
    })
}

fn read_to_string(path: &Path) -> Result<String, RunxExportLoadError> {
    fs::read_to_string(path).map_err(|source| io_error("reading", path, source))
}

fn io_error(action: &str, path: &Path, source: std::io::Error) -> RunxExportLoadError {
    RunxExportLoadError::Io {
        context: format!("{action} {}", display_path(path)),
        source,
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
