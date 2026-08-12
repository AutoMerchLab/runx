use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use runx_contracts::{JsonObject, JsonValue};
use serde::{Deserialize, Serialize};

use super::{SkillRunError, invalid};
use crate::RuntimeError;

/// Resolution values retain their authority lane instead of flattening every
/// caller-supplied value into an agent answer. Approval provenance is needed
/// both by live resume files and by inline harness cases.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ResolutionAnswers {
    values: JsonObject,
    human_approvals: BTreeSet<String>,
    request_digests: BTreeMap<String, String>,
    #[serde(default)]
    digest_bound_requests: BTreeSet<String>,
}

impl ResolutionAnswers {
    pub(super) fn agent(values: JsonObject) -> Self {
        Self {
            values,
            human_approvals: BTreeSet::new(),
            request_digests: BTreeMap::new(),
            digest_bound_requests: BTreeSet::new(),
        }
    }

    pub(super) fn from_lanes(
        answers: JsonObject,
        approvals: impl IntoIterator<Item = (String, JsonValue)>,
    ) -> Result<Self, SkillRunError> {
        let mut resolved = Self::agent(answers);
        for (gate_id, decision) in approvals {
            if !is_human_approval_payload(&decision) {
                return Err(invalid(format!(
                    "approvals.{gate_id} must be a boolean or {{approved: boolean, reason?: string}}"
                )));
            }
            if resolved.values.insert(gate_id.clone(), decision).is_some() {
                return Err(invalid(format!(
                    "request {gate_id} is declared in both answers and approvals"
                )));
            }
            resolved.human_approvals.insert(gate_id);
        }
        Ok(resolved)
    }

    pub(super) fn get(&self, request_id: &str) -> Option<&JsonValue> {
        self.values.get(request_id)
    }

    pub(super) fn is_human_approval(&self, request_id: &str) -> bool {
        self.human_approvals.contains(request_id)
    }

    pub(super) fn request_digest(&self, request_id: &str) -> Option<&str> {
        self.request_digests.get(request_id).map(String::as_str)
    }

    pub(super) fn requires_request_digest(&self, request_id: &str) -> bool {
        self.digest_bound_requests.contains(request_id)
    }

    /// Carry validated resolutions across graph pauses. Nested graphs restart
    /// from their owning outer step, so later continuations must retain the
    /// earlier answers without asking the operator to repeat completed gates.
    /// Conflicts fail closed, including attempts to change an answer's actor
    /// lane or its request-digest binding.
    pub(super) fn merge(&mut self, incoming: Self) -> Result<(), SkillRunError> {
        for (request_id, value) in incoming.values {
            let had_existing = self.values.contains_key(&request_id);
            if let Some(existing) = self.values.get(&request_id) {
                if existing != &value {
                    return Err(invalid(format!(
                        "continuation changed the recorded resolution for {request_id}"
                    )));
                }
            } else {
                self.values.insert(request_id.clone(), value);
            }

            let existing_is_human = self.human_approvals.contains(&request_id);
            let incoming_is_human = incoming.human_approvals.contains(&request_id);
            if had_existing && existing_is_human != incoming_is_human {
                return Err(invalid(format!(
                    "continuation changed the recorded authority lane for {request_id}"
                )));
            }
            if incoming_is_human {
                self.human_approvals.insert(request_id);
            }
        }
        for (request_id, digest) in incoming.request_digests {
            if let Some(existing) = self.request_digests.get(&request_id) {
                if existing != &digest {
                    return Err(invalid(format!(
                        "continuation changed the request digest binding for {request_id}"
                    )));
                }
            } else {
                self.request_digests.insert(request_id, digest);
            }
        }
        self.digest_bound_requests
            .extend(incoming.digest_bound_requests);
        Ok(())
    }
}

pub(super) fn read_answers(path: &Path) -> Result<ResolutionAnswers, SkillRunError> {
    let raw = fs::read_to_string(path)
        .map_err(|source| RuntimeError::io(format!("reading {}", path.display()), source))?;
    let value = serde_json::from_str::<JsonValue>(&raw).map_err(|source| {
        RuntimeError::json(format!("parsing answers file {}", path.display()), source)
    })?;
    match value {
        JsonValue::Object(object) => normalize_answers(object),
        _ => Err(invalid("answers file must be a JSON object")),
    }
}

