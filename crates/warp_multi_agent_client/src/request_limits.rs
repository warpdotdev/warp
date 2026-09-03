/// Cloud Armor rejects `/ai/multi-agent` bodies whose Content-Length is strictly greater than
/// this decimal byte count, not 50 MiB.
pub const MULTI_AGENT_REQUEST_SIZE_LIMIT_BYTES: usize = 50_000_000;

pub const REQUEST_TOO_LARGE_USER_MESSAGE: &str = "This conversation exceeds Warp's 50MB limit and \
    cannot be continued. Please start a new conversation.";

/// Hedged copy for 403s that cannot be proven to be the size rule. Cloud Armor HTML has no
/// matched-rule discriminator, so a 403 may also be an IP allowlist or other edge deny.
pub const FORBIDDEN_FALLBACK_USER_MESSAGE: &str =
    "This request was blocked and cannot be continued. Please start a new conversation.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForbiddenBody {
    StructuredMessage(String),
    Opaque,
}

pub fn encoded_len_exceeds_request_size_limit(encoded_len: usize) -> bool {
    encoded_len > MULTI_AGENT_REQUEST_SIZE_LIMIT_BYTES
}

pub fn classify_forbidden_body(body: &str) -> ForbiddenBody {
    let trimmed = body.trim();
    if trimmed.is_empty() || looks_like_html(trimmed) {
        return ForbiddenBody::Opaque;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return ForbiddenBody::Opaque;
    };
    let Some(message) = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
    else {
        return ForbiddenBody::Opaque;
    };
    if looks_like_html(message) {
        return ForbiddenBody::Opaque;
    }

    ForbiddenBody::StructuredMessage(message.to_owned())
}

pub fn user_message_for_forbidden_body(body: &str) -> String {
    match classify_forbidden_body(body) {
        ForbiddenBody::StructuredMessage(message) => message,
        ForbiddenBody::Opaque => FORBIDDEN_FALLBACK_USER_MESSAGE.to_owned(),
    }
}

fn looks_like_html(body: &str) -> bool {
    body.trim_start().starts_with('<')
}

#[cfg(test)]
#[path = "request_limits_tests.rs"]
mod tests;
