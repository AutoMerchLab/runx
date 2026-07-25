use std::collections::BTreeMap;
use std::path::Path;

#[cfg(feature = "mcp")]
use runx_parser::SkillMcpServer;
use runx_parser::SkillSource;

use crate::RuntimeError;
use crate::sandbox::SandboxPlan;
#[cfg(feature = "mcp")]
use crate::sandbox::prepare_mcp_process_sandbox;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SandboxServices;

impl SandboxServices {
    #[cfg(feature = "cli-tool")]
    pub(crate) fn child_base_env(
        self,
        base_env: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        crate::sandbox::child_base_env(base_env)
    }

    #[cfg(any(feature = "cli-tool", feature = "external-adapter"))]
    pub(crate) fn process_plan(
        self,
        source: &SkillSource,
        environment: &runx_contracts::EnvironmentRequirements,
        skill_directory: &Path,
        inputs: &runx_contracts::JsonObject,
        base_env: &BTreeMap<String, String>,
    ) -> Result<SandboxPlan, RuntimeError> {
        crate::sandbox::prepare_process_sandbox(
            source,
            environment,
            skill_directory,
            inputs,
            base_env,
        )
    }

    #[cfg(feature = "cli-tool")]
    pub(crate) fn native_command_plan(
        self,
        command: String,
        args: Vec<String>,
        cwd: &Path,
        workspace_root: &Path,
        explicit_env: &BTreeMap<String, String>,
        base_env: &BTreeMap<String, String>,
    ) -> Result<SandboxPlan, RuntimeError> {
        crate::sandbox::prepare_native_command_sandbox(
            command,
            args,
            cwd,
            workspace_root,
            explicit_env,
            base_env,
        )
    }

    #[cfg(feature = "mcp")]
    pub(crate) fn mcp_process_plan(
        self,
        source: &SkillSource,
        environment: &runx_contracts::EnvironmentRequirements,
        server: &SkillMcpServer,
        skill_directory: &Path,
        base_env: &BTreeMap<String, String>,
    ) -> Result<SandboxPlan, RuntimeError> {
        prepare_mcp_process_sandbox(source, environment, server, skill_directory, base_env)
    }
}