fn normalize_answers(mut object: JsonObject) -> Result<ResolutionAnswers, SkillRunError> {
    let nested_shape = object.contains_key("answers")
        || object.contains_key("approvals")
        || object.contains_key("request_digests");
    if !nested_shape {
        return Ok(ResolutionAnswers::agent(object));
    }
    let extra = object
        .keys()
        .filter(|key| !matches!(key.as_str(), "answers" | "approvals" | "request_digests"))
        .cloned()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(invalid(format!(
            "answers file mixes top-level keys [{}] with the nested answers/approvals shape",
            extra.join(", ")
        )));
    }
    let answers = match object.remove("answers") {
        Some(JsonValue::Object(nested)) => nested,
        Some(_) => return Err(invalid("answers field must be a JSON object")),
        None => JsonObject::new(),
    };
    let approvals = match object.remove("approvals") {
        Some(JsonValue::Object(approvals)) => approvals,
        Some(_) => return Err(invalid("approvals field must be a JSON object")),
        None => JsonObject::new(),
    };
    let request_digests = match object.remove("request_digests") {
        Some(JsonValue::Object(digests)) => digests
            .into_iter()
            .map(|(id, digest)| match digest {
                JsonValue::String(digest) if !digest.trim().is_empty() => Ok((id, digest)),
                _ => Err(invalid(
                    "request_digests values must be non-empty digest strings",
                )),
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?,
        Some(_) => return Err(invalid("request_digests field must be a JSON object")),
        None => BTreeMap::new(),
    };
    let mut resolved = ResolutionAnswers::from_lanes(answers, approvals)?;
    if !request_digests.is_empty() {
        resolved
            .digest_bound_requests
            .extend(resolved.values.keys().cloned());
    }
    resolved.request_digests = request_digests;
    Ok(resolved)
}

fn is_human_approval_payload(value: &JsonValue) -> bool {
    match value {
        JsonValue::Bool(_) => true,
        JsonValue::Object(object) => {
            matches!(object.get("approved"), Some(JsonValue::Bool(_)))
                && object
                    .keys()
                    .all(|key| matches!(key.as_str(), "approved" | "reason"))
                && match object.get("reason") {
                    None => true,
                    Some(JsonValue::String(reason)) => !reason.trim().is_empty(),
                    Some(_) => false,
                }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{ResolutionAnswers, normalize_answers};
    use runx_contracts::{JsonObject, JsonValue};

    #[test]
    fn nested_approvals_retain_host_attested_human_provenance() -> Result<(), String> {
        let approval_id = "send.approval";
        let answers = normalize_answers(JsonObject::from([
            (
                "answers".to_owned(),
                JsonValue::Object(JsonObject::from([(
                    "agent.task".to_owned(),
                    JsonValue::String("done".to_owned()),
                )])),
            ),
            (
                "approvals".to_owned(),
                JsonValue::Object(JsonObject::from([(
                    approval_id.to_owned(),
                    JsonValue::Object(JsonObject::from([
                        ("approved".to_owned(), JsonValue::Bool(true)),
                        (
                            "reason".to_owned(),
                            JsonValue::String("operator authorized the exact send".to_owned()),
                        ),
                    ])),
                )])),
            ),
        ]))
        .map_err(|error| error.to_string())?;

        assert!(answers.is_human_approval(approval_id));
        assert!(!answers.is_human_approval("agent.task"));
        assert!(matches!(
            answers.get(approval_id),
            Some(JsonValue::Object(_))
        ));
        Ok(())
    }

    #[test]
    fn flat_answers_do_not_gain_human_approval_authority() -> Result<(), String> {
        let answers = normalize_answers(JsonObject::from([(
            "send.approval".to_owned(),
            JsonValue::Bool(true),
        )]))
        .map_err(|error| error.to_string())?;

        assert!(!answers.is_human_approval("send.approval"));
        Ok(())
    }

    #[test]
    fn continuation_merge_retains_completed_approval_and_new_agent_answer() -> Result<(), String> {
        let mut accumulated = ResolutionAnswers::from_lanes(
            JsonObject::new(),
            [("action.approval".to_owned(), JsonValue::Bool(true))],
        )
        .map_err(|error| error.to_string())?;
        accumulated
            .merge(ResolutionAnswers::agent(JsonObject::from([(
                "delegate.result".to_owned(),
                JsonValue::String("completed".to_owned()),
            )])))
            .map_err(|error| error.to_string())?;

        assert!(accumulated.is_human_approval("action.approval"));
        assert_eq!(
            accumulated.get("delegate.result"),
            Some(&JsonValue::String("completed".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn continuation_merge_rejects_authority_lane_changes() -> Result<(), String> {
        let mut accumulated = ResolutionAnswers::from_lanes(
            JsonObject::new(),
            [("action.approval".to_owned(), JsonValue::Bool(true))],
        )
        .map_err(|error| error.to_string())?;
        let Err(error) = accumulated.merge(ResolutionAnswers::agent(JsonObject::from([(
            "action.approval".to_owned(),
            JsonValue::Bool(true),
        )]))) else {
            return Err("an agent answer replaced a human approval".to_owned());
        };

        assert!(error.to_string().contains("authority lane"));
        Ok(())
    }
}
