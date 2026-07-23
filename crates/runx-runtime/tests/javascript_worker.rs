use std::sync::{Arc, Barrier};
use std::thread;

use runx_contracts::{JsonNumber, JsonObject, JsonValue};

use crate::javascript_worker_support::{JavaScriptPackage, expected_json, success_json};

#[test]
fn javascript_worker_reuses_a_process_without_reusing_javascript_state()
-> Result<(), Box<dyn std::error::Error>> {
    let package = JavaScriptPackage::new(
        "export default ({ value }) => { const prior = globalThis.runxLeak; globalThis.runxLeak = value; return { prior: prior ?? null, value }; };",
    )?;

    let first = package.invoke(JsonObject::from([(
        "value".to_owned(),
        JsonValue::String("first".to_owned()),
    )]))?;
    let second = package.invoke(JsonObject::from([(
        "value".to_owned(),
        JsonValue::String("second".to_owned()),
    )]))?;

    assert_eq!(
        success_json(&first)?,
        expected_json(serde_json::json!({"prior": null, "value": "first"}))
    );
    assert_eq!(
        success_json(&second)?,
        expected_json(serde_json::json!({"prior": null, "value": "second"}))
    );
    assert_eq!(package.session_stats().spawned_process_count, 1);
    Ok(())
}

#[test]
fn javascript_session_multiplexes_concurrent_invocations_in_one_process()
-> Result<(), Box<dyn std::error::Error>> {
    let package = Arc::new(JavaScriptPackage::with_max_concurrency(
        "export default ({ value, rounds }) => { let digest = 0; for (let i = 0; i < rounds; i += 1) digest = (digest + i) % 1000003; return { value, digest }; };",
        4,
    )?);
    let barrier = Arc::new(Barrier::new(5));
    let handles = (0_u64..4)
        .map(|value| {
            let package = package.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                package.invoke(JsonObject::from([
                    (
                        "value".to_owned(),
                        JsonValue::Number(JsonNumber::U64(value)),
                    ),
                    (
                        "rounds".to_owned(),
                        JsonValue::Number(JsonNumber::U64(100_000)),
                    ),
                ]))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();

    for handle in handles {
        let output = handle
            .join()
            .map_err(|_| std::io::Error::other("JavaScript invocation thread panicked"))??;
        assert!(output.succeeded(), "{}", output.stderr);
    }
    let stats = package.session_stats();
    assert_eq!(stats.spawned_process_count, 1);
    assert_eq!(stats.peak_in_flight, 4);
    Ok(())
}

#[test]
fn javascript_worker_resolves_only_the_validated_in_memory_bundle()
-> Result<(), Box<dyn std::error::Error>> {
    let package = JavaScriptPackage::new(
        "export default () => ({ now: Date.now(), process: typeof process, fetch: typeof fetch, require: typeof require });",
    )?;
    let output = package.invoke(JsonObject::new())?;

    assert_eq!(
        success_json(&output)?,
        expected_json(serde_json::json!({
            "now": 0,
            "process": "undefined",
            "fetch": "undefined",
            "require": "undefined"
        }))
    );
    Ok(())
}

#[test]
fn javascript_worker_resolves_static_relative_imports_from_the_validated_bundle()
-> Result<(), Box<dyn std::error::Error>> {
    let package = JavaScriptPackage::with_modules(
        "import { answer } from './lib/answer.mjs'; export default () => ({ answer });",
        [("lib/answer.mjs", "export const answer = 42;")],
    )?;
    let output = package.invoke(JsonObject::new())?;

    assert_eq!(
        success_json(&output)?,
        expected_json(serde_json::json!({"answer": 42}))
    );
    Ok(())
}

#[test]
fn volume_independent_artifacts_drive_one_worker_across_bounded_pages()
-> Result<(), Box<dyn std::error::Error>> {
    let package = JavaScriptPackage::new(
        "export default (inputs) => {\n  const page = inputs.runx_page;\n  const state = page.state ?? { count: 0, sum: 0 };\n  for (const raw of page.records) { const value = JSON.parse(raw); state.count += 1; state.sum += value.value; }\n  const runx_page = { state };\n  return page.eof ? { runx_page, result: state } : { runx_page };\n};",
    )?;
    let records = (0_u64..20_000)
        .map(|value| format!("{{\"value\":{value},\"padding\":\"{}\"}}", "x".repeat(32)))
        .collect::<Vec<_>>();
    let archive = format!("window.YTD.items.part0 = [{}]", records.join(","));

    let output = package.invoke_paged(
        "archive.data",
        &archive,
        64 * 1024,
        JsonObject::new(),
    )?;

    assert_eq!(
        success_json(&output)?,
        expected_json(serde_json::json!({
            "result": {
                "count": 20_000,
                "sum": 199_990_000_u64
            }
        }))
    );
    assert_eq!(package.session_stats().spawned_process_count, 1);
    let page_count = output
        .metadata
        .get("local_artifact_pages")
        .and_then(JsonValue::as_object)
        .and_then(|metadata| metadata.get("page_count"));
    assert!(matches!(page_count, Some(JsonValue::Number(JsonNumber::U64(count))) if *count > 1));
    assert!(!output.stdout.contains("archive.data"));
    Ok(())
}
