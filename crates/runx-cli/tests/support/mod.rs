// Each integration test compiles this module separately and uses a different helper subset.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_norway::{Mapping, Value};

const FIXTURE_SIGNING_SEED: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

pub fn repo_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}

pub fn temp_root(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let root = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    if root.exists() {
        let _ignored = fs::remove_dir_all(&root);
    }
    root
}

pub fn isolated_target_temp_root(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = repo_root()?
        .join("crates")
        .join("target")
        .join(prefix)
        .join(format!("{}-{nanos}", std::process::id()));
    fs::remove_dir_all(&path).ok();
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn signed_runx_command(signing_key_id: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runx"));
    command.env("NO_COLOR", "1");
    apply_fixture_signing(&mut command, signing_key_id);
    command
}

pub fn isolated_runx_command(signing_key_id: &str) -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = isolated_runx_command_with_inherited_cwd(signing_key_id);
    command.current_dir(repo_root()?);
    Ok(command)
}

pub fn isolated_runx_command_with_inherited_cwd(signing_key_id: &str) -> Command {
    let mut command = unsigned_runx_command_with_inherited_cwd();
    apply_fixture_signing(&mut command, signing_key_id);
    command
}

pub fn unsigned_runx_command_with_inherited_cwd() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_runx"));
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command.env("NO_COLOR", "1");
    command
}

pub fn unsigned_runx_command_at(root: &Path) -> Command {
    let mut command = unsigned_runx_command_with_inherited_cwd();
    command
        .current_dir(root)
        .env("RUNX_HOME", root.join("home"));
    command
}

pub fn apply_fixture_signing(command: &mut Command, signing_key_id: &str) {
    command.env("RUNX_RECEIPT_SIGN_KID", signing_key_id);
    command.env(
        "RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64",
        FIXTURE_SIGNING_SEED,
    );
    command.env("RUNX_RECEIPT_SIGN_ISSUER_TYPE", "hosted");
}

pub struct GovernedHarnessFixture {
    path: PathBuf,
    root: PathBuf,
}

impl GovernedHarnessFixture {
    pub fn path_str(&self) -> Result<&str, Box<dyn std::error::Error>> {
        self.path
            .to_str()
            .ok_or_else(|| "non-utf8 governed harness path".into())
    }
}

impl Drop for GovernedHarnessFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

pub fn governed_harness_fixture(
    fixture: &str,
) -> Result<GovernedHarnessFixture, Box<dyn std::error::Error>> {
    let repo = repo_root()?;
    let source_path = repo.join(fixture);
    let source = fs::read_to_string(&source_path)?;
    let parent = source_path
        .parent()
        .ok_or("harness fixture path has no parent")?;
    let root = isolated_target_temp_root("governed-harness")?;
    let file_name = source_path
        .file_name()
        .ok_or("harness fixture path has no file name")?;
    let path = root.join(file_name);
    fs::write(&path, governed_harness_yaml(&source, parent)?)?;
    Ok(GovernedHarnessFixture { path, root })
}

pub fn write_agent_task_skill(root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let skill_dir = root.join("issue-intake");
    fs::create_dir_all(&skill_dir)?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: issue-intake\n---\n# Issue Intake\n",
    )?;
    fs::write(
        skill_dir.join("X.yaml"),
        r#"
skill: issue-intake
runners:
  intake:
    default: true
    type: agent-task
    agent: builder
    task: issue-intake
    outputs:
      intake_report: object
    inputs:
      thread_title:
        type: string
        required: false
"#,
    )?;
    Ok(skill_dir)
}

fn governed_harness_yaml(
    source: &str,
    fixture_parent: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut document = serde_norway::from_str::<Value>(source)?;
    let root = document
        .as_mapping_mut()
        .ok_or("harness fixture root must be a mapping")?;
    rewrite_harness_target(root, fixture_parent)?;
    rewrite_receipt_expectation(root)?;
    Ok(serde_norway::to_string(&document)?)
}

fn rewrite_harness_target(
    root: &mut Mapping,
    fixture_parent: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(target) = root.get_mut("target") else {
        return Ok(());
    };
    let target_path = target
        .as_str()
        .ok_or("harness fixture target must be a string")?;
    let absolute_target = if Path::new(target_path).is_absolute() {
        PathBuf::from(target_path)
    } else {
        fixture_parent.join(target_path)
    }
    .canonicalize()?;
    *target = Value::String(absolute_target.to_string_lossy().into_owned());
    Ok(())
}

fn rewrite_receipt_expectation(root: &mut Mapping) -> Result<(), Box<dyn std::error::Error>> {
    let Some(expect) = root.get_mut("expect") else {
        return Ok(());
    };
    let expect = expect
        .as_mapping_mut()
        .ok_or("harness fixture expect must be a mapping")?;
    if expect.contains_key("receipt") {
        let mut receipt = Mapping::new();
        receipt.insert(
            Value::String("schema".to_owned()),
            Value::String("runx.receipt.v1".to_owned()),
        );
        expect.insert(Value::String("receipt".to_owned()), Value::Mapping(receipt));
    }
    Ok(())
}
