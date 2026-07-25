// Module rationale: the sandbox root owns orchestration
// tests that exercise the split backend, command, env, metadata, and policy
// modules together.
mod backend;
mod command;
mod env;
mod metadata;
mod policy;
mod template;

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use runx_contracts::JsonObject;
use runx_core::policy::{CwdPolicy, SandboxProfile};
use runx_parser::SkillSandbox;
use runx_parser::{SkillMcpServer, SkillSource};

use crate::RuntimeError;

use self::backend::SandboxRuntime;
#[cfg(not(windows))]
use self::backend::resolve_javascript_worker_runtime;
use self::backend::resolve_sandbox_runtime;
#[cfg(not(windows))]
use self::command::javascript_worker_spawn_command;
use self::command::{SandboxSpawnCommand, sandbox_network_enabled, sandbox_spawn_command};
#[cfg(feature = "cli-tool")]
use self::env::child_base_env as sandbox_child_base_env;
use self::env::{
    child_env, cleanup_paths_quietly, prepare_sandbox_tmp_env, sandbox_private_tmp_enabled,
};
use self::metadata::sandbox_metadata_with_runtime;
use self::policy::{
    execution_workspace_root, resolve_cwd, resolve_cwd_value, resolved_skill_directory,
    resolved_writable_paths, validate_sandbox, validated_writable_paths, workspace_cwd,
};
use self::template::resolve_template;

pub use self::metadata::sandbox_metadata;

#[cfg(feature = "cli-tool")]
pub(crate) fn child_base_env(
    base_env: &std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, String>, RuntimeError> {
    sandbox_child_base_env(base_env)
}

pub(crate) const RUNX_SANDBOX_ALLOW_DECLARED_POLICY_ONLY_ENV: &str =
    "RUNX_SANDBOX_ALLOW_DECLARED_POLICY_ONLY";

// One source of truth for the host runtime paths a dynamically linked Linux
// command needs inside bubblewrap. Every consumer uses `--ro-bind-try`, so
// merged-/usr layouts and distributions without one of these paths remain
// portable without widening the sandbox to the host root.
const LINUX_RUNTIME_READONLY_PATHS: [&str; 6] = [
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc/ld.so.cache",
];

#[derive(Clone, Debug, PartialEq)]
pub struct SandboxPlan {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub metadata: JsonObject,
    pub cleanup_paths: Vec<PathBuf>,
}

pub(crate) struct SandboxProcessPlan {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) metadata: JsonObject,
    pub(crate) cleanup_paths: Vec<PathBuf>,
}

impl SandboxPlan {
    pub(crate) fn into_process_plan(mut self) -> SandboxProcessPlan {
        SandboxProcessPlan {
            command: std::mem::take(&mut self.command),
            args: std::mem::take(&mut self.args),
            cwd: std::mem::take(&mut self.cwd),
            env: std::mem::take(&mut self.env),
            metadata: std::mem::take(&mut self.metadata),
            cleanup_paths: std::mem::take(&mut self.cleanup_paths),
        }
    }
}

impl Drop for SandboxPlan {
    fn drop(&mut self) {
        cleanup_paths_quietly(&self.cleanup_paths);
    }
}

