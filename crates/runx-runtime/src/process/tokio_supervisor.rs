use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;

use thiserror::Error;

#[derive(Clone, Debug)]
pub(crate) struct TokioProcessSpec {
    pub(crate) label: &'static str,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) env: BTreeMap<String, String>,
}

impl TokioProcessSpec {
    pub(crate) fn new(
        label: &'static str,
        command: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            label,
            command: command.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: BTreeMap::new(),
        }
    }

    pub(crate) fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub(crate) fn env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }
}

#[derive(Debug, Error)]
pub(crate) enum TokioProcessSupervisorError {
    #[error("failed to spawn {label} command '{command}' in cwd '{cwd}': {source}")]
    Spawn {
        label: &'static str,
        command: String,
        cwd: String,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn spawn_tokio_process(
    spec: TokioProcessSpec,
) -> Result<tokio::process::Child, TokioProcessSupervisorError> {
    super::ensure_host_process_containment().map_err(|source| {
        TokioProcessSupervisorError::Spawn {
            label: spec.label,
            command: spec.command.clone(),
            cwd: spec.cwd.display().to_string(),
            source,
        }
    })?;
    let mut command = tokio::process::Command::new(&spec.command);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);
    command
        .spawn()
        .map_err(|source| TokioProcessSupervisorError::Spawn {
            label: spec.label,
            command: spec.command,
            cwd: spec.cwd.display().to_string(),
            source,
        })
}

#[cfg(unix)]
fn configure_process_group(command: &mut tokio::process::Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut tokio::process::Command) {}
