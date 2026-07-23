use runx_contracts::{JsonObject, JsonValue};

use crate::hosted_api::{
    AuthenticatedHostedApiEnvironment, HostedConnectAction, execute_hosted_connect,
    request::send_json,
};
use crate::http::{HttpMethod, RuntimeHttpTransport as Transport};

mod error;

pub use error::ProviderOperationError;

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderOperationRequest {
    pub grant_id: String,
    pub operation: String,
    pub target: String,
    pub input: JsonObject,
    pub expected_access: Option<ProviderOperationAccess>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderOperationAccess {
    Read,
    Mutate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedProviderGrant {
    pub grant_id: String,
    pub provider: String,
    pub scopes: Vec<String>,
    pub status: String,
}

impl ProviderOperationAccess {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Mutate => "mutate",
        }
    }
}

pub fn list_provider_grants<T: Transport + ?Sized>(
    transport: &T,
    environment: &AuthenticatedHostedApiEnvironment,
) -> Result<Vec<HostedProviderGrant>, ProviderOperationError> {
    let response = execute_hosted_connect(transport, environment, HostedConnectAction::List)?;
    let response: JsonValue = serde_json::from_value(response).map_err(|error| {
        ProviderOperationError::InvalidResponse(format!(
            "grant response could not be projected: {error}"
        ))
    })?;
    let response = response.as_object().ok_or_else(|| {
        ProviderOperationError::InvalidResponse("grant response must be an object".to_owned())
    })?;
    if response.get("status").and_then(JsonValue::as_str) != Some("success") {
        return Err(ProviderOperationError::InvalidResponse(
            "grant response status is not success".to_owned(),
        ));
    }
    let grants = response
        .get("grants")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            ProviderOperationError::InvalidResponse(
                "grant response grants must be an array".to_owned(),
            )
        })?;
    grants
        .iter()
        .map(parse_provider_grant)
        .collect::<Result<Vec<_>, _>>()
}

