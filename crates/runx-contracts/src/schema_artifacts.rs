//! Published JSON Schema artifact manifest.
//!
//! This is the authoritative Rust-side list consumed by the schema generation
//! gate. Keep filenames aligned with `oss/schemas/*.json`.
use serde_json as schema_json;

use crate::schema::RunxSchema;
use crate::{
    Act, ActAssignment, ActResultEnvelope, AgentActInvocation, AgentContextEnvelope, ApprovalGate,
    Artifact, Authority, AuthorityProof, AuthoritySubsetProof, CredentialDeliveryObservation,
    CredentialDeliveryProfile, CredentialDeliveryRequest, CredentialDeliveryResponse,
    CredentialEnvelope, Decision, DevReport, DoctorReport, EffectFinalityReceipt,
    ExternalAdapterCancellationFrame, ExternalAdapterCredentialRequest,
    ExternalAdapterHostResolutionFrame, ExternalAdapterInvocation, ExternalAdapterManifest,
    ExternalAdapterResponse, Fixture, HandoffSignal, HandoffState, LedgerEntry, OperationalPolicy,
    OperationalProposal, Output, PacketIndex, Question, Receipt, Redaction, Reference,
    ReferenceLink, RegistryBinding, ResolutionRequest, ResolutionResponse, ReviewReceiptOutput,
    RunSummary, RunxListReport, ScopeAdmission, Signal, SkillApplyResult,
    SkillArchitectureDecision, SkillArchitecturePlan, SkillChangeBundle, SkillChangeDraft,
    SkillValidationResult, SourcePacket, SuppressionRecord, ThreadOutboxProviderFetch,
    ThreadOutboxProviderManifest, ThreadOutboxProviderObservation, ThreadOutboxProviderPush,
    ToolManifest, Verification,
};

#[derive(Clone, Debug, PartialEq)]
pub struct SchemaArtifact {
    pub file_name: &'static str,
    pub schema: schema_json::Value,
}

#[must_use]
// Function rationale: the artifact manifest is deliberately one
// ordered list so the published schema set is auditable against `oss/schemas`.
pub fn generated_schema_artifacts() -> Vec<SchemaArtifact> {
    vec![
        artifact::<Output>("output.schema.json"),
        artifact::<AgentContextEnvelope>("agent-context-envelope.schema.json"),
        artifact::<AgentActInvocation>("agent-act-invocation.schema.json"),
        artifact::<Question>("question.schema.json"),
        artifact::<ApprovalGate>("approval-gate.schema.json"),
        artifact::<ResolutionRequest>("resolution-request.schema.json"),
        artifact::<ResolutionResponse>("resolution-response.schema.json"),
        artifact::<ActResultEnvelope>("act-result.schema.json"),
        artifact::<CredentialEnvelope>("credential-envelope.schema.json"),
        artifact::<ScopeAdmission>("scope-admission.schema.json"),
        artifact::<AuthorityProof>("authority-proof.schema.json"),
        artifact::<CredentialDeliveryProfile>("credential-delivery-profile.schema.json"),
        artifact::<CredentialDeliveryRequest>("credential-delivery-request.schema.json"),
        artifact::<CredentialDeliveryResponse>("credential-delivery-response.schema.json"),
        artifact::<CredentialDeliveryObservation>("credential-delivery-observation.schema.json"),
        artifact::<ThreadOutboxProviderManifest>("thread-outbox-provider-manifest.schema.json"),
        artifact::<ThreadOutboxProviderPush>("thread-outbox-provider-push.schema.json"),
        artifact::<ThreadOutboxProviderFetch>("thread-outbox-provider-fetch.schema.json"),
        artifact::<ThreadOutboxProviderObservation>(
            "thread-outbox-provider-observation.schema.json",
        ),
        artifact::<DoctorReport>("doctor.schema.json"),
        artifact::<DevReport>("dev.schema.json"),
        artifact::<RunxListReport>("list.schema.json"),
        artifact::<RunSummary>("run-summary.schema.json"),
        artifact::<SkillArchitectureDecision>("skill-architecture-decision.schema.json"),
        artifact::<SkillArchitecturePlan>("skill-architecture-plan.schema.json"),
        artifact::<SkillChangeDraft>("skill-change-draft.schema.json"),
        artifact::<SkillChangeBundle>("skill-change-bundle.schema.json"),
        artifact::<SkillValidationResult>("skill-validation-result.schema.json"),
        artifact::<SkillApplyResult>("skill-apply-result.schema.json"),
        artifact::<Fixture>("fixture.schema.json"),
        artifact::<ToolManifest>("tool-manifest.schema.json"),
        artifact::<PacketIndex>("packet-index.schema.json"),
        artifact::<ActAssignment>("act-assignment.schema.json"),
        artifact::<ExternalAdapterManifest>("external-adapter-manifest.schema.json"),
        artifact::<ExternalAdapterInvocation>("external-adapter-invocation.schema.json"),
        artifact::<ExternalAdapterResponse>("external-adapter-response.schema.json"),
        artifact::<ExternalAdapterHostResolutionFrame>(
            "external-adapter-host-resolution.schema.json",
        ),
        artifact::<ExternalAdapterCancellationFrame>("external-adapter-cancellation.schema.json"),
        artifact::<ExternalAdapterCredentialRequest>(
            "external-adapter-credential-request.schema.json",
        ),
        artifact::<Reference>("reference.schema.json"),
        artifact::<ReferenceLink>("reference-link.schema.json"),
        artifact::<Authority>("authority.schema.json"),
        artifact::<AuthoritySubsetProof>("authority-subset-proof.schema.json"),
        artifact::<Signal>("signal.schema.json"),
        artifact::<SourcePacket>("source-packet.schema.json"),
        artifact::<Decision>("decision.schema.json"),
        artifact::<Act>("act.schema.json"),
        artifact::<Verification>("verification.schema.json"),
        artifact::<Receipt>("receipt.schema.json"),
        artifact::<EffectFinalityReceipt>("effect-finality-receipt.schema.json"),
        artifact::<Artifact>("artifact.schema.json"),
        artifact::<Redaction>("redaction.schema.json"),
        artifact::<LedgerEntry>("ledger-entry.schema.json"),
        artifact::<HandoffSignal>("handoff-signal.schema.json"),
        artifact::<HandoffState>("handoff-state.schema.json"),
        artifact::<SuppressionRecord>("suppression-record.schema.json"),
        artifact::<OperationalPolicy>("operational-policy.schema.json"),
        artifact::<OperationalProposal>("operational-proposal.schema.json"),
        artifact::<RegistryBinding>("registry-binding.schema.json"),
        artifact::<ReviewReceiptOutput>("review-receipt-output.schema.json"),
    ]
}

fn artifact<T: RunxSchema>(file_name: &'static str) -> SchemaArtifact {
    SchemaArtifact {
        file_name,
        schema: T::json_schema(),
    }
}
