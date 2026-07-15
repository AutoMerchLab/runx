use runx_contracts::{JsonObject, JsonValue};

pub fn json_failure_value(message: &str, code: &str) -> JsonValue {
    let mut error = JsonObject::new();
    error.insert("code".to_owned(), JsonValue::String(code.to_owned()));
    error.insert("message".to_owned(), JsonValue::String(message.to_owned()));

    let mut envelope = JsonObject::new();
    envelope.insert("status".to_owned(), JsonValue::String("failure".to_owned()));
    envelope.insert("error".to_owned(), JsonValue::Object(error));
    JsonValue::Object(envelope)
}

pub fn json_failure_output(message: &str, code: &str) -> String {
    match serde_json::to_string_pretty(&json_failure_value(message, code)) {
        Ok(json) => format!("{json}\n"),
        Err(_) => {
            "{\n  \"status\": \"failure\",\n  \"error\": {\n    \"code\": \"serialize_output\",\n    \"message\": \"failed to serialize CLI failure\"\n  }\n}\n"
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::json_failure_output;

    #[test]
    fn failure_envelope_has_one_exact_common_shape() -> Result<(), serde_json::Error> {
        let value: serde_json::Value =
            serde_json::from_str(&json_failure_output("bad input", "invalid_args"))?;
        assert_eq!(value["status"], "failure");
        assert_eq!(value["error"]["code"], "invalid_args");
        assert_eq!(value["error"]["message"], "bad input");
        assert_eq!(value.as_object().map(serde_json::Map::len), Some(2));
        assert_eq!(
            value["error"].as_object().map(serde_json::Map::len),
            Some(2)
        );
        Ok(())
    }
}
