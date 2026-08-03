use std::collections::BTreeMap;

use runx_contracts::{JsonObject, JsonValue};
use runx_core::policy::admit_agent_tool_ref;

use crate::ValidationError;

use super::{
    FIELDS, RawSkillIr, SkillArtifactContract, SkillGovernance, SkillIdempotencyPolicy, SkillInput,
    SkillRetryPolicy, field_value, first_value, nested_value, validate_execution_semantics,
};

pub(super) fn validate_skill_governance(
    raw: &RawSkillIr,
    runx: Option<&JsonObject>,
    risk: Option<&JsonValue>,
) -> Result<SkillGovernance, ValidationError> {
    Ok(SkillGovernance {
        retry: validate_retry(
            first_value(raw.frontmatter.get("retry"), field_value(runx, "retry")),
            "retry",
        )?,
        idempotency: validate_idempotency(
            first_value(
                raw.frontmatter.get("idempotency"),
                field_value(runx, "idempotency"),
            ),
            "idempotency",
        )?,
        mutating: validate_mutating(
            first_value(
                first_value(
                    raw.frontmatter.get("mutating"),
                    nested_value(risk, "mutating"),
                ),
                field_value(runx, "mutating"),
            ),
            "mutating",
        )?,
        artifacts: validate_artifact_contract(field_value(runx, "artifacts"), "runx.artifacts")?,
        allowed_tools: validate_allowed_tools(
            field_value(runx, "allowed_tools"),
            "runx.allowed_tools",
        )?,
        execution: validate_execution_semantics(
            first_value(
                raw.frontmatter.get("execution"),
                field_value(runx, "execution"),
            ),
            "execution",
        )?,
    })
}

