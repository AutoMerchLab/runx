use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::RuntimeError;
use crate::effects::RuntimeEffectRegistry;
use crate::execution::harness::HarnessFixtureKind;
use crate::execution::runner::RuntimeOptions;
use crate::execution::skill_front::{PackageHarnessReport, SkillRunError, SkillRunGraphAdapter};
use crate::receipts::paths::{RUNX_CWD_ENV, RUNX_RECEIPT_DIR_ENV};
use crate::receipts::store::LocalReceiptStore;

use super::run_inline_harness_with_effects;
use crate::execution::skill_front::runner_manifest::resolve_skill_dir;

/// Run every harness case owned by a skill package: inline `harness.cases`
/// plus conventional `fixtures/*.yaml` files. Discovery is deterministic and
/// this is the single package entry point used by both the CLI and publishing.
pub(crate) fn run_package_harness_with_effects(
    skill_path: &Path,
    receipt_dir: Option<&Path>,
    env: Option<&BTreeMap<String, String>>,
    effects: &RuntimeEffectRegistry,
) -> Result<PackageHarnessReport, SkillRunError> {
    let skill_dir = resolve_skill_dir(skill_path)?;
    let base_env = env
        .cloned()
        .unwrap_or_else(crate::services::process_env_snapshot);
    let cwd = std::env::current_dir()
        .map_err(|source| RuntimeError::io("resolving cwd for package harness", source))?;
    let operator_workspace = crate::config::resolve_runx_workspace_base(&base_env, &cwd);
    let harness = PackageHarnessEnvironment::prepare(base_env, &operator_workspace, receipt_dir)?;
    let mut report = run_inline_harness_with_effects(
        &skill_dir,
        Some(&harness.receipt_dir),
        Some(&harness.env),
        effects,
    )?;
    replay_conventional_fixtures(&skill_dir, &harness, effects, &mut report)?;
    finalize_report(&mut report);
    Ok(report)
}

fn replay_conventional_fixtures(
    skill_dir: &Path,
    harness: &PackageHarnessEnvironment,
    effects: &RuntimeEffectRegistry,
    report: &mut PackageHarnessReport,
) -> Result<(), SkillRunError> {
    let fixture_paths = conventional_fixture_paths(skill_dir)?;
    if fixture_paths.is_empty() {
        return Ok(());
    }
    let mut options = RuntimeOptions::from_env_or_local_development(harness.env.clone())?;
    options.created_at = crate::time::DEFAULT_CREATED_AT.to_owned();
    options.effects = effects.clone();
    let receipt_store = LocalReceiptStore::new(&harness.receipt_dir);
    for fixture_path in fixture_paths {
        report.case_count += 1;
        match crate::execution::harness::run_harness_fixture_with_adapter(
            &fixture_path,
            SkillRunGraphAdapter::default(),
            options.clone(),
        ) {
            Ok(output) => {
                persist_fixture_receipts(&receipt_store, &options, &output)?;
                if matches!(output.fixture.kind, HarnessFixtureKind::Graph) {
                    report.graph_case_count += 1;
                }
                report.case_names.push(output.fixture.name);
                report.receipt_ids.push(output.receipt.id.to_string());
            }
            Err(error) => report
                .assertion_errors
                .push(format!("{}: {error}", fixture_path.display())),
        }
    }
    Ok(())
}

fn persist_fixture_receipts(
    receipt_store: &LocalReceiptStore,
    options: &RuntimeOptions,
    output: &crate::execution::harness::HarnessReplayOutput,
) -> Result<(), SkillRunError> {
    let policy = options.receipt_signature.signature_policy();
    for receipt in &output.step_receipts {
        receipt_store.write_receipt_with_policy(receipt, policy)?;
    }
    receipt_store.write_receipt_with_policy(&output.receipt, policy)?;
    Ok(())
}

fn finalize_report(report: &mut PackageHarnessReport) {
    report.assertion_error_count = report.assertion_errors.len();
    report.status = if report.assertion_errors.is_empty() {
        "passed"
    } else {
        "failed"
    };
}

struct PackageHarnessEnvironment {
    env: BTreeMap<String, String>,
    receipt_dir: PathBuf,
    scratch_root: Option<PathBuf>,
}

