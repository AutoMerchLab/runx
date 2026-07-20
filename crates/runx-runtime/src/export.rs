use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use runx_parser::{
    CatalogVisibility, SkillInput, SkillRunnerDefinition, SkillRunnerManifest, ValidatedSkill,
};

mod discover;
mod resolve;

use discover::{discover_skill_paths, read_optional_runner_manifest, read_validated_skill};
use resolve::resolve_skill_ref;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunxExportSkill {
    pub name: String,
    pub description: String,
    pub inputs: BTreeMap<String, RunxExportSkillInput>,
    pub runner_names: Vec<String>,
    pub requires_runner_selection: bool,
    pub abs_dir: PathBuf,
    pub mode: RunxExportMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunxExportMode {
    Delegated,
    NativeInstructions { body: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunxExportSkillInput {
    pub required: bool,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RunxExportLoadOptions<'a> {
    pub root: &'a Path,
    pub refs: &'a [String],
    pub official_roots: Vec<PathBuf>,
}

#[derive(Debug)]
pub enum RunxExportLoadError {
    InvalidArgs(String),
    Io {
        context: String,
        source: std::io::Error,
    },
    Parse(String),
}

impl std::fmt::Display for RunxExportLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgs(message) | Self::Parse(message) => formatter.write_str(message),
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
        }
    }
}

impl std::error::Error for RunxExportLoadError {}

pub fn load_export_skills(
    root: &Path,
    refs: &[String],
) -> Result<Vec<RunxExportSkill>, RunxExportLoadError> {
    load_export_skills_with_options(RunxExportLoadOptions {
        root,
        refs,
        official_roots: Vec::new(),
    })
}

pub fn load_export_skills_with_options(
    options: RunxExportLoadOptions<'_>,
) -> Result<Vec<RunxExportSkill>, RunxExportLoadError> {
    let explicit = !options.refs.is_empty();
    let paths = if explicit {
        options
            .refs
            .iter()
            .map(|reference| resolve_skill_ref(options.root, reference, &options.official_roots))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        discover_skill_paths(options.root)?
    };

    let mut skills = Vec::new();
    for skill_dir in paths {
        let manifest = read_optional_runner_manifest(&skill_dir)?;
        if !explicit && manifest_visibility(&manifest) == Some(CatalogVisibility::Internal) {
            continue;
        }
        skills.push(load_export_skill(skill_dir, manifest)?);
    }
    validate_unique_export_names(&mut skills)?;
    Ok(skills)
}

fn load_export_skill(
    skill_dir: PathBuf,
    manifest: Option<SkillRunnerManifest>,
) -> Result<RunxExportSkill, RunxExportLoadError> {
    let skill = read_validated_skill(&skill_dir)?;
    let mode = export_mode(&skill);
    let inputs = export_skill_inputs(&skill, manifest.as_ref());
    validate_export_skill_inputs(&inputs)?;
    let runner_names = manifest
        .as_ref()
        .map(|manifest| manifest.runners.keys().cloned().collect())
        .unwrap_or_default();
    let requires_runner_selection = manifest.as_ref().is_some_and(|manifest| {
        manifest.runners.len() > 1 && default_runner(Some(manifest)).is_none()
    });
    Ok(RunxExportSkill {
        name: export_skill_name(&skill.name)?,
        description: skill
            .description
            .unwrap_or_else(|| "Run this skill through runx governance.".to_owned()),
        inputs: inputs
            .into_iter()
            .map(|(name, input)| {
                (
                    name,
                    RunxExportSkillInput {
                        required: input.required,
                        description: input.description,
                    },
                )
            })
            .collect(),
        runner_names,
        requires_runner_selection,
        abs_dir: skill_dir,
        mode,
    })
}

fn validate_unique_export_names(skills: &mut [RunxExportSkill]) -> Result<(), RunxExportLoadError> {
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in skills.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(RunxExportLoadError::InvalidArgs(format!(
                "multiple skills normalize to the export name {:?}",
                pair[0].name
            )));
        }
    }
    Ok(())
}

fn export_mode(skill: &ValidatedSkill) -> RunxExportMode {
    let is_runtime_guide = skill.name == "runx"
        && skill.source.source_type == runx_parser::SourceKind::CliTool
        && skill
            .source
            .command
            .as_deref()
            .and_then(|command| Path::new(command).file_name())
            .is_some_and(|command| command == "runx");
    if is_runtime_guide {
        return RunxExportMode::NativeInstructions {
            body: skill.body.clone(),
        };
    }
    RunxExportMode::Delegated
}

fn export_skill_inputs(
    skill: &ValidatedSkill,
    manifest: Option<&SkillRunnerManifest>,
) -> BTreeMap<String, SkillInput> {
    if !skill.inputs.is_empty() {
        return skill.inputs.clone();
    }
    default_runner(manifest)
        .map(|runner| runner.inputs.clone())
        .unwrap_or_default()
}

fn default_runner(manifest: Option<&SkillRunnerManifest>) -> Option<&SkillRunnerDefinition> {
    let manifest = manifest?;
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

fn export_skill_name(name: &str) -> Result<String, RunxExportLoadError> {
    if name.contains('\\') {
        return Err(RunxExportLoadError::InvalidArgs(format!(
            "skill name {name:?} cannot be exported because it is not a safe path segment"
        )));
    }
    let segments = name.split('/').collect::<Vec<_>>();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || matches!(*segment, "." | "..")
            || segment.starts_with('-')
            || segment.ends_with('-')
            || !segment.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    }) {
        return Err(RunxExportLoadError::InvalidArgs(format!(
            "skill name {name:?} cannot be exported because it cannot be normalized to a safe name"
        )));
    }
    Ok(segments.join("-"))
}

fn validate_export_skill_inputs(
    inputs: &BTreeMap<String, runx_parser::SkillInput>,
) -> Result<(), RunxExportLoadError> {
    for name in inputs.keys() {
        if !is_export_input_name(name) || is_reserved_skill_flag(name) {
            return Err(RunxExportLoadError::InvalidArgs(format!(
                "skill input {name:?} cannot be exported because it is not a safe runx skill flag"
            )));
        }
    }
    Ok(())
}

fn is_export_input_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_reserved_skill_flag(name: &str) -> bool {
    matches!(
        name,
        "answers"
            | "credential"
            | "json"
            | "non_interactive"
            | "receipt_dir"
            | "run_id"
            | "secret_env"
    )
}

fn manifest_visibility(
    manifest: &Option<runx_parser::SkillRunnerManifest>,
) -> Option<CatalogVisibility> {
    manifest
        .as_ref()
        .and_then(|manifest| manifest.catalog.as_ref())
        .map(|catalog| catalog.visibility)
}
