//! Registry publication preparation from parser-owned package truth.

mod files;
mod harness;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::{RegistryPackageFile, RegistryPublishHarnessReport};
use crate::{LoadedSkillPackage, LocalOrchestrator, RuntimeError};

pub(super) const MAX_PUBLISH_FILE_BYTES: usize = 512 * 1024;
pub(super) const MAX_PUBLISH_TOTAL_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_PUBLISH_FILE_COUNT: usize = 128;

#[derive(Debug, Error)]
pub enum RegistryPublishPackageError {
    #[error("{message}")]
    Invalid { message: String },
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

impl RegistryPublishPackageError {
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RegistryPublishPackageRequest<'a> {
    pub subject: &'a str,
    pub profile: Option<&'a Path>,
    pub env: &'a BTreeMap<String, String>,
    pub cwd: &'a Path,
}

pub struct RegistryPublishPackage {
    markdown: String,
    profile_document: Option<String>,
    package_files: Vec<RegistryPackageFile>,
    harness: Option<harness::PublishHarnessPackage>,
}

impl RegistryPublishPackage {
    #[must_use]
    pub fn markdown(&self) -> &str {
        &self.markdown
    }

    #[must_use]
    pub fn profile_document(&self) -> Option<&str> {
        self.profile_document.as_deref()
    }

    #[must_use]
    pub fn package_files(&self) -> &[RegistryPackageFile] {
        &self.package_files
    }

    pub fn run_harness(
        &self,
        orchestrator: &LocalOrchestrator,
        env: &BTreeMap<String, String>,
    ) -> Result<RegistryPublishHarnessReport, RegistryPublishPackageError> {
        let harness = self.harness.as_ref().ok_or_else(|| {
            RegistryPublishPackageError::invalid(
                "publish requires a skill execution profile and at least one harness case",
            )
        })?;
        harness::run_publish_harness(orchestrator, harness.path(), env)
    }

