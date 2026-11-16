use std::ffi::OsStr;

use clap::builder::TypedValueParser;

use super::*;

fn parse_share_request(value: &str) -> Result<ShareRequest, clap::Error> {
    let cmd = clap::Command::new("test");
    let parser = ShareRequestParser;
    parser.parse_ref(&cmd, None, OsStr::new(value))
}

#[test]
fn test_parse_team_default() {
    let result = parse_share_request("team").unwrap();
    assert!(matches!(result.subject, ShareSubject::Team));
    assert!(matches!(result.access_level, ShareAccessLevel::View));
}

#[test]
fn test_parse_team_view() {
    let result = parse_share_request("team:view").unwrap();
    assert!(matches!(result.subject, ShareSubject::Team));
    assert!(matches!(result.access_level, ShareAccessLevel::View));
}

#[test]
fn test_parse_team_edit() {
    let result = parse_share_request("team:edit").unwrap();
    assert!(matches!(result.subject, ShareSubject::Team));
    assert!(matches!(result.access_level, ShareAccessLevel::Edit));
}

#[test]
fn test_parse_user_default() {
    let result = parse_share_request("ben@warp.dev").unwrap();
    match result.subject {
        ShareSubject::User { email } => assert_eq!(email, "ben@warp.dev"),
        _ => panic!("Expected User subject"),
    }
    assert!(matches!(result.access_level, ShareAccessLevel::View));
}

#[test]
fn test_parse_user_view() {
    let result = parse_share_request("ben@warp.dev:view").unwrap();
    match result.subject {
        ShareSubject::User { email } => assert_eq!(email, "ben@warp.dev"),
        _ => panic!("Expected User subject"),
    }
    assert!(matches!(result.access_level, ShareAccessLevel::View));
}

#[test]
fn test_parse_user_edit() {
    let result = parse_share_request("ben@warp.dev:edit").unwrap();
    match result.subject {
        ShareSubject::User { email } => assert_eq!(email, "ben@warp.dev"),
        _ => panic!("Expected User subject"),
    }
    assert!(matches!(result.access_level, ShareAccessLevel::Edit));
}

#[test]
fn test_parse_invalid_format() {
    let result = parse_share_request("invalid");
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_access_level() {
    let result = parse_share_request("team:invalid");
    assert!(result.is_err());
}

#[test]
fn test_parse_public_default() {
    let result = parse_share_request("public").unwrap();
    assert!(matches!(result.subject, ShareSubject::Public));
    assert!(matches!(result.access_level, ShareAccessLevel::View));
}

#[test]
fn test_parse_public_view() {
    let result = parse_share_request("public:view").unwrap();
    assert!(matches!(result.subject, ShareSubject::Public));
    assert!(matches!(result.access_level, ShareAccessLevel::View));
}

#[test]
fn test_parse_public_edit() {
    let result = parse_share_request("public:edit").unwrap();
    assert!(matches!(result.subject, ShareSubject::Public));
    assert!(matches!(result.access_level, ShareAccessLevel::Edit));
}

#[test]
fn test_parse_public_invalid_access_level() {
    let result = parse_share_request("public:invalid");
    assert!(result.is_err());
}

#[test]
fn test_public_request_display() {
    let request = ShareRequest {
        subject: ShareSubject::Public,
        access_level: ShareAccessLevel::View,
    };
    assert_eq!(format!("{request}"), "public:view");

    let request = ShareRequest {
        subject: ShareSubject::Public,
        access_level: ShareAccessLevel::Edit,
    };
    assert_eq!(format!("{request}"), "public:edit");
}
