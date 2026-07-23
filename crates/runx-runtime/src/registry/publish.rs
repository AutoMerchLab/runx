use serde::{Deserialize, Serialize};

use super::{RegistryPackageFile, RegistryPublishHarnessReport};
use crate::hosted_api::{HostedApiOperationError, request::send_json};
use crate::http::{HttpMethod, RuntimeHttpTransport};

#[derive(Serialize)]
pub struct HostedSkillPublishRequest<'a> {
    pub markdown: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_document: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<&'a str>,
    #[serde(skip_serializing_if = "slice_empty")]
    pub package_files: &'a [RegistryPackageFile],
}

#[derive(Serialize)]
pub struct HostedAdminSkillPublishRequest<'a> {
    pub owner: &'a str,
    pub markdown: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_document: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<&'a str>,
    #[serde(skip_serializing_if = "is_false")]
    pub upsert: bool,
    #[serde(skip_serializing_if = "slice_empty")]
    pub package_files: &'a [RegistryPackageFile],
    pub harness: &'a RegistryPublishHarnessReport,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostedSkillPublishResult {
    pub status: String,
    pub skill_id: String,
    pub owner: String,
    pub name: String,
    pub version: String,
    pub digest: String,
    #[serde(default)]
    pub profile_digest: Option<String>,
    pub trust_tier: String,
    pub install_command: String,
    pub run_command: String,
    pub public_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryPublishError {
    #[error(transparent)]
    Operation(#[from] HostedApiOperationError),
    #[error("remote registry publish returned an invalid contract: {0}")]
    Contract(String),
}

pub fn publish_hosted_skill(
    transport: &impl RuntimeHttpTransport,
    registry_url: &str,
    token: &str,
    request: &HostedSkillPublishRequest<'_>,
) -> Result<HostedSkillPublishResult, RegistryPublishError> {
    let body = serde_json::to_string(request).map_err(invalid_request_json)?;
    let envelope: HostedSkillPublishEnvelope = send_json(
        transport,
        registry_url,
        "registry publish",
        HttpMethod::Post,
        "/v1/skills",
        Some(token),
        Some(body),
    )?;
    if envelope.status != "success" || envelope.publish.status != "published" {
        return Err(RegistryPublishError::Contract(format!(
            "unsuccessful status: envelope={}, publish={}",
            envelope.status, envelope.publish.status
        )));
    }
    Ok(envelope.publish)
}

pub fn publish_hosted_admin_skill(
    transport: &impl RuntimeHttpTransport,
    registry_url: &str,
    token: &str,
    request: &HostedAdminSkillPublishRequest<'_>,
) -> Result<HostedSkillPublishResult, RegistryPublishError> {
    let body = serde_json::to_string(request).map_err(invalid_request_json)?;
    let envelope: HostedAdminSkillPublishEnvelope = send_json(
        transport,
        registry_url,
        "registry admin publish",
        HttpMethod::Post,
        "/v1/admin/registry/publish",
        Some(token),
        Some(body),
    )?;
    if envelope.status != "success"
        || !matches!(envelope.publish.status.as_str(), "published" | "unchanged")
    {
        return Err(RegistryPublishError::Contract(format!(
            "unsuccessful status: envelope={}, publish={}",
            envelope.status, envelope.publish.status
        )));
    }
    Ok(envelope.publish.into_hosted_result())
}

fn invalid_request_json(error: serde_json::Error) -> RegistryPublishError {
    HostedApiOperationError::InvalidRequest {
        operation: "registry publish request",
        message: error.to_string(),
    }
    .into()
}

fn slice_empty<T>(values: &&[T]) -> bool {
    values.is_empty()
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Deserialize)]
struct HostedSkillPublishEnvelope {
    status: String,
    publish: HostedSkillPublishResult,
}

#[derive(Deserialize)]
struct HostedAdminSkillPublishEnvelope {
    status: String,
    publish: HostedAdminSkillPublishResult,
}

