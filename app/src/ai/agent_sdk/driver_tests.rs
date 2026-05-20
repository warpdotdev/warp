use std::{collections::HashMap, ffi::OsString, fs, path::Path, sync::Arc, time::Duration};

use futures::channel::oneshot;
use warp_cli::agent::Harness;
use warp_cli::{
    OZ_CLI_ENV, OZ_HARNESS_ENV, OZ_PARENT_RUN_ID_ENV, OZ_RUN_ID_ENV, SERVER_ROOT_URL_OVERRIDE_ENV,
    SESSION_SHARING_SERVER_URL_OVERRIDE_ENV, WS_SERVER_URL_OVERRIDE_ENV,
};
use warp_core::channel::ChannelState;

use repo_metadata::{DirectoryWatcher, RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier};
use tempfile::TempDir;
use warp_cli::skill::SkillSpec;
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, SingletonEntity as _};

use super::{
    build_secret_env_vars, AgentDriver, IdleTimeoutSender,
    LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV, LEGACY_OZ_PARENT_STATE_ROOT_ENV,
    OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV, OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
};
use crate::ai::agent::{
    task::TaskId, AIAgentActionResult, AIAgentActionResultType, AIAgentInput, AIAgentOutput,
    AIAgentOutputMessage, ArtifactCreatedData, MessageId, UploadArtifactResult,
};
use crate::ai::mcp::parsing::normalize_mcp_json;
use crate::ai::{agent_sdk::task_env_vars, ambient_agents::AmbientAgentTaskId};
use crate::{
    ai::{cloud_environments::GithubRepo, skills::SkillManager},
    test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view},
};
use warp_managed_secrets::ManagedSecretValue;

#[test]
fn test_normalize_single_cli_server() {
    let input = r#"{"command": "npx", "args": ["-y", "mcp-server"]}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should wrap with a generated name
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(server["command"].as_str().unwrap(), "npx");
}

#[test]
fn test_normalize_single_sse_server() {
    let input = r#"{"url": "http://localhost:3000/mcp", "headers": {"API_KEY": "value"}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should wrap with a generated name
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(server["url"].as_str().unwrap(), "http://localhost:3000/mcp");
}

#[test]
fn test_normalize_already_wrapped_server() {
    let input = r#"{"my-server": {"command": "npx", "args": []}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should return as-is (no command/url at top level)
    assert_eq!(result, input);
}

#[test]
fn test_normalize_mcp_servers_wrapper() {
    let input = r#"{"mcpServers": {"server-name": {"command": "npx", "args": []}}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should return as-is (no command/url at top level)
    assert_eq!(result, input);
}

#[test]
fn test_normalize_servers_wrapper() {
    let input = r#"{"servers": {"server-name": {"url": "http://example.com"}}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should return as-is (no command/url at top level)
    assert_eq!(result, input);
}

#[test]
fn test_normalize_invalid_json() {
    let input = "not valid json";
    let result = normalize_mcp_json(input);

    assert!(result.is_err());
}

#[test]
fn test_normalize_cli_server_with_env() {
    let input = r#"{"command": "npx", "args": ["-y", "mcp-server"], "env": {"API_KEY": "secret"}}"#;
    let result = normalize_mcp_json(input).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(server["env"]["API_KEY"].as_str().unwrap(), "secret");
}

#[test]
fn test_normalize_sse_server_with_headers() {
    let input =
        r#"{"url": "http://localhost:5000/mcp", "headers": {"Authorization": "Bearer token"}}"#;
    let result = normalize_mcp_json(input).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(
        server["headers"]["Authorization"].as_str().unwrap(),
        "Bearer token"
    );
}

// ── IdleTimeoutSender tests ──────────────────────────────────────────────────────

#[test]
fn idle_timeout_sender_send_now_delivers_value() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_now(42);
    assert_eq!(rx.try_recv().unwrap(), Some(42));
}

#[test]
fn idle_timeout_sender_send_now_only_delivers_once() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_now(1);
    idle_timeout.end_run_now(2);
    assert_eq!(rx.try_recv().unwrap(), Some(1));
}

