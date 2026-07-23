use std::error::Error;

use criterion::Criterion;
use runx_contracts::{JsonNumber, JsonObject, JsonValue};
use runx_runtime::{InvocationStatus, SkillOutput};

#[path = "volume_paths/artifact_io.rs"]
mod artifact_io;
#[path = "volume_paths/event_paging.rs"]
mod event_paging;
#[path = "volume_paths/twitter_selection.rs"]
mod twitter_selection;

#[allow(clippy::expect_used)]
pub(super) fn register(c: &mut Criterion) {
    artifact_io::register(c);
    event_paging::register(c);
    twitter_selection::register(c);
}

fn output_object(output: SkillOutput) -> Result<JsonObject, Box<dyn Error>> {
    if output.status != InvocationStatus::Success {
        return Err(std::io::Error::other(output.stderr).into());
    }
    match serde_json::from_str::<JsonValue>(&output.stdout)? {
        JsonValue::Object(object) => Ok(object),
        _ => Err(std::io::Error::other("runtime output was not an object").into()),
    }
}

fn wrapped_data(output: SkillOutput, name: &str) -> Result<JsonObject, Box<dyn Error>> {
    output_object(output)?
        .remove(name)
        .and_then(|value| match value {
            JsonValue::Object(mut wrapper) => wrapper.remove("data"),
            _ => None,
        })
        .and_then(|value| match value {
            JsonValue::Object(data) => Some(data),
            _ => None,
        })
        .ok_or_else(|| std::io::Error::other(format!("runtime output omitted {name}.data")).into())
}

fn u64_field(object: &JsonObject, field: &str) -> Result<u64, Box<dyn Error>> {
    match object.get(field) {
        Some(JsonValue::Number(JsonNumber::U64(value))) => Ok(*value),
        Some(JsonValue::Number(JsonNumber::I64(value))) => Ok(u64::try_from(*value)?),
        _ => Err(std::io::Error::other(format!("runtime output omitted numeric {field}")).into()),
    }
}

fn bool_field(object: &JsonObject, field: &str) -> Result<bool, Box<dyn Error>> {
    object
        .get(field)
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| {
            std::io::Error::other(format!("runtime output omitted boolean {field}")).into()
        })
}

fn record_native_metric(
    name: &str,
    executor: &runx_runtime::adapters::agent_tools::RuntimeToolExecutor,
) -> Result<(), Box<dyn Error>> {
    super::record_resource_metric(
        name,
        super::session_metric(executor.javascript_session_stats()),
    )
}
