use std::io::{BufReader, BufWriter};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;

use thiserror::Error;

use crate::engine::{EngineError, evaluate};
use crate::protocol::{
    JAVASCRIPT_STACK_BYTES, MAX_CONCURRENT_INVOCATIONS, MAX_FRAME_BYTES, PROTOCOL_VERSION,
    ProtocolError, WorkerFailureCode, WorkerRequest, WorkerResponse, read_frame, write_frame,
};

#[derive(Debug, Error)]
pub enum WorkerServerError {
    #[error("worker limits could not be installed: {0}")]
    Limits(String),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("worker request thread could not be created: {0}")]
    Thread(std::io::Error),
    #[error("worker request coordination failed: {0}")]
    Coordination(String),
    #[error("worker response thread panicked")]
    ResponseThreadPanicked,
}

pub fn serve() -> Result<(), WorkerServerError> {
    crate::limits::install().map_err(|error| WorkerServerError::Limits(error.to_string()))?;
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    let Some(request) = read_frame::<WorkerRequest>(&mut reader, MAX_FRAME_BYTES)? else {
        return Ok(());
    };
    match request {
        WorkerRequest::Hello { protocol_version } if protocol_version == PROTOCOL_VERSION => {
            write_frame(
                &mut writer,
                &WorkerResponse::Ready {
                    protocol_version: PROTOCOL_VERSION,
                },
                MAX_FRAME_BYTES,
            )?;
        }
        _ => {
            write_frame(
                &mut writer,
                &WorkerResponse::Failure {
                    protocol_version: PROTOCOL_VERSION,
                    invocation_id: None,
                    code: WorkerFailureCode::InvalidProtocol,
                    message: "worker protocol handshake mismatch".to_owned(),
                    discard_worker: true,
                },
                MAX_FRAME_BYTES,
            )?;
            return Ok(());
        }
    }
    drop(writer);

    let (response_tx, response_rx) = mpsc::channel();
    let response_writer = thread::Builder::new()
        .name("runx-js-response-writer".to_owned())
        .spawn(move || write_responses(response_rx))
        .map_err(WorkerServerError::Thread)?;
    let active = Arc::new(ActiveInvocations::default());

    while let Some(request) = read_frame::<WorkerRequest>(&mut reader, MAX_FRAME_BYTES)? {
        let WorkerRequest::Invoke {
            protocol_version,
            invocation_id,
            entry_module,
            export_name,
            modules,
            inputs,
            limits,
        } = request
        else {
            send_failure(
                &response_tx,
                None,
                WorkerFailureCode::InvalidProtocol,
                "worker handshake may occur only once",
                true,
            )?;
            break;
        };
        if protocol_version != PROTOCOL_VERSION {
            send_failure(
                &response_tx,
                Some(invocation_id),
                WorkerFailureCode::InvalidProtocol,
                "worker invocation protocol mismatch",
                true,
            )?;
            break;
        }

        active.acquire(MAX_CONCURRENT_INVOCATIONS)?;
        let response_tx = response_tx.clone();
        let active_for_invocation = active.clone();
        let spawn = thread::Builder::new()
            .name("runx-js-invocation".to_owned())
            .stack_size(JAVASCRIPT_STACK_BYTES)
            .spawn(move || {
                let response = match evaluate(&entry_module, &export_name, &modules, inputs, limits)
                {
                    Ok(output) => WorkerResponse::Result {
                        protocol_version: PROTOCOL_VERSION,
                        invocation_id,
                        output,
                    },
                    Err(error) => engine_failure(invocation_id, &error),
                };
                let _ignored = response_tx.send(response);
                active_for_invocation.release();
            });
        if let Err(error) = spawn {
            active.release();
            return Err(WorkerServerError::Thread(error));
        }
    }

    active.wait_until_idle()?;
    drop(response_tx);
    match response_writer.join() {
        Ok(result) => result?,
        Err(_) => return Err(WorkerServerError::ResponseThreadPanicked),
    }
    Ok(())
}

#[derive(Default)]
struct ActiveInvocations {
    count: Mutex<usize>,
    available: Condvar,
}

impl ActiveInvocations {
    fn acquire(&self, maximum: usize) -> Result<(), WorkerServerError> {
        let mut count = self.lock_count()?;
        while *count >= maximum {
            count = self.available.wait(count).map_err(|_| {
                WorkerServerError::Coordination(
                    "active-invocation condition variable was poisoned".to_owned(),
                )
            })?;
        }
        *count += 1;
        Ok(())
    }

    fn release(&self) {
        if let Ok(mut count) = self.count.lock() {
            *count = count.saturating_sub(1);
            self.available.notify_all();
        }
    }

    fn wait_until_idle(&self) -> Result<(), WorkerServerError> {
        let mut count = self.lock_count()?;
        while *count > 0 {
            count = self.available.wait(count).map_err(|_| {
                WorkerServerError::Coordination(
                    "active-invocation condition variable was poisoned".to_owned(),
                )
            })?;
        }
        Ok(())
    }

    fn lock_count(&self) -> Result<std::sync::MutexGuard<'_, usize>, WorkerServerError> {
        self.count.lock().map_err(|_| {
            WorkerServerError::Coordination("active-invocation mutex was poisoned".to_owned())
        })
    }
}

fn write_responses(responses: mpsc::Receiver<WorkerResponse>) -> Result<(), WorkerServerError> {
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    for response in responses {
        write_frame(&mut writer, &response, MAX_FRAME_BYTES)?;
    }
    Ok(())
}

fn engine_failure(invocation_id: String, error: &EngineError) -> WorkerResponse {
    WorkerResponse::Failure {
        protocol_version: PROTOCOL_VERSION,
        invocation_id: Some(invocation_id),
        code: error.code,
        message: error.message.clone(),
        // Each invocation receives a fresh Boa context and module loader. A
        // typed module/execution failure therefore invalidates only this
        // invocation; process retirement is reserved for protocol or process
        // containment failures.
        discard_worker: false,
    }
}

fn send_failure(
    responses: &mpsc::Sender<WorkerResponse>,
    invocation_id: Option<String>,
    code: WorkerFailureCode,
    message: &str,
    discard_worker: bool,
) -> Result<(), WorkerServerError> {
    responses
        .send(WorkerResponse::Failure {
            protocol_version: PROTOCOL_VERSION,
            invocation_id,
            code,
            message: message.to_owned(),
            discard_worker,
        })
        .map_err(|_| {
            WorkerServerError::Coordination("worker response channel disconnected".to_owned())
        })
}
