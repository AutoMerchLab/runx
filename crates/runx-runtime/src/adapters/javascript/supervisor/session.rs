use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use runx_contracts::javascript_worker::{
    MAX_FRAME_BYTES, PROTOCOL_VERSION, WorkerRequest, WorkerResponse, write_frame,
};

use crate::RuntimeError;

use super::limiter::InFlightLimiter;
use super::process::{
    BoundedStderr, StartingChild, capture_stderr, resolve_worker_path, stop_child,
};
use super::response_reader::{PendingResponses, read_responses};
use super::{WorkerInvocationResult, lock, worker_error};

const WORKER_START_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) struct WorkerSession {
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    pending: Arc<Mutex<PendingResponses>>,
    stderr: Arc<Mutex<BoundedStderr>>,
    response_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    pub(super) isolation: runx_contracts::JsonObject,
    pub(super) in_flight: InFlightLimiter,
    terminated: AtomicBool,
}

impl WorkerSession {
    pub(super) fn start(max_concurrency: usize) -> Result<Self, RuntimeError> {
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
        let child = command.spawn().map_err(|source| {
            RuntimeError::io("spawning deterministic JavaScript worker", source)
        })?;
        let mut starting = StartingChild::new(child);
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
        let (ready_tx, ready_rx) = mpsc::channel();
        let pending = Arc::new(Mutex::new(PendingResponses::new()));
        let stderr = Arc::new(Mutex::new(BoundedStderr::default()));
        let mut session = Self {
            child: Mutex::new(Some(starting.take()?)),
            stdin: Mutex::new(Some(stdin)),
            pending: pending.clone(),
            stderr: stderr.clone(),
            response_reader: None,
            stderr_reader: None,
            isolation: sandbox.metadata,
            in_flight: InFlightLimiter::new(max_concurrency),
            terminated: AtomicBool::new(false),
        };
        session.response_reader = Some(
            thread::Builder::new()
                .name("runx-js-worker-reader".to_owned())
                .spawn(move || read_responses(stdout, ready_tx, pending))
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
        let ready = ready_rx
            .recv_timeout(WORKER_START_TIMEOUT)
            .map_err(|error| {
                worker_error(format!(
                    "deterministic JavaScript worker did not complete its handshake: {error}"
                ))
            })?;
        match ready.map_err(worker_error)? {
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
        let _permit = self.in_flight.acquire()?;
        if self.terminated.load(Ordering::Acquire) {
            return Err(worker_error(
                "deterministic JavaScript worker session is closed",
            ));
        }
        let (response_tx, response_rx) = mpsc::channel();
        {
            let mut pending = lock(&self.pending, "registering JavaScript worker invocation")?;
            if pending
                .insert(invocation_id.to_owned(), response_tx)
                .is_some()
            {
                return Err(worker_error("duplicate JavaScript worker invocation id"));
            }
        }
        if let Err(error) = self.write_request(request) {
            self.remove_pending(invocation_id);
            return Err(error);
        }
        let response = match response_rx.recv_timeout(timeout) {
            Ok(Ok(response)) => response,
            Ok(Err(message)) => return Err(worker_error(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.remove_pending(invocation_id);
                self.terminate();
                return Err(worker_error(format!(
                    "deterministic JavaScript worker exceeded {} ms wall limit; stderr: {}",
                    timeout.as_millis(),
                    self.stderr_text()
                )));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(worker_error(format!(
                    "deterministic JavaScript worker exited without a response; stderr: {}",
                    self.stderr_text()
                )));
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

    fn write_request(&self, request: &WorkerRequest) -> Result<(), RuntimeError> {
        let mut stdin = lock(&self.stdin, "locking JavaScript worker stdin")?;
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| worker_error("deterministic JavaScript worker stdin is closed"))?;
        write_frame(stdin, request, MAX_FRAME_BYTES)
            .map_err(|error| worker_error(error.to_string()))
    }

    fn remove_pending(&self, invocation_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(invocation_id);
        }
    }

    fn stderr_text(&self) -> String {
        lock(&self.stderr, "reading JavaScript worker stderr")
            .map(|capture| capture.render())
            .unwrap_or_else(|error| error.to_string())
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
