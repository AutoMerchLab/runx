use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use runx_contracts::Reference;
use runx_parser::GraphStep;

use super::{
    EffectAdmission, EffectOutputRequest, EffectReceiptRequest, EffectReplay,
    EffectReplayOutputRequest, EffectReplayReceiptRequest, EffectStepRequest, RuntimeEffect,
    RuntimeEffectError,
};

#[derive(Clone)]
pub struct RuntimeEffectRegistry {
    families: BTreeMap<&'static str, Arc<dyn RuntimeEffect>>,
}

impl RuntimeEffectRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            families: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_effect<T>(effect: T) -> Self
    where
        T: RuntimeEffect + 'static,
    {
        let mut registry = Self::empty();
        let family = effect.family();
        registry.families.insert(family, Arc::new(effect));
        registry
    }

    pub fn register_effect<T>(&mut self, effect: T) -> Result<(), RuntimeEffectError>
    where
        T: RuntimeEffect + 'static,
    {
        let family = effect.family();
        if self.families.contains_key(family) {
            return Err(RuntimeEffectError::DuplicateFamily {
                family: family.to_owned(),
            });
        }
        self.families.insert(family, Arc::new(effect));
        Ok(())
    }

    pub(crate) fn find_replay(
        &self,
        request: EffectStepRequest<'_>,
    ) -> Result<Option<EffectReplay>, RuntimeEffectError> {
        for effect in self.families.values() {
            if let Some(replay) = effect.find_replay(request)? {
                return Ok(Some(replay));
            }
        }
        Ok(None)
    }

    pub(crate) fn recover_pending(
        &self,
        request: EffectStepRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        for effect in self.families.values() {
            effect.recover_pending(request)?;
        }
        Ok(())
    }

    pub(crate) fn admit(
        &self,
        request: EffectStepRequest<'_>,
    ) -> Result<Option<EffectAdmission>, RuntimeEffectError> {
        for effect in self.families.values() {
            if let Some(admission) = effect.admit(request)? {
                return Ok(Some(admission));
            }
        }
        Ok(None)
    }

    pub(crate) fn prepare_output(
        &self,
        request: EffectOutputRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        let family = request.admission.family();
        self.require_effect(family)?.prepare_output(request)
    }

    pub(crate) fn finalize_output(
        &self,
        request: EffectReceiptRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        let family = request.admission.family();
        self.require_effect(family)?.finalize_output(request)
    }

    pub(crate) fn persist(
        &self,
        request: EffectReceiptRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        let family = request.admission.family();
        self.require_effect(family)?.persist(request)
    }

    pub(crate) fn prepare_replay_output(
        &self,
        request: EffectReplayOutputRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        let family = request.replay.family();
        self.require_effect(family)?.prepare_replay_output(request)
    }

    pub(crate) fn validate_replay(
        &self,
        request: EffectReplayReceiptRequest<'_>,
    ) -> Result<(), RuntimeEffectError> {
        let family = request.replay.family();
        self.require_effect(family)?.validate_replay(request)
    }

    pub(crate) fn authority_grant_refs(
        &self,
        admission: &EffectAdmission,
    ) -> Result<Vec<Reference>, RuntimeEffectError> {
        self.require_effect(admission.family())?
            .authority_grant_refs(admission)
    }

    pub(crate) fn authority_scope_refs(
        &self,
        admission: &EffectAdmission,
    ) -> Result<Vec<Reference>, RuntimeEffectError> {
        self.require_effect(admission.family())?
            .authority_scope_refs(admission)
    }

    pub(crate) fn replay_authority_grant_refs(
        &self,
        replay: &EffectReplay,
    ) -> Result<Vec<Reference>, RuntimeEffectError> {
        self.require_effect(replay.family())?
            .replay_authority_grant_refs(replay)
    }

    pub(crate) fn allows_parallel_step(&self, step: &GraphStep) -> bool {
        self.families
            .values()
            .all(|effect| effect.can_run_parallel(step))
    }

    fn require_effect(
        &self,
        family: &'static str,
    ) -> Result<&dyn RuntimeEffect, RuntimeEffectError> {
        self.families.get(family).map(Arc::as_ref).ok_or_else(|| {
            RuntimeEffectError::MissingFamily {
                family: family.to_owned(),
            }
        })
    }
}

impl Default for RuntimeEffectRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Debug for RuntimeEffectRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let families = self.families.keys().copied().collect::<Vec<_>>();
        formatter
            .debug_struct("RuntimeEffectRegistry")
            .field("families", &families)
            .finish()
    }
}
