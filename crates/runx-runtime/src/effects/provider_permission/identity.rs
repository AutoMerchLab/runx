use std::collections::BTreeMap;

use runx_contracts::JsonObject;
#[cfg(feature = "catalog")]
use runx_contracts::JsonValue;

use crate::effects::{EffectStepRequest, RuntimeEffectError};
#[cfg(feature = "catalog")]
use crate::{
    AuthenticatedHostedApiEnvironment, HostedApiEnvironment, HostedProviderGrant,
    hosted_private_network_allowed, list_provider_grants,
};

#[cfg(feature = "catalog")]
use super::PROVIDER_PERMISSION_EFFECT_FAMILY;
#[cfg(not(feature = "catalog"))]
use super::approval::required_provider_input;
#[cfg(feature = "catalog")]
use super::policy::{missing_scopes, required_verb_field};
use super::policy::{provider_permission_policy_error, required_scopes_for};
use super::{
    PROVIDER_PERMISSION_GRANT_ID_ENV, PROVIDER_PERMISSION_GRANTED_SCOPES_ENV,
    PROVIDER_PERMISSION_PRINCIPAL_REF_ENV, ProviderPermissionEffect,
};

pub(super) struct NativeProviderResolution {
    pub(super) env: BTreeMap<String, String>,
    pub(super) principal_ref: String,
}

fn explicit_native_provider_resolution(
    env: &BTreeMap<String, String>,
) -> Option<NativeProviderResolution> {
    let grant_id = env
        .get(PROVIDER_PERMISSION_GRANT_ID_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let granted_scopes = env
        .get(PROVIDER_PERMISSION_GRANTED_SCOPES_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let principal_ref = env
        .get(PROVIDER_PERMISSION_PRINCIPAL_REF_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    (grant_id.is_some() && granted_scopes.is_some()).then_some(NativeProviderResolution {
        env: env.clone(),
        principal_ref: principal_ref?.to_owned(),
    })
}

#[cfg(not(feature = "catalog"))]
impl ProviderPermissionEffect {
    pub(super) fn native_provider_env(
        &self,
        request: &EffectStepRequest<'_>,
        policy: &JsonObject,
    ) -> Result<NativeProviderResolution, RuntimeEffectError> {
        let _ = required_provider_input(request.inputs, "expected_provider")?;
        let _ = required_scopes_for(request, policy)?;
        explicit_native_provider_resolution(request.env).ok_or_else(|| {
            provider_permission_policy_error(format!(
                "native provider tools require explicit {PROVIDER_PERMISSION_GRANT_ID_ENV}, {PROVIDER_PERMISSION_GRANTED_SCOPES_ENV}, and {PROVIDER_PERMISSION_PRINCIPAL_REF_ENV} without the hosted provider feature"
            ))
        })
    }
}

#[cfg(feature = "catalog")]
impl ProviderPermissionEffect {
    pub(super) fn native_provider_env(
        &self,
        request: &EffectStepRequest<'_>,
        policy: &JsonObject,
    ) -> Result<NativeProviderResolution, RuntimeEffectError> {
        let provider = required_expected_provider(request)?;
        let required_scopes = required_scopes_for(request, policy)?;
        let env = request.env.clone();
        if let Some(resolved) = explicit_native_provider_resolution(&env) {
            return Ok(resolved);
        }
        self.resolve_hosted_provider_env(request, policy, provider, required_scopes, env)
    }

    fn resolve_hosted_provider_env(
        &self,
        request: &EffectStepRequest<'_>,
        policy: &JsonObject,
        provider: &str,
        required_scopes: Vec<String>,
        mut env: BTreeMap<String, String>,
    ) -> Result<NativeProviderResolution, RuntimeEffectError> {
        let explicit_grant = env
            .get(PROVIDER_PERMISSION_GRANT_ID_ENV)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let explicit_scopes = env
            .get(PROVIDER_PERMISSION_GRANTED_SCOPES_ENV)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());
        let transport = self
            .http_transport(hosted_private_network_allowed(false, &env))
            .map_err(|error| provider_permission_policy_error(error.to_string()))?;
        let resolved = HostedApiEnvironment::resolve(None, None, &env, request.graph_dir)
            .map_err(|error| provider_permission_policy_error(error.to_string()))?;
        let environment = self
            .authenticated_environment(&resolved, transport.as_ref())
            .map_err(|error| provider_permission_policy_error(error.to_string()))?;
        let principal_ref = format!("runx:principal:{}", environment.principal_id());
        if explicit_grant.is_some() && explicit_scopes.is_some() {
            return Ok(NativeProviderResolution { env, principal_ref });
        }
        let grants = self
            .hosted_grants(&resolved, &environment, transport.as_ref())
            .map_err(|error| provider_permission_policy_error(error.to_string()))?;
        let verb = required_verb_field(policy)?;
        let grant = select_hosted_provider_grant(
            &grants,
            provider,
            &required_scopes,
            explicit_grant.as_deref(),
        )
        .map_err(|message| RuntimeEffectError::Denied {
            family: PROVIDER_PERMISSION_EFFECT_FAMILY.to_owned(),
            verb,
            message,
        })?;
        env.insert(
            PROVIDER_PERMISSION_GRANT_ID_ENV.to_owned(),
            grant.grant_id.clone(),
        );
        env.insert(
            PROVIDER_PERMISSION_GRANTED_SCOPES_ENV.to_owned(),
            grant.scopes.join(","),
        );
        Ok(NativeProviderResolution { env, principal_ref })
    }

    fn hosted_grants<T: crate::http::RuntimeHttpTransport + ?Sized>(
        &self,
        resolved: &HostedApiEnvironment,
        environment: &AuthenticatedHostedApiEnvironment,
        transport: &T,
    ) -> Result<Vec<HostedProviderGrant>, crate::ProviderOperationError> {
        let mut cached = self
            .hosted_grants
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached_environment, grants)) = cached.as_ref()
            && cached_environment == resolved
        {
            return Ok(grants.clone());
        }
        let grants = list_provider_grants(transport, environment)?;
        *cached = Some((resolved.clone(), grants.clone()));
        Ok(grants)
    }

    pub(super) fn authenticated_environment<T: crate::http::RuntimeHttpTransport + ?Sized>(
        &self,
        resolved: &HostedApiEnvironment,
        transport: &T,
    ) -> Result<AuthenticatedHostedApiEnvironment, crate::HostedApiError> {
        let mut cached = self
            .authenticated_environment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached_environment, authenticated)) = cached.as_ref()
            && cached_environment == resolved
        {
            return Ok(authenticated.clone());
        }
        let authenticated = resolved.authenticate(transport)?;
        *cached = Some((resolved.clone(), authenticated.clone()));
        Ok(authenticated)
    }
}

