use clap::Parser as _;
use serde_json::json;
use warp_cli::agent::{AgentCommand, Harness, RunAgentArgs};
use warp_cli::artifact::{
    ArtifactCommand, DownloadArtifactArgs, GetArtifactArgs, UploadArtifactArgs,
};
use warp_cli::task::{MessageCommand, MessageSendArgs, MessageWatchArgs, TaskCommand};
use warp_cli::{Args, CliCommand, Command};
use warp_core::telemetry::TelemetryEvent;

use super::{
    CommandAuthentication, command_authentication, command_requires_auth,
    command_to_telemetry_event, computer_use_model_override, reconcile_task_harness,
};
use crate::ai::llms::LLMId;

const TASK_ID: &str = "00000000-0000-0000-0000-000000000001";

fn parse_run_args(extra_args: &[&str]) -> RunAgentArgs {
    let args = Args::try_parse_from(
        ["warp", "agent", "run", "--prompt", "hello"]
            .iter()
            .copied()
            .chain(extra_args.iter().copied()),
    )
    .expect("`warp agent run` args should parse");

    let Some(Command::CommandLine(command)) = args.command() else {
        panic!("Expected `warp agent run` command");
    };
    let CliCommand::Agent(AgentCommand::Run(run_args)) = command.as_ref() else {
        panic!("Expected `warp agent run` command");
    };
    run_args.clone()
}

#[test]
fn computer_use_model_flag_becomes_the_task_override_for_the_oz_harness() {
    let args = parse_run_args(&["--computer-use-model", "claude-4-5-sonnet"]);

    assert_eq!(
        computer_use_model_override(&args)
            .as_ref()
            .map(LLMId::as_str),
        Some("claude-4-5-sonnet")
    );
}

#[test]
fn computer_use_model_flag_is_ignored_for_third_party_harnesses() {
    let args = parse_run_args(&[
        "--harness",
        "claude",
        "--computer-use-model",
        "claude-4-5-sonnet",
    ]);

    assert_eq!(computer_use_model_override(&args), None);
}

#[test]
fn task_has_no_computer_use_model_override_without_the_flag() {
    let args = parse_run_args(&[]);

    assert_eq!(computer_use_model_override(&args), None);
}

#[test]
fn logout_does_not_require_auth() {
    assert!(!command_requires_auth(&CliCommand::Logout));
}

#[test]
fn login_does_not_require_auth() {
    assert!(!command_requires_auth(&CliCommand::Login));
}

#[test]
fn pending_api_key_is_selected_for_command_authentication() {
    assert_eq!(
        command_authentication(Some("api-key".to_owned()), false),
        Some(CommandAuthentication::PendingApiKey("api-key".to_owned()))
    );
}

#[test]
fn pending_api_key_takes_precedence_over_persisted_auth() {
    assert_eq!(
        command_authentication(Some("api-key".to_owned()), true),
        Some(CommandAuthentication::PendingApiKey("api-key".to_owned()))
    );
}

#[test]
fn persisted_auth_is_refreshed_without_pending_api_key() {
    assert_eq!(
        command_authentication(None, true),
        Some(CommandAuthentication::RefreshUser)
    );
}

#[test]
fn logged_out_command_has_no_authentication_source() {
    assert_eq!(command_authentication(None, false), None);
}

#[test]
fn artifact_download_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Download(DownloadArtifactArgs {
            artifact_uid: "artifact-123".to_string(),
            out: None,
        },)
    )));
}

#[test]
fn run_message_send_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Run(
        TaskCommand::Message(MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),)
    )));
}

#[test]
fn artifact_get_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Get(GetArtifactArgs {
            artifact_uid: "artifact-123".to_string(),
        },)
    )));
}

#[test]
fn artifact_upload_requires_auth() {
    assert!(command_requires_auth(&CliCommand::Artifact(
        ArtifactCommand::Upload(UploadArtifactArgs {
            path: "artifact.txt".into(),
            run_id: Some("run-123".to_string()),
            conversation_id: None,
            description: None,
        },)
    )));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_uses_canonical_harness_from_env() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "  CLAUDE  ") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "claude" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_supports_claude_code_alias() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "CLAUDE_CODE") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "claude" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_supports_opencode_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("OZ_HARNESS", "opencode") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };

    assert_eq!(event.payload(), Some(json!({ "harness": "opencode" })));
}

#[test]
#[serial_test::serial]
fn run_message_send_telemetry_defaults_to_unknown_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Send(MessageSendArgs {
            to: vec!["run-456".to_string()],
            subject: "subject".to_string(),
            body: "body".to_string(),
            sender_run_id: "run-123".to_string(),
        }),
    )));

    assert_eq!(event.payload(), Some(json!({ "harness": "unknown" })));
}

#[test]
fn reconcile_task_harness_adopts_task_harness_when_cli_uses_default() {
    let mut selected_harness = Harness::Oz;
    let harness = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect("default harness should adopt task harness");

    assert_eq!(selected_harness, Harness::Claude);
    assert_eq!(harness.harness(), Harness::Claude);
}

#[test]
fn reconcile_task_harness_allows_matching_explicit_harness() {
    let mut selected_harness = Harness::Claude;
    let harness = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect("matching harness should succeed");

    assert_eq!(selected_harness, Harness::Claude);
    assert_eq!(harness.harness(), Harness::Claude);
}

#[test]
fn reconcile_task_harness_rejects_explicit_mismatch() {
    let mut selected_harness = Harness::Gemini;
    let err = reconcile_task_harness(TASK_ID, &mut selected_harness, Harness::Claude)
        .expect_err("mismatched harness should fail");

    assert_eq!(selected_harness, Harness::Gemini);
    assert!(err.to_string().contains("Task"));
    assert!(err.to_string().contains("--harness gemini"));
    assert!(err.to_string().contains("claude"));
}

#[test]
#[serial_test::serial]
fn run_message_watch_telemetry_defaults_to_unknown_harness() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OZ_HARNESS") };
    let event = command_to_telemetry_event(&CliCommand::Run(TaskCommand::Message(
        MessageCommand::Watch(MessageWatchArgs {
            run_id: "run-123".to_string(),
            since_sequence: 0,
        }),
    )));

    assert_eq!(event.payload(), Some(json!({ "harness": "unknown" })));
}
