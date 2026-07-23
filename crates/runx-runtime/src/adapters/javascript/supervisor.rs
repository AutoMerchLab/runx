use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use runx_contracts::javascript_worker::{
    InvocationLimits, MAX_FRAME_BYTES, MAX_STDERR_BYTES, PROTOCOL_VERSION, WorkerFailureCode,
    WorkerRequest, WorkerResponse, read_frame, write_frame,
};

use crate::RuntimeError;

const WORKER_PATH_ENV: &str = "RUNX_JS_WORKER_PATH";
const WORKER_START_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(super) struct WorkerInvocation {
    pub(super) entry_module: String,
    pub(super) export_name: String,
    pub(super) modules: BTreeMap<String, String>,
    pub(super) inputs: serde_json::Value,
    pub(super) limits: InvocationLimits,
}

pub(super) enum WorkerInvocationResult {
    Success(serde_json::Value),
    Failure {
        code: WorkerFailureCode,
        message: String,
        discard_worker: bool,
    },
}

pub(super) struct WorkerInvocationOutcome {
    pub(super) result: WorkerInvocationResult,
    pub(super) isolation: runx_contracts::JsonObject,
}

pub(super) struct JavaScriptWorkerSupervisor {
    state: Mutex<Option<Arc<WorkerSession>>>,
    next_invocation: AtomicU64,
    spawn_count: AtomicU64,
    peak_in_flight: AtomicUsize,
    max_concurrency: usize,
}

impl JavaScriptWorkerSupervisor {
    pub(super) fn new(max_concurrency: usize) -> Self {
        Self {
            state: Mutex::new(None),
            next_invocation: AtomicU64::new(1),
            spawn_count: AtomicU64::new(0),
            peak_in_flight: AtomicUsize::new(0),
            max_concurrency,
        }
    }

    pub(super) fn invoke(
        &self,
        invocation: WorkerInvocation,
    ) -> Result<WorkerInvocationOutcome, RuntimeError> {
        let invocation_id = format!(
            "js-{}",
            self.next_invocation.fetch_add(1, Ordering::Relaxed)
        );
        let timeout = Duration::from_millis(invocation.limits.wall_milliseconds);
        let request = WorkerRequest::Invoke {
            protocol_version: PROTOCOL_VERSION,
            invocation_id: invocation_id.clone(),
            entry_module: invocation.entry_module,
            export_name: invocation.export_name,
            modules: invocation.modules,
            inputs: invocation.inputs,
            limits: invocation.limits,
        };
        let session = self.session()?;
        let isolation = session.isolation.clone();
        let result = session.invoke(&invocation_id, &request, timeout);
        self.peak_in_flight
            .fetch_max(session.in_flight.peak(), Ordering::Relaxed);
        match result {
            Ok(response) => {
                let discard = matches!(
                    &response,
                    WorkerInvocationResult::Failure {
                        discard_worker: true,
                        ..
                    }
                );
                if discard {
                    self.discard_session(&session, true)?;
                }
                Ok(WorkerInvocationOutcome {
                    result: response,
                    isolation,
                })
            }
            Err(error) => {
                self.discard_session(&session, true)?;
                Err(error)
            }
        }
    }

    fn session(&self) -> Result<Arc<WorkerSession>, RuntimeError> {
        let mut state = lock(&self.state, "locking JavaScript worker supervisor")?;
        if state.is_none() {
            *state = Some(Arc::new(WorkerSession::start(self.max_concurrency)?));
            self.spawn_count.fetch_add(1, Ordering::Relaxed);
        }
        state
            .as_ref()
            .cloned()
            .ok_or_else(|| worker_error("worker session disappeared before invocation"))
    }

