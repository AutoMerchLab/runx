//! Native exact-command execution for project-owned operator profiles.
//!
//! Skills retain their domain-specific profile and result semantics. Runx owns
//! argv execution, workspace containment, environment admission, process-tree
//! supervision, output bounds, credential redaction, and evidence digests.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use runx_contracts::{JsonNumber, JsonObject, JsonValue};

use super::{NativeInvocation, invalid_input};
use crate::RuntimeError;
use crate::process::{ProcessSpec, STANDARD_PROCESS_OUTPUT_BYTES, run_process};
use crate::services::{NativeCommandSandboxRequest, SandboxServices};

mod capability;
mod input;
mod result;

pub(super) use capability::CAPABILITIES;
use capability::CommandInput;
use capability::{CommandExecutionOutput, CommandPlan, CommandPlanOutput};

use input::prepare;
use result::{observe_command, render_execution};

const TOOL: &str = "command.execute";
const MIN_TIMEOUT_MS: u64 = 1_000;
const MAX_TIMEOUT_MS: u64 = 900_000;
const OUTPUT_LIMIT_BYTES: usize = STANDARD_PROCESS_OUTPUT_BYTES;
const MAX_COMMAND_ARGS: usize = 128;
const MAX_COMMAND_ARG_BYTES: usize = 8 * 1024;
const MAX_ENV: usize = 64;
const MAX_ENV_VALUE_BYTES: usize = 8 * 1024;
const NETWORK_SCOPE: &str = "net:process";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputMode {
    Digest,
    Text,
    Json,
}

impl OutputMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Digest => "digest",
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

struct PreparedCommand {
    command: String,
    args: Vec<String>,
    repo_root: PathBuf,
    cwd: PathBuf,
    cwd_relative: String,
    explicit_env: BTreeMap<String, String>,
    network: bool,
    timeout_ms: u64,
    output_mode: OutputMode,
    command_digest: String,
}

fn plan(
    invocation: &NativeInvocation<'_, CommandInput>,
) -> Result<CommandPlanOutput, RuntimeError> {
    let command = prepare(invocation)?;
    Ok(CommandPlanOutput {
        command_plan: CommandPlan {
            schema: "runx.command.plan.v1".to_owned(),
            command_digest: command.command_digest,
            cwd: command.cwd_relative,
            network: command.network,
            timeout_ms: command.timeout_ms,
            output_mode: command.output_mode.as_str().to_owned(),
            env_names: command.explicit_env.keys().cloned().collect(),
        },
    })
}

fn execute(
    invocation: &NativeInvocation<'_, CommandInput>,
) -> Result<CommandExecutionOutput, RuntimeError> {
    let command = prepare(invocation)?;
    invocation
        .credential_delivery
        .reject_process_env_boundary("native command.execute")
        .map_err(|error| invalid_input(TOOL, error.to_string()))?;
    let sandbox = SandboxServices.native_command_plan(NativeCommandSandboxRequest {
        command: command.command.clone(),
        args: command.args.clone(),
        cwd: &command.cwd,
        workspace_root: &command.repo_root,
        explicit_env: &command.explicit_env,
        network: command.network,
        base_env: invocation.env,
    })?;
    let sandbox = sandbox.into_process_plan();
    let outcome = run_process(
        ProcessSpec::new("native command", sandbox.command, OUTPUT_LIMIT_BYTES)
            .args(sandbox.args)
            .cwd(sandbox.cwd)
            .env(sandbox.env)
            .timeout(Some(Duration::from_millis(command.timeout_ms)))
            .cleanup_paths(sandbox.cleanup_paths),
    )
    .map_err(|error| invalid_input(TOOL, error.to_string()))?;
    let observation = observe_command(command.output_mode, outcome, invocation.credential_delivery);
    render_execution(command, observation)
}

fn exit_code(value: Option<i32>) -> JsonValue {
    value.map_or(JsonValue::Null, |value| {
        JsonValue::Number(JsonNumber::I64(i64::from(value)))
    })
}