#[derive(Deserialize)]
struct HostedAdminSkillPublishResult {
    status: String,
    skill_id: String,
    name: String,
    version: String,
    digest: String,
    #[serde(default)]
    profile_digest: Option<String>,
    #[serde(default)]
    record: Option<HostedAdminSkillRecord>,
    link: HostedSkillPublishLink,
}

impl HostedAdminSkillPublishResult {
    fn into_hosted_result(self) -> HostedSkillPublishResult {
        let owner = self
            .record
            .as_ref()
            .map(|record| record.owner.clone())
            .or_else(|| {
                self.skill_id
                    .split_once('/')
                    .map(|(owner, _)| owner.to_owned())
            })
            .unwrap_or_default();
        let trust_tier = self
            .record
            .as_ref()
            .and_then(|record| record.trust_tier.clone())
            .unwrap_or_else(|| "first_party".to_owned());
        HostedSkillPublishResult {
            status: self.status,
            public_url: self.link.public_url(&self.skill_id, &self.version),
            skill_id: self.skill_id,
            owner,
            name: self.name,
            version: self.version,
            digest: self.digest,
            profile_digest: self.profile_digest,
            trust_tier,
            install_command: self.link.install_command,
            run_command: self.link.run_command,
        }
    }
}

#[derive(Deserialize)]
struct HostedAdminSkillRecord {
    owner: String,
    #[serde(default)]
    trust_tier: Option<String>,
}

#[derive(Deserialize)]
struct HostedSkillPublishLink {
    install_command: String,
    run_command: String,
    #[serde(default)]
    public_url: Option<String>,
    #[serde(default)]
    link: Option<String>,
}

impl HostedSkillPublishLink {
    fn public_url(&self, skill_id: &str, version: &str) -> String {
        self.public_url
            .as_deref()
            .or(self
                .link
                .as_deref()
                .filter(|link| link.starts_with("http://") || link.starts_with("https://")))
            .map(str::to_owned)
            .unwrap_or_else(|| runx_skill_public_url(skill_id, version))
    }
}

fn runx_skill_public_url(skill_id: &str, version: &str) -> String {
    let (owner, name) = skill_id.split_once('/').unwrap_or(("", skill_id));
    format!(
        "https://runx.ai/x/{}/{}@{}",
        encode_path_component(owner),
        encode_path_component(name),
        encode_path_component(version)
    )
}

