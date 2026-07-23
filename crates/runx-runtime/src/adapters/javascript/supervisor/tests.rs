#![allow(clippy::expect_used)]

#[cfg(unix)]
use std::fs;

use runx_contracts::javascript_worker::MAX_STDERR_BYTES;

use super::JavaScriptWorkerSupervisor;
use super::process::BoundedStderr;
#[cfg(unix)]
use super::process::{worker_binary_name, worker_candidates};

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
