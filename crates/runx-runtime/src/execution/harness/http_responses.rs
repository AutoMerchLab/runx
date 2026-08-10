use std::collections::BTreeMap;

use runx_parser::HarnessHttpResponseFixture;

use crate::effects::RuntimeEffectRegistry;
use crate::http::{RuntimeHttpHeader, RuntimeHttpResponse};

/// Attach exact fixture bytes to a cloned execution registry. No public
/// runtime input, environment variable, or provider configuration can create
/// this state; only the harness front calls this function.
pub(crate) fn effects_with_harness_http_responses(
    effects: &RuntimeEffectRegistry,
    fixtures: &BTreeMap<String, HarnessHttpResponseFixture>,
) -> RuntimeEffectRegistry {
    if fixtures.is_empty() {
        return effects.clone();
    }
    let responses = fixtures
        .iter()
        .map(|(url, fixture)| {
            let mut response = RuntimeHttpResponse::new(fixture.status, fixture.body.clone());
            response.headers = fixture
                .headers
                .iter()
                .map(|(name, value)| RuntimeHttpHeader::new(name, value))
                .collect();
            (url.clone(), response)
        })
        .collect();
    effects.clone().with_harness_http_responses(responses)
}