pub fn prepare_process_sandbox(
    source: &SkillSource,
    environment: &runx_contracts::EnvironmentRequirements,
    skill_directory: &Path,
    inputs: &JsonObject,
    base_env: &BTreeMap<String, String>,
) -> Result<SandboxPlan, RuntimeError> {
    let command = source.command.clone().ok_or(RuntimeError::MissingCommand)?;
    let sandbox = source.sandbox.as_ref();
    validate_sandbox(sandbox)?;
    let workspace_cwd = workspace_cwd(base_env)?;
    let skill_directory = resolved_skill_directory(skill_directory, workspace_cwd.as_deref())?;
    let workspace_root = execution_workspace_root(workspace_cwd.as_deref(), &skill_directory);
    let cwd = resolve_cwd(source, sandbox, &skill_directory, workspace_cwd.as_deref())?;
    let args = source
        .args
        .iter()
        .map(|arg| resolve_template(arg, inputs, base_env))
        .collect();
    let writable_paths = resolved_writable_paths(sandbox, inputs, base_env);
    let validated_writable_paths =
        validated_writable_paths(sandbox, &writable_paths, &cwd, &workspace_root)?;
    let runtime = resolve_sandbox_runtime(sandbox, base_env)?;
    let private_tmp_enabled = sandbox_private_tmp_enabled(sandbox, runtime.as_ref());
    let mut cleanup_paths = Vec::new();
    let mut sandbox_base_env = base_env.clone();
    sandbox_base_env.insert(
        crate::receipts::paths::RUNX_CWD_ENV.to_owned(),
        workspace_root.to_string_lossy().into_owned(),
    );
    prepare_sandbox_tmp_env(sandbox, &runtime, &mut sandbox_base_env, &mut cleanup_paths)?;
    let env = match child_env(environment, &sandbox_base_env, inputs, &mut cleanup_paths) {
        Ok(env) => env,
        Err(error) => {
            cleanup_paths_quietly(&cleanup_paths);
            return Err(error);
        }
    };
    prepare_writable_bind_sources_or_cleanup(
        runtime.as_ref(),
        &validated_writable_paths,
        &cleanup_paths,
    )?;
    let (command, args) = sandbox_spawn_command(SandboxSpawnCommand {
        runtime: runtime.as_ref(),
        command,
        args,
        cwd: &cwd,
        skill_directory: &skill_directory,
        workspace_cwd: &workspace_root,
        writable_paths: &validated_writable_paths,
        network: sandbox_network_enabled(sandbox),
        private_tmp: cleanup_paths.first().map(PathBuf::as_path),
    });
    Ok(SandboxPlan {
        command,
        args,
        cwd,
        env,
        metadata: sandbox_metadata_with_runtime(
            sandbox,
            environment,
            &writable_paths,
            runtime.as_ref(),
            private_tmp_enabled,
        ),
        cleanup_paths,
    })
}

/// Prepare the fixed sandbox used by the generic native command capability.
///
/// Unlike a package-declared CLI tool, `command.execute` cannot choose its
/// sandbox, network, environment allowlist, or credential delivery. The
/// runtime pins those controls here and requires a real platform enforcer.
#[cfg(feature = "cli-tool")]
pub(crate) fn prepare_native_command_sandbox(
    command: String,
    args: Vec<String>,
    cwd: &Path,
    workspace_root: &Path,
    explicit_env: &BTreeMap<String, String>,
    base_env: &BTreeMap<String, String>,
) -> Result<SandboxPlan, RuntimeError> {
    let sandbox = native_command_sandbox(workspace_root);
    validate_sandbox(Some(&sandbox))?;

    let writable_paths = resolved_writable_paths(Some(&sandbox), &JsonObject::new(), base_env);
    let validated_writable_paths =
        validated_writable_paths(Some(&sandbox), &writable_paths, cwd, workspace_root)?;
    let mut sandbox_base_env = base_env.clone();
    sandbox_base_env.insert(
        crate::receipts::paths::RUNX_CWD_ENV.to_owned(),
        workspace_root.to_string_lossy().into_owned(),
    );
    let runtime = resolve_sandbox_runtime(Some(&sandbox), &sandbox_base_env)?;
    let private_tmp_enabled = sandbox_private_tmp_enabled(Some(&sandbox), runtime.as_ref());
    let mut cleanup_paths = Vec::new();
    prepare_sandbox_tmp_env(
        Some(&sandbox),
        &runtime,
        &mut sandbox_base_env,
        &mut cleanup_paths,
    )?;
    let mut env = sandbox_child_base_env(&sandbox_base_env)?;
    env.extend(explicit_env.clone());
    prepare_writable_bind_sources_or_cleanup(
        runtime.as_ref(),
        &validated_writable_paths,
        &cleanup_paths,
    )?;
    let (command, args) = sandbox_spawn_command(SandboxSpawnCommand {
        runtime: runtime.as_ref(),
        command,
        args,
        cwd,
        skill_directory: workspace_root,
        workspace_cwd: workspace_root,
        writable_paths: &validated_writable_paths,
        network: false,
        private_tmp: cleanup_paths.first().map(PathBuf::as_path),
    });
    Ok(SandboxPlan {
        command,
        args,
        cwd: cwd.to_path_buf(),
        env,
        metadata: sandbox_metadata_with_runtime(
            Some(&sandbox),
            &runx_contracts::EnvironmentRequirements::default(),
            &writable_paths,
            runtime.as_ref(),
            private_tmp_enabled,
        ),
        cleanup_paths,
    })
}

