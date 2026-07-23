use std::io::{BufReader, Read};
use std::sync::mpsc;

use runx_contracts::javascript_worker::{MAX_FRAME_BYTES, WorkerResponse, read_frame};

pub(super) type WorkerFrameResult = Result<WorkerResponse, String>;

pub(super) fn read_responses(stdout: impl Read, responses: mpsc::Sender<WorkerFrameResult>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_frame::<WorkerResponse>(&mut reader, MAX_FRAME_BYTES) {
            Ok(Some(response)) => {
                if responses.send(Ok(response)).is_err() {
                    return;
                }
            }
            Ok(None) => {
                let _ignored = responses.send(Err(
                    "deterministic JavaScript worker exited without completing its invocation"
                        .to_owned(),
                ));
                return;
            }
            Err(error) => {
                let _ignored = responses.send(Err(format!(
                    "deterministic JavaScript worker protocol failed: {error}"
                )));
                return;
            }
        }
    }
}