#[cfg(feature = "catalog")]
fn required_expected_provider<'a>(
    request: &EffectStepRequest<'a>,
) -> Result<&'a str, RuntimeEffectError> {
    request
        .inputs
        .get("expected_provider")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            provider_permission_policy_error(
                "native provider tools require expected_provider".to_owned(),
            )
        })
}

#[cfg(feature = "catalog")]
pub(super) fn select_hosted_provider_grant<'a>(
    grants: &'a [HostedProviderGrant],
    provider: &str,
    required_scopes: &[String],
    explicit_grant: Option<&str>,
) -> Result<&'a HostedProviderGrant, String> {
    let mut candidates = grants
        .iter()
        .filter(|grant| grant.status == "active")
        .filter(|grant| grant.provider == provider)
        .filter(|grant| explicit_grant.is_none_or(|expected| grant.grant_id == expected))
        .filter(|grant| missing_scopes(required_scopes, &grant.scopes).is_empty())
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.grant_id.cmp(&right.grant_id));
    match candidates.as_slice() {
        [grant] => Ok(*grant),
        [] if explicit_grant.is_some() => Err(format!(
            "configured provider grant does not authorize {provider} scopes [{}]",
            required_scopes.join(", ")
        )),
        [] => Err(format!(
            "no active Runx Connect grant authorizes {provider} scopes [{}]",
            required_scopes.join(", ")
        )),
        _ => Err(format!(
            "multiple active Runx Connect grants authorize {provider} scopes [{}]; select one with {PROVIDER_PERMISSION_GRANT_ID_ENV}",
            required_scopes.join(", ")
        )),
    }
}
