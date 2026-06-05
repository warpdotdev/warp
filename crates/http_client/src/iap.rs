/// The response header set by GCP Identity-Aware Proxy on its generated responses.
pub const IAP_GENERATED_RESPONSE_HEADER: &str = "x-goog-iap-generated-response";

/// HTTP header used to attach the IAP bearer token to outbound requests.
pub const IAP_PROXY_AUTH_HEADER: &str = "Proxy-Authorization";

pub fn proxy_auth_header(token: &str) -> (&'static str, String) {
    (IAP_PROXY_AUTH_HEADER, format!("Bearer {token}"))
}

/// Returns `true` if the given status + headers appear to be an IAP-generated
/// challenge (302, 401, or 403 with the IAP response header present). Useful
/// for detecting stale credentials and triggering a re-fetch.
pub fn is_iap_challenge(status: reqwest::StatusCode, headers: &http::HeaderMap) -> bool {
    let is_challenge_status = status == reqwest::StatusCode::FOUND
        || status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN;

    is_challenge_status && headers.get(IAP_GENERATED_RESPONSE_HEADER).is_some()
}

/// Source of the current IAP bearer token.
pub trait IapTokenProvider: Send + Sync {
    fn cached_token(&self) -> Option<String>;
}

#[cfg(test)]
#[path = "iap_tests.rs"]
mod tests;