#[test]
fn idle_timeout_sender_send_after_delivers_after_timeout() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_after(Duration::from_millis(50), 99);

    // Not yet delivered.
    assert_eq!(rx.try_recv().unwrap(), None);

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rx.try_recv().unwrap(), Some(99));
}

#[test]
fn idle_timeout_sender_cancel_prevents_delivery() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_after(Duration::from_millis(50), 99);
    idle_timeout.cancel_idle_timeout();

    std::thread::sleep(Duration::from_millis(100));
    // Sender was not consumed, so the channel is still open but empty.
    assert_eq!(rx.try_recv().unwrap(), None);
}

#[test]
fn idle_timeout_sender_cancel_then_send_now_delivers() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_after(Duration::from_millis(50), 1);
    idle_timeout.cancel_idle_timeout();
    idle_timeout.end_run_now(2);

    assert_eq!(rx.try_recv().unwrap(), Some(2));
}

#[test]
fn idle_timeout_sender_later_send_after_supersedes_earlier() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    // First timer: long timeout.
    idle_timeout.end_run_after(Duration::from_secs(10), 1);
    // Second timer: short timeout. The first is implicitly cancelled.
    idle_timeout.end_run_after(Duration::from_millis(50), 2);

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rx.try_recv().unwrap(), Some(2));
}

#[test]
fn task_env_vars_include_parent_run_id_when_present() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), Some("parent-run-123"), Harness::Claude);
    let overrides_allowed = ChannelState::channel().allows_server_url_overrides();

    assert_eq!(
        env_vars.get(&OsString::from(OZ_RUN_ID_ENV)),
        Some(&OsString::from(task_id.to_string()))
    );
    assert_eq!(
        env_vars.get(&OsString::from(OZ_PARENT_RUN_ID_ENV)),
        Some(&OsString::from("parent-run-123"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(OZ_HARNESS_ENV)),
        Some(&OsString::from("claude"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV)),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(
            LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV
        )),
        Some(&OsString::from("1"))
    );
    assert!(env_vars
        .get(&OsString::from(OZ_CLI_ENV))
        .is_some_and(|value| !value.is_empty()));

    let server_root_url = ChannelState::server_root_url().into_owned();
    if overrides_allowed && !server_root_url.is_empty() {
        assert_eq!(
            env_vars.get(&OsString::from(SERVER_ROOT_URL_OVERRIDE_ENV)),
            Some(&OsString::from(server_root_url))
        );
    } else {
        assert!(!env_vars.contains_key(&OsString::from(SERVER_ROOT_URL_OVERRIDE_ENV)));
    }

    let ws_server_url = ChannelState::ws_server_url().into_owned();
    if overrides_allowed && !ws_server_url.is_empty() {
        assert_eq!(
            env_vars.get(&OsString::from(WS_SERVER_URL_OVERRIDE_ENV)),
            Some(&OsString::from(ws_server_url))
        );
    } else {
        assert!(!env_vars.contains_key(&OsString::from(WS_SERVER_URL_OVERRIDE_ENV)));
    }

    if overrides_allowed {
        match ChannelState::session_sharing_server_url() {
            Some(url) if !url.is_empty() => assert_eq!(
                env_vars.get(&OsString::from(SESSION_SHARING_SERVER_URL_OVERRIDE_ENV)),
                Some(&OsString::from(url.into_owned()))
            ),
            _ => {
                assert!(!env_vars
                    .contains_key(&OsString::from(SESSION_SHARING_SERVER_URL_OVERRIDE_ENV)))
            }
        }
    } else {
        assert!(!env_vars.contains_key(&OsString::from(SESSION_SHARING_SERVER_URL_OVERRIDE_ENV)));
    }
}

