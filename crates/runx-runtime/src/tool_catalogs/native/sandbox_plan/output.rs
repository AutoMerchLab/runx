use runx_contracts::JsonValue;
use serde::{Deserialize, Serialize};

use crate::CapabilityOutput;

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SandboxPlanOutput {
    pub(super) hardening_profile: HardeningProfile,
}

impl CapabilityOutput for SandboxPlanOutput {}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct HardeningProfile {
    schema: String,
    decision: String,
    workload: Workload,
    declaration: Declaration,
    source_evidence: SourceEvidence,
    admission: Admission,
    unsupported_controls: Vec<String>,
    residual_risk: ResidualRisk,
    applied: bool,
    validation: Validation,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct Workload {
    ref_form: String,
    image_digest: String,
    skill_ref: String,
    class: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct Declaration {
    profile: String,
    cwd_policy: String,
    env_allowlist: Option<Vec<String>>,
    network: bool,
    writable_paths: Vec<String>,
    require_enforcement: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct SourceEvidence {
    source_ref: String,
    source_digest: String,
    observed_at: String,
    provenance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct Admission {
    status: String,
    reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct ResidualRisk {
    level: String,
    reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct Validation {
    status: String,
    findings: Vec<JsonValue>,
}
