use http::{HeaderMap, HeaderValue};
use reqwest::StatusCode;

use super::*;

#[test]
fn only_staging_application_destinations_allow_opt_in() {
    for url in [
        "https://staging.warp.dev/graphql/v2",
        "https://oz.staging.warp.dev/api/v1/agent",
        "wss://platform.staging.warp.dev/graphql/v2",
        "https://staging.warp.dev/api/v1/agent/artifacts/uid",
        "https://staging.warp.dev/api/v1/oauth/token/other",
    ] {
        assert!(
            is_opt_in_destination(&reqwest::Url::parse(url).unwrap()),
            "{url}"
        );
    }
    for url in [
        "http://staging.warp.dev/graphql/v2",
        "https://staging.warp.dev:8080/graphql/v2",
        "https://user@staging.warp.dev/graphql/v2",
        "https://user:password@staging.warp.dev/graphql/v2",
        "https://staging.warp.dev.evil.example/graphql/v2",
        "https://staging.warp.dev/shared_sessions/test",
        "https://staging.warp.dev/__/auth/handler",
        "https://staging.warp.dev/growth/test",
        "https://staging.warp.dev/assets/client/static/logo.png",
        "https://staging.warp.dev/assets/client/wasm/warp.wasm",
        "https://staging.warp.dev/_rs/cdn/test",
        "https://staging.warp.dev/_rs/api/test",
        "https://staging.warp.dev/_iap/graphql/v2",
        "https://staging.warp.dev/agent/otlp/v1/traces",
        "https://staging.warp.dev/namespace/webhook/v2/test",
        "https://staging.warp.dev/block/test",
        "https://staging.warp.dev/referral/test",
        "https://staging.warp.dev/jira/forge/test",
        "https://staging.warp.dev/slack/webhook/app",
        "https://staging.warp.dev/slack/interactivity/app",
        "https://staging.warp.dev/azure-devops/webhook/app",
        "https://staging.warp.dev/stripe/webhook",
        "https://staging.warp.dev/namespace/webhook",
        "https://staging.warp.dev/figma/webhook",
        "https://staging.warp.dev/linear/webhook",
        "https://staging.warp.dev/slack/webhook",
        "https://staging.warp.dev/slack/interactivity",
        "https://staging.warp.dev/github/webhook",
        "https://staging.warp.dev/cloud-scheduler-agent/webhook",
        "https://staging.warp.dev/.well-known/openid-configuration",
        "https://staging.warp.dev/api/v1/oauth/jwks.json",
        "https://staging.warp.dev/teams/activities",
        "https://staging.warp.dev/api/v1/oauth/token",
        "https://staging.warp.dev/api/v1/agent/artifacts/uid/download",
        "https://staging.warp.dev/iap/blocked",
        "https://oz.staging.warp.dev/iap/enable",
        "https://platform.staging.warp.dev/iap/disable",
        "https://platform.staging.warp.dev/static/js/app.js",
    ] {
        assert!(
            !is_opt_in_destination(&reqwest::Url::parse(url).unwrap()),
            "{url}"
        );
    }
}

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
