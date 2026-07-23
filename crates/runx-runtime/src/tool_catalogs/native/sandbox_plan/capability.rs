use runx_contracts::{JsonNumber, JsonObject, JsonValue};
use serde::{Deserialize, Serialize};

use crate::{
    CapabilityAdmission, CapabilityApproval, CapabilityArtifacts, CapabilityDefinition,
    CapabilityEffect, CapabilityField, CapabilityInput,
};

use super::super::capability::{NativeCapability, TypedNativeCapability};
use super::output::SandboxPlanOutput;

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SandboxPlanInput {
    pub(super) workload: JsonObject,
    pub(super) as_of: String,
    pub(super) max_age_days: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) baseline: Option<JsonObject>,
}

impl CapabilityInput for SandboxPlanInput {
    fn defaults() -> JsonObject {
        JsonObject::from([(
            "max_age_days".to_owned(),
            JsonValue::Number(JsonNumber::U64(30)),
        )])
    }
}

const FIELDS: &[CapabilityField] = &[
    CapabilityField {
        name: "workload",
        description: "Immutable workload identity and source-bound runtime requirements.",
    },
    CapabilityField {
        name: "as_of",
        description: "Explicit RFC3339 evaluation time for requirements freshness.",
    },
    CapabilityField {
        name: "max_age_days",
        description: "Maximum accepted requirements age in days.",
    },
    CapabilityField {
        name: "baseline",
        description: "Optional network, writable-path, environment, and enforcement ceilings.",
    },
];

static PLAN: TypedNativeCapability<SandboxPlanInput, SandboxPlanOutput> =
    TypedNativeCapability::new(
        CapabilityDefinition {
            id: "sandbox.plan",
            owner: "runx-runtime/sandbox",
            summary: "Compile workload requirements into the narrowest admissible sandbox declaration.",
            scopes: &[],
            effect: CapabilityEffect::Read,
            approval: CapabilityApproval::None,
            artifacts: CapabilityArtifacts::Named {
                output: "hardening_profile",
                packet: "runx.hardening.v1",
            },
            admission: CapabilityAdmission::RuntimeInvariant(
                "sandbox declarations must be compiled by the admission policy that enforces them",
            ),
            fields: FIELDS,
        },
        super::prepare,
    );

pub(in crate::tool_catalogs::native) const CAPABILITIES: &[&dyn NativeCapability] = &[&PLAN];
