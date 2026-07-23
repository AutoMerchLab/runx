//! Portable contracts for one digest-bound skill-authoring transaction.

use serde::{Deserialize, Serialize};

use crate::schema::{NonEmptyString, RunxSchema};

pub const SKILL_ARCHITECTURE_DECISION_SCHEMA: &str = "runx.skill.architecture_decision.v1";
pub const SKILL_ARCHITECTURE_PLAN_SCHEMA: &str = "runx.skill.architecture_plan.v1";
pub const SKILL_CHANGE_DRAFT_SCHEMA: &str = "runx.skill.change_draft.v1";
pub const SKILL_CHANGE_BUNDLE_SCHEMA: &str = "runx.skill.change_bundle.v1";
pub const SKILL_VALIDATION_RESULT_SCHEMA: &str = "runx.skill.validation_result.v1";
pub const SKILL_APPLY_RESULT_SCHEMA: &str = "runx.skill.apply_result.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
pub enum SkillArchitectureDecisionSchema {
    #[serde(rename = "runx.skill.architecture_decision.v1")]
    V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
pub enum SkillArchitecturePlanSchema {
    #[serde(rename = "runx.skill.architecture_plan.v1")]
    V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
pub enum SkillChangeBundleSchema {
    #[serde(rename = "runx.skill.change_bundle.v1")]
    V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
pub enum SkillChangeDraftSchema {
    #[serde(rename = "runx.skill.change_draft.v1")]
    V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
pub enum SkillValidationResultSchema {
    #[serde(rename = "runx.skill.validation_result.v1")]
    V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
pub enum SkillApplyResultSchema {
    #[serde(rename = "runx.skill.apply_result.v1")]
    V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillArchitectureDisposition {
    Build,
    ExtendExisting,
    NoSkill,
    NeedsCore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillExecutionLane {
    Manual,
    Graph,
    AgentTask,
    NativeCapability,
    DomainModule,
    CliTool,
    ProviderAdapter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillEffectClass {
    None,
    Read,
    Draft,
    Mutate,
    Financial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillApprovalRequirement {
    None,
    Policy,
    Human,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillProofKind {
    Contract,
    Harness,
    Regression,
    Security,
    Performance,
    OperatorTrial,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillKnowledgeContract {
    pub purpose: NonEmptyString,
    pub evidence_required: Vec<NonEmptyString>,
    pub decision_logic: Vec<NonEmptyString>,
    pub stop_conditions: Vec<NonEmptyString>,
    pub recovery: Vec<NonEmptyString>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillBehaviorDecision {
    pub id: NonEmptyString,
    pub outcome: NonEmptyString,
    pub lane: SkillExecutionLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_ref: Option<NonEmptyString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_module_justification: Option<NonEmptyString>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillNativeReuseEvidence {
    pub inspected_capabilities: Vec<NonEmptyString>,
    pub selected_capabilities: Vec<NonEmptyString>,
    pub missing_capabilities: Vec<NonEmptyString>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillEffectRequirement {
    pub effect: SkillEffectClass,
    pub authority_scopes: Vec<NonEmptyString>,
    pub approval: SkillApprovalRequirement,
    pub provider_boundary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillChainPlan {
    pub context_skills: Vec<NonEmptyString>,
    pub routes: Vec<NonEmptyString>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceBudget {
    pub max_files: u64,
    pub max_executable_lines: u64,
    pub max_fanout: u64,
    pub max_process_spawns: u64,
    pub network_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillProofRequirement {
    pub name: NonEmptyString,
    pub kind: SkillProofKind,
    pub expected: NonEmptyString,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
#[runx_schema(id = "runx.skill.architecture_decision.v1")]
pub struct SkillArchitectureDecision {
    pub schema: SkillArchitectureDecisionSchema,
    pub disposition: SkillArchitectureDisposition,
    pub objective: NonEmptyString,
    pub operator_value: NonEmptyString,
    pub knowledge_contract: SkillKnowledgeContract,
    pub required_behaviors: Vec<SkillBehaviorDecision>,
    pub native_reuse: SkillNativeReuseEvidence,
    pub effects: Vec<SkillEffectRequirement>,
    pub skill_chain: SkillChainPlan,
    pub resource_budget: SkillResourceBudget,
    pub preservation_obligations: Vec<NonEmptyString>,
    pub deletions: Vec<NonEmptyString>,
    pub proof_plan: Vec<SkillProofRequirement>,
}

/// The exact architecture decision admitted against one inspected package
/// state. `plan_digest` binds both fields so a change bundle cannot silently
/// reuse the decision after the target package changes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
#[runx_schema(id = "runx.skill.architecture_plan.v1")]
pub struct SkillArchitecturePlan {
    pub schema: SkillArchitecturePlanSchema,
    pub base_digest: NonEmptyString,
    pub plan_digest: NonEmptyString,
    pub architecture: SkillArchitectureDecision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillChangeDecision {
    Write,
    NoSkill,
    NoChange,
    NeedsCore,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillFileWrite {
    pub path: NonEmptyString,
    pub contents: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillExpectedOutput {
    pub name: NonEmptyString,
    pub value_type: NonEmptyString,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet: Option<NonEmptyString>,
}

/// Agent-authored package bytes and intent before the runtime binds them to an
/// inspected package state. This contract deliberately contains no digest:
/// agents never calculate or copy runtime integrity values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
#[runx_schema(id = "runx.skill.change_draft.v1")]
pub struct SkillChangeDraft {
    pub schema: SkillChangeDraftSchema,
    pub decision: SkillChangeDecision,
    pub summary: NonEmptyString,
    pub non_goals: Vec<NonEmptyString>,
    pub writes: Vec<SkillFileWrite>,
    pub deletes: Vec<NonEmptyString>,
    pub expected_outputs: Vec<SkillExpectedOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
#[runx_schema(id = "runx.skill.change_bundle.v1")]
pub struct SkillChangeBundle {
    pub schema: SkillChangeBundleSchema,
    pub decision: SkillChangeDecision,
    pub base_digest: NonEmptyString,
    pub plan_digest: NonEmptyString,
    pub architecture: SkillArchitectureDecision,
    pub summary: NonEmptyString,
    pub non_goals: Vec<NonEmptyString>,
    pub writes: Vec<SkillFileWrite>,
    pub deletes: Vec<NonEmptyString>,
    pub expected_outputs: Vec<SkillExpectedOutput>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillValidationCheckStatus {
    Passed,
    Skipped,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillValidationCheck {
    pub name: NonEmptyString,
    pub status: SkillValidationCheckStatus,
    pub detail: NonEmptyString,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillPackageMetrics {
    pub files: u64,
    pub bytes: u64,
    pub production_lines: u64,
    pub test_lines: u64,
    pub generated_lines: u64,
    pub executable_files: u64,
    pub executable_lines: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillPackageDelta {
    pub files: i64,
    pub bytes: i64,
    pub production_lines: i64,
    pub test_lines: i64,
    pub generated_lines: i64,
    pub executable_files: i64,
    pub executable_lines: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
#[runx_schema(id = "runx.skill.validation_result.v1")]
pub struct SkillValidationResult {
    pub schema: SkillValidationResultSchema,
    pub target_dir: NonEmptyString,
    pub base_digest: NonEmptyString,
    pub plan_digest: NonEmptyString,
    pub candidate_digest: NonEmptyString,
    pub checks: Vec<SkillValidationCheck>,
    pub before: SkillPackageMetrics,
    pub after: SkillPackageMetrics,
    pub delta: SkillPackageDelta,
    pub residual_risks: Vec<NonEmptyString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillApplyVerdict {
    Unchanged,
    NeedsCore,
    ValidatedAndApplied,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, RunxSchema)]
#[serde(deny_unknown_fields)]
#[runx_schema(id = "runx.skill.apply_result.v1")]
pub struct SkillApplyResult {
    pub schema: SkillApplyResultSchema,
    pub target_dir: NonEmptyString,
    pub decision: SkillChangeDecision,
    pub verdict: SkillApplyVerdict,
    pub base_digest: NonEmptyString,
    pub plan_digest: NonEmptyString,
    pub package_digest: NonEmptyString,
    pub changed_paths: Vec<NonEmptyString>,
    pub deleted_paths: Vec<NonEmptyString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<SkillValidationResult>,
    pub residual_risks: Vec<NonEmptyString>,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        SkillArchitectureDecision, SkillArchitecturePlan, SkillChangeBundle, SkillChangeDraft,
    };
    use crate::schema::RunxSchema;

    #[test]
    fn skill_authoring_contract_rejects_unknown_nested_fields() {
        let mut value = architecture_fixture();
        value["knowledge_contract"]["unowned"] = json!(true);
        assert!(serde_json::from_value::<SkillArchitectureDecision>(value).is_err());
    }

    #[test]
    fn skill_authoring_contract_binds_closed_bundle_and_plan() {
        let value = json!({
            "schema": "runx.skill.change_bundle.v1",
            "decision": "write",
            "base_digest": digest('a'),
            "plan_digest": digest('b'),
            "architecture": architecture_fixture(),
            "summary": "Create the bounded package.",
            "non_goals": ["Do not add a provider adapter."],
            "writes": [
                { "path": "SKILL.md", "contents": "---\nname: demo\n---\n" },
                { "path": "X.yaml", "contents": "skill: demo\n" }
            ],
            "deletes": [],
            "expected_outputs": [
                { "name": "decision", "value_type": "object", "packet": "demo.decision.v1" }
            ]
        });
        assert!(serde_json::from_value::<SkillChangeBundle>(value.clone()).is_ok());

        let mut unknown = value;
        unknown["writes"][0]["mode"] = json!("executable");
        assert!(serde_json::from_value::<SkillChangeBundle>(unknown).is_err());
    }

    #[test]
    fn skill_authoring_generated_objects_are_recursively_closed() {
        assert_closed_objects(&SkillArchitectureDecision::json_schema());
        assert_closed_objects(&SkillArchitecturePlan::json_schema());
        assert_closed_objects(&SkillChangeDraft::json_schema());
        assert_closed_objects(&SkillChangeBundle::json_schema());
    }

    fn architecture_fixture() -> Value {
        json!({
            "schema": "runx.skill.architecture_decision.v1",
            "disposition": "build",
            "objective": "Create a bounded decision skill.",
            "operator_value": "Turn supplied evidence into one reviewable decision.",
            "knowledge_contract": {
                "purpose": "Guide the operator through the bounded decision.",
                "evidence_required": ["A supplied objective."],
                "decision_logic": ["Preserve the objective exactly."],
                "stop_conditions": ["Stop when evidence is missing."],
                "recovery": ["Resume with the missing evidence."]
            },
            "required_behaviors": [{
                "id": "decide",
                "outcome": "Produce the decision packet.",
                "lane": "agent_task"
            }],
            "native_reuse": {
                "inspected_capabilities": ["runx.skill.inspect"],
                "selected_capabilities": [],
                "missing_capabilities": []
            },
            "effects": [{
                "effect": "none",
                "authority_scopes": [],
                "approval": "none",
                "provider_boundary": false
            }],
            "skill_chain": { "context_skills": [], "routes": [] },
            "resource_budget": {
                "max_files": 4,
                "max_executable_lines": 0,
                "max_fanout": 1,
                "max_process_spawns": 0,
                "network_allowed": false
            },
            "preservation_obligations": ["Keep the manual substantive."],
            "deletions": [],
            "proof_plan": [{
                "name": "bounded-harness",
                "kind": "harness",
                "expected": "The supplied-answer fixture seals."
            }]
        })
    }

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn assert_closed_objects(value: &Value) {
        match value {
            Value::Object(object) => {
                if object.get("type") == Some(&Value::String("object".to_owned())) {
                    assert_eq!(
                        object.get("additionalProperties"),
                        Some(&Value::Bool(false)),
                        "open object schema: {value}"
                    );
                }
                for child in object.values() {
                    assert_closed_objects(child);
                }
            }
            Value::Array(values) => values.iter().for_each(assert_closed_objects),
            _ => {}
        }
    }
}