#[cfg(feature = "cli-tool")]
fn native_command_sandbox(workspace_root: &Path) -> SkillSandbox {
    SkillSandbox {
        profile: SandboxProfile::WorkspaceWrite,
        cwd_policy: Some(CwdPolicy::Workspace),
        network: Some(false),
        writable_paths: vec![workspace_root.to_string_lossy().into_owned()],
        require_enforcement: Some(true),
        approved_escalation: None,
        raw: JsonObject::new(),
    }
}

/// Prepare the runtime-owned containment boundary for the deterministic
/// JavaScript worker. Package authors cannot influence this plan: it carries no
/// package/workspace mount, environment, credential, writable path, or network
/// permission.
pub(crate) fn prepare_javascript_worker_sandbox(
    worker_path: &Path,
) -> Result<SandboxPlan, RuntimeError> {
    let cwd = javascript_worker_cwd()?;
    #[cfg(windows)]
    {
        return Ok(SandboxPlan {
            command: worker_path.to_string_lossy().into_owned(),
            args: Vec::new(),
            cwd,
            env: BTreeMap::new(),
            metadata: JsonObject::new(),
            cleanup_paths: Vec::new(),
        });
    }

    #[cfg(not(windows))]
    {
        let sandbox = SkillSandbox {
            profile: SandboxProfile::Readonly,
            cwd_policy: Some(CwdPolicy::SkillDirectory),
            network: Some(false),
            writable_paths: Vec::new(),
            require_enforcement: Some(true),
            approved_escalation: None,
            raw: JsonObject::new(),
        };
        let runtime = resolve_javascript_worker_runtime()?;
        let (command, args) = javascript_worker_spawn_command(runtime.as_ref(), worker_path, &cwd)?;
        Ok(SandboxPlan {
            command,
            args,
            cwd,
            env: BTreeMap::new(),
            metadata: sandbox_metadata_with_runtime(
                Some(&sandbox),
                &runx_contracts::EnvironmentRequirements::default(),
                &[],
                runtime.as_ref(),
                false,
            ),
            cleanup_paths: Vec::new(),
        })
    }
}

fn javascript_worker_cwd() -> Result<PathBuf, RuntimeError> {
    let cwd = std::env::temp_dir();
    std::fs::canonicalize(&cwd)
        .map_err(|source| RuntimeError::io("resolving deterministic worker cwd", source))
}

