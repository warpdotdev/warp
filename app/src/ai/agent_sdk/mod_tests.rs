use std::sync::Arc;

use clap::Parser;
use serde_json::json;
use warp_cli::agent::{AgentCommand, Harness, RunAgentArgs};
use warp_cli::artifact::{
    ArtifactCommand, DownloadArtifactArgs, GetArtifactArgs, UploadArtifactArgs,
};
use warp_cli::task::{MessageCommand, MessageSendArgs, MessageWatchArgs, TaskCommand};
use warp_cli::{Args, CliCommand, Command};
use warp_core::telemetry::TelemetryEvent;
use warpui::{App, SingletonEntity, WindowId};

use super::{
    AgentDriverRunner, CommandAuthentication, command_authentication, command_requires_auth,
    command_to_telemetry_event, reconcile_task_harness, resolve_local_run_team_scopes,
};
use crate::ai::agent_sdk::driver::AgentDriverOptions;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, AgentConfigSnapshot, MockAIClient};
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::user_workspaces::{TeamScope, UserWorkspaces};
use crate::workspaces::workspace::{Workspace, WorkspaceUid};

const TASK_ID: &str = "00000000-0000-0000-0000-000000000001";

fn parse_run_agent_args(args: &[&str]) -> RunAgentArgs {
    let parsed = Args::try_parse_from(std::iter::once("warp").chain(args.iter().copied()))
        .expect("agent run args should parse");
    let Some(Command::CommandLine(command)) = parsed.command() else {
        panic!("expected a CLI command");
    };
    let CliCommand::Agent(AgentCommand::Run(args)) = command.as_ref() else {
        panic!("expected `agent run`");
    };
    args.clone()
}

fn team(uid: i64, name: &str) -> Team {
    Team {
        uid: ServerId::from(uid),
        name: name.to_string(),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        feature_model_choice: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
}

fn initialize_team_scope_test_app(app: &mut App, teams: Vec<Team>) {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(1)),
        "Workspace".to_string(),
        None,
        None,
    );
    workspace.teams = teams;
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![workspace],
            ctx,
        )
    });
}

fn agent_driver_options() -> AgentDriverOptions {
    AgentDriverOptions {
        working_dir: std::env::current_dir().unwrap(),
        task_id: None,
        parent_run_id: None,
        should_share: false,
        idle_on_complete: None,
        idle_on_fail: None,
        secrets: Default::default(),
        resume: None,
        cloud_providers: vec![],
        environment: None,
        additional_source_repos: vec![],
        repository_head_overrides: vec![],
        remove_repository_origins: false,
        selected_harness: Harness::Oz,
        third_party_harness_model_config: None,
        team_scope: None,
        snapshot_disabled: None,
        snapshot_upload_timeout: None,
        snapshot_script_timeout: None,
        checkpoint_interval: None,
        skip_initial_turn: false,
        strict_mcp_startup: false,
        mcp_startup_timeout: None,
    }
}

#[test]
fn multi_team_run_passes_selected_team_to_task_creation_and_headless_window() {
    App::test((), |mut app| async move {
        let first_team = team(7, "First");
        let first_team_uid = first_team.uid;
        let selected_team = team(8, "Selected");
        let selected_team_uid = selected_team.uid;
        initialize_team_scope_test_app(&mut app, vec![first_team, selected_team]);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        let window_id = WindowId::new();
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.register_window(window_id, Some(first_team_uid), ctx);
        });
        let args = parse_run_agent_args(&[
            "agent",
            "run",
            "--prompt",
            "hello",
            &format!("--team={selected_team_uid}"),
        ]);
        let team_scopes = app
            .read(|ctx| resolve_local_run_team_scopes(&args, ctx))
            .unwrap()
            .expect("new local run should resolve scopes");
        assert_eq!(team_scopes.resolved.team_uid(), Some(selected_team_uid));

        let mut ai_client = MockAIClient::new();
        ai_client
            .expect_create_agent_task()
            .times(1)
            .withf(move |_, _, _, _, scope| scope.team_uid() == Some(selected_team_uid))
            .returning(|_, _, _, _, _| Ok(TASK_ID.parse().unwrap()));
        let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
        let mut driver_options = agent_driver_options();
        let runner = app.add_singleton_model(|_| AgentDriverRunner);
        let foreground = runner.update(&mut app, |_, ctx| ctx.spawner());

        AgentDriverRunner::initialize_new_task(
            &foreground,
            &ai_client,
            "hello".to_string(),
            AgentConfigSnapshot::default(),
            team_scopes,
            &mut driver_options,
        )
        .await
        .unwrap();

        assert_eq!(
            driver_options
                .team_scope
                .as_deref()
                .and_then(TeamScope::team_uid),
            Some(selected_team_uid)
        );
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_team_for_window_from_scope(
                window_id,
                driver_options.team_scope.as_deref().unwrap(),
                ctx,
            );
        });
        assert_eq!(
            app.read(|ctx| UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id)),
            Some(selected_team_uid)
        );
    });
}

#[test]
fn task_id_run_skips_cli_team_resolution_and_new_run_scopes() {
    let args = parse_run_agent_args(&[
        "agent",
        "run",
        "--task-id",
        TASK_ID,
        "--team=not-a-team-uid",
    ]);

    App::test((), |app| async move {
        assert!(
            app.read(|ctx| resolve_local_run_team_scopes(&args, ctx))
                .unwrap()
                .is_none()
        );
    });
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
