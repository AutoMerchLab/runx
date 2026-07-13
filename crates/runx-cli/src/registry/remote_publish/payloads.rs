use runx_runtime::registry::RegistryPublishHarnessReport;
use serde::{Deserialize, Serialize};

use super::super::package::HostedSkillPackageFile;

#[derive(Serialize)]
pub(super) struct HostedSkillPublishRequest<'a> {
    pub(super) markdown: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) profile_document: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) version: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) package_files: &'a Vec<HostedSkillPackageFile>,
}

#[derive(Serialize)]
pub(super) struct HostedAdminSkillPublishRequest<'a> {
    pub(super) owner: &'a str,
    pub(super) markdown: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) profile_document: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) version: Option<&'a str>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) upsert: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) package_files: &'a Vec<HostedSkillPackageFile>,
    pub(super) harness: &'a RegistryPublishHarnessReport,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct HostedSkillPublishEnvelope {
    pub(super) status: String,
    pub(super) publish: HostedSkillPublishResult,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct HostedAdminSkillPublishEnvelope {
    pub(super) status: String,
    pub(super) publish: HostedAdminSkillPublishResult,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct HostedAdminSkillPublishResult {
    pub(super) status: String,
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
    pub(super) fn into_hosted_result(self) -> HostedSkillPublishResult {
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct HostedAdminSkillRecord {
    owner: String,
    #[serde(default)]
    trust_tier: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(in crate::registry) struct HostedSkillPublishResult {
    pub(in crate::registry) status: String,
    pub(in crate::registry) skill_id: String,
    pub(in crate::registry) owner: String,
    pub(in crate::registry) name: String,
    pub(in crate::registry) version: String,
    pub(in crate::registry) digest: String,
    #[serde(default)]
    pub(in crate::registry) profile_digest: Option<String>,
    pub(in crate::registry) trust_tier: String,
    pub(in crate::registry) install_command: String,
    pub(in crate::registry) run_command: String,
    pub(in crate::registry) public_url: String,
}
