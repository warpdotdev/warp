use clap::Parser;

use super::*;
use crate::share::{ShareAccessLevel, ShareSubject};
use crate::{Args, CliCommand, Command};

/// Parse `warp <args...>` and extract the `TerminalShareArgs`, panicking if the
/// input is not a `terminal share` invocation.
fn parse_terminal_share(args: &[&str]) -> TerminalShareArgs {
    let full: Vec<&str> = std::iter::once("warp")
        .chain(args.iter().copied())
        .collect();
    let parsed = Args::try_parse_from(full).expect("terminal share args should parse");
    let Some(Command::CommandLine(boxed)) = parsed.command else {
        panic!("Expected a CLI command");
    };
    match *boxed {
        CliCommand::Terminal(TerminalCommand::Share(args)) => args,
        other => panic!("Expected `terminal share` command, got {other:?}"),
    }
}

#[test]
fn terminal_share_parses_with_default_share() {
    let args = parse_terminal_share(&["terminal", "share"]);
    // No `--share` flag parses to `None`. At runtime this shares the session
    // owner-only (share-with-self) — no recipient ACL is applied (spec
    // Behavior #4). See `run_share` in `app/src/ai/agent_sdk/terminal.rs`.
    assert!(args.share.share.is_none());
    assert!(args.working_dir.is_none());
}

#[test]
fn terminal_share_bare_flag_parses_to_empty_recipients() {
    let args = parse_terminal_share(&["terminal", "share", "--share"]);
    // A bare `--share` (no value) parses to `Some(vec![])`. Like an omitted
    // flag, this collapses to no recipients at runtime, i.e. owner-only
    // sharing (spec Behavior #4 / Q2).
    let requests = args
        .share
        .share
        .expect("bare --share should parse to Some(empty)");
    assert!(
        requests.is_empty(),
        "bare --share should carry no recipients"
    );
}

#[test]
fn terminal_share_parses_team_edit() {
    let args = parse_terminal_share(&["terminal", "share", "--share", "team:edit"]);
    let requests = args.share.share.expect("expected parsed share requests");
    assert_eq!(requests.len(), 1);
    assert!(matches!(requests[0].subject, ShareSubject::Team));
    assert!(matches!(requests[0].access_level, ShareAccessLevel::Edit));
}

#[test]
fn terminal_share_parses_multiple_recipients() {
    let args = parse_terminal_share(&[
        "terminal",
        "share",
        "--share",
        "public",
        "--share",
        "user@example.com:view",
    ]);
    let requests = args.share.share.expect("expected parsed share requests");
    assert_eq!(requests.len(), 2);
    assert!(matches!(requests[0].subject, ShareSubject::Public));
    assert!(matches!(requests[0].access_level, ShareAccessLevel::View));
    match &requests[1].subject {
        ShareSubject::User { email } => assert_eq!(email, "user@example.com"),
        other => panic!("Expected a user subject, got {other:?}"),
    }
    assert!(matches!(requests[1].access_level, ShareAccessLevel::View));
}

#[test]
fn terminal_share_parses_working_dir() {
    let args = parse_terminal_share(&["terminal", "share", "--working-dir", "/tmp/x"]);
    assert_eq!(
        args.working_dir.as_ref().and_then(|p| p.to_str()),
        Some("/tmp/x")
    );
}

#[test]
fn terminal_share_rejects_invalid_recipient() {
    let result = Args::try_parse_from(["warp", "terminal", "share", "--share", "nope"]);
    assert!(
        result.is_err(),
        "an invalid share recipient should fail to parse"
    );
}