#[test]
fn task_env_vars_omit_parent_run_id_when_absent() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440001".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), None, Harness::Oz);
    let overrides_allowed = ChannelState::channel().allows_server_url_overrides();

    assert_eq!(
        env_vars.get(&OsString::from(OZ_RUN_ID_ENV)),
        Some(&OsString::from(task_id.to_string()))
    );
    assert!(!env_vars.contains_key(&OsString::from(OZ_PARENT_RUN_ID_ENV)));
    assert_eq!(
        env_vars.get(&OsString::from(OZ_HARNESS_ENV)),
        Some(&OsString::from("oz"))
    );
    assert!(!env_vars.contains_key(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV)));
    assert!(!env_vars.contains_key(&OsString::from(
        LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV
    )));
    assert_eq!(
        env_vars.contains_key(&OsString::from(SERVER_ROOT_URL_OVERRIDE_ENV)),
        overrides_allowed && !ChannelState::server_root_url().is_empty()
    );
    assert_eq!(
        env_vars.contains_key(&OsString::from(WS_SERVER_URL_OVERRIDE_ENV)),
        overrides_allowed && !ChannelState::ws_server_url().is_empty()
    );
}

#[test]
fn task_env_vars_enable_external_parent_listener_for_claude_runs_without_parent_run_id() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440002".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), None, Harness::Claude);
    assert_eq!(
        env_vars.get(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV)),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(
            LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV
        )),
        Some(&OsString::from("1"))
    );
}

#[test]
#[serial_test::serial]
fn task_env_vars_propagate_message_listener_state_root_with_legacy_alias() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440003".parse().unwrap();
    std::env::set_var(
        OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
        "/tmp/message-listener-root",
    );
    let env_vars = task_env_vars(Some(&task_id), None, Harness::Claude);
    std::env::remove_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV);

    assert_eq!(
        env_vars.get(&OsString::from(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV)),
        Some(&OsString::from("/tmp/message-listener-root"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(LEGACY_OZ_PARENT_STATE_ROOT_ENV)),
        Some(&OsString::from("/tmp/message-listener-root"))
    );
}

#[test]
fn task_env_vars_can_use_opencode_harness() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440004".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), Some("parent-run-456"), Harness::OpenCode);

    assert_eq!(
        env_vars.get(&OsString::from(OZ_HARNESS_ENV)),
        Some(&OsString::from("opencode"))
    );
}

#[test]
fn json_format_output_includes_filename_for_file_artifact_created_event() {
    let output = AIAgentOutput {
        messages: vec![AIAgentOutputMessage::artifact_created(
            MessageId::new("message-1".to_string()),
            ArtifactCreatedData::File {
                artifact_uid: "artifact-uid".to_string(),
                filepath: "outputs/report.txt".to_string(),
                filename: "report.txt".to_string(),
                mime_type: "text/plain".to_string(),
                description: Some("Build output for the latest run".to_string()),
                size_bytes: 42,
            },
        )],
        ..Default::default()
    };

    let mut bytes = Vec::new();
    super::output::json::format_output(&output, &mut bytes).expect("json formatting should work");

    let value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("output should be valid json");

    assert_eq!(value["type"], "artifact_created");
    assert_eq!(value["artifact_type"], "file");
    assert_eq!(value["artifact_uid"], "artifact-uid");
    assert_eq!(value["filepath"], "outputs/report.txt");
    assert_eq!(value["filename"], "report.txt");
    assert_eq!(value["mime_type"], "text/plain");
    assert_eq!(value["description"], "Build output for the latest run");
    assert_eq!(value["size_bytes"], 42);
}

#[test]
fn json_format_input_omits_filepath_and_description_for_proto_upload_result() {
    let input = AIAgentInput::ActionResult {
        result: AIAgentActionResult {
            id: "tool-call-1".to_string().into(),
            task_id: TaskId::new("task-1".to_string()),
            result: AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Success {
                artifact_uid: "artifact-123".to_string(),
                filepath: None,
                mime_type: "text/plain".to_string(),
                description: None,
                size_bytes: 42,
            }),
        },
        context: Arc::from([]),
    };

    let mut bytes = Vec::new();
    super::output::json::format_input(&input, &mut bytes).expect("json formatting should work");

    let value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("output should be valid json");

    assert_eq!(value["type"], "tool_result");
    assert_eq!(value["tool"], "upload_artifact");
    assert_eq!(value["artifact_uid"], "artifact-123");
    assert_eq!(value["mime_type"], "text/plain");
    assert_eq!(value["size_bytes"], 42);
    assert!(value.get("filepath").is_none());
    assert!(value.get("description").is_none());
}