    fn discard_session(
        &self,
        session: &Arc<WorkerSession>,
        terminate: bool,
    ) -> Result<(), RuntimeError> {
        let mut state = lock(&self.state, "discarding JavaScript worker session")?;
        if state
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            state.take();
            if terminate {
                session.terminate();
            }
        }
        Ok(())
    }

    pub(super) fn spawn_count(&self) -> u64 {
        self.spawn_count.load(Ordering::Relaxed)
    }

    pub(super) fn peak_in_flight(&self) -> usize {
        self.peak_in_flight.load(Ordering::Relaxed)
    }
}

type PendingResponse = Result<WorkerResponse, String>;
type PendingResponses = BTreeMap<String, mpsc::Sender<PendingResponse>>;

struct WorkerSession {
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    pending: Arc<Mutex<PendingResponses>>,
    stderr: Arc<Mutex<BoundedStderr>>,
    response_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    isolation: runx_contracts::JsonObject,
    in_flight: InFlightLimiter,
    terminated: AtomicBool,
}

impl WorkerSession {
    fn start(max_concurrency: usize) -> Result<Self, RuntimeError> {
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

    fn invoke(
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

    fn terminate(&self) {
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

struct InFlightLimiter {
    maximum: usize,
    count: Mutex<usize>,
    available: Condvar,
    peak: AtomicUsize,
}

impl InFlightLimiter {
    fn new(maximum: usize) -> Self {
        Self {
            maximum,
            count: Mutex::new(0),
            available: Condvar::new(),
            peak: AtomicUsize::new(0),
        }
    }

    fn acquire(&self) -> Result<InFlightPermit<'_>, RuntimeError> {
        let mut count = lock(&self.count, "locking JavaScript in-flight limiter")?;
        while *count >= self.maximum {
            count = self.available.wait(count).map_err(|_| {
                worker_error("waiting for JavaScript in-flight capacity: mutex poisoned")
            })?;
        }
        *count += 1;
        self.peak.fetch_max(*count, Ordering::Relaxed);
        Ok(InFlightPermit { limiter: self })
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::Relaxed)
    }
}

struct InFlightPermit<'a> {
    limiter: &'a InFlightLimiter,
}

impl Drop for InFlightPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut count) = self.limiter.count.lock() {
            *count = count.saturating_sub(1);
            self.limiter.available.notify_one();
        }
    }
}

struct StartingChild(Option<Child>);

impl StartingChild {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child_mut(&mut self) -> Result<&mut Child, RuntimeError> {
        self.0
            .as_mut()
            .ok_or_else(|| worker_error("starting worker child is unavailable"))
    }

