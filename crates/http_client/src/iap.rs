/// The response header set by GCP Identity-Aware Proxy on its generated responses.
pub const IAP_GENERATED_RESPONSE_HEADER: &str = "x-goog-iap-generated-response";

/// HTTP header used to attach the IAP bearer token to outbound requests.
pub const IAP_PROXY_AUTH_HEADER: &str = "Proxy-Authorization";
pub const IAP_OPT_IN_HEADER: &str = "x-warp-iap-opt-in";

pub fn is_opt_in_destination(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "https" | "wss")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }

    let path = url.path();
    if matches!(path, "/iap/blocked" | "/iap/enable" | "/iap/disable") {
        return false;
    }

    match url.host_str() {
        Some("oz.staging.warp.dev" | "platform.staging.warp.dev") => {
            !path.starts_with("/static/js/")
        }
        Some("staging.warp.dev") => {
            let public_prefixes = [
                "/__/auth/",
                "/growth/",
                "/shared_sessions/",
                "/assets/client/static/",
                "/assets/client/wasm/",
                "/_rs/cdn/",
                "/_rs/api/",
                "/_iap/",
                "/agent/otlp/",
                "/namespace/webhook/v2/",
                "/block/",
                "/referral/",
                "/jira/forge/",
                "/slack/webhook/",
                "/slack/interactivity/",
                "/azure-devops/webhook/",
            ];
            let public_paths = [
                "/stripe/webhook",
                "/namespace/webhook",
                "/figma/webhook",
                "/linear/webhook",
                "/slack/webhook",
                "/slack/interactivity",
                "/github/webhook",
                "/cloud-scheduler-agent/webhook",
                "/.well-known/openid-configuration",
                "/api/v1/oauth/jwks.json",
                "/teams/activities",
                "/api/v1/oauth/token",
            ];
            let artifact_download = path
                .strip_prefix("/api/v1/agent/artifacts/")
                .and_then(|path| path.strip_suffix("/download"))
                .is_some_and(|id| !id.is_empty() && !id.contains('/'));

            !public_prefixes.iter().any(|prefix| path.starts_with(prefix))
                && !public_paths.contains(&path)
                && !artifact_download
        }
        _ => false,
    }
}

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

    fn routing_opt_in_enabled(&self) -> bool {
        false
    }

    fn routing_header(&self, url: &reqwest::Url) -> Option<(&'static str, &'static str)> {
        (self.routing_opt_in_enabled() && is_opt_in_destination(url))
            .then_some((IAP_OPT_IN_HEADER, "1"))
    }
}

#[cfg(test)]
#[path = "iap_tests.rs"]
mod tests;
