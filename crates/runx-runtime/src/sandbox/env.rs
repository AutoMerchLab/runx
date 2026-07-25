use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use runx_contracts::{EnvironmentRequirements, JsonObject};
use runx_core::policy::SandboxProfile;
use runx_parser::SkillSandbox;

use crate::RuntimeError;
use crate::receipts::paths::{RUNX_CWD_ENV, RUNX_RECEIPT_DIR_ENV};
use crate::receipts::signing::{
    RUNX_RECEIPT_VERIFY_ED25519_PUBLIC_KEY_BASE64_ENV, RUNX_RECEIPT_VERIFY_KID_ENV,
};

use super::backend::SandboxRuntime;
use super::policy::{sandbox_violation, workspace_cwd};
use super::template::json_value_env;

const MAX_INLINE_INPUTS_BYTES: usize = 48 * 1024;
const MAX_INLINE_INPUT_VALUE_BYTES: usize = 8 * 1024;

pub(super) fn child_env(
    requirements: &EnvironmentRequirements,
    base_env: &BTreeMap<String, String>,
    inputs: &JsonObject,
    cleanup_paths: &mut Vec<PathBuf>,
) -> Result<BTreeMap<String, String>, RuntimeError> {
    let mut env = child_base_env(base_env)?;
    env.extend(crate::execution_environment::resolve_environment(
        requirements,
        base_env,
    )?);
    let serialized = serde_json::to_string(inputs)
        .map_err(|source| RuntimeError::json("serializing runtime inputs", source))?;
    let (inputs_path, cleanup_path) = write_inputs_file(base_env, &serialized)?;
    env.insert("RUNX_INPUTS_PATH".to_owned(), inputs_path);
    if serialized.len() <= MAX_INLINE_INPUTS_BYTES {
        env.insert("RUNX_INPUTS_JSON".to_owned(), serialized);
    }
    push_cleanup_path(cleanup_paths, cleanup_path.clone());
    let mut input_env_names = BTreeMap::new();
    for (index, (key, value)) in inputs.iter().enumerate() {
        let serialized = json_value_env(value)?;
        let env_name = input_env_name(key);
        let path_env_name = format!("{env_name}_PATH");
        register_input_env_name(&mut input_env_names, &env_name, key)?;
        register_input_env_name(&mut input_env_names, &path_env_name, key)?;
        reject_runtime_input_env_collision(&env, &env_name, key)?;
        reject_runtime_input_env_collision(&env, &path_env_name, key)?;
        let value_path = write_input_value_file(&cleanup_path, index, &serialized)?;
        env.insert(path_env_name, value_path);
        if serialized.len() <= MAX_INLINE_INPUT_VALUE_BYTES {
            env.insert(env_name, serialized);
        }
    }
    Ok(env)
}

pub(super) fn child_base_env(
    base_env: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, RuntimeError> {
    let mut env = allowed_base_env(base_env);
    env.insert(
        RUNX_CWD_ENV.to_owned(),
        workspace_root(base_env)?.to_string_lossy().into_owned(),
    );
    if let Some(receipt_dir) = base_env.get(RUNX_RECEIPT_DIR_ENV) {
        env.insert(RUNX_RECEIPT_DIR_ENV.to_owned(), receipt_dir.clone());
    }
    for key in [
        RUNX_RECEIPT_VERIFY_KID_ENV,
        RUNX_RECEIPT_VERIFY_ED25519_PUBLIC_KEY_BASE64_ENV,
    ] {
        if let Some(value) = base_env.get(key) {
            env.insert(key.to_owned(), value.clone());
        }
    }
    Ok(env)
}

fn workspace_root(base_env: &BTreeMap<String, String>) -> Result<PathBuf, RuntimeError> {
    workspace_cwd(base_env)?.ok_or_else(|| {
        sandbox_violation(format!(
            "sandbox environment requires {} or {}",
            crate::receipts::paths::RUNX_CWD_ENV,
            crate::receipts::paths::INIT_CWD_ENV
        ))
    })
}

fn write_inputs_file(
    base_env: &BTreeMap<String, String>,
    serialized: &str,
) -> Result<(String, PathBuf), RuntimeError> {
    let dir = create_workspace_tmp(base_env, "cli-inputs", "creating inputs temp dir")?;
    let path = dir.join("inputs.json");
    let mut file = fs::File::create(&path)
        .map_err(|source| RuntimeError::io("creating inputs temp file", source))?;
    file.write_all(serialized.as_bytes())
        .map_err(|source| RuntimeError::io("writing inputs temp file", source))?;
    Ok((path.to_string_lossy().into_owned(), dir))
}

fn allowed_base_env(base_env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    crate::execution_environment::process_baseline_environment(base_env)
}

fn reject_runtime_input_env_collision(
    environment: &BTreeMap<String, String>,
    name: &str,
    input: &str,
) -> Result<(), RuntimeError> {
    if environment.contains_key(name) {
        return Err(sandbox_violation(format!(
            "input {input:?} runtime environment variable {name} collides with declared environment"
        )));
    }
    Ok(())
}

fn register_input_env_name<'a>(
    names: &mut BTreeMap<String, &'a str>,
    env_name: &str,
    input: &'a str,
) -> Result<(), RuntimeError> {
    if let Some(prior_key) = names.insert(env_name.to_owned(), input) {
        return Err(sandbox_violation(format!(
            "input keys {prior_key:?} and {input:?} collide on environment variable {env_name}"
        )));
    }
    Ok(())
}

fn write_input_value_file(
    directory: &std::path::Path,
    index: usize,
    serialized: &str,
) -> Result<String, RuntimeError> {
    let path = directory.join(format!("input-{index}.json"));
    let mut file = fs::File::create(&path)
        .map_err(|source| RuntimeError::io("creating input value file", source))?;
    file.write_all(serialized.as_bytes())
        .map_err(|source| RuntimeError::io("writing input value file", source))?;
    Ok(path.to_string_lossy().into_owned())
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
    let private_tmp = create_workspace_tmp(env, "sandbox", "creating sandbox private temp dir")?;
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

fn create_workspace_tmp(
    base_env: &BTreeMap<String, String>,
    label: &str,
    operation: &'static str,
) -> Result<PathBuf, RuntimeError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let path = workspace_root(base_env)?
        .join(".runx")
        .join("tmp")
        .join(format!("{label}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path).map_err(|source| RuntimeError::io(operation, source))?;
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