fn parse_provider_grant(value: &JsonValue) -> Result<HostedProviderGrant, ProviderOperationError> {
    let grant = value.as_object().ok_or_else(|| {
        ProviderOperationError::InvalidResponse("provider grant must be an object".to_owned())
    })?;
    let grant_id = required_response_string(grant, "grant_id")?.to_owned();
    validate_provider_grant_id(&grant_id)?;
    let provider = required_response_string(grant, "provider")?.to_owned();
    let status = required_response_string(grant, "status")?.to_owned();
    let scopes = grant
        .get("scopes")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            ProviderOperationError::InvalidResponse(
                "provider grant scopes must be an array".to_owned(),
            )
        })?
        .iter()
        .map(|scope| {
            scope
                .as_str()
                .map(str::trim)
                .filter(|scope| !scope.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    ProviderOperationError::InvalidResponse(
                        "provider grant scopes must be non-empty strings".to_owned(),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HostedProviderGrant {
        grant_id,
        provider,
        scopes,
        status,
    })
}

pub fn invoke_provider_operation<T: Transport + ?Sized>(
    transport: &T,
    environment: &AuthenticatedHostedApiEnvironment,
    request: &ProviderOperationRequest,
) -> Result<JsonObject, ProviderOperationError> {
    validate_provider_grant_id(&request.grant_id)?;
    validate_provider_operation(&request.operation)?;
    let mut body = JsonObject::from([
        (
            "grant_id".to_owned(),
            JsonValue::String(request.grant_id.clone()),
        ),
        (
            "operation".to_owned(),
            JsonValue::String(request.operation.clone()),
        ),
        (
            "target".to_owned(),
            JsonValue::String(request.target.clone()),
        ),
        ("input".to_owned(), JsonValue::Object(request.input.clone())),
    ]);
    if let Some(access) = request.expected_access {
        body.insert(
            "access".to_owned(),
            JsonValue::String(access.as_str().to_owned()),
        );
    }
    let body = serde_json::to_string(&body).map_err(|error| {
        crate::hosted_api::HostedApiOperationError::InvalidRequest {
            operation: "provider operation request",
            message: error.to_string(),
        }
    })?;
    let response: JsonObject = send_json(
        transport,
        environment.base_url(),
        "provider operation",
        HttpMethod::Post,
        "/v1/provider-operations",
        Some(environment.token()),
        Some(body),
    )?;
    parse_provider_operation_response(response, request)
}

fn parse_provider_operation_response(
    response: JsonObject,
    request: &ProviderOperationRequest,
) -> Result<JsonObject, ProviderOperationError> {
    validate_operation_readback(&response, request)?;
    project_operation_readback(response)
}

fn validate_operation_readback(
    response: &JsonObject,
    request: &ProviderOperationRequest,
) -> Result<(), ProviderOperationError> {
    if response.get("status").and_then(JsonValue::as_str) != Some("success") {
        return Err(ProviderOperationError::InvalidResponse(
            "response status is not success".to_owned(),
        ));
    }
    required_response_string(response, "provider")?;
    let operation = required_response_string(response, "operation")?;
    if operation != request.operation {
        return Err(ProviderOperationError::InvalidResponse(format!(
            "response operation {operation:?} does not match requested operation {:?}",
            request.operation
        )));
    }
    let target = required_response_string(response, "target")?;
    if target != request.target {
        return Err(ProviderOperationError::InvalidResponse(format!(
            "response target {target:?} does not match requested target {:?}",
            request.target
        )));
    }
    let access = response.get("access").and_then(JsonValue::as_str);
    if let Some(expected) = request.expected_access
        && access != Some(expected.as_str())
    {
        return Err(ProviderOperationError::InvalidResponse(format!(
            "response access {access:?} does not match requested access {:?}",
            expected.as_str()
        )));
    }
    if response.get("result").is_none() {
        return Err(ProviderOperationError::InvalidResponse(
            "response result is missing".to_owned(),
        ));
    }
    required_response_string(response, "readback_ref")?;
    if request.expected_access == Some(ProviderOperationAccess::Mutate) {
        required_response_string(response, "operation_id")?;
        let expected_idempotency_key = required_response_string(&request.input, "idempotency_key")?;
        let actual_idempotency_key = required_response_string(response, "idempotency_key")?;
        if actual_idempotency_key != expected_idempotency_key {
            return Err(ProviderOperationError::InvalidResponse(
                "response idempotency_key does not match the runtime-derived request key"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn project_operation_readback(response: JsonObject) -> Result<JsonObject, ProviderOperationError> {
    let provider = required_response_string(&response, "provider")?;
    let operation = required_response_string(&response, "operation")?;
    let target = required_response_string(&response, "target")?;
    let result = response.get("result").cloned().ok_or_else(|| {
        ProviderOperationError::InvalidResponse("response result is missing".to_owned())
    })?;
    let mut readback = JsonObject::from([
        ("status".to_owned(), JsonValue::String("success".to_owned())),
        (
            "provider".to_owned(),
            JsonValue::String(provider.to_owned()),
        ),
        (
            "operation".to_owned(),
            JsonValue::String(operation.to_owned()),
        ),
        ("target".to_owned(), JsonValue::String(target.to_owned())),
        ("result".to_owned(), result),
    ]);
    for field in ["operation_id", "idempotency_key", "readback_ref"] {
        if let Some(value) = response.get(field) {
            readback.insert(field.to_owned(), value.clone());
        }
    }
    Ok(readback)
}

fn required_response_string<'a>(
    object: &'a JsonObject,
    field: &str,
) -> Result<&'a str, ProviderOperationError> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProviderOperationError::InvalidResponse(format!(
                "response {field} must be a non-empty string"
            ))
        })
}

pub fn validate_provider_grant_id(value: &str) -> Result<(), ProviderOperationError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 200
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '?' | '#'))
    {
        return Err(ProviderOperationError::InvalidGrantId);
    }
    Ok(())
}

pub fn validate_provider_operation(value: &str) -> Result<(), ProviderOperationError> {
    let value = value.trim();
    let mut segments = value.split('.');
    let first = segments.next().unwrap_or_default();
    if value.len() > 100
        || !valid_operation_segment(first)
        || !segments.next().is_some_and(valid_operation_segment)
        || !segments.all(valid_operation_segment)
    {
        return Err(ProviderOperationError::InvalidOperation);
    }
    Ok(())
}

