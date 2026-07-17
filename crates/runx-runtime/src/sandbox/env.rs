use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use runx_contracts::JsonObject;
use runx_core::policy::{SandboxProfile, is_reserved_runx_sandbox_env_name};
use runx_parser::SkillSandbox;

use crate::RuntimeError;
use crate::receipts::paths::{RUNX_CWD_ENV, RUNX_RECEIPT_DIR_ENV};

use super::backend::SandboxRuntime;
use super::policy::{sandbox_violation, workspace_cwd};
use super::template::json_value_env;

const MAX_INLINE_INPUTS_BYTES: usize = 48 * 1024;
const MAX_INLINE_INPUT_VALUE_BYTES: usize = 8 * 1024;
pub(super) const DEFAULT_ENV_ALLOWLIST: [&str; 9] = [
    "PATH",
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

pub(super) fn child_env(
    sandbox: Option<&SkillSandbox>,
    base_env: &BTreeMap<String, String>,
    inputs: &JsonObject,
    cleanup_paths: &mut Vec<PathBuf>,
) -> Result<BTreeMap<String, String>, RuntimeError> {
    let mut env = child_base_env(sandbox, base_env)?;
    let serialized = serde_json::to_string(inputs)
        .map_err(|source| RuntimeError::json("serializing runtime inputs", source))?;
    if serialized.len() > MAX_INLINE_INPUTS_BYTES {
        let (inputs_path, cleanup_path) = write_inputs_file(base_env, &serialized)?;
        env.insert("RUNX_INPUTS_PATH".to_owned(), inputs_path);
        push_cleanup_path(cleanup_paths, cleanup_path);
    } else {
        env.insert("RUNX_INPUTS_JSON".to_owned(), serialized);
    }
    let mut input_env_names = BTreeMap::new();
    for (key, value) in inputs {
        let serialized = json_value_env(value)?;
        if serialized.len() <= MAX_INLINE_INPUT_VALUE_BYTES {
            let env_name = input_env_name(key);
            if let Some(prior_key) = input_env_names.insert(env_name.clone(), key) {
                return Err(sandbox_violation(format!(
                    "input keys {prior_key:?} and {key:?} collide on environment variable {env_name}"
                )));
            }
            env.insert(env_name, serialized);
        }
    }
    Ok(env)
}

pub(super) fn child_base_env(
    sandbox: Option<&SkillSandbox>,
    base_env: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, RuntimeError> {
    let mut env = allowed_base_env(sandbox, base_env)?;
    env.insert(
        RUNX_CWD_ENV.to_owned(),
        workspace_root(base_env)?.to_string_lossy().into_owned(),
    );
    if let Some(receipt_dir) = base_env.get(RUNX_RECEIPT_DIR_ENV) {
        env.insert(RUNX_RECEIPT_DIR_ENV.to_owned(), receipt_dir.clone());
    }
    Ok(env)
}

fn workspace_root(base_env: &BTreeMap<String, String>) -> Result<PathBuf, RuntimeError> {
    workspace_cwd(base_env)?.map_or_else(
        || {
            std::env::current_dir()
                .map_err(|source| RuntimeError::io("resolving workspace cwd", source))
        },
        Ok,
    )
}

fn write_inputs_file(
    base_env: &BTreeMap<String, String>,
    serialized: &str,
) -> Result<(String, PathBuf), RuntimeError> {
    let temp_root = base_env
        .get("TMPDIR")
        .or_else(|| base_env.get("TMP"))
        .or_else(|| base_env.get("TEMP"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = temp_root.join(format!("runx-cli-inputs-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir)
        .map_err(|source| RuntimeError::io("creating inputs temp dir", source))?;
    let path = dir.join("inputs.json");
    let mut file = fs::File::create(&path)
        .map_err(|source| RuntimeError::io("creating inputs temp file", source))?;
    file.write_all(serialized.as_bytes())
        .map_err(|source| RuntimeError::io("writing inputs temp file", source))?;
    Ok((path.to_string_lossy().into_owned(), dir))
}

fn allowed_base_env(
    sandbox: Option<&SkillSandbox>,
    base_env: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, RuntimeError> {
    let mut allowed = DEFAULT_ENV_ALLOWLIST
        .iter()
        .filter_map(|key| {
            base_env
                .get(*key)
                .cloned()
                .map(|value| ((*key).to_owned(), value))
        })
        .collect::<BTreeMap<_, _>>();
    if let Some(env_allowlist) = sandbox.and_then(|sandbox| sandbox.env_allowlist.as_ref()) {
        for key in env_allowlist {
            if is_reserved_runx_sandbox_env_name(key) {
                return Err(sandbox_violation(format!(
                    "sandbox env_allowlist cannot include reserved runx environment variable {key}"
                )));
            }
            if let Some(value) = base_env.get(key) {
                allowed.insert(key.clone(), value.clone());
            }
        }
    }
    allowed.retain(|key, _| !is_reserved_runx_sandbox_env_name(key));
    Ok(allowed)
}

pub(super) fn prepare_sandbox_tmp_env(
    sandbox: Option<&SkillSandbox>,
    runtime: &Option<SandboxRuntime>,
    env: &mut BTreeMap<String, String>,
    cleanup_paths: &mut Vec<PathBuf>,
) -> Result<(), RuntimeError> {
    if !sandbox_private_tmp_enabled(sandbox, runtime.as_ref()) {
        return Ok(());
    }
    let private_tmp = create_private_tmp()?;
    let private_tmp_str = private_tmp.to_string_lossy().into_owned();
    env.insert("TMPDIR".to_owned(), private_tmp_str.clone());
    env.insert("TMP".to_owned(), private_tmp_str.clone());
    env.insert("TEMP".to_owned(), private_tmp_str);
    cleanup_paths.push(private_tmp);
    Ok(())
}

pub(super) fn sandbox_private_tmp_enabled(
    sandbox: Option<&SkillSandbox>,
    runtime: Option<&SandboxRuntime>,
) -> bool {
    sandbox.is_some_and(|sandbox| sandbox.profile != SandboxProfile::UnrestrictedLocalDev)
        && !matches!(runtime, Some(SandboxRuntime::Direct))
}

fn create_private_tmp() -> Result<PathBuf, RuntimeError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let path =
        std::env::temp_dir().join(format!("runx-local-sandbox-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path)
        .map_err(|source| RuntimeError::io("creating sandbox private temp dir", source))?;
    Ok(path)
}

fn push_cleanup_path(cleanup_paths: &mut Vec<PathBuf>, cleanup_path: PathBuf) {
    if cleanup_paths
        .iter()
        .any(|existing| cleanup_path.starts_with(existing))
    {
        return;
    }
    cleanup_paths.push(cleanup_path);
}

pub(super) fn cleanup_paths_quietly(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_dir_all(path);
    }
}

fn input_env_name(key: &str) -> String {
    let mut suffix = String::new();
    let mut pending_separator = false;
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_separator && !suffix.is_empty() {
                suffix.push('_');
            }
            suffix.push(ch.to_ascii_uppercase());
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    format!("RUNX_INPUT_{suffix}")
}