pub fn prepare_mcp_process_sandbox(
    source: &SkillSource,
    environment: &runx_contracts::EnvironmentRequirements,
    server: &SkillMcpServer,
    skill_directory: &Path,
    base_env: &BTreeMap<String, String>,
) -> Result<SandboxPlan, RuntimeError> {
    let sandbox = source.sandbox.as_ref();
    validate_sandbox(sandbox)?;
    let workspace_cwd = workspace_cwd(base_env)?;
    let skill_directory = resolved_skill_directory(skill_directory, workspace_cwd.as_deref())?;
    let workspace_root = execution_workspace_root(workspace_cwd.as_deref(), &skill_directory);
    let cwd = resolve_cwd_value(
        server.cwd.as_deref(),
        sandbox,
        &skill_directory,
        workspace_cwd.as_deref(),
    )?;
    let writable_paths = resolved_writable_paths(sandbox, &JsonObject::new(), base_env);
    let validated_writable_paths =
        validated_writable_paths(sandbox, &writable_paths, &cwd, &workspace_root)?;
    let runtime = resolve_sandbox_runtime(sandbox, base_env)?;
    let private_tmp_enabled = sandbox_private_tmp_enabled(sandbox, runtime.as_ref());
    let mut cleanup_paths = Vec::new();
    let mut sandbox_base_env = base_env.clone();
    sandbox_base_env.insert(
        crate::receipts::paths::RUNX_CWD_ENV.to_owned(),
        workspace_root.to_string_lossy().into_owned(),
    );
    prepare_sandbox_tmp_env(sandbox, &runtime, &mut sandbox_base_env, &mut cleanup_paths)?;
    let env = match child_env(
        environment,
        &sandbox_base_env,
        &JsonObject::new(),
        &mut cleanup_paths,
    ) {
        Ok(env) => env,
        Err(error) => {
            cleanup_paths_quietly(&cleanup_paths);
            return Err(error);
        }
    };
    prepare_writable_bind_sources_or_cleanup(
        runtime.as_ref(),
        &validated_writable_paths,
        &cleanup_paths,
    )?;
    let (command, args) = sandbox_spawn_command(SandboxSpawnCommand {
        runtime: runtime.as_ref(),
        command: server.command.clone(),
        args: server.args.clone(),
        cwd: &cwd,
        skill_directory: &skill_directory,
        workspace_cwd: &workspace_root,
        writable_paths: &validated_writable_paths,
        network: sandbox_network_enabled(sandbox),
        private_tmp: cleanup_paths.first().map(PathBuf::as_path),
    });
    Ok(SandboxPlan {
        command,
        args,
        cwd,
        env,
        metadata: sandbox_metadata_with_runtime(
            sandbox,
            environment,
            &writable_paths,
            runtime.as_ref(),
            private_tmp_enabled,
        ),
        cleanup_paths,
    })
}

// Bubblewrap bind sources must exist on the host. A missing exact writable path
// is therefore materialized as a file before spawn; writable directories must
// already exist so the runtime never guesses a broader directory grant.
fn prepare_writable_bind_sources(
    runtime: Option<&SandboxRuntime>,
    writable_paths: &[PathBuf],
) -> Result<(), RuntimeError> {
    if !matches!(runtime, Some(SandboxRuntime::Bubblewrap { .. })) {
        return Ok(());
    }
    let mut created_paths = Vec::new();
    let result = writable_paths.iter().try_for_each(|path| {
        if path.exists() {
            return Ok(());
        }
        if !path.parent().is_some_and(Path::is_dir) {
            return Err(RuntimeError::io(
                format!("materializing sandbox writable file {}", path.display()),
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "writable file parent directory does not exist",
                ),
            ));
        }
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| {
                RuntimeError::io(
                    format!("materializing sandbox writable file {}", path.display()),
                    source,
                )
            })?;
        created_paths.push(path.clone());
        Ok(())
    });
    if result.is_err() {
        for path in created_paths {
            let _ignored = std::fs::remove_file(path);
        }
    }
    result
}

