use runx_contracts::{JsonNumber, JsonObject, JsonValue, sha256_prefixed};
use url::Url;

use super::{is_sha256, object, text};

#[derive(Clone)]
pub(super) struct IndexedSource {
    source_digest: String,
    provider_content_digest: String,
    final_url: String,
    status: u64,
    extracted: String,
    provenance: JsonObject,
}

impl IndexedSource {
    pub(super) fn from_fetch(
        source: &JsonObject,
        max_characters: u64,
    ) -> Result<Self, Vec<String>> {
        let extracted = text(source.get("extracted"));
        let final_url = text(source.get("final_url"));
        let provider_digest = text(source.get("content_digest"));
        let provenance = object(source.get("provenance"));
        let status = coerced_nonnegative_integer(source.get("status"));
        let bytes = coerced_nonnegative_number(provenance.get("bytes"));
        let truncated = provenance.get("truncated").and_then(JsonValue::as_bool);
        let blockers = source_blockers(SourceFields {
            source,
            extracted: &extracted,
            final_url: &final_url,
            provider_digest: &provider_digest,
            provenance,
            status,
            bytes: bytes.as_ref(),
            truncated,
            max_characters,
        });
        if !blockers.is_empty() {
            return Err(blockers);
        }

        Ok(Self {
            source_digest: sha256_prefixed(extracted.as_bytes()),
            provider_content_digest: provider_digest,
            final_url,
            status: status.unwrap_or_default(),
            extracted,
            provenance: normalized_provenance(provenance, bytes, truncated),
        })
    }

    pub(super) fn character_count(&self) -> u64 {
        self.extracted.encode_utf16().count() as u64
    }

    pub(super) fn digest(&self) -> &str {
        &self.source_digest
    }

    pub(super) fn digest_json(&self) -> JsonValue {
        JsonValue::String(self.source_digest.clone())
    }

    pub(super) fn as_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from([
            ("source_digest".to_owned(), self.digest_json()),
            (
                "provider_content_digest".to_owned(),
                JsonValue::String(self.provider_content_digest.clone()),
            ),
            (
                "final_url".to_owned(),
                JsonValue::String(self.final_url.clone()),
            ),
            (
                "status".to_owned(),
                JsonValue::Number(JsonNumber::U64(self.status)),
            ),
            (
                "extracted".to_owned(),
                JsonValue::String(self.extracted.clone()),
            ),
            (
                "provenance".to_owned(),
                JsonValue::Object(self.provenance.clone()),
            ),
        ]))
    }

    pub(super) fn index_material(&self) -> JsonValue {
        let mut value = self.as_json().as_object().cloned().unwrap_or_default();
        value.remove("extracted");
        value.insert(
            "extracted_digest".to_owned(),
            JsonValue::String(sha256_prefixed(self.extracted.as_bytes())),
        );
        JsonValue::Object(value)
    }

    pub(super) fn evidence_json(&self) -> JsonValue {
        JsonValue::Object(JsonObject::from([
            ("evidence_digest".to_owned(), self.digest_json()),
            (
                "provider_content_digest".to_owned(),
                JsonValue::String(self.provider_content_digest.clone()),
            ),
            (
                "final_url".to_owned(),
                JsonValue::String(self.final_url.clone()),
            ),
            (
                "provenance".to_owned(),
                JsonValue::Object(self.provenance.clone()),
            ),
        ]))
    }
}

struct SourceFields<'a> {
    source: &'a JsonObject,
    extracted: &'a str,
    final_url: &'a str,
    provider_digest: &'a str,
    provenance: &'a JsonObject,
    status: Option<u64>,
    bytes: Option<&'a JsonValue>,
    truncated: Option<bool>,
    max_characters: u64,
}

fn source_blockers(fields: SourceFields<'_>) -> Vec<String> {
    let mut blockers = Vec::new();
    check(
        text(fields.source.get("decision")) == "ready",
        "decision is not ready",
        &mut blockers,
    );
    check(
        matches!(fields.status, Some(200..=299)),
        "status is not 2xx",
        &mut blockers,
    );
    check(
        is_http_url(fields.final_url),
        "final_url is not http(s)",
        &mut blockers,
    );
    check(
        is_sha256(fields.provider_digest),
        "content_digest is not sha256",
        &mut blockers,
    );
    check(
        !fields.extracted.is_empty(),
        "extracted text is missing",
        &mut blockers,
    );
    if fields.extracted.encode_utf16().count() as u64 > fields.max_characters {
        blockers.push(format!(
            "extracted text exceeds {} characters",
            fields.max_characters
        ));
    }
    check(
        !text(fields.provenance.get("fetched_at")).is_empty(),
        "provenance.fetched_at is missing",
        &mut blockers,
    );
    check(
        fields.bytes.is_some(),
        "provenance.bytes is invalid",
        &mut blockers,
    );
    check(
        fields.truncated.is_some(),
        "provenance.truncated is missing",
        &mut blockers,
    );
    blockers
}

fn check(condition: bool, message: &str, blockers: &mut Vec<String>) {
    if !condition {
        blockers.push(message.to_owned());
    }
}

fn normalized_provenance(
    provenance: &JsonObject,
    bytes: Option<JsonValue>,
    truncated: Option<bool>,
) -> JsonObject {
    JsonObject::from([
        (
            "fetched_at".to_owned(),
            JsonValue::String(text(provenance.get("fetched_at"))),
        ),
        ("bytes".to_owned(), bytes.unwrap_or(JsonValue::Null)),
        (
            "truncated".to_owned(),
            JsonValue::Bool(truncated.unwrap_or_default()),
        ),
        (
            "redirects".to_owned(),
            JsonValue::Array(
                provenance
                    .get("redirects")
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default(),
            ),
        ),
    ])
}

pub(super) fn unwrap_fetch_packet(value: &JsonValue) -> &JsonObject {
    let packet = object(Some(value));
    if let Some(data) = packet.get("data").and_then(JsonValue::as_object) {
        return data;
    }
    packet
        .get("fetch_result")
        .and_then(JsonValue::as_object)
        .and_then(|result| result.get("data"))
        .and_then(JsonValue::as_object)
        .unwrap_or(packet)
}

fn coerced_nonnegative_integer(value: Option<&JsonValue>) -> Option<u64> {
    match value {
        Some(JsonValue::Number(JsonNumber::U64(value))) => Some(*value),
        Some(JsonValue::Number(JsonNumber::I64(value))) if *value >= 0 => Some(*value as u64),
        Some(JsonValue::Number(JsonNumber::F64(value)))
            if value.is_finite() && *value >= 0.0 && value.fract() == 0.0 =>
        {
            Some(*value as u64)
        }
        Some(JsonValue::String(value)) => value.trim().parse().ok(),
        _ => None,
    }
}

fn coerced_nonnegative_number(value: Option<&JsonValue>) -> Option<JsonValue> {
    match value {
        Some(JsonValue::Number(number)) if number.as_f64().is_some_and(|value| value >= 0.0) => {
            Some(JsonValue::Number(number.clone()))
        }
        Some(JsonValue::String(value)) => value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| JsonValue::Number(JsonNumber::F64(value))),
        _ => None,
    }
}

fn is_http_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}