impl PackageHarnessEnvironment {
    fn prepare(
        mut env: BTreeMap<String, String>,
        operator_workspace: &Path,
        receipt_dir: Option<&Path>,
    ) -> Result<Self, SkillRunError> {
        let receipt_dir = receipt_dir
            .map(Path::to_path_buf)
            .or_else(|| env.get(RUNX_RECEIPT_DIR_ENV).map(PathBuf::from))
            .unwrap_or_else(|| operator_workspace.join(".runx").join("receipts"));
        let receipt_dir = if receipt_dir.is_absolute() {
            receipt_dir
        } else {
            operator_workspace.join(receipt_dir)
        };
        env.insert(
            RUNX_RECEIPT_DIR_ENV.to_owned(),
            receipt_dir.to_string_lossy().into_owned(),
        );
        let scratch_root = unique_scratch_root(operator_workspace);
        let workspace = scratch_root.join("workspace");
        fs::create_dir_all(&workspace).map_err(|source| {
            RuntimeError::io(
                format!(
                    "creating isolated harness workspace {}",
                    workspace.display()
                ),
                source,
            )
        })?;
        env.insert(
            RUNX_CWD_ENV.to_owned(),
            workspace.to_string_lossy().into_owned(),
        );
        Ok(Self {
            env,
            receipt_dir,
            scratch_root: Some(scratch_root),
        })
    }
}

fn unique_scratch_root(operator_workspace: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    operator_workspace
        .join(".runx")
        .join("harness")
        .join(format!("run-{}-{nanos}", std::process::id()))
}

impl Drop for PackageHarnessEnvironment {
    fn drop(&mut self) {
        if let Some(root) = self.scratch_root.take() {
            let _ignored = fs::remove_dir_all(root);
        }
    }
}

fn conventional_fixture_paths(skill_dir: &Path) -> Result<Vec<PathBuf>, SkillRunError> {
    let fixtures_dir = skill_dir.join("fixtures");
    let entries = match fs::read_dir(&fixtures_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(
                RuntimeError::io(format!("reading {}", fixtures_dir.display()), source).into(),
            );
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| {
            RuntimeError::io(format!("reading {}", fixtures_dir.display()), source)
        })?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| matches!(extension, "yaml" | "yml"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{PackageHarnessEnvironment, RUNX_CWD_ENV, RUNX_RECEIPT_DIR_ENV};

    #[test]
    fn package_harness_uses_disposable_workspace_owned_runx_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let operator_workspace = unique_test_root("isolated")?;
        fs::create_dir_all(&operator_workspace)?;
        let harness =
            PackageHarnessEnvironment::prepare(BTreeMap::new(), &operator_workspace, None)?;
        let workspace = PathBuf::from(
            harness
                .env
                .get(RUNX_CWD_ENV)
                .ok_or("missing isolated RUNX_CWD")?,
        );
        let scratch_root = harness.scratch_root.clone().ok_or("missing scratch root")?;

        assert!(workspace.starts_with(operator_workspace.join(".runx").join("harness")));
        assert_eq!(workspace, scratch_root.join("workspace"));
        assert_eq!(
            harness.env.get(RUNX_RECEIPT_DIR_ENV),
            Some(
                &operator_workspace
                    .join(".runx")
                    .join("receipts")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        drop(harness);
        assert!(!scratch_root.exists());
        fs::remove_dir_all(operator_workspace)?;
        Ok(())
    }

    #[test]
    fn explicit_harness_workspace_owns_disposable_run_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let operator_workspace = unique_test_root("explicit-operator")?;
        fs::create_dir_all(&operator_workspace)?;
        let mut env = BTreeMap::new();
        env.insert(
            RUNX_CWD_ENV.to_owned(),
            operator_workspace.to_string_lossy().into_owned(),
        );
        let harness = PackageHarnessEnvironment::prepare(env, &operator_workspace, None)?;
        let scratch_root = harness.scratch_root.clone().ok_or("missing scratch root")?;

        assert!(scratch_root.starts_with(operator_workspace.join(".runx").join("harness")));
        assert_eq!(
            harness.env.get(RUNX_CWD_ENV),
            Some(
                &scratch_root
                    .join("workspace")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        drop(harness);
        assert!(!scratch_root.exists());
        fs::remove_dir_all(operator_workspace)?;
        Ok(())
    }

    #[test]
    fn relative_explicit_receipt_dir_is_anchored_before_workspace_isolation()
    -> Result<(), Box<dyn std::error::Error>> {
        let operator_workspace = unique_test_root("explicit-receipts")?;
        fs::create_dir_all(&operator_workspace)?;
        let harness = PackageHarnessEnvironment::prepare(
            BTreeMap::new(),
            &operator_workspace,
            Some(PathBuf::from(".runx/custom-receipts").as_path()),
        )?;

        let expected = operator_workspace.join(".runx").join("custom-receipts");
        assert_eq!(harness.receipt_dir, expected);
        assert_eq!(
            harness.env.get(RUNX_RECEIPT_DIR_ENV),
            Some(&expected.to_string_lossy().into_owned())
        );
        drop(harness);
        fs::remove_dir_all(operator_workspace)?;
        Ok(())
    }

    fn unique_test_root(label: &str) -> Result<PathBuf, std::io::Error> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        Ok(std::env::current_dir()?
            .join(".runx")
            .join("tests")
            .join(format!(
                "package-harness-{label}-{}-{nanos}",
                std::process::id()
            )))
    }
}
