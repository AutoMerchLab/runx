use std::collections::BTreeMap;

use runx_contracts::{JsonObject, JsonValue};
use serde::{Deserialize, Serialize};

use crate::CapabilityOutput;

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct EvidenceIndexOutput {
    source_index: SourceIndex,
}

impl CapabilityOutput for EvidenceIndexOutput {}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct SourceIndex {
    decision: String,
    objective: String,
    sources: Vec<IndexedSource>,
    source_digests: Vec<String>,
    source_evidence: Vec<SourceEvidence>,
    index_digest: String,
    blockers: Vec<String>,
    limits: IndexLimits,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct IndexedSource {
    source_digest: String,
    provider_content_digest: String,
    final_url: String,
    status: u64,
    extracted: String,
    provenance: SourceProvenance,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct SourceEvidence {
    evidence_digest: String,
    provider_content_digest: String,
    final_url: String,
    provenance: SourceProvenance,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct SourceProvenance {
    fetched_at: String,
    bytes: JsonValue,
    truncated: bool,
    redirects: Vec<JsonValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct IndexLimits {
    max_sources: u64,
    max_source_characters: u64,
    max_total_characters: u64,
    supplied_sources: u64,
    indexed_sources: u64,
    indexed_characters: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
pub(super) struct EvidenceVerifyOutput {
    verification: Verification,
    #[serde(flatten)]
    artifacts: BTreeMap<String, JsonObject>,
}

impl CapabilityOutput for EvidenceVerifyOutput {}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct Verification {
    status: String,
    findings: Vec<Finding>,
    admitted_source_digests: Vec<String>,
    admitted_context_digests: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, runx_contracts::schema::RunxSchema)]
#[serde(deny_unknown_fields)]
struct Finding {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}
