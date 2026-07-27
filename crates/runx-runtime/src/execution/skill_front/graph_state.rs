use super::graph::GraphSkillRunState;
use super::{GRAPH_SKILL_STATE_SCHEMA, SkillRunError, identifier_segment, invalid};

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use runx_contracts::{JsonObject, JsonValue};

use crate::RuntimeError;
use crate::execution::orchestrator::SkillRunRequest;
use crate::services::{ReceiptServices, WorkspaceEnv};

#[derive(Default)]
pub(super) struct GraphResolutionAnswers {
    values: JsonObject,
    human_approvals: BTreeSet<String>,
}

impl GraphResolutionAnswers {
    pub(super) fn agent(values: JsonObject) -> Self {
        Self {
            values,
            human_approvals: BTreeSet::new(),
        }
    }

    pub(super) fn get(&self, request_id: &str) -> Option<&JsonValue> {
        self.values.get(request_id)
    }

    pub(super) fn is_human_approval(&self, request_id: &str) -> bool {
        self.human_approvals.contains(request_id)
    }
}

pub(super) fn read_answers(path: &Path) -> Result<GraphResolutionAnswers, SkillRunError> {
    let raw = fs::read_to_string(path)
        .map_err(|source| RuntimeError::io(format!("reading {}", path.display()), source))?;
    let value = serde_json::from_str::<JsonValue>(&raw).map_err(|source| {
        RuntimeError::json(format!("parsing answers file {}", path.display()), source)
    })?;
    let answers = match value {
        JsonValue::Object(object) => normalize_answers(object)?,
        _ => return Err(invalid("answers file must be a JSON object")),
    };
    Ok(answers)
}

fn normalize_answers(mut object: JsonObject) -> Result<GraphResolutionAnswers, SkillRunError> {
    let nested_shape = object.contains_key("answers") || object.contains_key("approvals");
    if !nested_shape {
        return Ok(GraphResolutionAnswers::agent(object));
    }
    let extra = object
        .keys()
        .filter(|key| !matches!(key.as_str(), "answers" | "approvals"))
        .cloned()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(invalid(format!(
            "answers file mixes top-level keys [{}] with the nested answers/approvals shape",
            extra.join(", ")
        )));
    }
    let mut answers = match object.remove("answers") {
        Some(JsonValue::Object(nested)) => nested,
        Some(_) => return Err(invalid("answers field must be a JSON object")),
        None => JsonObject::new(),
    };
    let approvals = match object.remove("approvals") {
        Some(JsonValue::Object(approvals)) => approvals,
        Some(_) => return Err(invalid("approvals field must be a JSON object")),
        None => JsonObject::new(),
    };
    let mut human_approvals = BTreeSet::new();
    for (gate_id, decision) in approvals {
        if !is_human_approval_payload(&decision) {
            return Err(invalid(format!(
                "approvals.{gate_id} must be a boolean or {{approved: boolean, reason?: string}}"
            )));
        }
        if answers.insert(gate_id.clone(), decision).is_some() {
            return Err(invalid(format!(
                "request {gate_id} is declared in both answers and approvals"
            )));
        }
        human_approvals.insert(gate_id);
    }
    Ok(GraphResolutionAnswers {
        values: answers,
        human_approvals,
    })
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

fn graph_state_path(
    request: &SkillRunRequest,
    workspace: &WorkspaceEnv,
    receipts: &ReceiptServices,
    run_id: &str,
) -> PathBuf {
    let receipt_path = receipts.resolve_path(workspace, request.receipt_dir.as_deref(), None);
    receipt_path
        .path
        .join("runs")
        .join(format!("{}.graph-state.json", identifier_segment(run_id)))
}

pub(super) fn write_graph_state(
    request: &SkillRunRequest,
    workspace: &WorkspaceEnv,
    receipts: &ReceiptServices,
    run_id: &str,
    state: &GraphSkillRunState,
) -> Result<(), SkillRunError> {
    let path = graph_state_path(request, workspace, receipts, run_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| RuntimeError::io(format!("creating {}", parent.display()), source))?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|source| RuntimeError::json("serializing graph state", source))?;
    let temp_path = graph_state_temp_path(&path);
    fs::write(&temp_path, bytes)
        .map_err(|source| RuntimeError::io(format!("writing {}", temp_path.display()), source))?;
    fs::rename(&temp_path, &path).map_err(|source| {
        let _ignored = fs::remove_file(&temp_path);
        RuntimeError::io(
            format!("replacing {} with {}", path.display(), temp_path.display()),
            source,
        )
    })?;
    Ok(())
}

pub(super) fn read_graph_state(
    request: &SkillRunRequest,
    workspace: &WorkspaceEnv,
    receipts: &ReceiptServices,
    run_id: &str,
    runner_name: &str,
    package_digest: &str,
    execution_closure_digest: &str,
) -> Result<GraphSkillRunState, SkillRunError> {
    let path = graph_state_path(request, workspace, receipts, run_id);
    let raw = fs::read_to_string(&path)
        .map_err(|source| RuntimeError::io(format!("reading {}", path.display()), source))?;
    let state: GraphSkillRunState = serde_json::from_str(&raw).map_err(|source| {
        invalid(format!(
            "graph state file {} is malformed; the run cannot resume safely without a valid checkpoint: {source}",
            path.display()
        ))
    })?;
    if state.schema != GRAPH_SKILL_STATE_SCHEMA {
        return Err(invalid(format!(
            "graph state schema mismatch for run {run_id}: expected {GRAPH_SKILL_STATE_SCHEMA}, got {}",
            state.schema
        )));
    }
    if state.run_id != run_id {
        return Err(invalid(format!(
            "graph state run_id mismatch: expected {run_id}, got {}",
            state.run_id
        )));
    }
    if state.runner_name != runner_name {
        return Err(invalid(format!(
            "graph state runner_name mismatch for run {run_id}: expected {runner_name}, got {}",
            state.runner_name
        )));
    }
    if state.package_digest != package_digest {
        return Err(invalid(format!(
            "graph state package_digest mismatch for run {run_id}: expected {package_digest}, got {}",
            state.package_digest
        )));
    }
    if state.execution_closure_digest != execution_closure_digest {
        return Err(invalid(format!(
            "graph state execution_closure_digest mismatch for run {run_id}: expected {execution_closure_digest}, got {}",
            state.execution_closure_digest
        )));
    }
    Ok(state)
}

fn graph_state_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("graph-state.json");
    path.with_file_name(format!("{file_name}.{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::normalize_answers;
    use runx_contracts::{JsonObject, JsonValue};

    #[test]
    fn nested_approvals_retain_authenticated_human_provenance() -> Result<(), String> {
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
}
