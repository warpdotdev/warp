use clap::Parser;

use super::*;
use crate::{Args, CliCommand, Command};

#[derive(Debug, Parser)]
struct TestRunner {
    #[command(subcommand)]
    command: RunnerCommand,
}

fn parse_command(argv: &[&str]) -> RunnerCommand {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestRunner::try_parse_from(full)
        .expect("parse succeeds")
        .command
}

fn parse_command_err(argv: &[&str]) -> clap::Error {
    let mut full = vec!["test"];
    full.extend_from_slice(argv);
    TestRunner::try_parse_from(full).expect_err("parse fails")
}

/// Parses a full `oz runner ...` invocation and returns the [`RunnerCommand`].
fn parse_via_cli(argv: &[&str]) -> RunnerCommand {
    let full: Vec<&str> = std::iter::once("oz").chain(argv.iter().copied()).collect();
    let parsed = Args::try_parse_from(full).expect("parse succeeds");
    let Some(Command::CommandLine(boxed)) = parsed.command else {
        panic!("Expected a CLI command");
    };
    match *boxed {
        CliCommand::Runner(command) => command,
        other => panic!("Expected a runner command, got {other:?}"),
    }
}

#[test]
fn list_parses_via_cli() {
    let command = parse_via_cli(&["runner", "list"]);
    let RunnerCommand::List(args) = command else {
        panic!("Expected list command");
    };
    assert!(args.sort_by.is_none());
    assert!(args.json_output.filter.is_none());
}

#[test]
fn list_accepts_sort_by() {
    let command = parse_command(&["list", "--sort-by", "last-updated"]);
    let RunnerCommand::List(args) = command else {
        panic!("Expected list command");
    };
    assert_eq!(args.sort_by, Some(RunnerSortByArg::LastUpdated));
}

#[test]
fn list_accepts_jq_filter() {
    let command = parse_command(&["list", "--jq", ".[]"]);
    let RunnerCommand::List(args) = command else {
        panic!("Expected list command");
    };
    assert!(args.json_output.filter.is_some());
    assert!(args.json_output.force_json_output());
}

#[test]
fn create_requires_name() {
    let err = parse_command_err(&["create"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn create_parses_basic_args() {
    let command = parse_command(&["create", "--name", "my-runner"]);
    let RunnerCommand::Create(args) = command else {
        panic!("Expected create command");
    };
    assert_eq!(args.name, "my-runner");
    // Scope defaults to neither flag set; the SDK resolves this to Team.
    assert!(!args.scope.team);
    assert!(!args.scope.personal);
    // OS/arch have defaults.
    assert_eq!(args.os, RunnerOsArg::Linux);
    assert_eq!(args.arch, RunnerArchArg::X8664);
}

#[test]
fn create_rejects_description_over_limit() {
    let long = "x".repeat(241);
    let err = parse_command_err(&["create", "--name", "r", "--description", &long]);
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn create_accepts_description_at_limit() {
    let at_limit = "x".repeat(240);
    let command = parse_command(&["create", "--name", "r", "--description", &at_limit]);
    let RunnerCommand::Create(args) = command else {
        panic!("Expected create command");
    };
    assert_eq!(args.description.as_deref(), Some(at_limit.as_str()));
}

#[test]
fn create_accepts_vcpus_and_memory_together() {
    let command = parse_command(&["create", "--name", "r", "--vcpus", "4", "--memory-gb", "16"]);
    let RunnerCommand::Create(args) = command else {
        panic!("Expected create command");
    };
    assert_eq!(args.vcpus, Some(4));
    assert_eq!(args.memory_gb, Some(16));
}

#[test]
fn create_rejects_vcpus_without_memory() {
    let err = parse_command_err(&["create", "--name", "r", "--vcpus", "4"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn create_rejects_memory_without_vcpus() {
    let err = parse_command_err(&["create", "--name", "r", "--memory-gb", "16"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn create_accepts_scope_personal() {
    let command = parse_command(&["create", "--name", "r", "--personal"]);
    let RunnerCommand::Create(args) = command else {
        panic!("Expected create command");
    };
    assert!(args.scope.personal);
    assert!(!args.scope.team);
}

#[test]
fn update_by_uid() {
    let command = parse_command(&["update", "runner-123", "--description", "updated"]);
    let RunnerCommand::Update(args) = command else {
        panic!("Expected update command");
    };
    assert_eq!(args.id.as_deref(), Some("runner-123"));
    assert!(args.name.is_none());
    assert_eq!(args.description.as_deref(), Some("updated"));
}

#[test]
fn update_by_name() {
    let command = parse_command(&["update", "--name", "my-runner", "--arch", "aarch64"]);
    let RunnerCommand::Update(args) = command else {
        panic!("Expected update command");
    };
    assert!(args.id.is_none());
    assert_eq!(args.name.as_deref(), Some("my-runner"));
    assert_eq!(args.arch, Some(RunnerArchArg::Aarch64));
}

#[test]
fn update_requires_identifier() {
    let err = parse_command_err(&["update", "--description", "updated"]);
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn delete_with_force() {
    let command = parse_command(&["delete", "runner-123", "--force"]);
    let RunnerCommand::Delete(args) = command else {
        panic!("Expected delete command");
    };
    assert_eq!(args.id, "runner-123");
    assert!(args.force);
}

#[test]
fn validate_os_config_rejects_macos_version_with_linux() {
    let err = validate_os_config(
        RunnerOsArg::Linux,
        None,
        Some(RunnerMacosVersionArg::Macos14),
    )
    .expect_err("macos-version with linux is rejected");
    assert!(err.contains("--macos-version"), "got: {err}");
}

#[test]
fn validate_os_config_rejects_docker_image_with_macos() {
    let err = validate_os_config(RunnerOsArg::Macos, Some("ubuntu:latest"), None)
        .expect_err("docker-image with macos is rejected");
    assert!(err.contains("--docker-image"), "got: {err}");
}

#[test]
fn validate_os_config_accepts_matching_linux() {
    validate_os_config(RunnerOsArg::Linux, Some("ubuntu:latest"), None)
        .expect("docker-image with linux is valid");
}

#[test]
fn validate_os_config_accepts_matching_macos() {
    validate_os_config(
        RunnerOsArg::Macos,
        None,
        Some(RunnerMacosVersionArg::Macos15),
    )
    .expect("macos-version with macos is valid");
}
