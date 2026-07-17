use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use runx_contracts::{AuthorityVerb, JsonObject, Receipt, Reference};
use runx_core::state_machine::AuthorityAdmissionWitness;
use runx_parser::GraphStep;

use crate::adapter::SkillOutput;

use super::RuntimeEffectError;

pub trait RuntimeEffect: Send + Sync {
    fn family(&self) -> &'static str;

    fn can_run_parallel(&self, step: &GraphStep) -> bool {
        let _ = step;
        true
    }

    fn find_replay(
        &self,
        request: EffectStepRequest<'_>,
    ) -> Result<Option<EffectReplay>, RuntimeEffectError> {
        let _ = request;
        Ok(None)
    }

    fn recover_pending(&self, request: EffectStepRequest<'_>) -> Result<(), RuntimeEffectError> {
        let _ = request;
        Ok(())
    }

    fn admit(
        &self,
        request: EffectStepRequest<'_>,
    ) -> Result<Option<EffectAdmission>, RuntimeEffectError> {
        let _ = request;
        Ok(None)
    }

    fn prepare_output(&self, request: EffectOutputRequest<'_>) -> Result<(), RuntimeEffectError> {
        let _ = request;
        Ok(())
    }

    fn finalize_output(&self, request: EffectReceiptRequest<'_>) -> Result<(), RuntimeEffectError> {
        let _ = request;
        Ok(())
    }

    fn persist(&self, request: EffectReceiptRequest<'_>) -> Result<(), RuntimeEffectError> {
        let _ = request;
        Ok(())
    }

    fn prepare_replay_output(
        &self,
        request: EffectReplayOutputRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        let _ = request;
        Ok(())
    }

    fn validate_replay(
        &self,
        request: EffectReplayReceiptRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        let _ = request;
        Ok(())
    }

    fn authority_grant_refs(
        &self,
        admission: &EffectAdmission,
    ) -> Result<Vec<Reference>, RuntimeEffectError> {
        let _ = admission;
        Ok(Vec::new())
    }

    fn authority_scope_refs(
        &self,
        admission: &EffectAdmission,
    ) -> Result<Vec<Reference>, RuntimeEffectError> {
        let _ = admission;
        Ok(Vec::new())
    }

    fn replay_authority_grant_refs(
        &self,
        replay: &EffectReplay,
    ) -> Result<Vec<Reference>, RuntimeEffectError> {
        let _ = replay;
        Ok(Vec::new())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EffectStepRequest<'a> {
    pub step: &'a GraphStep,
    pub inputs: &'a JsonObject,
    pub env: &'a BTreeMap<String, String>,
    pub graph_dir: &'a Path,
}

pub struct EffectOutputRequest<'a> {
    pub step: &'a GraphStep,
    pub admission: &'a EffectAdmission,
    pub claim: &'a JsonObject,
    pub output: &'a mut SkillOutput,
}

pub struct EffectReceiptRequest<'a> {
    pub step: &'a GraphStep,
    pub graph_dir: &'a Path,
    pub admission: &'a EffectAdmission,
    pub claim: &'a JsonObject,
    pub output: &'a mut SkillOutput,
    pub receipt: &'a Receipt,
    pub env: &'a BTreeMap<String, String>,
}

pub struct EffectReplayOutputRequest<'a> {
    pub step: &'a GraphStep,
    pub replay: &'a EffectReplay,
    pub output: &'a mut SkillOutput,
}

pub struct EffectReplayReceiptRequest<'a> {
    pub step: &'a GraphStep,
    pub replay: &'a EffectReplay,
    pub receipt: &'a Receipt,
    pub output: &'a SkillOutput,
    pub claim: &'a JsonObject,
}

#[derive(Clone)]
pub struct EffectAdmission {
    family: &'static str,
    verb: AuthorityVerb,
    witness: AuthorityAdmissionWitness,
    context: Arc<dyn Any + Send + Sync>,
}

impl EffectAdmission {
    #[must_use]
    pub fn new<T>(
        family: &'static str,
        verb: AuthorityVerb,
        witness: AuthorityAdmissionWitness,
        context: T,
    ) -> Self
    where
        T: Any + Send + Sync + 'static,
    {
        Self {
            family,
            verb,
            witness,
            context: Arc::new(context),
        }
    }

    #[must_use]
    pub fn family(&self) -> &'static str {
        self.family
    }

    #[must_use]
    pub fn verb(&self) -> AuthorityVerb {
        self.verb.clone()
    }

    #[must_use]
    pub fn witness(&self) -> &AuthorityAdmissionWitness {
        &self.witness
    }

    #[must_use]
    pub fn context<T: Any>(&self) -> Option<&T> {
        self.context.as_ref().downcast_ref::<T>()
    }
}

impl fmt::Debug for EffectAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectAdmission")
            .field("family", &self.family)
            .field("verb", &self.verb)
            .field("witness", &self.witness)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct EffectReplay {
    family: &'static str,
    receipt_ref: String,
    receipt_created_at: String,
    receipt_digest: String,
    outputs: JsonObject,
    context: Arc<dyn Any + Send + Sync>,
}

impl EffectReplay {
    #[must_use]
    pub fn new<T>(
        family: &'static str,
        receipt_ref: impl Into<String>,
        receipt_created_at: impl Into<String>,
        receipt_digest: impl Into<String>,
        outputs: JsonObject,
        context: T,
    ) -> Self
    where
        T: Any + Send + Sync + 'static,
    {
        Self {
            family,
            receipt_ref: receipt_ref.into(),
            receipt_created_at: receipt_created_at.into(),
            receipt_digest: receipt_digest.into(),
            outputs,
            context: Arc::new(context),
        }
    }

    #[must_use]
    pub fn family(&self) -> &'static str {
        self.family
    }

    #[must_use]
    pub fn receipt_ref(&self) -> &str {
        &self.receipt_ref
    }

    #[must_use]
    pub fn receipt_created_at(&self) -> &str {
        &self.receipt_created_at
    }

    #[must_use]
    pub fn receipt_digest(&self) -> &str {
        &self.receipt_digest
    }

    #[must_use]
    pub fn outputs(&self) -> &JsonObject {
        &self.outputs
    }

    #[must_use]
    pub fn context<T: Any>(&self) -> Option<&T> {
        self.context.as_ref().downcast_ref::<T>()
    }
}

impl fmt::Debug for EffectReplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectReplay")
            .field("family", &self.family)
            .field("receipt_ref", &self.receipt_ref)
            .field("receipt_created_at", &self.receipt_created_at)
            .field("receipt_digest", &self.receipt_digest)
            .finish_non_exhaustive()
    }
}