    fn take(&mut self) -> Result<Child, RuntimeError> {
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

fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    if !crate::process::signal_process_group_id(child.id(), crate::process::ProcessSignal::Force) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn read_responses(
    stdout: impl Read,
    ready: mpsc::Sender<PendingResponse>,
    pending: Arc<Mutex<PendingResponses>>,
) {
    let mut reader = BufReader::new(stdout);
    let first = match read_frame::<WorkerResponse>(&mut reader, MAX_FRAME_BYTES) {
        Ok(Some(response)) => Ok(response),
        Ok(None) => Err("worker exited before its protocol handshake".to_owned()),
        Err(error) => Err(error.to_string()),
    };
    let ready_ok = first.is_ok();
    if ready.send(first).is_err() || !ready_ok {
        return;
    }
    loop {
        match read_frame::<WorkerResponse>(&mut reader, MAX_FRAME_BYTES) {
            Ok(Some(response)) => {
                let Some(invocation_id) = response_invocation_id(&response) else {
                    fail_pending(
                        &pending,
                        "deterministic JavaScript worker sent a response without an invocation id",
                    );
                    return;
                };
                let sender = match pending.lock() {
                    Ok(mut pending) => pending.remove(invocation_id),
                    Err(_) => {
                        fail_pending(
                            &pending,
                            "JavaScript worker pending-response mutex was poisoned",
                        );
                        return;
                    }
                };
                let Some(sender) = sender else {
                    fail_pending(
                        &pending,
                        "deterministic JavaScript worker returned an unknown invocation id",
                    );
                    return;
                };
                if sender.send(Ok(response)).is_err() {
                    continue;
                }
            }
            Ok(None) => {
                fail_pending(
                    &pending,
                    "deterministic JavaScript worker exited without completing pending invocations",
                );
                return;
            }
            Err(error) => {
                fail_pending(
                    &pending,
                    &format!("deterministic JavaScript worker protocol failed: {error}"),
                );
                return;
            }
        }
    }
}

fn response_invocation_id(response: &WorkerResponse) -> Option<&str> {
    match response {
        WorkerResponse::Result { invocation_id, .. } => Some(invocation_id),
        WorkerResponse::Failure {
            invocation_id: Some(invocation_id),
            ..
        } => Some(invocation_id),
        WorkerResponse::Ready { .. } | WorkerResponse::Failure { .. } => None,
    }
}

fn fail_pending(pending: &Mutex<PendingResponses>, message: &str) {
    let senders = match pending.lock() {
        Ok(mut pending) => std::mem::take(&mut *pending),
        Err(_) => return,
    };
    for sender in senders.into_values() {
        let _ignored = sender.send(Err(message.to_owned()));
    }
}

#[derive(Default)]
struct BoundedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

impl BoundedStderr {
    fn push(&mut self, chunk: &[u8]) {
        let remaining = MAX_STDERR_BYTES.saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        self.truncated |= chunk.len() > remaining;
    }

    fn render(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            text.push_str(" [truncated]");
        }
        text
    }
}

fn capture_stderr(mut stderr: impl Read, capture: &Mutex<BoundedStderr>) {
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

fn resolve_worker_path() -> Result<PathBuf, RuntimeError> {
    let explicit = std::env::var_os(WORKER_PATH_ENV).map(PathBuf::from);
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

fn worker_candidates(current: &Path, binary: &str) -> Vec<PathBuf> {
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
fn worker_binary_name() -> &'static str {
    "runx-js-worker.exe"
}

#[cfg(not(windows))]
fn worker_binary_name() -> &'static str {
    "runx-js-worker"
}

fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    context: &str,
) -> Result<std::sync::MutexGuard<'a, T>, RuntimeError> {
    mutex
        .lock()
        .map_err(|_| worker_error(format!("{context}: mutex poisoned")))
}

fn worker_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::JavaScriptWorker {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[cfg(unix)]
    #[test]
    fn worker_candidates_include_the_real_binary_directory_for_a_dev_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temporary worker layout");
        let target_dir = temp.path().join("target/debug");
        let link_dir = temp.path().join("bin");
        fs::create_dir_all(&target_dir).expect("target directory");
        fs::create_dir_all(&link_dir).expect("link directory");

        let runx = target_dir.join("runx");
        let worker = target_dir.join(worker_binary_name());
        fs::write(&runx, b"runx").expect("runx binary fixture");
        fs::write(&worker, b"worker").expect("worker binary fixture");
        let link = link_dir.join("runx");
        symlink(&runx, &link).expect("runx dev symlink");

        let canonical_worker = fs::canonicalize(worker).expect("canonical worker fixture");
        assert!(worker_candidates(&link, worker_binary_name()).contains(&canonical_worker));
    }

    #[test]
    fn bounded_stderr_never_retains_more_than_the_protocol_limit() {
        let mut capture = BoundedStderr::default();
        capture.push(&vec![b'x'; MAX_STDERR_BYTES + 10]);
        assert_eq!(capture.bytes.len(), MAX_STDERR_BYTES);
        assert!(capture.truncated);
    }

    #[test]
    fn supervisors_own_independent_session_state() {
        let first = JavaScriptWorkerSupervisor::new(1);
        let second = JavaScriptWorkerSupervisor::new(1);
        assert_eq!(first.spawn_count(), 0);
        assert_eq!(second.spawn_count(), 0);
    }
}
