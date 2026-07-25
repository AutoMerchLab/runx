use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use std::sync::mpsc;
#[cfg(target_os = "linux")]
use std::thread;

use runx_contracts::javascript_worker::MAX_STDERR_BYTES;

use crate::RuntimeError;

use super::super::WORKER_PATH_ENV;
use super::worker_error;

pub(super) struct StartingChild(Option<Child>);

impl StartingChild {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    pub(super) fn child_mut(&mut self) -> Result<&mut Child, RuntimeError> {
        self.0
            .as_mut()
            .ok_or_else(|| worker_error("starting worker child is unavailable"))
    }

    pub(super) fn take(&mut self) -> Result<Child, RuntimeError> {
        self.0
            .take()
            .ok_or_else(|| worker_error("starting worker child was already transferred"))
    }
}

impl Drop for StartingChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            stop_child(child);
        }
    }
}

#[cfg(target_os = "linux")]
struct SpawnRequest {
    command: Command,
    response: mpsc::SyncSender<std::io::Result<Child>>,
}

#[cfg(target_os = "linux")]
struct WorkerSpawnBroker {
    sender: mpsc::Sender<SpawnRequest>,
}

#[cfg(target_os = "linux")]
static WORKER_SPAWN_BROKER: OnceLock<Result<WorkerSpawnBroker, String>> = OnceLock::new();

#[cfg(target_os = "linux")]
pub(super) fn spawn_child(command: Command) -> Result<StartingChild, RuntimeError> {
    // bubblewrap's --die-with-parent follows the exact Linux task that spawned
    // it, not the surrounding process. A single process-lifetime broker gives
    // every worker a stable parent task without adding one idle host thread per
    // JavaScript session. Sessions still own and explicitly stop their child.
    let broker = worker_spawn_broker()?;
    let (response, receiver) = mpsc::sync_channel(1);
    broker
        .sender
        .send(SpawnRequest { command, response })
        .map_err(|_| worker_error("JavaScript worker spawn broker is unavailable"))?;
    let child = receiver
        .recv()
        .map_err(|_| worker_error("JavaScript worker spawn broker exited before responding"))?
        .map_err(|source| RuntimeError::io("spawning deterministic JavaScript worker", source))?;
    Ok(StartingChild::new(child))
}

#[cfg(target_os = "linux")]
fn worker_spawn_broker() -> Result<&'static WorkerSpawnBroker, RuntimeError> {
    match WORKER_SPAWN_BROKER.get_or_init(start_worker_spawn_broker) {
        Ok(broker) => Ok(broker),
        Err(message) => Err(worker_error(message.clone())),
    }
}

#[cfg(target_os = "linux")]
fn start_worker_spawn_broker() -> Result<WorkerSpawnBroker, String> {
    let (sender, receiver) = mpsc::channel::<SpawnRequest>();
    thread::Builder::new()
        .name("runx-js-worker-spawn".to_owned())
        .spawn(move || {
            while let Ok(mut request) = receiver.recv() {
                let result = request.command.spawn();
                if let Err(mpsc::SendError(Ok(mut child))) = request.response.send(result) {
                    stop_child(&mut child);
                }
            }
        })
        .map_err(|source| format!("starting JavaScript worker spawn broker: {source}"))?;
    Ok(WorkerSpawnBroker { sender })
}

#[cfg(not(target_os = "linux"))]
pub(super) fn spawn_child(mut command: Command) -> Result<StartingChild, RuntimeError> {
    let child = command
        .spawn()
        .map_err(|source| RuntimeError::io("spawning deterministic JavaScript worker", source))?;
    Ok(StartingChild::new(child))
}

pub(super) fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    if !crate::process::signal_process_group_id(child.id(), crate::process::ProcessSignal::Force) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[derive(Default)]
pub(super) struct BoundedStderr {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}

impl BoundedStderr {
    pub(super) fn push(&mut self, chunk: &[u8]) {
        let remaining = MAX_STDERR_BYTES.saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        self.truncated |= chunk.len() > remaining;
    }

    pub(super) fn render(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            text.push_str(" [truncated]");
        }
        text
    }
}

pub(super) fn capture_stderr(mut stderr: impl Read, capture: &Mutex<BoundedStderr>) {
    let mut chunk = [0_u8; 4096];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(count) => {
                if let Ok(mut capture) = capture.lock() {
                    capture.push(&chunk[..count]);
                }
            }
        }
    }
}

pub(super) fn resolve_worker_path(explicit: Option<&str>) -> Result<PathBuf, RuntimeError> {
    let explicit = explicit.map(PathBuf::from);
    let current = std::env::current_exe()
        .map_err(|source| RuntimeError::io("resolving current executable", source))?;
    let binary = worker_binary_name();
    if let Some(explicit) = explicit {
        if !explicit.is_absolute() {
            return Err(worker_error(format!(
                "{WORKER_PATH_ENV} must be an absolute operator-controlled path"
            )));
        }
        if !explicit.is_file() {
            return Err(worker_error(format!(
                "{WORKER_PATH_ENV} does not name a worker file: {}",
                explicit.display()
            )));
        }
        return canonical_worker_path(&explicit);
    }
    for candidate in worker_candidates(&current, binary) {
        if candidate.is_file() {
            return canonical_worker_path(&candidate);
        }
    }
    Err(worker_error(format!(
        "runx-js-worker is not installed beside the Runx binary; {WORKER_PATH_ENV} may name an absolute operator-controlled worker path"
    )))
}

pub(super) fn worker_candidates(current: &Path, binary: &str) -> Vec<PathBuf> {
    let mut executables = vec![current.to_path_buf()];
    if let Ok(canonical) = fs::canonicalize(current)
        && !executables.contains(&canonical)
    {
        executables.push(canonical);
    }

    let mut candidates = Vec::new();
    for executable in executables {
        let Some(parent) = executable.parent() else {
            continue;
        };
        push_unique(&mut candidates, parent.join(binary));
        if parent.file_name().and_then(|name| name.to_str()) == Some("deps")
            && let Some(target_dir) = parent.parent()
        {
            push_unique(&mut candidates, target_dir.join(binary));
        }
    }
    candidates
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn canonical_worker_path(path: &Path) -> Result<PathBuf, RuntimeError> {
    fs::canonicalize(path).map_err(|source| {
        RuntimeError::io(
            format!("canonicalizing JavaScript worker {}", path.display()),
            source,
        )
    })
}

#[cfg(windows)]
pub(super) fn worker_binary_name() -> &'static str {
    "runx-js-worker.exe"
}

#[cfg(not(windows))]
pub(super) fn worker_binary_name() -> &'static str {
    "runx-js-worker"
}
