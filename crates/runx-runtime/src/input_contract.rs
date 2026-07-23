use std::collections::BTreeMap;
use std::fmt;

use runx_contracts::JsonNumber;
use runx_contracts::{JsonObject, JsonValue};
use runx_parser::SkillInput;

/// Apply manifest defaults without overwriting a value the caller supplied.
pub(crate) fn apply_defaults(declared: &BTreeMap<String, SkillInput>, inputs: &mut JsonObject) {
    for (name, input) in declared {
        if !inputs.contains_key(name)
            && let Some(default) = &input.default
        {
            inputs.insert(name.clone(), default.clone());
        }
    }
}

/// Return required manifest inputs that are absent or explicitly null.
pub(crate) fn missing_required(
    declared: &BTreeMap<String, SkillInput>,
    inputs: &JsonObject,
) -> Vec<String> {
    declared
        .iter()
        .filter(|(name, input)| {
            input.required && input.default.is_none() && is_missing(inputs.get(*name))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

pub(crate) fn is_missing(value: Option<&JsonValue>) -> bool {
    matches!(value, None | Some(JsonValue::Null))
}

/// Build the one invocation map a declared local tool receives.
///
/// Runtime-resolved values have precedence over static values. Undeclared
/// values are not ambient tool input. Defaults, required fields, artifact
/// projection, and declared JSON types are enforced before any process starts.
#[cfg(feature = "catalog")]
pub(crate) fn materialize_tool_inputs(
    declared: &BTreeMap<String, SkillInput>,
    static_inputs: &JsonObject,
    resolved_inputs: &JsonObject,
) -> Result<JsonObject, InputContractError> {
    materialize_declared_inputs(declared, static_inputs, resolved_inputs, "tool")
}

pub(crate) fn materialize_runner_inputs(
    declared: &BTreeMap<String, SkillInput>,
    supplied: &JsonObject,
) -> Result<JsonObject, InputContractError> {
    materialize_declared_inputs(declared, supplied, &JsonObject::new(), "runner")
}

fn materialize_declared_inputs(
    declared: &BTreeMap<String, SkillInput>,
    static_inputs: &JsonObject,
    resolved_inputs: &JsonObject,
    owner: &'static str,
) -> Result<JsonObject, InputContractError> {
    let mut supplied = static_inputs.clone();
    supplied.extend(resolved_inputs.clone());

    declared
        .iter()
        .filter_map(|(name, input)| {
            materialize_input(owner, name, input, supplied.get(name)).transpose()
        })
        .collect()
}

fn materialize_input(
    owner: &'static str,
    name: &str,
    input: &SkillInput,
    supplied: Option<&JsonValue>,
) -> Result<Option<(String, JsonValue)>, InputContractError> {
    let value = supplied.or(input.default.as_ref());
    let Some(value) = value else {
        return if input.required {
            Err(InputContractError::new(
                name,
                format!("{owner} input '{name}' is required"),
            ))
        } else {
            Ok(None)
        };
    };
    if matches!(value, JsonValue::Null) {
        return if input.required {
            Err(InputContractError::new(
                name,
                format!("{owner} input '{name}' is required"),
            ))
        } else {
            Ok(None)
        };
    }

    let value = if input.artifact == Some(true) {
        unwrap_artifact(value, name).map_err(|message| InputContractError::new(name, message))?
    } else {
        value.clone()
    };
    if matches!(value, JsonValue::Null) {
        return if input.required {
            Err(InputContractError::new(
                name,
                format!("{owner} input '{name}' is required"),
            ))
        } else {
            Ok(None)
        };
    }
    if !input.accepts_value(&value) {
        return Err(InputContractError::new(
            name,
            format!(
                "{owner} input '{name}' must be {}, received {}",
                input.input_type,
                json_type(&value),
            ),
        ));
    }
    Ok(Some((name.to_owned(), value)))
}

fn unwrap_artifact(value: &JsonValue, name: &str) -> Result<JsonValue, String> {
    let JsonValue::Object(object) = value else {
        return Ok(value.clone());
    };
    if let Some(data) = object.get("data") {
        return Ok(data.clone());
    }
    for envelope in ["artifact", "output"] {
        if let Some(JsonValue::Object(nested)) = object.get(envelope)
            && let Some(data) = nested.get("data")
        {
            return Ok(data.clone());
        }
    }
    if object.contains_key("schema") || object.contains_key("meta") {
        return Err(format!(
            "tool input '{name}' is an artifact envelope without data"
        ));
    }
    Ok(value.clone())
}

fn json_type(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(JsonNumber::I64(_) | JsonNumber::U64(_)) => "integer",
        JsonValue::Number(JsonNumber::F64(_)) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputContractError {
    input: String,
    message: String,
}

impl InputContractError {
    fn new(input: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            message: message.into(),
        }
    }

    pub(crate) fn input(&self) -> &str {
        &self.input
    }
}

impl fmt::Display for InputContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