pub fn validate_skill_artifact_contract(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<SkillArtifactContract>, ValidationError> {
    validate_artifact_contract(value, field)
}

pub(super) fn validate_artifact_contract(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<SkillArtifactContract>, ValidationError> {
    let Some(record) = FIELDS.optional_object(value, field)? else {
        return Ok(None);
    };
    let emits = match record.get("emits") {
        Some(JsonValue::String(value)) => Some(vec![value.clone()]),
        value => FIELDS.optional_string_array(value, &format!("{field}.emits"))?,
    };
    let named_emits = validate_named_emits(
        first_value(record.get("named_emits"), record.get("namedEmits")),
        &format!("{field}.named_emits"),
    )?;
    let packets = validate_named_emits(record.get("packets"), &format!("{field}.packets"))?;
    if let Some(packet_outputs) = &packets {
        let Some(named_outputs) = &named_emits else {
            return Err(
                FIELDS.validation_error(format!("{field}.packets requires {field}.named_emits"))
            );
        };
        if let Some(output) = packet_outputs
            .keys()
            .find(|output| !named_outputs.contains_key(*output))
        {
            return Err(FIELDS.validation_error(format!(
                "{field}.packets.{output} must name an output declared by {field}.named_emits"
            )));
        }
    }
    let wrap_as = FIELDS.optional_non_empty_string(
        first_value(record.get("wrap_as"), record.get("wrapAs")),
        &format!("{field}.wrap_as"),
    )?;
    let packet =
        FIELDS.optional_non_empty_string(record.get("packet"), &format!("{field}.packet"))?;
    if packet.is_some() && wrap_as.is_none() {
        return Err(FIELDS.validation_error(format!(
            "{field}.packet requires {field}.wrap_as. Use named_emits for named packet outputs."
        )));
    }
    if emits.is_none() && named_emits.is_none() && packets.is_none() && wrap_as.is_none() {
        return Ok(None);
    }
    Ok(Some(SkillArtifactContract {
        emits,
        named_emits,
        packets,
        wrap_as,
        packet,
    }))
}

pub(crate) fn validate_inputs(
    inputs: JsonObject,
    field_prefix: &str,
) -> Result<BTreeMap<String, SkillInput>, ValidationError> {
    inputs
        .into_iter()
        .map(|(name, value)| {
            let field = format!("{field_prefix}.{name}");
            let input = FIELDS.required_object(Some(&value), &field)?;
            FIELDS.reject_unknown_fields(
                input,
                &field,
                &[
                    "type",
                    "required",
                    "description",
                    "default",
                    "artifact",
                    "packet",
                    "schema",
                ],
            )?;
            let input_type = FIELDS
                .optional_string(input.get("type"), &format!("{field}.type"))?
                .unwrap_or_else(|| "string".to_owned());
            if !matches!(
                input_type.as_str(),
                "array" | "boolean" | "integer" | "json" | "number" | "object" | "string"
            ) {
                return Err(FIELDS.validation_error(format!(
                    "{field}.type must be one of array, boolean, integer, json, number, object, or string"
                )));
            }
            let validated = SkillInput {
                input_type,
                required: FIELDS
                    .optional_bool(input.get("required"), &format!("{field}.required"))?
                    .unwrap_or(false),
                description: FIELDS.optional_string(
                    input.get("description"),
                    &format!("{field}.description"),
                )?,
                default: input.get("default").cloned(),
                artifact: FIELDS
                    .optional_bool(input.get("artifact"), &format!("{field}.artifact"))?,
                packet: FIELDS.optional_non_empty_string(
                    input.get("packet"),
                    &format!("{field}.packet"),
                )?,
                schema: FIELDS
                    .optional_object(input.get("schema"), &format!("{field}.schema"))?,
            };
            validate_input_schema(&validated, &field)?;
            Ok((name.clone(), validated))
        })
        .collect()
}

fn validate_input_schema(input: &SkillInput, field: &str) -> Result<(), ValidationError> {
    if input.packet.is_some() {
        if input.input_type != "json" {
            return Err(
                FIELDS.validation_error(format!("{field}.packet requires {field}.type to be json"))
            );
        }
        if input.schema.is_some() {
            return Err(FIELDS.validation_error(format!(
                "{field}.packet and {field}.schema are mutually exclusive; the packet catalog owns the schema"
            )));
        }
        if input.default.is_some() {
            return Err(FIELDS.validation_error(format!(
                "{field}.packet cannot declare a default packet value"
            )));
        }
        return Ok(());
    }
    let Some(fragment) = &input.schema else {
        if let Some(default) = &input.default
            && !input.accepts_value(default)
        {
            return Err(FIELDS.validation_error(format!(
                "{field}.default must match declared type {}",
                input.input_type
            )));
        }
        return Ok(());
    };
    let duplicate = ["type", "description", "default"]
        .into_iter()
        .find(|name| fragment.contains_key(*name));
    if let Some(duplicate) = duplicate {
        return Err(FIELDS.validation_error(format!(
            "{field}.schema.{duplicate} duplicates {field}.{duplicate}"
        )));
    }
    if let Some(examples) = fragment.get("examples")
        && !matches!(examples, JsonValue::Array(_))
    {
        return Err(FIELDS.validation_error(format!("{field}.schema.examples must be an array")));
    }
    let schema = serde_json::to_value(input.effective_schema()).map_err(|error| {
        FIELDS.validation_error(format!("{field}.schema could not be serialized: {error}"))
    })?;
    jsonschema::draft202012::meta::validate(&schema)
        .map_err(|error| FIELDS.validation_error(format!("{field}.schema is invalid: {error}")))?;
    let values = input
        .default
        .iter()
        .map(|value| ("default".to_owned(), value))
        .chain(
            input
                .schema
                .as_ref()
                .and_then(|fragment| fragment.get("examples"))
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, value)| (format!("schema.examples[{index}]"), value)),
        )
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let validator = jsonschema::draft202012::options()
        .build(&schema)
        .map_err(|error| FIELDS.validation_error(format!("{field}.schema is invalid: {error}")))?;
    for (label, value) in values {
        let instance = serde_json::to_value(value).map_err(|error| {
            FIELDS.validation_error(format!("{field}.{label} could not be serialized: {error}"))
        })?;
        if !validator.is_valid(&instance) {
            return Err(FIELDS.validation_error(format!(
                "{field}.{label} must match the declared input schema"
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_retry(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<SkillRetryPolicy>, ValidationError> {
    let Some(retry) = FIELDS.optional_object(value, field)? else {
        return Ok(None);
    };
    let max_attempts = FIELDS
        .optional_u64(retry.get("max_attempts"), &format!("{field}.max_attempts"))?
        .unwrap_or(1);
    if max_attempts == 0 {
        return Err(
            FIELDS.validation_error(format!("{field}.max_attempts must be a positive integer."))
        );
    }
    Ok(Some(SkillRetryPolicy { max_attempts }))
}

pub(super) fn validate_idempotency(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<SkillIdempotencyPolicy>, ValidationError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) if value.trim().is_empty() => {
            Err(FIELDS.validation_error(format!("{field} must not be empty.")))
        }
        Some(JsonValue::String(value)) => Ok(Some(SkillIdempotencyPolicy {
            key: Some(value.clone()),
        })),
        Some(value) => {
            let record = FIELDS.required_object(Some(value), field)?;
            Ok(Some(SkillIdempotencyPolicy {
                key: FIELDS
                    .optional_non_empty_string(record.get("key"), &format!("{field}.key"))?,
            }))
        }
    }
}

pub(super) fn validate_mutating(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<bool>, ValidationError> {
    FIELDS.optional_bool(value, field)
}

fn validate_named_emits(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<BTreeMap<String, String>>, ValidationError> {
    let Some(record) = FIELDS.optional_object(value, field)? else {
        return Ok(None);
    };
    record
        .into_iter()
        .map(|(key, value)| {
            let JsonValue::String(value) = value else {
                return Err(
                    FIELDS.validation_error(format!("{field}.{key} must be a non-empty string."))
                );
            };
            if value.trim().is_empty() {
                return Err(
                    FIELDS.validation_error(format!("{field}.{key} must be a non-empty string."))
                );
            }
            Ok((key, value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map(Some)
}

pub(super) fn validate_allowed_tools(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<Vec<String>>, ValidationError> {
    let Some(values) = FIELDS.optional_string_array(value, field)? else {
        return Ok(None);
    };
    for value in &values {
        let admission = admit_agent_tool_ref(value);
        if !admission.allowed {
            return Err(FIELDS.validation_error(format!(
                "{field} entry {value:?} is not an admissible agent tool ref: {}.",
                admission.reason
            )));
        }
    }
    Ok(Some(values))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use runx_contracts::{JsonObject, JsonValue};

    use super::{validate_artifact_contract, validate_inputs};

    fn packet_input(extra: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonObject {
        let mut input = JsonObject::from([
            ("type".to_owned(), JsonValue::String("json".to_owned())),
            ("required".to_owned(), JsonValue::Bool(true)),
            (
                "packet".to_owned(),
                JsonValue::String("runx.test.plan.v1".to_owned()),
            ),
        ]);
        input.extend(
            extra
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        JsonObject::from([("plan".to_owned(), JsonValue::Object(input))])
    }

    #[test]
    fn packet_input_reference_is_parser_owned() {
        let inputs = validate_inputs(packet_input([]), "inputs").expect("packet input");
        assert_eq!(
            inputs.get("plan").and_then(|input| input.packet.as_deref()),
            Some("runx.test.plan.v1")
        );
    }

    #[test]
    fn packet_input_reference_rejects_parallel_contract_ownership() {
        let inline_schema = packet_input([(
            "schema",
            JsonValue::Object(JsonObject::from([(
                "properties".to_owned(),
                JsonValue::Object(JsonObject::new()),
            )])),
        )]);
        let default_packet = packet_input([("default", JsonValue::Object(JsonObject::new()))]);
        let wrong_type = packet_input([("type", JsonValue::String("object".to_owned()))]);

        assert!(validate_inputs(inline_schema, "inputs").is_err());
        assert!(validate_inputs(default_packet, "inputs").is_err());
        assert!(validate_inputs(wrong_type, "inputs").is_err());
    }

    #[test]
    fn packet_bindings_must_reference_named_outputs() {
        let artifacts = JsonValue::Object(BTreeMap::from([
            (
                "named_emits".to_owned(),
                JsonValue::Object(BTreeMap::from([(
                    "plan".to_owned(),
                    JsonValue::String("plan".to_owned()),
                )])),
            ),
            (
                "packets".to_owned(),
                JsonValue::Object(BTreeMap::from([(
                    "other".to_owned(),
                    JsonValue::String("runx.plan.v1".to_owned()),
                )])),
            ),
        ]));

        assert!(validate_artifact_contract(Some(&artifacts), "artifacts").is_err());
    }

    #[test]
    fn named_output_and_packet_identity_are_preserved_separately()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = JsonValue::Object(BTreeMap::from([
            (
                "named_emits".to_owned(),
                JsonValue::Object(BTreeMap::from([(
                    "plan".to_owned(),
                    JsonValue::String("plan".to_owned()),
                )])),
            ),
            (
                "packets".to_owned(),
                JsonValue::Object(BTreeMap::from([(
                    "plan".to_owned(),
                    JsonValue::String("runx.plan.v1".to_owned()),
                )])),
            ),
        ]));

        let Some(contract) = validate_artifact_contract(Some(&artifacts), "artifacts")? else {
            return Err("artifact contract is missing".into());
        };

        assert_eq!(
            contract
                .named_emits
                .as_ref()
                .and_then(|outputs| outputs.get("plan"))
                .map(String::as_str),
            Some("plan")
        );
        assert_eq!(
            contract
                .packets
                .as_ref()
                .and_then(|packets| packets.get("plan"))
                .map(String::as_str),
            Some("runx.plan.v1")
        );
        Ok(())
    }
}
