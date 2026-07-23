#![allow(clippy::expect_used)]

use runx_contracts::{JsonObject, JsonValue};

use super::{SandboxPlanInput, build};
use crate::tool_catalogs::native::fixture_input;

#[test]
fn compiles_readonly_requirements_through_core_admission() -> Result<(), String> {
    let output = build(&inputs(JsonObject::from([
        ("network".to_owned(), JsonValue::Bool(false)),
        ("writable_paths".to_owned(), JsonValue::Array(Vec::new())),
    ])))
    .map_err(|error| error.to_string())?;
    assert_eq!(
        at(&output, &["hardening_profile", "decision"]),
        Some("ready")
    );
    assert_eq!(
        at(&output, &["hardening_profile", "declaration", "profile"]),
        Some("readonly")
    );
    assert_eq!(
        at(&output, &["hardening_profile", "admission", "status"]),
        Some("allow")
    );
    Ok(())
}

#[test]
fn refuses_controls_the_runtime_cannot_express() -> Result<(), String> {
    let output = build(&inputs(JsonObject::from([
        ("network".to_owned(), JsonValue::Bool(true)),
        (
            "writable_paths".to_owned(),
            JsonValue::Array(vec![JsonValue::String("tmp".to_owned())]),
        ),
    ])))
    .map_err(|error| error.to_string())?;
    assert_eq!(
        at(&output, &["hardening_profile", "decision"]),
        Some("unsupported_runtime_shape")
    );
    Ok(())
}

#[test]
fn core_policy_refuses_reserved_secret_environment() -> Result<(), String> {
    let output = build(&inputs(JsonObject::from([(
        "env_allowlist".to_owned(),
        JsonValue::Array(vec![JsonValue::String("RUNX_AGENT_API_KEY".to_owned())]),
    )])))
    .map_err(|error| error.to_string())?;
    assert_eq!(
        at(&output, &["hardening_profile", "decision"]),
        Some("refused")
    );
    assert_eq!(
        at(&output, &["hardening_profile", "admission", "status"]),
        Some("deny")
    );
    Ok(())
}

fn inputs(extra: JsonObject) -> SandboxPlanInput {
    let mut requirements = JsonObject::from([
        (
            "source_ref".to_owned(),
            JsonValue::String("requirements:fixture:v1".to_owned()),
        ),
        (
            "source_digest".to_owned(),
            JsonValue::String(format!("sha256:{}", "a".repeat(64))),
        ),
        (
            "observed_at".to_owned(),
            JsonValue::String("2026-07-17T10:00:00Z".to_owned()),
        ),
    ]);
    requirements.extend(extra);
    fixture_input(JsonObject::from([
        (
            "workload".to_owned(),
            JsonValue::Object(JsonObject::from([
                (
                    "skill_ref".to_owned(),
                    JsonValue::String("runx/example@1.0.0".to_owned()),
                ),
                ("requirements".to_owned(), JsonValue::Object(requirements)),
            ])),
        ),
        (
            "as_of".to_owned(),
            JsonValue::String("2026-07-17T12:00:00Z".to_owned()),
        ),
    ]))
    .expect("typed sandbox input")
}

fn at<'a>(value: &'a JsonValue, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    current.as_str()
}
