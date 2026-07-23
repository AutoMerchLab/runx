use std::collections::BTreeMap;
use std::io::BufReader;
use std::process::{Command, Stdio};

use runx_js_worker::protocol::{
    InvocationLimits, MAX_FRAME_BYTES, PROTOCOL_VERSION, WorkerRequest, WorkerResponse, read_frame,
    write_frame,
};

#[test]
fn packaged_worker_performs_the_versioned_handshake_and_invocation()
-> Result<(), Box<dyn std::error::Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_runx-js-worker"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("worker stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("worker stdout was not piped")?;
    let mut stdout = BufReader::new(stdout);

    write_frame(
        &mut stdin,
        &WorkerRequest::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        MAX_FRAME_BYTES,
    )?;
    let ready = read_frame::<WorkerResponse>(&mut stdout, MAX_FRAME_BYTES)?;
    if ready.is_none() {
        drop(stdin);
        let output = child.wait_with_output()?;
        return Err(format!(
            "worker exited before handshake: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    assert_eq!(
        ready,
        Some(WorkerResponse::Ready {
            protocol_version: PROTOCOL_VERSION
        })
    );

    write_frame(
        &mut stdin,
        &WorkerRequest::Invoke {
            protocol_version: PROTOCOL_VERSION,
            invocation_id: "process-test".to_owned(),
            entry_module: "main.mjs".to_owned(),
            export_name: "default".to_owned(),
            modules: BTreeMap::from([(
                "main.mjs".to_owned(),
                "export default ({ value }) => ({ value, now: Date.now() });".to_owned(),
            )]),
            inputs: serde_json::json!({"value": "runx"}),
            limits: InvocationLimits::default(),
        },
        MAX_FRAME_BYTES,
    )?;
    assert_eq!(
        read_frame::<WorkerResponse>(&mut stdout, MAX_FRAME_BYTES)?,
        Some(WorkerResponse::Result {
            protocol_version: PROTOCOL_VERSION,
            invocation_id: "process-test".to_owned(),
            output: serde_json::json!({"value": "runx", "now": 0})
        })
    );

    drop(stdin);
    assert!(child.wait()?.success());
    Ok(())
}

#[test]
fn packaged_worker_multiplexes_independent_invocations() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_runx-js-worker"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("worker stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("worker stdout was not piped")?;
    let mut stdout = BufReader::new(stdout);

    write_frame(
        &mut stdin,
        &WorkerRequest::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        MAX_FRAME_BYTES,
    )?;
    assert!(matches!(
        read_frame::<WorkerResponse>(&mut stdout, MAX_FRAME_BYTES)?,
        Some(WorkerResponse::Ready { .. })
    ));

    write_frame(
        &mut stdin,
        &WorkerRequest::Invoke {
            protocol_version: PROTOCOL_VERSION,
            invocation_id: "slow".to_owned(),
            entry_module: "main.mjs".to_owned(),
            export_name: "default".to_owned(),
            modules: BTreeMap::from([(
                "main.mjs".to_owned(),
                "export default ({ rounds }) => { let value = 0; for (let i = 0; i < rounds; i += 1) value = (value + i) % 1000003; return { value }; };".to_owned(),
            )]),
            inputs: serde_json::json!({"rounds": 100_000}),
            limits: InvocationLimits::default(),
        },
        MAX_FRAME_BYTES,
    )?;
    write_frame(
        &mut stdin,
        &WorkerRequest::Invoke {
            protocol_version: PROTOCOL_VERSION,
            invocation_id: "fast".to_owned(),
            entry_module: "main.mjs".to_owned(),
            export_name: "default".to_owned(),
            modules: BTreeMap::from([(
                "main.mjs".to_owned(),
                "export default ({ value }) => ({ value });".to_owned(),
            )]),
            inputs: serde_json::json!({"value": "done"}),
            limits: InvocationLimits::default(),
        },
        MAX_FRAME_BYTES,
    )?;

    assert!(matches!(
        read_frame::<WorkerResponse>(&mut stdout, MAX_FRAME_BYTES)?,
        Some(WorkerResponse::Result { invocation_id, .. }) if invocation_id == "fast"
    ));
    assert!(matches!(
        read_frame::<WorkerResponse>(&mut stdout, MAX_FRAME_BYTES)?,
        Some(WorkerResponse::Result { invocation_id, .. }) if invocation_id == "slow"
    ));

    drop(stdin);
    assert!(child.wait()?.success());
    Ok(())
}

#[test]
fn packaged_worker_rejects_a_protocol_version_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_runx-js-worker"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("worker stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("worker stdout was not piped")?;
    let mut stdout = BufReader::new(stdout);

    write_frame(
        &mut stdin,
        &WorkerRequest::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
        },
        MAX_FRAME_BYTES,
    )?;
    assert!(matches!(
        read_frame::<WorkerResponse>(&mut stdout, MAX_FRAME_BYTES)?,
        Some(WorkerResponse::Failure {
            protocol_version: PROTOCOL_VERSION,
            invocation_id: None,
            discard_worker: true,
            ..
        })
    ));

    drop(stdin);
    assert!(child.wait()?.success());
    Ok(())
}

#[test]
fn packaged_worker_rejects_an_inherited_environment() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_runx-js-worker"))
        .env_clear()
        .env("RUNX_UNTRUSTED_INHERITED_VALUE", "must-not-enter-worker")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("inherited an environment"),
        "worker did not explain the fail-closed environment rejection"
    );
    Ok(())
}