fn encode_path_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::http::{
        RuntimeHttpError, RuntimeHttpRequest, RuntimeHttpResponse, RuntimeHttpTransport,
    };

    #[test]
    fn publish_skill_sends_the_complete_package_contract() -> Result<(), Box<dyn std::error::Error>>
    {
        let transport = StubTransport::new(successful_skill_response("success", "published"));
        let files = vec![RegistryPackageFile {
            path: "run.mjs".to_owned(),
            content: "console.log('hello');\n".to_owned(),
        }];
        let result = publish_hosted_skill(
            &transport,
            "https://runx.test/",
            "rxk_secret",
            &HostedSkillPublishRequest {
                markdown: "---\nname: hello\n---\nHello.\n",
                profile_document: Some("skill: hello\nrunners: {}\n"),
                version: Some("sha-123"),
                package_files: &files,
            },
        )?;

        assert_eq!(result.skill_id, "kam/hello");
        let requests = transport.requests.borrow();
        assert_eq!(requests[0].method, HttpMethod::Post);
        assert_eq!(requests[0].url, "https://runx.test/v1/skills");
        assert!(requests[0].headers.iter().any(|header| {
            header.name == "authorization" && header.value == "Bearer rxk_secret"
        }));
        let body = request_body(&requests[0])?;
        assert_eq!(body["version"], "sha-123");
        assert_eq!(body["package_files"][0]["path"], "run.mjs");
        Ok(())
    }

    #[test]
    fn publish_skill_rejects_an_unsuccessful_success_status_envelope()
    -> Result<(), Box<dyn std::error::Error>> {
        let transport = StubTransport::new(successful_skill_response("failure", "rejected"));
        let result = publish_hosted_skill(
            &transport,
            "https://runx.test",
            "rxk_secret",
            &HostedSkillPublishRequest {
                markdown: "---\nname: hello\n---\nHello.\n",
                profile_document: None,
                version: None,
                package_files: &[],
            },
        );

        assert!(matches!(result, Err(RegistryPublishError::Contract(_))));
        Ok(())
    }

    #[test]
    fn publish_admin_sends_owner_harness_and_upsert() -> Result<(), Box<dyn std::error::Error>> {
        let transport = StubTransport::new(RuntimeHttpResponse::new(
            200,
            serde_json::json!({
                "status": "success",
                "publish": {
                    "status": "published",
                    "skill_id": "runx/hello",
                    "name": "hello",
                    "version": "sha-123",
                    "digest": "abc",
                    "profile_digest": "profile-abc",
                    "link": {
                        "install_command": "runx add runx/hello@sha-123",
                        "run_command": "runx skill runx/hello@sha-123"
                    },
                    "record": { "owner": "runx", "trust_tier": "first_party" }
                }
            })
            .to_string(),
        ));
        let files = vec![RegistryPackageFile {
            path: "run.mjs".to_owned(),
            content: "console.log('hello');\n".to_owned(),
        }];
        let harness = RegistryPublishHarnessReport {
            status: "passed".to_owned(),
            case_count: 1,
            assertion_error_count: 0,
            assertion_errors: Vec::new(),
            case_names: vec!["smoke".to_owned()],
            receipt_ids: vec!["rx_harness_1".to_owned()],
            graph_case_count: 0,
        };
        let result = publish_hosted_admin_skill(
            &transport,
            "https://runx.test/",
            "admin-token",
            &HostedAdminSkillPublishRequest {
                owner: "runx",
                markdown: "---\nname: hello\n---\nHello.\n",
                profile_document: Some("skill: hello\nrunners: {}\n"),
                version: Some("sha-123"),
                upsert: true,
                package_files: &files,
                harness: &harness,
            },
        )?;

        assert_eq!(result.owner, "runx");
        assert_eq!(result.public_url, "https://runx.ai/x/runx/hello@sha-123");
        let requests = transport.requests.borrow();
        assert_eq!(
            requests[0].url,
            "https://runx.test/v1/admin/registry/publish"
        );
        let body = request_body(&requests[0])?;
        assert_eq!(body["owner"], "runx");
        assert_eq!(body["upsert"], true);
        assert_eq!(body["harness"]["status"], "passed");
        Ok(())
    }

    fn successful_skill_response(status: &str, publish_status: &str) -> RuntimeHttpResponse {
        RuntimeHttpResponse::new(
            200,
            serde_json::json!({
                "status": status,
                "publish": {
                    "status": publish_status,
                    "skill_id": "kam/hello",
                    "owner": "kam",
                    "name": "hello",
                    "version": "sha-123",
                    "digest": "abc",
                    "trust_tier": "community",
                    "install_command": "runx add kam/hello@sha-123",
                    "run_command": "runx skill kam/hello@sha-123",
                    "public_url": "https://runx.test/x/kam/hello"
                }
            })
            .to_string(),
        )
    }

    fn request_body(request: &RuntimeHttpRequest) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_str(request.body.as_deref().unwrap_or_default())
    }

    struct StubTransport {
        requests: RefCell<Vec<RuntimeHttpRequest>>,
        response: RefCell<Option<RuntimeHttpResponse>>,
    }

    impl StubTransport {
        fn new(response: RuntimeHttpResponse) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
                response: RefCell::new(Some(response)),
            }
        }
    }

    impl RuntimeHttpTransport for StubTransport {
        fn send(
            &self,
            request: RuntimeHttpRequest,
        ) -> Result<RuntimeHttpResponse, RuntimeHttpError> {
            self.requests.borrow_mut().push(request);
            self.response
                .borrow_mut()
                .take()
                .ok_or_else(|| RuntimeHttpError::Transport {
                    message: "missing stub response".to_owned(),
                })
        }
    }
}
