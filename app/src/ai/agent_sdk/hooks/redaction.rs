use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum RedactedValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<RedactedValue>),
    Object(BTreeMap<String, RedactedValue>),
}

impl RedactedValue {
    pub(crate) fn object(
        fields: impl IntoIterator<Item = (impl Into<String>, RedactedValue)>,
    ) -> Self {
        Self::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    pub(crate) fn redacted(reason: &str, byte_count: usize) -> Self {
        Self::object([
            ("redacted", Self::Bool(true)),
            ("reason", Self::String(reason.into())),
            (
                "byte_count",
                Self::Number(serde_json::Number::from(byte_count)),
            ),
        ])
    }

    pub(crate) fn serialized_len(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
    }
}

impl From<&str> for RedactedValue {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

impl From<String> for RedactedValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<bool> for RedactedValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<u64> for RedactedValue {
    fn from(value: u64) -> Self {
        Self::Number(value.into())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TruncationMetadata {
    pub(crate) truncated: bool,
    pub(crate) original_bytes: usize,
    pub(crate) included_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RedactedText {
    pub(crate) value: String,
    pub(crate) truncation: Option<TruncationMetadata>,
}

#[derive(Clone)]
pub(crate) struct HookRedactor {
    known_secrets: Vec<String>,
    credential_patterns: Vec<Regex>,
}

impl HookRedactor {
    pub(crate) fn new(known_secrets: impl IntoIterator<Item = String>) -> Self {
        Self {
            known_secrets: known_secrets
                .into_iter()
                .filter(|secret| !secret.is_empty())
                .collect(),
            credential_patterns: vec![
                Regex::new(r"(?i)(authorization\s*:\s*(?:bearer|basic)\s+)\S+").unwrap(),
                Regex::new(
                    r#"(?i)((?:api[_-]?key|access[_-]?token|secret)\s*[=:]\s*["']?)[^\s"',;]+"#,
                )
                .unwrap(),
                Regex::new(r"\b(?:gh[opusr]_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{20,})\b").unwrap(),
            ],
        }
    }

    pub(crate) fn redact_text(&self, input: &str, maximum_bytes: usize) -> RedactedText {
        let mut redacted = input.to_owned();
        for secret in &self.known_secrets {
            redacted = redacted.replace(secret, "[REDACTED]");
        }
        for pattern in &self.credential_patterns {
            redacted = pattern
                .replace_all(&redacted, |captures: &regex::Captures<'_>| {
                    if captures.len() > 1 {
                        format!("{}[REDACTED]", captures.get(1).unwrap().as_str())
                    } else {
                        "[REDACTED]".into()
                    }
                })
                .into_owned();
        }
        crate::ai::agent::redaction::redact_secrets(&mut redacted);
        let original_bytes = redacted.len();
        if original_bytes <= maximum_bytes {
            return RedactedText {
                value: redacted,
                truncation: None,
            };
        }
        let included_bytes = floor_utf8_boundary(&redacted, maximum_bytes);
        redacted.truncate(included_bytes);
        RedactedText {
            value: redacted,
            truncation: Some(TruncationMetadata {
                truncated: true,
                original_bytes,
                included_bytes,
            }),
        }
    }
}

pub(crate) fn truncate_utf8(input: &str, maximum_bytes: usize) -> String {
    let boundary = floor_utf8_boundary(input, maximum_bytes);
    input[..boundary].to_owned()
}

fn floor_utf8_boundary(input: &str, maximum_bytes: usize) -> usize {
    let mut boundary = maximum_bytes.min(input.len());
    while !input.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

#[allow(dead_code)]
pub(crate) fn contains_prohibited_payload_key(value: &Value) -> bool {
    const PROHIBITED_KEYS: [&str; 9] = [
        "environment",
        "env",
        "authorization",
        "api_key",
        "secret",
        "attachment_bytes",
        "file_content",
        "transcript",
        "transcript_path",
    ];
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            PROHIBITED_KEYS
                .iter()
                .any(|prohibited| key.eq_ignore_ascii_case(prohibited))
                || contains_prohibited_payload_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_prohibited_payload_key),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod tests;
