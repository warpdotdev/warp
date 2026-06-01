/// The response header set by GCP Identity-Aware Proxy on its generated responses.
pub const IAP_GENERATED_RESPONSE_HEADER: &str = "x-goog-iap-generated-response";

/// HTTP header used to attach the IAP bearer token to outbound requests.
pub const IAP_PROXY_AUTH_HEADER: &str = "Proxy-Authorization";

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
mod tests {
    use http::{HeaderMap, HeaderValue};
    use reqwest::StatusCode;

    use super::*;

    fn headers_with_iap() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            IAP_GENERATED_RESPONSE_HEADER,
            HeaderValue::from_static("true"),
        );
        headers
    }

    #[test]
    fn challenge_statuses_with_iap_header_are_challenges() {
        for status in [
            StatusCode::FOUND,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
        ] {
            assert!(is_iap_challenge(status, &headers_with_iap()));
        }
    }

    #[test]
    fn challenge_status_without_iap_header_is_not_a_challenge() {
        assert!(!is_iap_challenge(StatusCode::FORBIDDEN, &HeaderMap::new()));
        assert!(!is_iap_challenge(
            StatusCode::UNAUTHORIZED,
            &HeaderMap::new()
        ));
    }

    #[test]
    fn non_challenge_status_with_iap_header_is_not_a_challenge() {
        assert!(!is_iap_challenge(StatusCode::OK, &headers_with_iap()));
        assert!(!is_iap_challenge(
            StatusCode::INTERNAL_SERVER_ERROR,
            &headers_with_iap()
        ));
    }
}
