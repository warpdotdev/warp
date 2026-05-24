use std::time::{Duration, SystemTime};

use warp_multi_agent_api as api;

use super::*;

#[test]
fn new_credentials_stores_region() {
    let creds = AwsCredentials::new(
        "AKID".to_string(),
        "secret".to_string(),
        Some("token".to_string()),
        None,
        "us-west-2".to_string(),
    );
    assert_eq!(creds.region(), "us-west-2");
}

#[test]
fn new_credentials_empty_region() {
    let creds = AwsCredentials::new(
        "AKID".to_string(),
        "secret".to_string(),
        None,
        None,
        String::new(),
    );
    assert_eq!(creds.region(), "");
}

#[test]
fn from_credentials_passes_region_to_proto() {
    let creds = AwsCredentials::new(
        "AKID".to_string(),
        "secret".to_string(),
        Some("token".to_string()),
        None,
        "eu-west-1".to_string(),
    );
    let proto: api::request::settings::api_keys::AwsCredentials = creds.into();
    assert_eq!(proto.region, "eu-west-1");
    assert_eq!(proto.access_key, "AKID");
    assert_eq!(proto.secret_key, "secret");
    assert_eq!(proto.session_token, "token");
}

#[test]
fn from_credentials_empty_session_token_defaults() {
    let creds = AwsCredentials::new(
        "AKID".to_string(),
        "secret".to_string(),
        None,
        None,
        "us-east-1".to_string(),
    );
    let proto: api::request::settings::api_keys::AwsCredentials = creds.into();
    assert_eq!(proto.session_token, "");
    assert_eq!(proto.region, "us-east-1");
}

#[test]
fn user_facing_components_loaded_shows_region() {
    let creds = AwsCredentials::new(
        "AKID".to_string(),
        "secret".to_string(),
        None,
        Some(SystemTime::now() + Duration::from_secs(3600)),
        "us-east-1".to_string(),
    );
    let state = AwsCredentialsState::Loaded {
        credentials: creds,
        loaded_at: SystemTime::now(),
    };
    let (title, detail, _icon) = state.user_facing_components();
    assert_eq!(title, "Credentials loaded");
    assert!(detail.contains("Region: us-east-1"));
}

#[test]
fn user_facing_components_loaded_no_region_when_empty() {
    let creds = AwsCredentials::new(
        "AKID".to_string(),
        "secret".to_string(),
        None,
        None,
        String::new(),
    );
    let state = AwsCredentialsState::Loaded {
        credentials: creds,
        loaded_at: SystemTime::now(),
    };
    let (title, detail, _icon) = state.user_facing_components();
    assert_eq!(title, "Credentials loaded");
    assert!(!detail.contains("Region"));
}

#[test]
fn user_facing_components_missing_state() {
    let (title, _, _) = AwsCredentialsState::Missing.user_facing_components();
    assert_eq!(title, "AWS credentials not configured");
}

#[test]
fn user_facing_components_disabled_state() {
    let (title, _, _) = AwsCredentialsState::Disabled.user_facing_components();
    assert_eq!(title, "AWS Bedrock Disabled");
}

#[test]
fn user_facing_components_failed_state() {
    let state = AwsCredentialsState::Failed {
        message: "bad config".to_string(),
    };
    let (title, detail, _) = state.user_facing_components();
    assert_eq!(title, "Unable to load credentials");
    assert_eq!(detail, "bad config");
}
