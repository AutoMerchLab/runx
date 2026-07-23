use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use runx_contracts::javascript_worker::{
    MAX_FRAME_BYTES, PROTOCOL_VERSION, WorkerRequest, WorkerResponse, write_frame,
};

use crate::RuntimeError;

use super::process::{BoundedStderr, capture_stderr, resolve_worker_path, spawn_child, stop_child};
use super::response_reader::{WorkerRead, read_responses};
use super::{WorkerInvocationResult, lock, worker_error};

// This bounds process startup and the protocol handshake, not JavaScript
// execution. A freshly downloaded macOS binary may remain in dyld while the OS
// performs first-launch policy verification, so this must tolerate cold-host
// startup without weakening the worker's 2-second invocation wall limit.
const WORKER_START_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct WorkerSession {
    child: Mutex<Option<Child>>,
    _active_process: crate::interrupt::ActiveProcessGroup,
    stdin: Mutex<Option<ChildStdin>>,
    responses: Mutex<mpsc::Receiver<WorkerRead>>,
    stderr: Arc<Mutex<BoundedStderr>>,
    response_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    pub(super) isolation: runx_contracts::JsonObject,
    terminated: AtomicBool,
}

impl WorkerSession {
    pub(super) fn start() -> Result<Self, RuntimeError> {
        crate::process::ensure_host_process_containment()
            .map_err(|source| RuntimeError::io("installing host process containment", source))?;
        let worker_path = resolve_worker_path()?;
        let sandbox =
            crate::sandbox::prepare_javascript_worker_sandbox(&worker_path)?.into_process_plan();
        if !sandbox.env.is_empty() || !sandbox.cleanup_paths.is_empty() {
            return Err(worker_error(
                "deterministic JavaScript worker sandbox must not carry environment or host cleanup paths",
            ));
        }
        let mut command = Command::new(&sandbox.command);
        command
            .args(&sandbox.args)
            .current_dir(&sandbox.cwd)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::process::configure_process_group(&mut command);
        let mut starting = spawn_child(command)?;
        let active_process =
            crate::interrupt::ActiveProcessGroup::register(starting.child_mut()?.id());
        let stdin =
            starting.child_mut()?.stdin.take().ok_or_else(|| {
                worker_error("deterministic JavaScript worker stdin was not piped")
            })?;
        let stdout =
            starting.child_mut()?.stdout.take().ok_or_else(|| {
                worker_error("deterministic JavaScript worker stdout was not piped")
            })?;
        let stderr_pipe =
            starting.child_mut()?.stderr.take().ok_or_else(|| {
                worker_error("deterministic JavaScript worker stderr was not piped")
            })?;
        let (response_tx, response_rx) = mpsc::channel();
        let stderr = Arc::new(Mutex::new(BoundedStderr::default()));
        let mut session = Self {
            child: Mutex::new(Some(starting.take()?)),
            _active_process: active_process,
            stdin: Mutex::new(Some(stdin)),
            responses: Mutex::new(response_rx),
            stderr: stderr.clone(),
            response_reader: None,
            stderr_reader: None,
            isolation: sandbox.metadata,
            terminated: AtomicBool::new(false),
        };
        session.response_reader = Some(
            thread::Builder::new()
                .name("runx-js-worker-reader".to_owned())
                .spawn(move || read_responses(stdout, response_tx))
                .map_err(|source| RuntimeError::io("starting JavaScript worker reader", source))?,
        );
        let stderr_capture = stderr.clone();
        session.stderr_reader = Some(
            thread::Builder::new()
                .name("runx-js-worker-stderr".to_owned())
                .spawn(move || capture_stderr(stderr_pipe, &stderr_capture))
                .map_err(|source| {
                    RuntimeError::io("starting JavaScript worker stderr capture", source)
                })?,
        );
        session.write_request(&WorkerRequest::Hello {
            protocol_version: PROTOCOL_VERSION,
        })?;
        let ready = session
            .receive_response(WORKER_START_TIMEOUT)
            .map_err(|error| {
                session.failure_with_stderr(format!(
                    "deterministic JavaScript worker did not complete its handshake: {error}"
                ))
            })?;
        let ready = ready.map_err(|message| session.failure_with_stderr(message))?;
        match ready {
            WorkerResponse::Ready { protocol_version } if protocol_version == PROTOCOL_VERSION => {
                Ok(session)
            }
            response => Err(worker_error(format!(
                "deterministic JavaScript worker handshake failed: {response:?}"
            ))),
        }
    }

