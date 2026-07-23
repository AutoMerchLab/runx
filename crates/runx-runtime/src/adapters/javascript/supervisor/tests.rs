#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
#[cfg(unix)]
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use runx_contracts::javascript_worker::InvocationLimits;
use runx_contracts::javascript_worker::MAX_STDERR_BYTES;

use super::process::BoundedStderr;
#[cfg(unix)]
use super::process::{worker_binary_name, worker_candidates};
use super::{JavaScriptWorkerSupervisor, WorkerInvocation, WorkerInvocationResult};

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

#[test]
fn timed_out_worker_does_not_fail_a_healthy_sibling() -> Result<(), Box<dyn std::error::Error>> {
    let supervisor = Arc::new(JavaScriptWorkerSupervisor::new(2));
    let barrier = Arc::new(Barrier::new(3));
    let slow = {
        let supervisor = supervisor.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            barrier.wait();
            supervisor.invoke(invocation(
                "export default () => { let value = 0; for (let index = 0; index < 9000000; index += 1) value = (value + index) % 1000003; return { value }; };",
                1,
            ))
        })
    };
    let healthy = {
        let supervisor = supervisor.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            barrier.wait();
            supervisor.invoke(invocation(
                "export default () => { let value = 0; for (let index = 0; index < 100000; index += 1) value = (value + index) % 1000003; return { healthy: true }; };",
                InvocationLimits::default().wall_milliseconds,
            ))
        })
    };
    barrier.wait();

    let slow_error = slow
        .join()
        .map_err(|_| "slow JavaScript invocation thread panicked")?
        .err()
        .map(|error| error.to_string())
        .ok_or("slow JavaScript invocation unexpectedly succeeded")?;
    assert!(
        slow_error.contains("exceeded 1 ms wall limit"),
        "{slow_error}"
    );
    let healthy = healthy
        .join()
        .map_err(|_| "healthy JavaScript invocation thread panicked")??;
    assert!(matches!(
        healthy.result,
        WorkerInvocationResult::Success(ref output)
            if output == &serde_json::json!({"healthy": true})
    ));
    assert_eq!(supervisor.spawn_count(), 2);
    assert_eq!(supervisor.peak_in_flight(), 2);

    let barrier = Arc::new(Barrier::new(3));
    let recovered = (0..2)
        .map(|_| {
            let supervisor = supervisor.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                supervisor.invoke(invocation(
                    "export default () => { let value = 0; for (let index = 0; index < 100000; index += 1) value = (value + index) % 1000003; return { recovered: true }; };",
                    InvocationLimits::default().wall_milliseconds,
                ))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for invocation in recovered {
        let outcome = invocation
            .join()
            .map_err(|_| "recovery JavaScript invocation thread panicked")??;
        assert!(matches!(
            outcome.result,
            WorkerInvocationResult::Success(ref output)
                if output == &serde_json::json!({"recovered": true})
        ));
    }
    assert_eq!(supervisor.spawn_count(), 3);
    Ok(())
}

fn invocation(source: &str, wall_milliseconds: u64) -> WorkerInvocation {
    WorkerInvocation {
        entry_module: "main.mjs".to_owned(),
        export_name: "default".to_owned(),
        modules: BTreeMap::from([("main.mjs".to_owned(), source.to_owned())]),
        inputs: serde_json::json!({}),
        limits: InvocationLimits {
            wall_milliseconds,
            ..InvocationLimits::default()
        },
    }
}
