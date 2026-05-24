use clap::Parser;

use super::*;
#[derive(Debug, Parser)]
struct TestApiKey {
    #[command(subcommand)]
    command: ApiKeyCommand,
}

#[derive(Debug, Parser)]
struct TestCreate {
    #[clap(flatten)]
    args: CreateApiKeyArgs,
}

fn parse_command(argv: &[&str]) -> ApiKeyCommand {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestApiKey::try_parse_from(full)
        .expect("parse succeeds")
        .command
}

fn parse_create(argv: &[&str]) -> CreateApiKeyArgs {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestCreate::try_parse_from(full)
        .expect("parse succeeds")
        .args
}

fn parse_create_err(argv: &[&str]) -> clap::Error {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestCreate::try_parse_from(full).expect_err("parse fails")
}

#[test]
fn create_requires_expiration_decision() {
    let err = parse_create_err(&["ci-key"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn create_rejects_multiple_expiration_decisions() {
    let err = parse_create_err(&["ci-key", "--expires-in", "30d", "--no-expiration"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn create_accepts_expires_in() {
    let args = parse_create(&["ci-key", "--expires-in", "30d", "--agent", "agent-123"]);
    assert_eq!(args.name, "ci-key");
    assert_eq!(args.agent_uid.as_deref(), Some("agent-123"));
    assert!(args.expiration.expires_in.is_some());
    assert!(args.expiration.expires_at.is_none());
    assert!(!args.expiration.no_expiration);
}

#[test]
fn create_accepts_no_expiration() {
    let args = parse_create(&["ci-key", "--no-expiration"]);
    assert_eq!(args.name, "ci-key");
    assert!(args.expiration.expires_in.is_none());
    assert!(args.expiration.expires_at.is_none());
    assert!(args.expiration.no_expiration);
}

#[test]
fn create_accepts_rfc3339_expiration() {
    let args = parse_create(&["ci-key", "--expires-at", "2026-06-01T12:00:00Z"]);
    assert_eq!(args.name, "ci-key");
    assert!(args.expiration.expires_in.is_none());
    assert!(args.expiration.expires_at.is_some());
    assert!(!args.expiration.no_expiration);
}

#[test]
fn delete_is_alias_for_expire() {
    let command = parse_command(&["delete", "deploy-key", "--force"]);
    let ApiKeyCommand::Expire(args) = command else {
        panic!("Expected expire command");
    };

    assert_eq!(args.key_uid, "deploy-key");
    assert!(args.force);
}
