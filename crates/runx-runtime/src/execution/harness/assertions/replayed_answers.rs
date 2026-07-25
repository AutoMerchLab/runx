use runx_contracts::{JsonObject, JsonValue};

use crate::execution::harness::runner::{HarnessReplayError, HarnessReplayOutput};

pub(super) fn assert_caller_answers_replayed(
    output: &HarnessReplayOutput,
) -> Result<(), HarnessReplayError> {
    let Some(answers) = output
        .fixture
        .caller
        .get("answers")
        .and_then(JsonValue::as_object)
    else {
        return Ok(());
    };
    let observed = output
        .skill_output
        .iter()
        .map(|skill_output| skill_output.value.clone())
        .chain(
            output
                .steps
                .iter()
                .map(|step| JsonValue::Object(step.contract.clone())),
        )
        .collect::<Vec<_>>();
    assert_replayed_answers(answers, &observed)
}

fn assert_replayed_answers(
    answers: &JsonObject,
    observed: &[JsonValue],
) -> Result<(), HarnessReplayError> {
    for (request_id, answer) in answers {
        let expected = json_text(answer);
        if observed.iter().any(|actual| json_contains(answer, actual)) {
            continue;
        }
        return Err(HarnessReplayError::Mismatch {
            field: format!("caller.answers.{request_id}.replayed"),
            expected,
            actual: json_text(&JsonValue::Array(observed.to_vec())),
        });
    }
    Ok(())
}

fn json_contains(expected: &JsonValue, actual: &JsonValue) -> bool {
    if json_text(expected) == json_text(actual) || json_subset_matches(expected, actual) {
        return true;
    }
    match actual {
        JsonValue::Array(values) => values.iter().any(|value| json_contains(expected, value)),
        JsonValue::Object(values) => values.values().any(|value| json_contains(expected, value)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => false,
    }
}

fn json_subset_matches(expected: &JsonValue, actual: &JsonValue) -> bool {
    let (JsonValue::Object(expected), JsonValue::Object(actual)) = (expected, actual) else {
        return false;
    };
    expected.iter().all(|(key, expected_value)| {
        actual
            .get(key)
            .is_some_and(|actual_value| match (expected_value, actual_value) {
                (JsonValue::Object(_), JsonValue::Object(_)) => {
                    json_subset_matches(expected_value, actual_value)
                }
                _ => json_text(expected_value) == json_text(actual_value),
            })
    })
}

fn json_text(value: &JsonValue) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|error| format!("<unserializable JSON value: {error}>"))
}

#[cfg(test)]
mod tests {
    use runx_contracts::{JsonNumber, JsonObject, JsonValue};

    use super::{HarnessReplayError, assert_replayed_answers};

    fn send_plan(decision: &str) -> JsonValue {
        JsonValue::Object(JsonObject::from([(
            "send_plan".to_owned(),
            JsonValue::Object(JsonObject::from([(
                "decision".to_owned(),
                JsonValue::String(decision.to_owned()),
            )])),
        )]))
    }

    #[test]
    fn supplied_caller_answers_are_exact_replay_oracles() {
        let answer = send_plan("ready");
        let answers = JsonObject::from([("agent_task.send-as.output".to_owned(), answer.clone())]);

        assert!(assert_replayed_answers(&answers, &[answer]).is_ok());
    }

    #[test]
    fn unused_or_changed_caller_answer_fails_with_request_path() {
        let answers =
            JsonObject::from([("agent_task.send-as.output".to_owned(), send_plan("ready"))]);

        let result = assert_replayed_answers(&answers, &[send_plan("needs_input")]);

        assert!(matches!(
            result,
            Err(HarnessReplayError::Mismatch { field, .. })
                if field == "caller.answers.agent_task.send-as.output.replayed"
        ));
    }

    #[test]
    fn fixtures_without_answers_have_no_replay_obligation() {
        assert!(assert_replayed_answers(&JsonObject::new(), &[]).is_ok());
    }

    #[test]
    fn integer_representations_compare_as_json_values() {
        let answers = JsonObject::from([(
            "agent_task.invoice.output".to_owned(),
            JsonValue::Number(JsonNumber::I64(1840)),
        )]);

        assert!(
            assert_replayed_answers(&answers, &[JsonValue::Number(JsonNumber::U64(1840))]).is_ok()
        );
    }

    #[test]
    fn nested_typed_packet_may_enrich_a_replayed_answer() {
        let answer = send_plan("ready");
        let answers = JsonObject::from([("agent_task.send-as.output".to_owned(), answer)]);
        let observed = JsonValue::Object(JsonObject::from([(
            "send_plan_packet".to_owned(),
            JsonValue::Object(JsonObject::from([(
                "data".to_owned(),
                JsonValue::Object(JsonObject::from([
                    (
                        "send_plan".to_owned(),
                        JsonValue::Object(JsonObject::from([
                            ("decision".to_owned(), JsonValue::String("ready".to_owned())),
                            (
                                "approval".to_owned(),
                                JsonValue::String("required".to_owned()),
                            ),
                        ])),
                    ),
                    (
                        "packet".to_owned(),
                        JsonValue::String("runx.send.plan.v1".to_owned()),
                    ),
                ])),
            )])),
        )]));

        assert!(assert_replayed_answers(&answers, &[observed]).is_ok());
    }
}