fn valid_operation_segment(segment: &str) -> bool {
    let mut characters = segment.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        && characters.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::HostedApiEnvironment;
    use crate::http::{
        RuntimeHttpError, RuntimeHttpRequest as HttpRequest, RuntimeHttpResponse as HttpResponse,
    };

    #[derive(Default)]
    struct StubTransport {
        requests: RefCell<Vec<HttpRequest>>,
        responses: RefCell<Vec<HttpResponse>>,
    }

    impl StubTransport {
        fn with_responses(responses: Vec<HttpResponse>) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
                responses: RefCell::new(responses.into_iter().rev().collect()),
            }
        }
    }

    impl Transport for StubTransport {
        fn send(&self, request: HttpRequest) -> Result<HttpResponse, RuntimeHttpError> {
            self.requests.borrow_mut().push(request);
            self.responses
                .borrow_mut()
                .pop()
                .ok_or_else(|| RuntimeHttpError::Transport {
                    message: "missing stub response".to_owned(),
                })
        }
    }

    #[test]
    fn provider_operation_authenticates_and_returns_bounded_readback() {
        let env = BTreeMap::from([("RUNX_PUBLIC_API_TOKEN".to_owned(), "rxk_test".to_owned())]);
        let transport = StubTransport::with_responses(vec![
            HttpResponse::new(
                200,
                serde_json::json!({
                    "status": "success",
                    "principal": {"principal_id": "operator:test"}
                })
                .to_string(),
            ),
            HttpResponse::new(
                200,
                serde_json::json!({
                    "status": "success",
                    "provider": "slack",
                    "operation": "thread.reply",
                    "target": "slack://T/C/2",
                    "access": "mutate",
                    "operation_id": "provider-op-1",
                    "idempotency_key": "runx:test-operation",
                    "readback_ref": "runx:provider-readback:provider-op-1",
                    "result": {"message_locator": "slack://T/C/2"}
                })
                .to_string(),
            ),
        ]);
        let environment = HostedApiEnvironment::resolve(
            Some("https://api.runx.test"),
            None,
            &env,
            Path::new("."),
        )
        .expect("environment")
        .authenticate(&transport)
        .expect("authenticated");
        let response = invoke_provider_operation(
            &transport,
            &environment,
            &ProviderOperationRequest {
                grant_id: "grant_slack_1".to_owned(),
                operation: "thread.reply".to_owned(),
                target: "slack://T/C/2".to_owned(),
                input: JsonObject::from([(
                    "idempotency_key".to_owned(),
                    JsonValue::String("runx:test-operation".to_owned()),
                )]),
                expected_access: Some(ProviderOperationAccess::Mutate),
            },
        )
        .expect("provider operation");

        assert_eq!(
            response.get("provider").and_then(JsonValue::as_str),
            Some("slack")
        );
        assert_eq!(transport.requests.borrow().len(), 2);
    }

    #[test]
    fn provider_grant_listing_returns_only_bounded_authority_metadata() {
        let env = BTreeMap::from([("RUNX_PUBLIC_API_TOKEN".to_owned(), "rxk_test".to_owned())]);
        let transport = StubTransport::with_responses(vec![
            HttpResponse::new(
                200,
                serde_json::json!({
                    "status": "success",
                    "principal": {"principal_id": "operator:test"}
                })
                .to_string(),
            ),
            HttpResponse::new(
                200,
                serde_json::json!({
                    "status": "success",
                    "grants": [{
                        "grant_id": "grant_slack_1",
                        "provider": "slack",
                        "scopes": ["channel.post"],
                        "status": "active",
                        "credential_material_bound": true
                    }]
                })
                .to_string(),
            ),
        ]);
        let environment = HostedApiEnvironment::resolve(
            Some("https://api.runx.test"),
            None,
            &env,
            Path::new("."),
        )
        .expect("environment")
        .authenticate(&transport)
        .expect("authenticated");

        let grants = list_provider_grants(&transport, &environment).expect("grants");

        assert_eq!(
            grants,
            vec![HostedProviderGrant {
                grant_id: "grant_slack_1".to_owned(),
                provider: "slack".to_owned(),
                scopes: vec!["channel.post".to_owned()],
                status: "active".to_owned(),
            }]
        );
        let requests = transport.requests.borrow();
        assert_eq!(requests[1].method, HttpMethod::Get);
        assert_eq!(requests[1].url, "https://api.runx.test/v1/grants");
        assert!(requests[1].body.is_none());
    }

    #[test]
    fn provider_operation_rejects_mismatched_readback() {
        let request = ProviderOperationRequest {
            grant_id: "grant_github_1".to_owned(),
            operation: "issue.read".to_owned(),
            target: "github://runxhq/runx/issues/1".to_owned(),
            input: JsonObject::new(),
            expected_access: Some(ProviderOperationAccess::Read),
        };
        let error = parse_provider_operation_response(
            response_object(serde_json::json!({
                "status": "success",
                "provider": "github",
                "operation": "issue.write",
                "target": "github://runxhq/runx/issues/1",
                "result": {}
            })),
            &request,
        )
        .expect_err("mismatch");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn provider_operation_rejects_mismatched_access_before_trusting_result() {
        let request = ProviderOperationRequest {
            grant_id: "grant_slack_1".to_owned(),
            operation: "thread.reply".to_owned(),
            target: "slack://T/C/2".to_owned(),
            input: JsonObject::new(),
            expected_access: Some(ProviderOperationAccess::Read),
        };
        let error = parse_provider_operation_response(
            response_object(serde_json::json!({
                "status": "success",
                "provider": "slack",
                "operation": "thread.reply",
                "target": "slack://T/C/2",
                "access": "mutate",
                "result": {"message_locator": "slack://T/C/2"}
            })),
            &request,
        )
        .expect_err("access mismatch");
        assert!(error.to_string().contains("response access"));
    }

    #[test]
    fn provider_operation_requires_explicit_success_status() {
        let request = ProviderOperationRequest {
            grant_id: "grant_slack_1".to_owned(),
            operation: "thread.read".to_owned(),
            target: "slack://T/C/2".to_owned(),
            input: JsonObject::new(),
            expected_access: Some(ProviderOperationAccess::Read),
        };
        let error = parse_provider_operation_response(
            response_object(serde_json::json!({
                "provider": "slack",
                "operation": "thread.read",
                "target": "slack://T/C/2",
                "access": "read",
                "result": {"messages": []}
            })),
            &request,
        )
        .expect_err("missing success status must fail closed");

        assert!(error.to_string().contains("status is not success"));
    }

    #[test]
    fn provider_operation_requires_readback_evidence() {
        let request = ProviderOperationRequest {
            grant_id: "grant_slack_1".to_owned(),
            operation: "thread.read".to_owned(),
            target: "slack://T/C/2".to_owned(),
            input: JsonObject::new(),
            expected_access: Some(ProviderOperationAccess::Read),
        };
        let error = parse_provider_operation_response(
            response_object(serde_json::json!({
                "status": "success",
                "provider": "slack",
                "operation": "thread.read",
                "target": "slack://T/C/2",
                "access": "read",
                "result": {"messages": []}
            })),
            &request,
        )
        .expect_err("provider reads require readback evidence");

        assert!(error.to_string().contains("readback_ref"));
    }

    #[test]
    fn provider_mutation_requires_matching_runtime_idempotency_evidence() {
        let request = ProviderOperationRequest {
            grant_id: "grant_slack_1".to_owned(),
            operation: "thread.reply".to_owned(),
            target: "slack://T/C/2".to_owned(),
            input: JsonObject::from([(
                "idempotency_key".to_owned(),
                JsonValue::String("runx:expected".to_owned()),
            )]),
            expected_access: Some(ProviderOperationAccess::Mutate),
        };
        let error = parse_provider_operation_response(
            response_object(serde_json::json!({
                "status": "success",
                "provider": "slack",
                "operation": "thread.reply",
                "target": "slack://T/C/2",
                "access": "mutate",
                "operation_id": "provider-op-1",
                "idempotency_key": "caller-controlled",
                "readback_ref": "runx:provider-readback:provider-op-1",
                "result": {"message_locator": "slack://T/C/2"}
            })),
            &request,
        )
        .expect_err("mismatched provider idempotency must fail closed");

        assert!(error.to_string().contains("runtime-derived request key"));
    }

    fn response_object(value: serde_json::Value) -> JsonObject {
        serde_json::from_value::<JsonValue>(value)
            .expect("provider response fixture must convert")
            .as_object()
            .expect("provider response fixture must be an object")
            .clone()
    }
}