fn error(code: &str, message: &str) -> JsonValue {
    JsonValue::Object(JsonObject::from([
        ("code".to_owned(), JsonValue::String(code.to_owned())),
        ("message".to_owned(), JsonValue::String(message.to_owned())),
    ]))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;

    #[cfg(unix)]
    use runx_contracts::JsonNumber;
    use runx_contracts::{JsonObject, JsonValue};

    use super::{CommandInput, execute, plan};
    #[cfg(feature = "catalog")]
    use crate::RuntimeEffectRegistry;
    use crate::credentials::CredentialDelivery;
    use crate::receipts::paths::RUNX_CWD_ENV;
    use crate::tool_catalogs::native::{NativeInvocation, fixture_input};

    #[cfg(unix)]
    #[test]
    fn executes_exact_argv_and_parses_one_json_object() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let env = BTreeMap::from([(
            RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);
        let inputs = fixture_input::<CommandInput>(JsonObject::from([
            (
                "command".to_owned(),
                JsonValue::String("/usr/bin/printf".to_owned()),
            ),
            (
                "args".to_owned(),
                JsonValue::Array(vec![JsonValue::String("{\"status\":\"ready\"}".to_owned())]),
            ),
            (
                "output_mode".to_owned(),
                JsonValue::String("json".to_owned()),
            ),
        ]))?;
        let delivery = CredentialDelivery::none();
        #[cfg(feature = "catalog")]
        let effects = RuntimeEffectRegistry::default();
        let output = json_output(execute(&NativeInvocation {
            inputs: &inputs,
            scopes: &[],
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })?)?;
        let execution = output
            .as_object()
            .and_then(|value| value.get("command_execution"))
            .and_then(JsonValue::as_object)
            .ok_or("missing output")?;
        assert_eq!(
            execution.get("decision"),
            Some(&JsonValue::String("completed".to_owned()))
        );
        assert_eq!(
            execution
                .get("json")
                .and_then(JsonValue::as_object)
                .and_then(|value| value.get("status")),
            Some(&JsonValue::String("ready".to_owned()))
        );
        assert_eq!(
            execution.get("exit_code"),
            Some(&JsonValue::Number(JsonNumber::I64(0)))
        );
        Ok(())
    }

    #[test]
    fn preserves_syntactically_valid_environment_names_without_guessing_authority()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let env = BTreeMap::from([(
            RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);
        let inputs = fixture_input::<CommandInput>(JsonObject::from([
            ("command".to_owned(), JsonValue::String("true".to_owned())),
            (
                "env".to_owned(),
                JsonValue::Object(JsonObject::from([(
                    "AUTHOR_NAME".to_owned(),
                    JsonValue::String("Ada".to_owned()),
                )])),
            ),
        ]))?;
        let delivery = CredentialDelivery::none();
        #[cfg(feature = "catalog")]
        let effects = RuntimeEffectRegistry::default();
        let output = json_output(plan(&NativeInvocation {
            inputs: &inputs,
            scopes: &[],
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })?)?;
        let env_names = output
            .as_object()
            .and_then(|value| value.get("command_plan"))
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("env_names"))
            .and_then(JsonValue::as_array)
            .ok_or("missing env names")?;
        assert_eq!(env_names, &[JsonValue::String("AUTHOR_NAME".to_owned())]);
        Ok(())
    }

    #[test]
    fn network_is_scope_bound_and_committed_to_the_command_plan()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let env = BTreeMap::from([(
            RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);
        let mut inputs = fixture_input::<CommandInput>(JsonObject::from([
            ("command".to_owned(), JsonValue::String("true".to_owned())),
            ("network".to_owned(), JsonValue::Bool(true)),
        ]))?;
        let delivery = CredentialDelivery::none();
        #[cfg(feature = "catalog")]
        let effects = RuntimeEffectRegistry::default();

        let denied = plan(&NativeInvocation {
            inputs: &inputs,
            scopes: &[],
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })
        .expect_err("network without net:process must fail closed");
        assert!(denied.to_string().contains("net:process"));

        let scopes = vec!["net:process".to_owned()];
        let network_plan = json_output(plan(&NativeInvocation {
            inputs: &inputs,
            scopes: &scopes,
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })?)?;
        let network_plan = network_plan
            .as_object()
            .and_then(|value| value.get("command_plan"))
            .and_then(JsonValue::as_object)
            .ok_or("missing network command plan")?;
        assert_eq!(network_plan.get("network"), Some(&JsonValue::Bool(true)));
        let network_digest = network_plan
            .get("command_digest")
            .and_then(JsonValue::as_str)
            .ok_or("missing network command digest")?;

        inputs.network = false;
        let offline_plan = json_output(plan(&NativeInvocation {
            inputs: &inputs,
            scopes: &[],
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })?)?;
        let offline_digest = offline_plan
            .as_object()
            .and_then(|value| value.get("command_plan"))
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("command_digest"))
            .and_then(JsonValue::as_str)
            .ok_or("missing offline command digest")?;
        assert_ne!(network_digest, offline_digest);
        Ok(())
    }

    #[test]
    fn native_command_sandbox_rejects_runtime_delivered_credentials_before_spawning()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let env = BTreeMap::from([(
            RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);
        let inputs = fixture_input::<CommandInput>(JsonObject::from([(
            "command".to_owned(),
            JsonValue::String("/usr/bin/true".to_owned()),
        )]))?;
        let delivery = CredentialDelivery::from_local_descriptor(
            "example",
            "api_key",
            "EXAMPLE_TOKEN",
            "local:example:test",
            vec!["example:read".to_owned()],
            "credential-sentinel",
        )?;
        #[cfg(feature = "catalog")]
        let effects = RuntimeEffectRegistry::default();
        let error = execute(&NativeInvocation {
            inputs: &inputs,
            scopes: &[],
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })
        .expect_err("generic command execution must reject delivered credentials");

        assert!(error.to_string().contains("not supported"));
        Ok(())
    }

    #[test]
    fn refuses_execution_when_the_approved_plan_digest_drifts()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let env = BTreeMap::from([(
            RUNX_CWD_ENV.to_owned(),
            workspace.path().to_string_lossy().into_owned(),
        )]);
        let mut inputs = fixture_input::<CommandInput>(JsonObject::from([(
            "command".to_owned(),
            JsonValue::String("true".to_owned()),
        )]))?;
        let delivery = CredentialDelivery::none();
        #[cfg(feature = "catalog")]
        let effects = RuntimeEffectRegistry::default();
        let planned = json_output(plan(&NativeInvocation {
            inputs: &inputs,
            scopes: &[],
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })?)?;
        let digest = planned
            .as_object()
            .and_then(|value| value.get("command_plan"))
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("command_digest"))
            .and_then(JsonValue::as_str)
            .ok_or("missing digest")?
            .to_owned();
        inputs.args = vec!["--drift".to_owned()];
        inputs.expected_command_digest = Some(digest);
        let error = execute(&NativeInvocation {
            inputs: &inputs,
            scopes: &[],
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: workspace.path(),
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })
        .expect_err("drifted execution must be rejected before spawning");
        assert!(error.to_string().contains("does not match"));
        Ok(())
    }

    fn json_output(output: impl serde::Serialize) -> Result<JsonValue, Box<dyn std::error::Error>> {
        Ok(serde_json::from_value(serde_json::to_value(output)?)?)
    }
}