fn prepare_writable_bind_sources_or_cleanup(
    runtime: Option<&SandboxRuntime>,
    writable_paths: &[PathBuf],
    cleanup_paths: &[PathBuf],
) -> Result<(), RuntimeError> {
    if let Err(error) = prepare_writable_bind_sources(runtime, writable_paths) {
        cleanup_paths_quietly(cleanup_paths);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    use runx_contracts::{JsonObject, JsonValue};
    use runx_core::policy::SandboxProfile;
    use runx_parser::{SkillSandbox, SourceKind};

    use super::backend::{SandboxRuntime, find_trusted_executable};
    use super::command::{
        sandbox_exec_path_filter_path, sandbox_exec_profile, sandbox_profile_string,
    };
    use super::env::{cleanup_paths_quietly, prepare_sandbox_tmp_env};
    use super::policy::{resolved_writable_paths, validated_writable_paths};

    #[test]
    fn bubblewrap_materializes_a_missing_exact_writable_file() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let output = temp.path().join("output.json");
        let runtime = SandboxRuntime::Bubblewrap {
            path: PathBuf::from("/usr/bin/bwrap"),
        };

        prepare_writable_bind_sources(Some(&runtime), std::slice::from_ref(&output))
            .map_err(|source| source.to_string())?;

        assert!(output.is_file());
        Ok(())
    }

    #[test]
    fn bubblewrap_rolls_back_files_when_a_writable_parent_is_missing() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let materialized = temp.path().join("output.json");
        let output = temp.path().join("missing/output.json");
        let runtime = SandboxRuntime::Bubblewrap {
            path: PathBuf::from("/usr/bin/bwrap"),
        };

        let error =
            prepare_writable_bind_sources(Some(&runtime), &[materialized.clone(), output.clone()])
                .err()
                .ok_or_else(|| "missing writable parent unexpectedly materialized".to_owned())?;

        assert!(
            error
                .to_string()
                .contains("parent directory does not exist")
        );
        assert!(!materialized.exists());
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn writable_paths_omit_unresolved_optional_templates() {
        let sandbox = SkillSandbox {
            profile: SandboxProfile::WorkspaceWrite,
            cwd_policy: None,
            network: None,
            writable_paths: vec![
                "{{workspace_path}}".to_owned(),
                "{{ fixture }}".to_owned(),
                "{{ env.RUNX_EFFECT_COUNT_PATH }}".to_owned(),
                "logs".to_owned(),
            ],
            require_enforcement: None,
            approved_escalation: None,
            raw: JsonObject::new(),
        };
        let inputs = [(
            "fixture".to_owned(),
            JsonValue::String("/tmp/runx-fixture".to_owned()),
        )]
        .into_iter()
        .collect();
        let env = [(
            "RUNX_EFFECT_COUNT_PATH".to_owned(),
            "/tmp/runx-effect-count.txt".to_owned(),
        )]
        .into_iter()
        .collect();

        assert_eq!(
            resolved_writable_paths(Some(&sandbox), &inputs, &env),
            vec![
                "/tmp/runx-fixture".to_owned(),
                "/tmp/runx-effect-count.txt".to_owned(),
                "logs".to_owned()
            ]
        );
    }

    #[test]
    fn trusted_enforcer_lookup_ignores_caller_path() {
        let trusted = find_trusted_executable("runx-test-enforcer-that-should-not-exist");
        assert!(trusted.is_none());
    }

    #[test]
    fn sandbox_exec_runtime_gets_private_writable_tmp_env() -> Result<(), String> {
        let workspace = tempfile::tempdir().map_err(|source| source.to_string())?;
        let sandbox = readonly_sandbox();
        let runtime = Some(SandboxRuntime::SandboxExec {
            path: PathBuf::from("/usr/bin/sandbox-exec"),
        });
        let mut env = BTreeMap::from([(
            crate::receipts::paths::RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);
        let mut cleanup_paths = Vec::new();
        prepare_sandbox_tmp_env(Some(&sandbox), &runtime, &mut env, &mut cleanup_paths)
            .map_err(|source| source.to_string())?;

        let tmpdir = env
            .get("TMPDIR")
            .ok_or_else(|| "TMPDIR was not set".to_owned())?;
        assert_eq!(env.get("TMP"), Some(tmpdir));
        assert_eq!(env.get("TEMP"), Some(tmpdir));
        assert_eq!(cleanup_paths, vec![PathBuf::from(tmpdir)]);
        assert!(Path::new(tmpdir).is_dir());
        assert!(Path::new(tmpdir).starts_with(workspace.path().join(".runx").join("tmp")));

        let profile =
            sandbox_exec_profile(Path::new("/workspace"), &[], true, Some(Path::new(tmpdir)));
        assert!(profile.contains("(allow file-write* (literal \"/dev/null\"))"));
        assert!(profile.contains("(allow mach-lookup)"));
        let tmp_filter_path = sandbox_exec_path_filter_path(Path::new(tmpdir));
        assert!(profile.contains(&format!(
            "(subpath \"{}\")",
            sandbox_profile_string(&tmp_filter_path)
        )));
        cleanup_paths_quietly(&cleanup_paths);
        Ok(())
    }

    #[test]
    fn sandbox_exec_profile_keeps_legitimate_writable_path() {
        let profile = sandbox_exec_profile(
            Path::new("/workspace"),
            &[PathBuf::from("/workspace/logs/output")],
            false,
            None,
        );

        assert!(profile.contains("(allow file-write* (literal \"/workspace/logs/output\"))"));
        assert!(!profile.contains("(allow network*)"));
    }

    #[test]
    fn sandbox_exec_profile_sanitizes_metacharacters_if_validation_is_bypassed() {
        let profile = sandbox_exec_profile(
            Path::new("/workspace"),
            &[PathBuf::from("safe\")) (allow network*)")],
            false,
            None,
        );

        assert!(!profile.contains("(allow network*)"));
        assert!(!profile.contains("(subpath \"/\""));
    }

    #[test]
    fn declared_policy_runtime_gets_private_tmp_env() -> Result<(), String> {
        let workspace = tempfile::tempdir().map_err(|source| source.to_string())?;
        let sandbox = readonly_sandbox();
        let runtime = Some(SandboxRuntime::DeclaredPolicyOnly {
            reason: "missing test backend".to_owned(),
        });
        let mut env = BTreeMap::from([(
            crate::receipts::paths::RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);
        let mut cleanup_paths = Vec::new();
        prepare_sandbox_tmp_env(Some(&sandbox), &runtime, &mut env, &mut cleanup_paths)
            .map_err(|source| source.to_string())?;

        let tmpdir = env
            .get("TMPDIR")
            .ok_or_else(|| "TMPDIR was not set".to_owned())?;
        assert_eq!(env.get("TMP"), Some(tmpdir));
        assert_eq!(env.get("TEMP"), Some(tmpdir));
        assert_eq!(cleanup_paths, vec![PathBuf::from(tmpdir)]);
        assert!(Path::new(tmpdir).is_dir());
        assert!(Path::new(tmpdir).starts_with(workspace.path().join(".runx").join("tmp")));

        cleanup_paths_quietly(&cleanup_paths);
        Ok(())
    }

    #[test]
    fn process_child_env_strips_receipt_signing_env() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let source = source_for_child_env(SourceKind::CliTool);
        let base_env = signing_env(temp.path());

        let plan = prepare_process_sandbox(
            &source,
            &Default::default(),
            temp.path(),
            &JsonObject::new(),
            &base_env,
        )
        .map_err(|source| source.to_string())?;

        assert_child_env_has_no_receipt_signing_env(&plan.env);
        Ok(())
    }

    #[test]
    fn process_child_env_rejects_declared_environment_that_shadows_runtime_input()
    -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let source = source_for_child_env(SourceKind::CliTool);
        let mut base_env = signing_env(temp.path());
        base_env.insert("RUNX_INPUT_MESSAGE".to_owned(), "declared".to_owned());
        let requirements = runx_contracts::EnvironmentRequirements {
            required: vec!["RUNX_INPUT_MESSAGE".to_owned()],
            optional: Vec::new(),
        };
        let inputs = JsonObject::from([(
            "message".to_owned(),
            JsonValue::String("runtime".to_owned()),
        )]);

        let error =
            prepare_process_sandbox(&source, &requirements, temp.path(), &inputs, &base_env)
                .err()
                .ok_or_else(|| "runtime input collision unexpectedly passed".to_owned())?;

        assert!(
            error
                .to_string()
                .contains("collides with declared environment")
        );
        Ok(())
    }

    #[test]
    fn mcp_process_child_env_strips_receipt_signing_env() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let source = source_for_child_env(SourceKind::Mcp);
        let server = SkillMcpServer {
            command: "node".to_owned(),
            args: vec!["server.mjs".to_owned()],
            cwd: Some(temp.path().to_string_lossy().into_owned()),
        };
        let base_env = signing_env(temp.path());

        let plan = prepare_mcp_process_sandbox(
            &source,
            &Default::default(),
            &server,
            temp.path(),
            &base_env,
        )
        .map_err(|source| source.to_string())?;

        assert_child_env_has_no_receipt_signing_env(&plan.env);
        Ok(())
    }

    fn readonly_sandbox() -> SkillSandbox {
        SkillSandbox {
            profile: SandboxProfile::Readonly,
            cwd_policy: None,
            network: None,
            writable_paths: Vec::new(),
            require_enforcement: None,
            approved_escalation: None,
            raw: JsonObject::new(),
        }
    }

    fn source_for_child_env(source_type: SourceKind) -> SkillSource {
        SkillSource {
            act: None,
            source_type,
            command: Some("node".to_owned()),
            module: None,
            javascript_export: None,
            pages: None,
            args: vec!["script.mjs".to_owned()],
            cwd: None,
            timeout_seconds: None,
            input_mode: None,
            environment: Default::default(),
            sandbox: None,
            server: None,
            tool: None,
            arguments: None,
            agent_card_url: None,
            agent_identity: None,
            agent: None,
            task: None,
            outputs: None,
            graph: None,
            external_adapter: None,
            thread_outbox_provider: None,
            raw: JsonObject::new(),
        }
    }

    fn signing_env(workspace: &Path) -> BTreeMap<String, String> {
        [
            ("PATH".to_owned(), "/usr/bin".to_owned()),
            (
                crate::receipts::RUNX_RECEIPT_SIGN_KID_ENV.to_owned(),
                "kid_prod".to_owned(),
            ),
            (
                crate::receipts::RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64_ENV.to_owned(),
                "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=".to_owned(),
            ),
            (
                crate::receipts::RUNX_RECEIPT_SIGN_ISSUER_TYPE_ENV.to_owned(),
                "hosted".to_owned(),
            ),
            (
                crate::receipts::paths::RUNX_CWD_ENV.to_owned(),
                workspace.to_string_lossy().into_owned(),
            ),
        ]
        .into_iter()
        .collect()
    }

    fn assert_child_env_has_no_receipt_signing_env(env: &BTreeMap<String, String>) {
        assert!(!env.contains_key(crate::receipts::RUNX_RECEIPT_SIGN_KID_ENV));
        assert!(!env.contains_key(crate::receipts::RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64_ENV));
        assert!(!env.contains_key(crate::receipts::RUNX_RECEIPT_SIGN_ISSUER_TYPE_ENV));
        assert_eq!(env.get("PATH"), Some(&"/usr/bin".to_owned()));
    }

    #[test]
    fn writable_path_rejects_sexpr_metacharacters() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).map_err(|source| source.to_string())?;
        let sandbox = SkillSandbox {
            profile: SandboxProfile::WorkspaceWrite,
            cwd_policy: None,
            network: None,
            writable_paths: Vec::new(),
            require_enforcement: None,
            approved_escalation: None,
            raw: JsonObject::new(),
        };

        let error = validated_writable_paths(
            Some(&sandbox),
            &["safe\")) (allow network*)".to_owned()],
            &workspace,
            &workspace,
        )
        .err()
        .ok_or_else(|| "sexpr metacharacter path unexpectedly passed".to_owned())?;

        assert!(
            error.to_string().contains("profile metacharacters"),
            "unexpected error: {error}"
        );
        Ok(())
    }

    #[test]
    fn workspace_write_allows_uncreated_nested_workspace_path() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).map_err(|source| source.to_string())?;
        let sandbox = SkillSandbox {
            profile: SandboxProfile::WorkspaceWrite,
            cwd_policy: None,
            network: None,
            writable_paths: Vec::new(),
            require_enforcement: None,
            approved_escalation: None,
            raw: JsonObject::new(),
        };

        validated_writable_paths(
            Some(&sandbox),
            &["dist/cache/output.json".to_owned()],
            &workspace,
            &workspace,
        )
        .map(|_| ())
        .map_err(|source| source.to_string())
    }

    #[test]
    #[cfg(unix)]
    fn workspace_write_rejects_symlink_escape() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|source| source.to_string())?;
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&workspace).map_err(|source| source.to_string())?;
        fs::create_dir_all(&outside).map_err(|source| source.to_string())?;
        std::os::unix::fs::symlink(&outside, workspace.join("link"))
            .map_err(|source| source.to_string())?;
        let sandbox = SkillSandbox {
            profile: SandboxProfile::WorkspaceWrite,
            cwd_policy: None,
            network: None,
            writable_paths: Vec::new(),
            require_enforcement: None,
            approved_escalation: None,
            raw: JsonObject::new(),
        };

        let error = validated_writable_paths(
            Some(&sandbox),
            &["link/escape.txt".to_owned()],
            &workspace,
            &workspace,
        )
        .err()
        .ok_or_else(|| "symlink escape unexpectedly passed".to_owned())?;

        assert!(
            error.to_string().contains("outside workspace"),
            "unexpected error: {error}"
        );
        Ok(())
    }
}
