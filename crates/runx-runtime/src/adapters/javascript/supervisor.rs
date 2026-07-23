use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use runx_contracts::javascript_worker::{
    InvocationLimits, PROTOCOL_VERSION, WorkerFailureCode, WorkerRequest,
};

use crate::RuntimeError;

mod pool;
mod process;
mod response_reader;
mod session;

use pool::WorkerPool;

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
    pool: WorkerPool,
    next_invocation: AtomicU64,
}

impl JavaScriptWorkerSupervisor {
    pub(super) fn new(max_concurrency: usize) -> Self {
        Self {
            pool: WorkerPool::new(max_concurrency),
            next_invocation: AtomicU64::new(1),
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
        let mut lease = self.pool.acquire()?;
        let isolation = lease.session().isolation.clone();
        let result = lease.session().invoke(&invocation_id, &request, timeout);
        match result {
            Ok(response) => {
                let discard = matches!(
                    &response,
                    WorkerInvocationResult::Failure {
                        discard_worker: true,
                        ..
                    }
                );
                if !discard {
                    lease.mark_reusable();
                }
                Ok(WorkerInvocationOutcome {
                    result: response,
                    isolation,
                })
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn spawn_count(&self) -> u64 {
        self.pool.spawn_count()
    }

    pub(super) fn peak_in_flight(&self) -> usize {
        self.pool.peak_active()
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