    pub(super) fn invoke(
        &self,
        invocation_id: &str,
        request: &WorkerRequest,
        timeout: Duration,
    ) -> Result<WorkerInvocationResult, RuntimeError> {
        if self.terminated.load(Ordering::Acquire) {
            return Err(worker_error(
                "deterministic JavaScript worker session is closed",
            ));
        }
        self.write_request(request)?;
        let response = match self.receive_response(timeout) {
            Ok(Ok(response)) => response,
            Ok(Err(message)) => return Err(self.failure_with_stderr(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.terminate();
                return Err(self.failure_with_stderr(format!(
                    "deterministic JavaScript worker exceeded {} ms wall limit",
                    timeout.as_millis()
                )));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(self.failure_with_stderr(
                    "deterministic JavaScript worker exited without a response",
                ));
            }
        };
        match response {
            WorkerResponse::Result {
                protocol_version,
                invocation_id: response_id,
                output,
            } if protocol_version == PROTOCOL_VERSION && response_id == invocation_id => {
                Ok(WorkerInvocationResult::Success(output))
            }
            WorkerResponse::Failure {
                protocol_version,
                invocation_id: Some(response_id),
                code,
                message,
                discard_worker,
            } if protocol_version == PROTOCOL_VERSION && response_id == invocation_id => {
                Ok(WorkerInvocationResult::Failure {
                    code,
                    message,
                    discard_worker,
                })
            }
            response => Err(worker_error(format!(
                "deterministic JavaScript worker response mismatch: {response:?}"
            ))),
        }
    }

    fn receive_response(&self, timeout: Duration) -> Result<WorkerRead, mpsc::RecvTimeoutError> {
        let responses = self
            .responses
            .lock()
            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)?;
        responses.recv_timeout(timeout)
    }

    fn write_request(&self, request: &WorkerRequest) -> Result<(), RuntimeError> {
        let mut stdin = lock(&self.stdin, "locking JavaScript worker stdin")?;
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| worker_error("deterministic JavaScript worker stdin is closed"))?;
        write_frame(stdin, request, MAX_FRAME_BYTES)
            .map_err(|error| worker_error(error.to_string()))
    }

    fn stderr_text(&self) -> String {
        lock(&self.stderr, "reading JavaScript worker stderr")
            .map(|capture| capture.render())
            .unwrap_or_else(|error| error.to_string())
    }

    fn failure_with_stderr(&self, message: impl std::fmt::Display) -> RuntimeError {
        let mut message = message.to_string();
        if let Some(status) = self.exited_status() {
            message.push_str(&format!("; exit status: {status}"));
        }
        let stderr = self.stderr_text();
        if !stderr.is_empty() {
            message.push_str(&format!("; stderr: {stderr}"));
        }
        worker_error(message)
    }

    fn exited_status(&self) -> Option<String> {
        let mut child = self.child.lock().ok()?;
        let child = child.as_mut()?;
        child
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.to_string())
    }

    pub(super) fn terminate(&self) {
        if self.terminated.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut stdin) = self.stdin.lock() {
            stdin.take();
        }
        if let Ok(mut child) = self.child.lock()
            && let Some(child) = child.as_mut()
        {
            stop_child(child);
        }
    }
}

impl Drop for WorkerSession {
    fn drop(&mut self) {
        self.terminate();
        if let Some(reader) = self.response_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}
