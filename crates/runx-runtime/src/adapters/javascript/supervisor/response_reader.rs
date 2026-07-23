use std::collections::BTreeMap;
use std::io::{BufReader, Read};
use std::sync::{Arc, Mutex, mpsc};

use runx_contracts::javascript_worker::{MAX_FRAME_BYTES, WorkerResponse, read_frame};

pub(super) type PendingResponse = Result<WorkerResponse, String>;
pub(super) type PendingResponses = BTreeMap<String, mpsc::Sender<PendingResponse>>;

pub(super) fn read_responses(
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
