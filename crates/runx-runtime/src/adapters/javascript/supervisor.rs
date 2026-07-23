use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use runx_contracts::javascript_worker::{
    InvocationLimits, PROTOCOL_VERSION, WorkerFailureCode, WorkerRequest,
};

use crate::RuntimeError;

mod limiter;
mod process;
mod response_reader;
mod session;

use session::WorkerSession;

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
mod tests;