// ── build_secret_env_vars tests ──────────────────────────────────────────────

#[test]
#[serial_test::serial]
fn raw_value_only_writes_under_secret_name() {
    std::env::remove_var("MY_SECRET");
    let secrets = HashMap::from([(
        "MY_SECRET".to_string(),
        ManagedSecretValue::raw_value("s3cret"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("MY_SECRET")),
        Some(&OsString::from("s3cret"))
    );
    assert_eq!(env_vars.len(), 1);
}

#[test]
#[serial_test::serial]
fn anthropic_api_key_writes_anthropic_env_var() {
    std::env::remove_var("ANTHROPIC_API_KEY");
    let secrets = HashMap::from([(
        "my-custom-name".to_string(),
        ManagedSecretValue::anthropic_api_key("sk-ant-test-key"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("ANTHROPIC_API_KEY")),
        Some(&OsString::from("sk-ant-test-key"))
    );
}

#[test]
#[serial_test::serial]
fn typed_secret_overrides_raw_value_with_same_env_name() {
    std::env::remove_var("ANTHROPIC_API_KEY");
    let typed_key = "sk-ant-typed-key-abcdef";
    let raw_key = "sk-ant-raw-key-ghijkl";
    let secrets = HashMap::from([
        (
            "my-auth".to_string(),
            ManagedSecretValue::anthropic_api_key(typed_key),
        ),
        (
            "ANTHROPIC_API_KEY".to_string(),
            ManagedSecretValue::raw_value(raw_key),
        ),
    ]);
    // Run multiple times to defeat HashMap iteration order flakiness.
    for _ in 0..20 {
        let env_vars = build_secret_env_vars(&secrets);
        assert_eq!(
            env_vars.get(&OsString::from("ANTHROPIC_API_KEY")),
            Some(&OsString::from(typed_key)),
            "Typed secret must always override RawValue with the same env name"
        );
    }
}

#[test]
#[serial_test::serial]
fn bedrock_api_key_writes_all_bedrock_env_vars() {
    std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK");
    std::env::remove_var("CLAUDE_CODE_USE_BEDROCK");
    std::env::remove_var("AWS_REGION");
    let secrets = HashMap::from([
        (
            "bedrock-secret".to_string(),
            ManagedSecretValue::anthropic_bedrock_api_key("token-123", "us-west-2"),
        ),
        (
            "AWS_REGION".to_string(),
            ManagedSecretValue::raw_value("eu-west-1"),
        ),
    ]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("AWS_BEARER_TOKEN_BEDROCK")),
        Some(&OsString::from("token-123"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("CLAUDE_CODE_USE_BEDROCK")),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_REGION")),
        Some(&OsString::from("us-west-2")),
        "Typed Bedrock secret should win over RawValue for AWS_REGION"
    );
}

#[test]
#[serial_test::serial]
fn bedrock_access_key_writes_all_aws_env_vars() {
    std::env::remove_var("AWS_ACCESS_KEY_ID");
    std::env::remove_var("AWS_SECRET_ACCESS_KEY");
    std::env::remove_var("AWS_SESSION_TOKEN");
    std::env::remove_var("CLAUDE_CODE_USE_BEDROCK");
    std::env::remove_var("AWS_REGION");
    let secrets = HashMap::from([(
        "bedrock-access".to_string(),
        ManagedSecretValue::anthropic_bedrock_access_key(
            "AKID",
            "secret-key",
            Some("session-tok".to_string()),
            "ap-southeast-1",
        ),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("AWS_ACCESS_KEY_ID")),
        Some(&OsString::from("AKID"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_SECRET_ACCESS_KEY")),
        Some(&OsString::from("secret-key"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_SESSION_TOKEN")),
        Some(&OsString::from("session-tok"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("CLAUDE_CODE_USE_BEDROCK")),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_REGION")),
        Some(&OsString::from("ap-southeast-1"))
    );
}

#[test]
#[serial_test::serial]
fn raw_value_skipped_when_process_env_already_set() {
    std::env::set_var("WORKER_TOKEN", "injected-value");
    let secrets = HashMap::from([(
        "WORKER_TOKEN".to_string(),
        ManagedSecretValue::raw_value("managed-value"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    // The worker-injected env var wins; env_vars should NOT contain it
    // because the child inherits the process env directly.
    assert!(!env_vars.contains_key(&OsString::from("WORKER_TOKEN")));
    std::env::remove_var("WORKER_TOKEN");
}

#[test]
#[serial_test::serial]
fn worker_injected_env_wins_over_typed_secret() {
    std::env::set_var("ANTHROPIC_API_KEY", "worker-key");
    let secrets = HashMap::from([(
        "my-auth".to_string(),
        ManagedSecretValue::anthropic_api_key("managed-key"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    // The typed secret should be skipped entirely; the child inherits
    // ANTHROPIC_API_KEY from the process env.
    assert!(!env_vars.contains_key(&OsString::from("ANTHROPIC_API_KEY")));
    std::env::remove_var("ANTHROPIC_API_KEY");
}

#[test]
#[serial_test::serial]
fn worker_injected_env_skips_entire_bedrock_secret() {
    // Only AWS_REGION is worker-injected; the entire Bedrock secret should
    // be atomically skipped — no partial insertion.
    std::env::set_var("AWS_REGION", "us-east-1");
    std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK");
    std::env::remove_var("CLAUDE_CODE_USE_BEDROCK");
    let secrets = HashMap::from([(
        "bedrock-secret".to_string(),
        ManagedSecretValue::anthropic_bedrock_api_key("token-456", "eu-central-1"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert!(
        !env_vars.contains_key(&OsString::from("AWS_BEARER_TOKEN_BEDROCK")),
        "Entire Bedrock secret must be skipped when any field is worker-injected"
    );
    assert!(!env_vars.contains_key(&OsString::from("CLAUDE_CODE_USE_BEDROCK")));
    assert!(!env_vars.contains_key(&OsString::from("AWS_REGION")));
    std::env::remove_var("AWS_REGION");
}

// ── Skill-loading integration test ───────────────────────────────────────────

/// Verifies that `load_environment_skills` loads every skill from an env repo
/// while `load_global_skills` loads only the explicitly requested subset from a
/// global-only repo.
///
/// The test writes real SKILL.md files on disk, seeds `RepoMetadataModel` with a
/// minimal file-tree for the env repo so the indexing wait resolves immediately,
/// and drives both loading methods through a live `AgentDriver` model via a
/// `ModelSpawner`.
#[test]
fn split_loading_env_loads_all_global_loads_subset() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // Create real skill files on disk.
        let temp = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(temp.path()).unwrap();

        // Environment repo: three skills. All should be loaded.
        let env_repo = working_dir.join("env-repo");
        write_skill_file(&env_repo, "build");
        write_skill_file(&env_repo, "test-skill");
        write_skill_file(&env_repo, "deploy");

        // Global-only repo: three skills; only "linter" is explicitly requested.
        let global_repo = working_dir.join("global-repo");
        write_skill_file(&global_repo, "linter");
        write_skill_file(&global_repo, "formatter");
        write_skill_file(&global_repo, "docs");

        // Trigger a real filesystem scan of the env repo so `repository_indexed`
        // resolves immediately once indexing completes.
        let env_repo_std = StandardizedPath::from_local_canonicalized(&env_repo).unwrap();
        let repo_handle = DirectoryWatcher::handle(&app).update(&mut app, |watcher, ctx| {
            watcher.add_directory(env_repo_std.clone(), ctx).unwrap()
        });
        let (indexed_tx, indexed_rx) = futures::channel::oneshot::channel::<()>();
        let tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(indexed_tx)));
        let env_repo_for_event = env_repo_std.clone();
        app.update(|ctx| {
            let tx_cell = tx_cell.clone();
            ctx.subscribe_to_model(
                &RepoMetadataModel::handle(ctx),
                move |_, event: &RepoMetadataEvent, _ctx| {
                    if let RepoMetadataEvent::RepositoryUpdated {
                        id: RepositoryIdentifier::Local(path),
                    } = event
                    {
                        if *path == env_repo_for_event {
                            if let Some(tx) = tx_cell.borrow_mut().take() {
                                let _ = tx.send(());
                            }
                        }
                    }
                },
            );
        });
        RepoMetadataModel::handle(&app).update(&mut app, |model: &mut RepoMetadataModel, ctx| {
            model.index_directory(repo_handle, ctx).unwrap();
        });
        indexed_rx.await.expect("env repo should be indexed");

        // Construct a minimal AgentDriver backed by a stub terminal view.
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver_handle = app.add_model(|ctx| {
            let terminal_driver =
                super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
            AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx)
        });

        // Run both loading methods through the driver's ModelSpawner.
        let (done_tx, done_rx) = futures::channel::oneshot::channel::<()>();
        let env_repos = vec![GithubRepo::new("org".to_string(), "env-repo".to_string())];
        let global_repos = vec![GithubRepo::new(
            "org".to_string(),
            "global-repo".to_string(),
        )];
        let global_specs: Vec<SkillSpec> = ["org/global-repo:linter".to_string()]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        driver_handle.update(&mut app, |_, ctx| {
            let spawner = ctx.spawner();
            ctx.spawn(
                async move {
                    AgentDriver::load_environment_skills(&spawner, env_repos).await;
                    AgentDriver::load_global_skills(&spawner, global_specs, global_repos).await;
                    let _ = done_tx.send(());
                },
                |_, _, _| {},
            );
        });
        done_rx.await.expect("loading task should complete");

        // Verify SkillManager contains the right skills.
        // is_cloud_environment=true (set by both loaders), so get_skills_for_working_directory
        // with cwd=None returns all registered skills.
        let skill_names = SkillManager::handle(&app).read(&app, |manager: &SkillManager, ctx| {
            manager
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        });

        assert!(
            skill_names.contains(&"build".to_string()),
            "env skill 'build' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"test-skill".to_string()),
            "env skill 'test-skill' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"deploy".to_string()),
            "env skill 'deploy' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"linter".to_string()),
            "requested global skill 'linter' should be loaded; got: {skill_names:?}"
        );
        assert!(
            !skill_names.contains(&"formatter".to_string()),
            "unrequested global skill 'formatter' should NOT be loaded; got: {skill_names:?}"
        );
        assert!(
            !skill_names.contains(&"docs".to_string()),
            "unrequested global skill 'docs' should NOT be loaded; got: {skill_names:?}"
        );
    });
}

/// Verifies that when a repo is in both the environment list and the global skill
/// specs, all skills from that repo are loaded (environment wins), the targeted
/// global skill is present, and no skill is registered more than once.
#[test]
fn overlap_repo_in_env_and_global_loads_all_skills_without_duplicates() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let temp = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(temp.path()).unwrap();

        // A single repo with three skills, appearing in both the environment
        // and a global spec that targets only one of them.
        let shared_repo = working_dir.join("shared-repo");
        write_skill_file(&shared_repo, "deploy");
        write_skill_file(&shared_repo, "lint");
        write_skill_file(&shared_repo, "test-cmd");

        // Index the repo so `load_environment_skills` can scan it.
        let shared_repo_std = StandardizedPath::from_local_canonicalized(&shared_repo).unwrap();
        let repo_handle = DirectoryWatcher::handle(&app).update(&mut app, |watcher, ctx| {
            watcher.add_directory(shared_repo_std.clone(), ctx).unwrap()
        });
        let (indexed_tx, indexed_rx) = futures::channel::oneshot::channel::<()>();
        let tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(indexed_tx)));
        let shared_repo_for_event = shared_repo_std.clone();
        app.update(|ctx| {
            let tx_cell = tx_cell.clone();
            ctx.subscribe_to_model(
                &RepoMetadataModel::handle(ctx),
                move |_, event: &RepoMetadataEvent, _ctx| {
                    if let RepoMetadataEvent::RepositoryUpdated {
                        id: RepositoryIdentifier::Local(path),
                    } = event
                    {
                        if *path == shared_repo_for_event {
                            if let Some(tx) = tx_cell.borrow_mut().take() {
                                let _ = tx.send(());
                            }
                        }
                    }
                },
            );
        });
        RepoMetadataModel::handle(&app).update(&mut app, |model: &mut RepoMetadataModel, ctx| {
            model.index_directory(repo_handle, ctx).unwrap();
        });
        indexed_rx.await.expect("shared repo should be indexed");

        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver_handle = app.add_model(|ctx| {
            let terminal_driver =
                super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
            AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx)
        });

        // The same repo is listed in both env repos and global repos.
        // The global spec targets only "deploy".
        let (done_tx, done_rx) = futures::channel::oneshot::channel::<()>();
        let env_repos = vec![GithubRepo::new(
            "org".to_string(),
            "shared-repo".to_string(),
        )];
        let global_repos = vec![GithubRepo::new(
            "org".to_string(),
            "shared-repo".to_string(),
        )];
        let global_specs: Vec<SkillSpec> = ["org/shared-repo:deploy".to_string()]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        driver_handle.update(&mut app, |_, ctx| {
            let spawner = ctx.spawner();
            ctx.spawn(
                async move {
                    AgentDriver::load_environment_skills(&spawner, env_repos).await;
                    AgentDriver::load_global_skills(&spawner, global_specs, global_repos).await;
                    let _ = done_tx.send(());
                },
                |_, _, _| {},
            );
        });
        done_rx.await.expect("loading task should complete");

        let skill_names = SkillManager::handle(&app).read(&app, |manager: &SkillManager, ctx| {
            manager
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        });

        // All three skills from the repo are present (env loading wins).
        assert!(
            skill_names.contains(&"deploy".to_string()),
            "'deploy' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"lint".to_string()),
            "'lint' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"test-cmd".to_string()),
            "'test-cmd' should be loaded; got: {skill_names:?}"
        );

        // No skill is duplicated.
        let deploy_count = skill_names.iter().filter(|n| *n == "deploy").count();
        assert_eq!(
            deploy_count, 1,
            "'deploy' should appear exactly once; got: {skill_names:?}"
        );
    });
}

/// Write a minimal SKILL.md at `{repo}/.agents/skills/{name}/SKILL.md`.
/// The name is derived from the parent directory name, so no frontmatter is required.
fn write_skill_file(repo: &Path, name: &str) {
    let skill_dir = repo.join(".agents").join("skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), format!("Skill: {name}.")).unwrap();
}

#[test]
#[serial_test::serial]
fn openai_api_key_exports_only_api_key_not_base_url() {
    // The OpenAI typed secret should only export OPENAI_API_KEY as an env var.
    // base_url is piped through the structured secret to the harness instead.
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("OPENAI_BASE_URL");
    let secrets = HashMap::from([(
        "openai-key".to_string(),
        ManagedSecretValue::openai_api_key(
            "sk-test-key",
            Some("https://us.api.openai.com/v1".to_string()),
        ),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("OPENAI_API_KEY")),
        Some(&OsString::from("sk-test-key")),
        "OPENAI_API_KEY should be exported from the typed secret"
    );
    assert!(
        !env_vars.contains_key(&OsString::from("OPENAI_BASE_URL")),
        "OPENAI_BASE_URL should NOT be exported as an env var"
    );
}