    #[must_use]
    pub fn into_parts(self) -> RegistryPublishPackageParts {
        let Self {
            markdown,
            profile_document,
            package_files,
            harness,
        } = self;
        drop(harness);
        RegistryPublishPackageParts {
            markdown,
            profile_document,
            package_files,
        }
    }
}

pub struct RegistryPublishPackageParts {
    pub markdown: String,
    pub profile_document: Option<String>,
    pub package_files: Vec<RegistryPackageFile>,
}

pub fn prepare_registry_publish_package(
    request: RegistryPublishPackageRequest<'_>,
) -> Result<RegistryPublishPackage, RegistryPublishPackageError> {
    let subject_path =
        crate::resolve_path_from_user_input(request.subject, request.env, request.cwd, true);
    let profile_path =
        resolve_profile_path(&subject_path, request.profile, request.env, request.cwd);
    let load_path = profile_path.as_deref().unwrap_or(&subject_path);
    let loaded = crate::load_validated_skill_package(load_path)?;
    prepare_loaded_package(&loaded, request.env, request.cwd)
}

fn resolve_profile_path(
    subject_path: &Path,
    profile: Option<&Path>,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Option<PathBuf> {
    profile
        .map(|path| crate::resolve_path_from_user_input(&path.to_string_lossy(), env, cwd, true))
        .or_else(|| {
            let package_dir = if subject_path.is_dir() {
                subject_path
            } else {
                subject_path.parent()?
            };
            let candidate = package_dir.join("X.yaml");
            candidate.exists().then_some(candidate)
        })
}

fn prepare_loaded_package(
    loaded: &LoadedSkillPackage,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<RegistryPublishPackage, RegistryPublishPackageError> {
    let profile_document = loaded
        .profile_path
        .as_deref()
        .and_then(|path| loaded.package.file_text(path))
        .map(str::to_owned);
    let package_files = files::collect_publish_package_files(loaded, env, cwd)?;
    let harness =
        harness::stage_publish_harness(loaded, profile_document.as_deref(), &package_files)?;
    Ok(RegistryPublishPackage {
        markdown: loaded.package.manual_markdown.clone(),
        profile_document,
        package_files,
        harness,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use super::{RegistryPublishPackageRequest, prepare_registry_publish_package};

    #[test]
    fn publish_package_projects_validated_source_without_a_second_walk()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: publish-shape\ndescription: Publish shape fixture.\n---\n# Publish shape\n",
        )?;
        fs::write(
            root.join("X.yaml"),
            r#"skill: publish-shape
harness:
  files:
    - fixtures/helper.mjs
  cases:
    - name: smoke
      runner: main
      inputs:
        helper: ./fixtures/helper.mjs
      expect:
        status: sealed
runners:
  main:
    default: true
    type: cli-tool
    command: node
    args: [run.mjs]
    outputs:
      plan: object
    artifacts:
      wrap_as: packet
      packet: runx.test.publish-shape.v1
"#,
        )?;
        fs::write(
            root.join("run.mjs"),
            "console.log(JSON.stringify({plan: {}}))\n",
        )?;
        fs::create_dir_all(root.join("tools/example/echo"))?;
        fs::write(
            root.join("tools/example/echo/manifest.json"),
            r#"{
  "schema": "runx.tool.manifest.v1",
  "name": "example.echo",
  "source": { "type": "cli-tool", "command": "node", "args": ["./src/run.mjs"] }
}
"#,
        )?;
        fs::create_dir_all(root.join("tools/example/echo/src"))?;
        fs::write(
            root.join("tools/example/echo/src/run.mjs"),
            "import { echo } from './echo.mjs';\nprocess.stdout.write(echo());\n",
        )?;
        fs::write(
            root.join("tools/example/echo/src/echo.mjs"),
            "export const echo = () => '{}';\n",
        )?;
        fs::write(root.join("notes.txt"), "not published\n")?;
        fs::write(root.join(".env"), "SECRET=not-published\n")?;
        fs::create_dir_all(root.join("references"))?;
        fs::write(root.join("references/operator.md"), "# Operator\n")?;
        fs::create_dir_all(root.join("fixtures"))?;
        fs::write(root.join("fixtures/helper.mjs"), "console.log('helper')\n")?;
        fs::create_dir_all(root.join("context/review"))?;
        fs::write(
            root.join("context/review/SKILL.md"),
            "---\nname: review-context\ndescription: Review context.\n---\n# Review\n",
        )?;
        fs::write(
            root.join("context/review/X.yaml"),
            "skill: review-context\nrunners:\n  context:\n    default: true\n    type: agent\n",
        )?;
        fs::create_dir_all(root.join("packets"))?;
        fs::write(
            root.join("packets/publish-shape.schema.json"),
            "{\"x-runx-packet-id\":\"runx.test.publish-shape.v1\",\"type\":\"object\"}\n",
        )?;

        let package = prepare_registry_publish_package(RegistryPublishPackageRequest {
            subject: root.to_str().ok_or("temporary path is not UTF-8")?,
            profile: None,
            env: &BTreeMap::new(),
            cwd: root,
        })?;
        let paths = package
            .package_files()
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"run.mjs"));
        assert!(paths.contains(&"references/operator.md"));
        assert!(paths.contains(&"context/review/SKILL.md"));
        assert!(paths.contains(&"context/review/X.yaml"));
        assert!(paths.contains(&"packets/publish-shape.schema.json"));
        assert!(!paths.contains(&"notes.txt"));
        assert!(!paths.contains(&".env"));
        assert!(paths.contains(&"fixtures/helper.mjs"));
        assert!(paths.contains(&"tools/example/echo/manifest.json"));
        assert!(paths.contains(&"tools/example/echo/src/run.mjs"));
        assert!(paths.contains(&"tools/example/echo/src/echo.mjs"));

        let harness_path = package
            .harness
            .as_ref()
            .ok_or("staged harness missing")?
            .path()
            .to_path_buf();
        assert!(harness_path.join("fixtures/helper.mjs").is_file());
        drop(package);
        assert!(!harness_path.exists());
        Ok(())
    }

    #[test]
    fn declared_packet_without_schema_fails_before_publish()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: missing-packet\ndescription: Missing packet fixture.\n---\n# Missing\n",
        )?;
        fs::write(
            root.join("X.yaml"),
            r#"skill: missing-packet
runners:
  main:
    default: true
    type: cli-tool
    command: node
    args: [run.mjs]
    outputs:
      plan: object
    artifacts:
      wrap_as: packet
      packet: runx.test.missing.v1
"#,
        )?;
        fs::write(
            root.join("run.mjs"),
            "console.log(JSON.stringify({plan: {}}))\n",
        )?;

        let error = prepare_registry_publish_package(RegistryPublishPackageRequest {
            subject: root.to_str().ok_or("temporary path is not UTF-8")?,
            profile: None,
            env: &BTreeMap::new(),
            cwd: root,
        })
        .err()
        .ok_or("missing packet schema must fail")?;
        assert!(error.to_string().contains("was not found"));
        Ok(())
    }
}
