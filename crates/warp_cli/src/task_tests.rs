//! Tests for `ListTasksArgs` clap parsing.
use chrono::TimeZone;
use clap::Parser;

use super::*;

/// Tiny wrapper so we can parse `ListTasksArgs` in isolation, without wiring up the whole Args
/// type from `lib.rs`.
#[derive(Debug, Parser)]
struct TestApp {
    #[clap(flatten)]
    args: ListTasksArgs,
}

fn parse(argv: &[&str]) -> TestApp {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestApp::try_parse_from(full).expect("parse succeeds")
}

fn parse_err(argv: &[&str]) -> clap::Error {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestApp::try_parse_from(full).expect_err("parse fails")
}

#[test]
fn defaults_match_pre_change_behavior() {
    let TestApp { args } = parse(&[]);
    assert_eq!(args.limit, 10);
    assert!(args.state.is_empty());
    assert!(args.source.is_none());
    assert!(args.execution_location.is_none());
    assert!(args.creator.is_none());
    assert!(args.environment.is_none());
    assert!(args.skill.is_none());
    assert!(args.schedule.is_none());
    assert!(args.ancestor_run.is_none());
    assert!(args.name.is_none());
    assert!(args.model.is_none());
    assert!(args.artifact_type.is_none());
    assert!(args.created_after.is_none());
    assert!(args.created_before.is_none());
    assert!(args.updated_after.is_none());
    assert!(args.query.is_none());
    assert!(args.sort_by.is_none());
    assert!(args.sort_order.is_none());
    assert!(args.cursor.is_none());
}

#[test]
fn state_flag_is_repeatable() {
    let TestApp { args } = parse(&["--state", "failed", "--state", "error"]);
    assert_eq!(args.state, vec![RunStateArg::Failed, RunStateArg::Error]);
}

#[test]
fn all_filter_flags_parse() {
    let TestApp { args } = parse(&[
        "--limit",
        "42",
        "--state",
        "in-progress",
        "--source",
        "api",
        "--execution-location",
        "remote",
        "--creator",
        "user-uid",
        "--environment",
        "env-123",
        "--skill",
        "owner/repo:SKILL.md",
        "--schedule",
        "sched-1",
        "--ancestor-run",
        "run-parent",
        "--name",
        "nightly",
        "--model",
        "claude-4-5",
        "--artifact-type",
        "pull-request",
        "--created-after",
        "2026-04-01T00:00:00Z",
        "--created-before",
        "2026-04-02T00:00:00Z",
        "--updated-after",
        "2026-04-03T12:30:00Z",
        "-q",
        "oz run",
        "--sort-by",
        "created-at",
        "--sort-order",
        "asc",
        "--cursor",
        "abcd==",
    ]);

    assert_eq!(args.limit, 42);
    assert_eq!(args.state, vec![RunStateArg::InProgress]);
    assert_eq!(args.source, Some(RunSourceArg::Api));
    assert_eq!(args.execution_location, Some(ExecutionLocationArg::Remote));
    assert_eq!(args.creator.as_deref(), Some("user-uid"));
    assert_eq!(args.environment.as_deref(), Some("env-123"));
    assert_eq!(args.skill.as_deref(), Some("owner/repo:SKILL.md"));
    assert_eq!(args.schedule.as_deref(), Some("sched-1"));
    assert_eq!(args.ancestor_run.as_deref(), Some("run-parent"));
    assert_eq!(args.name.as_deref(), Some("nightly"));
    assert_eq!(args.model.as_deref(), Some("claude-4-5"));
    assert_eq!(args.artifact_type, Some(ArtifactTypeArg::PullRequest));
    assert_eq!(
        args.created_after,
        Some(Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap())
    );
    assert_eq!(
        args.created_before,
        Some(Utc.with_ymd_and_hms(2026, 4, 2, 0, 0, 0).unwrap())
    );
    assert_eq!(
        args.updated_after,
        Some(Utc.with_ymd_and_hms(2026, 4, 3, 12, 30, 0).unwrap())
    );
    assert_eq!(args.query.as_deref(), Some("oz run"));
    assert_eq!(args.sort_by, Some(RunSortByArg::CreatedAt));
    assert_eq!(args.sort_order, Some(SortOrderArg::Asc));
    assert_eq!(args.cursor.as_deref(), Some("abcd=="));
}

#[test]
fn invalid_state_is_rejected() {
    let err = parse_err(&["--state", "bogus"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn invalid_sort_by_is_rejected() {
    let err = parse_err(&["--sort-by", "random"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn invalid_execution_location_is_rejected() {
    let err = parse_err(&["--execution-location", "moon"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn invalid_artifact_type_is_rejected() {
    let err = parse_err(&["--artifact-type", "poem"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn invalid_created_after_timestamp_is_rejected() {
    let err = parse_err(&["--created-after", "not-a-timestamp"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn timestamps_are_converted_to_utc() {
    // Non-UTC offsets should be normalized to UTC in the parsed value.
    let TestApp { args } = parse(&["--updated-after", "2026-04-03T12:30:00+02:00"]);
    assert_eq!(
        args.updated_after,
        Some(Utc.with_ymd_and_hms(2026, 4, 3, 10, 30, 0).unwrap())
    );
}
