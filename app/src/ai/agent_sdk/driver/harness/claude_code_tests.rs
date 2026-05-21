use mockall::predicate::eq;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;
use uuid::Uuid;
use warp_cli::{OZ_HARNESS_ENV, OZ_PARENT_RUN_ID_ENV, OZ_RUN_ID_ENV};

use super::*;
use crate::ai::agent_events::{AgentMessageEventMetadata, MessageHydrator};
use crate::ai::agent_sdk::driver::harness::claude_transcript::encode_cwd;
use crate::ai::agent_sdk::driver::OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV;
use crate::server::server_api::ai::{AIClient, MockAIClient, ReadAgentMessageResponse};
use crate::server::server_api::ServerApiProvider;

fn sample_parent_bridge_message(
    sequence: i64,
    message_id: &str,
    subject: &str,
    body: &str,
) -> MessageBridgeMessageRecord {
    MessageBridgeMessageRecord {
        sequence,
        message_id: message_id.to_string(),
        sender_run_id: "parent-run-456".to_string(),
        subject: subject.to_string(),
        body: body.to_string(),
        occurred_at: "2026-04-17T15:46:00Z".to_string(),
    }
}

fn sample_staged_parent_bridge_message(
    sequence: i64,
    message_id: &str,
) -> MessageBridgeMessageRecord {
    MessageBridgeMessageRecord {
        sequence,
        message_id: message_id.to_string(),
        sender_run_id: String::new(),
        subject: String::new(),
        body: String::new(),
        occurred_at: "2026-04-17T15:46:00Z".to_string(),
    }
}

fn write_surfaced_parent_bridge_message(state_dir: &Path, record: &MessageBridgeMessageRecord) {
    fs::write(
        parent_bridge_surfaced_message_path(state_dir, record.sequence, &record.message_id),
        serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

#[test]
fn claude_command_uses_session_id_when_not_resuming() {
    let uuid = Uuid::new_v4();
    let cmd = claude_command("claude", &uuid, "/tmp/prompt.txt", None, None, false);
    assert!(
        cmd.contains(&format!("--session-id {uuid}")),
        "expected --session-id flag in non-resume command, got: {cmd}"
    );
    assert!(
        !cmd.contains("--resume"),
        "non-resume command should not contain --resume, got: {cmd}"
    );
}

#[test]
fn claude_command_uses_resume_flag_when_resuming() {
    let uuid = Uuid::new_v4();
    let cmd = claude_command("claude", &uuid, "/tmp/prompt.txt", None, None, true);
    assert!(
        cmd.contains(&format!("--resume {uuid}")),
        "expected --resume flag in resume command, got: {cmd}"
    );
    assert!(
        !cmd.contains("--session-id"),
        "resume command should not contain --session-id, got: {cmd}"
    );
}

#[test]
fn claude_command_pipes_prompt_path() {
    let uuid = Uuid::new_v4();
    let cmd = claude_command(
        "claude",
        &uuid,
        "/tmp/prompt with spaces.txt",
        None,
        None,
        true,
    );
    assert!(
        cmd.contains("< '/tmp/prompt with spaces.txt'"),
        "expected single-quoted stdin redirect of the prompt path, got: {cmd}"
    );
    assert!(
        cmd.contains("--dangerously-skip-permissions"),
        "expected --dangerously-skip-permissions, got: {cmd}"
    );
}

#[test]
fn write_session_index_entry_creates_expected_entry() {
    let tmp = TempDir::new().unwrap();
    let cwd = Path::new("/my/project");
    let session_id = Uuid::new_v4();

    write_session_index_entry(session_id, cwd, tmp.path()).unwrap();

    let index_path = tmp.path().join("sessions-index.json");
    let index: Value = serde_json::from_slice(&fs::read(index_path).unwrap()).unwrap();
    let session_key = session_id.to_string();
    let entry = &index[&session_key];
    let encoded = encode_cwd(cwd);

    assert_eq!(entry["sessionId"], Value::String(session_key.clone()));
    assert_eq!(
        entry["cwd"],
        Value::String(cwd.to_string_lossy().into_owned())
    );
    assert_eq!(entry["projectPath"], Value::String(encoded.clone()));
    assert_eq!(
        entry["transcriptPath"],
        Value::String(format!("projects/{encoded}/{session_id}.jsonl"))
    );
}

#[test]
fn serialize_claude_mcp_config_cli_server() {
    let servers = HashMap::from([(
        "test-server".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::CLIServer {
                command: "node".to_string(),
                args: vec!["server.js".to_string()],
                env: HashMap::from([("API_KEY".to_string(), "secret".to_string())]),
                working_directory: None,
            },
        },
    )]);
    let json = serialize_claude_mcp_config(&servers).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let server = &parsed["mcpServers"]["test-server"];
    assert_eq!(server["type"], "stdio");
    assert_eq!(server["command"], "node");
    assert_eq!(server["args"][0], "server.js");
    assert_eq!(server["env"]["API_KEY"], "secret");
}

#[test]
fn serialize_claude_mcp_config_cli_server_with_cwd() {
    let servers = HashMap::from([(
        "test-server".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::CLIServer {
                command: "node".to_string(),
                args: vec!["server.js".to_string()],
                env: HashMap::new(),
                working_directory: Some("/opt/mcp".to_string()),
            },
        },
    )]);
    let json = serialize_claude_mcp_config(&servers).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let server = &parsed["mcpServers"]["test-server"];
    assert_eq!(server["cwd"], "/opt/mcp");
}

#[test]
fn serialize_claude_mcp_config_cli_server_omits_cwd_when_none() {
    let servers = HashMap::from([(
        "test-server".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::CLIServer {
                command: "node".to_string(),
                args: vec![],
                env: HashMap::new(),
                working_directory: None,
            },
        },
    )]);
    let json = serialize_claude_mcp_config(&servers).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let server = &parsed["mcpServers"]["test-server"];
    assert!(server.get("cwd").is_none());
}

#[test]
fn serialize_claude_mcp_config_sse_server() {
    let servers = HashMap::from([(
        "remote".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::SSEServer {
                url: "https://mcp.example.com".to_string(),
                headers: HashMap::from([("Authorization".to_string(), "Bearer tok".to_string())]),
            },
        },
    )]);
    let json = serialize_claude_mcp_config(&servers).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let server = &parsed["mcpServers"]["remote"];
    assert_eq!(server["type"], "http");
    assert_eq!(server["url"], "https://mcp.example.com");
    assert_eq!(server["headers"]["Authorization"], "Bearer tok");
}

#[test]
#[serial_test::serial]
fn parent_bridge_root_prefers_environment_override() {
    let tmp = TempDir::new().unwrap();
    std::env::set_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV, tmp.path());
    let root = parent_bridge_root().unwrap();
    std::env::remove_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV);

    assert_eq!(root, tmp.path());
}

#[test]
fn stage_parent_bridge_message_writes_message_record() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();
    let record = sample_staged_parent_bridge_message(42, "msg-123");

    stage_parent_bridge_message(&state_dir, &record).unwrap();

    let staged_path = parent_bridge_staged_message_path(&state_dir, 42, "msg-123");
    let staged_record: MessageBridgeMessageRecord =
        serde_json::from_slice(&fs::read(&staged_path).unwrap()).unwrap();
    assert_eq!(staged_record.sequence, 42);
    assert_eq!(staged_record.message_id, "msg-123");
    assert!(staged_record.sender_run_id.is_empty());
}

#[tokio::test]
async fn parent_bridge_event_cursor_defaults_to_zero_when_missing() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    assert_eq!(read_parent_bridge_event_cursor(&state_dir).unwrap(), 0);
    assert!(!parent_bridge_event_cursor_file(&state_dir).exists());
}

#[tokio::test]
async fn parent_bridge_event_cursor_round_trips() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    write_parent_bridge_event_cursor(&state_dir, 42).unwrap();

    assert_eq!(read_parent_bridge_event_cursor(&state_dir).unwrap(), 42);
    assert!(parent_bridge_event_cursor_file(&state_dir).exists());
}

#[test]
#[serial_test::serial]
fn message_bridge_cleanup_preserves_state_for_wakeable_runs() {
    let tmp = TempDir::new().unwrap();
    std::env::set_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV, tmp.path());

    let session_id = Uuid::new_v4();
    let bridge = MessageBridge::new("run-123".to_string(), session_id).unwrap();
    let state_dir = tmp.path().join(session_id.to_string());
    ensure_parent_bridge_state_dir(&state_dir).unwrap();
    let record = sample_staged_parent_bridge_message(42, "msg-123");
    stage_parent_bridge_message(&state_dir, &record).unwrap();
    write_parent_bridge_event_cursor(&state_dir, 42).unwrap();

    bridge
        .cleanup(MessageBridgeCleanupDisposition::PreserveState)
        .unwrap();
    std::env::remove_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV);

    assert!(state_dir.exists());
    assert!(parent_bridge_staged_message_path(&state_dir, 42, "msg-123").exists());
    assert_eq!(read_parent_bridge_event_cursor(&state_dir).unwrap(), 42);
}

#[test]
#[serial_test::serial]
fn message_bridge_cleanup_removes_state_for_non_wakeable_runs() {
    let tmp = TempDir::new().unwrap();
    std::env::set_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV, tmp.path());

    let session_id = Uuid::new_v4();
    let bridge = MessageBridge::new("run-123".to_string(), session_id).unwrap();
    let state_dir = tmp.path().join(session_id.to_string());
    ensure_parent_bridge_state_dir(&state_dir).unwrap();
    stage_parent_bridge_message(
        &state_dir,
        &sample_staged_parent_bridge_message(42, "msg-123"),
    )
    .unwrap();
    write_parent_bridge_event_cursor(&state_dir, 42).unwrap();

    bridge
        .cleanup(MessageBridgeCleanupDisposition::RemoveState)
        .unwrap();
    std::env::remove_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV);

    assert!(!state_dir.exists());
}

#[tokio::test]
async fn prepare_parent_bridge_hook_output_moves_selected_messages_to_surfaced_dir() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let first = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    stage_parent_bridge_message(
        &state_dir,
        &sample_staged_parent_bridge_message(42, "msg-123"),
    )
    .unwrap();
    stage_parent_bridge_message(
        &state_dir,
        &sample_staged_parent_bridge_message(43, "msg-456"),
    )
    .unwrap();

    let mut ai_client = MockAIClient::new();
    let expected_first = first.clone();
    ai_client
        .expect_read_agent_message()
        .with(eq("msg-123"))
        .times(1)
        .returning(move |_| {
            Ok(ReadAgentMessageResponse {
                message_id: expected_first.message_id.clone(),
                sender_run_id: expected_first.sender_run_id.clone(),
                subject: expected_first.subject.clone(),
                body: expected_first.body.clone(),
                sent_at: "2026-04-17T15:46:00Z".to_string(),
                delivered_at: None,
                read_at: Some("2026-04-17T15:46:02Z".to_string()),
            })
        });
    let hydrator = MessageHydrator::new(
        Arc::new(ai_client) as Arc<dyn crate::server::server_api::ai::AIClient>
    );

    let max_context_chars = parent_bridge_char_count(MESSAGE_BRIDGE_CONTEXT_PREAMBLE)
        + parent_bridge_char_count(&render_parent_bridge_message_block(&first));
    prepare_parent_bridge_hook_output(&hydrator, &state_dir, max_context_chars)
        .await
        .unwrap();

    let hook_output: MessageBridgeHookOutput =
        serde_json::from_slice(&fs::read(parent_bridge_hook_output_file(&state_dir)).unwrap())
            .unwrap();
    assert_eq!(hook_output.surfaced_count, 1);
    assert_eq!(hook_output.remaining_staged_count, 1);
    assert!(hook_output.additional_context.contains("Please pivot"));
    assert!(!hook_output.additional_context.contains("Second update"));
    let surfaced_path = parent_bridge_surfaced_message_path(&state_dir, 42, "msg-123");
    assert!(surfaced_path.exists());
    assert!(parent_bridge_staged_message_path(&state_dir, 43, "msg-456").exists());
    assert!(!parent_bridge_staged_message_path(&state_dir, 42, "msg-123").exists());
    let surfaced_record: MessageBridgeMessageRecord =
        serde_json::from_slice(&fs::read(&surfaced_path).unwrap()).unwrap();
    assert_eq!(surfaced_record.subject, first.subject);
    assert_eq!(surfaced_record.body, first.body);
}

#[tokio::test]
async fn acknowledge_parent_bridge_hook_output_marks_messages_delivered_and_clears_state() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let record = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    write_surfaced_parent_bridge_message(&state_dir, &record);
    fs::write(
        parent_bridge_hook_output_file(&state_dir),
        serde_json::to_vec(&MessageBridgeHookOutput {
            additional_context: "context".to_string(),
            remaining_staged_count: 0,
            surfaced_count: 1,
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(parent_bridge_hook_output_ack_file(&state_dir), "").unwrap();

    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_mark_message_delivered()
        .with(eq("msg-123"))
        .times(1)
        .returning(|_| Ok(()));
    let hydrator = MessageHydrator::new(
        Arc::new(ai_client) as Arc<dyn crate::server::server_api::ai::AIClient>
    );

    acknowledge_parent_bridge_hook_output(&hydrator, &state_dir)
        .await
        .unwrap();

    assert!(!parent_bridge_surfaced_message_path(&state_dir, 42, "msg-123").exists());
    assert!(!parent_bridge_hook_output_file(&state_dir).exists());
    assert!(!parent_bridge_hook_output_ack_file(&state_dir).exists());
}

#[tokio::test]
async fn prepare_parent_bridge_hook_output_reuses_surfaced_records_without_rehydrating() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let record = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    write_surfaced_parent_bridge_message(&state_dir, &record);

    let mut ai_client = MockAIClient::new();
    ai_client.expect_read_agent_message().times(0);
    let hydrator = MessageHydrator::new(
        Arc::new(ai_client) as Arc<dyn crate::server::server_api::ai::AIClient>
    );

    let max_context_chars = parent_bridge_char_count(MESSAGE_BRIDGE_CONTEXT_PREAMBLE)
        + parent_bridge_char_count(&render_parent_bridge_message_block(&record));
    prepare_parent_bridge_hook_output(&hydrator, &state_dir, max_context_chars)
        .await
        .unwrap();

    let hook_output: MessageBridgeHookOutput =
        serde_json::from_slice(&fs::read(parent_bridge_hook_output_file(&state_dir)).unwrap())
            .unwrap();
    assert_eq!(hook_output.surfaced_count, 1);
    assert_eq!(hook_output.remaining_staged_count, 0);
    assert!(hook_output.additional_context.contains(&record.subject));
}

#[tokio::test]
async fn prepare_parent_bridge_hook_output_truncates_single_large_message() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();
    stage_parent_bridge_message(
        &state_dir,
        &sample_staged_parent_bridge_message(42, "msg-123"),
    )
    .unwrap();

    let long_body = "x".repeat(200);
    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_read_agent_message()
        .with(eq("msg-123"))
        .times(1)
        .returning(move |_| {
            Ok(ReadAgentMessageResponse {
                message_id: "msg-123".to_string(),
                sender_run_id: "parent-run-456".to_string(),
                subject: "Please pivot".to_string(),
                body: long_body.clone(),
                sent_at: "2026-04-17T15:46:00Z".to_string(),
                delivered_at: None,
                read_at: Some("2026-04-17T15:46:02Z".to_string()),
            })
        });
    let hydrator = MessageHydrator::new(
        Arc::new(ai_client) as Arc<dyn crate::server::server_api::ai::AIClient>
    );

    let max_context_chars = parent_bridge_char_count(MESSAGE_BRIDGE_CONTEXT_PREAMBLE) + 48;
    prepare_parent_bridge_hook_output(&hydrator, &state_dir, max_context_chars)
        .await
        .unwrap();

    let hook_output: MessageBridgeHookOutput =
        serde_json::from_slice(&fs::read(parent_bridge_hook_output_file(&state_dir)).unwrap())
            .unwrap();
    assert_eq!(hook_output.surfaced_count, 1);
    assert!(
        hook_output.additional_context.ends_with("..."),
        "expected truncated context, got: {}",
        hook_output.additional_context
    );
    assert!(parent_bridge_char_count(&hook_output.additional_context) <= max_context_chars);
}

#[tokio::test]
async fn acknowledge_parent_bridge_hook_output_ignores_missing_ack_marker() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let record = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );
    write_surfaced_parent_bridge_message(&state_dir, &record);

    let mut ai_client = MockAIClient::new();
    ai_client.expect_mark_message_delivered().times(0);
    let hydrator = MessageHydrator::new(
        Arc::new(ai_client) as Arc<dyn crate::server::server_api::ai::AIClient>
    );

    acknowledge_parent_bridge_hook_output(&hydrator, &state_dir)
        .await
        .unwrap();

    assert!(parent_bridge_surfaced_message_path(&state_dir, 42, "msg-123").exists());
}

#[test]
fn prepare_claude_config_creates_config_file_without_api_suffix() {
    let tmp = TempDir::new().unwrap();
    let claude_json_path = tmp.path().join(".claude.json");
    let working_dir = tmp.path().join("workspace/project");

    prepare_claude_config(&claude_json_path, &working_dir, None).unwrap();

    let claude_config: Value =
        serde_json::from_slice(&fs::read(claude_json_path).unwrap()).unwrap();
    assert_eq!(claude_config["hasCompletedOnboarding"], Value::Bool(true));
    assert_eq!(
        claude_config["lspRecommendationDisabled"],
        Value::Bool(true)
    );
    let working_dir_key = working_dir.to_string_lossy().to_string();
    assert_eq!(
        claude_config["projects"][working_dir_key]["hasTrustDialogAccepted"],
        Value::Bool(true)
    );
    assert_eq!(claude_config.get("customApiKeyResponses"), None);
}

#[test]
fn prepare_claude_config_creates_config_file_with_api_suffix() {
    let tmp = TempDir::new().unwrap();
    let claude_json_path = tmp.path().join(".claude.json");
    let working_dir = tmp.path().join("workspace/project");

    prepare_claude_config(
        &claude_json_path,
        &working_dir,
        Some("QLWn-dUnuwQ-hIhDiAAA"),
    )
    .unwrap();

    let claude_config: Value =
        serde_json::from_slice(&fs::read(claude_json_path).unwrap()).unwrap();
    assert_eq!(
        claude_config["customApiKeyResponses"]["approved"],
        serde_json::json!(["QLWn-dUnuwQ-hIhDiAAA"]),
    );
}

#[test]
fn prepare_claude_config_merges_existing_config() {
    let tmp = TempDir::new().unwrap();
    let claude_json_path = tmp.path().join(".claude.json");
    fs::write(
        &claude_json_path,
        r#"{"theme":"dark","projects":{"/existing/project":{"allowedTools":["Bash"],"nested":{"value":2}}},"customApiKeyResponses":{"approved":["existing-suffix-12345"]}}"#,
    )
    .unwrap();

    let working_dir = tmp.path().join("workspace/project");
    prepare_claude_config(
        &claude_json_path,
        &working_dir,
        Some("new-suffix-1234567890"),
    )
    .unwrap();

    let claude_config: Value =
        serde_json::from_slice(&fs::read(claude_json_path).unwrap()).unwrap();
    assert_eq!(claude_config["theme"], "dark");
    assert_eq!(
        claude_config["lspRecommendationDisabled"],
        Value::Bool(true)
    );
    assert_eq!(
        claude_config["projects"]["/existing/project"]["allowedTools"],
        serde_json::json!(["Bash"])
    );
    assert_eq!(
        claude_config["projects"]["/existing/project"]["nested"]["value"],
        2
    );
    // Both existing and new suffixes should be present.
    assert_eq!(
        claude_config["customApiKeyResponses"]["approved"],
        serde_json::json!(["existing-suffix-12345", "new-suffix-1234567890"]),
    );
    let working_dir_key = working_dir.to_string_lossy().to_string();
    assert_eq!(
        claude_config["projects"][working_dir_key]["hasTrustDialogAccepted"],
        Value::Bool(true)
    );
}

#[test]
fn prepare_claude_config_no_duplicate_suffix() {
    let tmp = TempDir::new().unwrap();
    let claude_json_path = tmp.path().join(".claude.json");
    fs::write(
        &claude_json_path,
        r#"{"customApiKeyResponses":{"approved":["QLWn-dUnuwQ-hIhDiAAA"]}}"#,
    )
    .unwrap();

    let working_dir = tmp.path().join("workspace/project");
    prepare_claude_config(
        &claude_json_path,
        &working_dir,
        Some("QLWn-dUnuwQ-hIhDiAAA"),
    )
    .unwrap();

    let claude_config: Value =
        serde_json::from_slice(&fs::read(claude_json_path).unwrap()).unwrap();
    assert_eq!(
        claude_config["customApiKeyResponses"]["approved"],
        serde_json::json!(["QLWn-dUnuwQ-hIhDiAAA"]),
    );
}

#[test]
fn prepare_claude_config_none_suffix_preserves_existing_responses() {
    let tmp = TempDir::new().unwrap();
    let claude_json_path = tmp.path().join(".claude.json");
    fs::write(
        &claude_json_path,
        r#"{"customApiKeyResponses":{"approved":["existing-suffix-12345"],"rejected":["bad-key"]}}"#,
    )
    .unwrap();

    let working_dir = tmp.path().join("workspace/project");
    prepare_claude_config(&claude_json_path, &working_dir, None).unwrap();

    let claude_config: Value =
        serde_json::from_slice(&fs::read(claude_json_path).unwrap()).unwrap();
    assert_eq!(
        claude_config["customApiKeyResponses"]["approved"],
        serde_json::json!(["existing-suffix-12345"]),
    );
    assert_eq!(
        claude_config["customApiKeyResponses"]["rejected"],
        serde_json::json!(["bad-key"]),
    );
}

#[test]
#[serial_test::serial]
fn prepare_claude_environment_config_without_config_dir_uses_home_global_config() {
    let home_dir = TempDir::new().unwrap();
    let old_home = std::env::var_os("HOME");
    let old_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    std::env::set_var("HOME", home_dir.path());
    std::env::remove_var("CLAUDE_CONFIG_DIR");

    let working_dir = home_dir.path().join("workspace/project");
    prepare_claude_environment_config(&working_dir, &HashMap::new()).unwrap();

    assert!(home_dir.path().join(CLAUDE_JSON_FILE_NAME).exists());
    assert!(home_dir
        .path()
        .join(".claude")
        .join(CLAUDE_SETTINGS_FILE_NAME)
        .exists());
    assert!(!home_dir
        .path()
        .join(".claude")
        .join(CLAUDE_JSON_FILE_NAME)
        .exists());

    match old_home {
        Some(home) => std::env::set_var("HOME", home),
        None => std::env::remove_var("HOME"),
    }
    match old_config_dir {
        Some(dir) => std::env::set_var("CLAUDE_CONFIG_DIR", dir),
        None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
    }
}

#[test]
#[serial_test::serial]
fn prepare_claude_environment_config_with_config_dir_uses_dir_global_config() {
    let home_dir = TempDir::new().unwrap();
    let claude_config_dir = TempDir::new().unwrap();
    let old_home = std::env::var_os("HOME");
    let old_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    std::env::set_var("HOME", home_dir.path());
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_config_dir.path());

    let working_dir = home_dir.path().join("workspace/project");
    prepare_claude_environment_config(&working_dir, &HashMap::new()).unwrap();

    assert!(claude_config_dir
        .path()
        .join(CLAUDE_JSON_FILE_NAME)
        .exists());
    assert!(claude_config_dir
        .path()
        .join(CLAUDE_SETTINGS_FILE_NAME)
        .exists());
    assert!(!home_dir.path().join(CLAUDE_JSON_FILE_NAME).exists());

    match old_home {
        Some(home) => std::env::set_var("HOME", home),
        None => std::env::remove_var("HOME"),
    }
    match old_config_dir {
        Some(dir) => std::env::set_var("CLAUDE_CONFIG_DIR", dir),
        None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
    }
}
#[test]
#[serial_test::serial]
fn resolve_suffix_from_resolved_env_vars() {
    std::env::remove_var(ANTHROPIC_API_KEY_ENV);
    let key = "sk-ant-api03-abcdefghij1234567890ABCDEFGHIJ1234567890abcdefghij1234567890QLWn-dUnuwQ-hIhDiAAA";
    let resolved = HashMap::from([(OsString::from("ANTHROPIC_API_KEY"), OsString::from(key))]);
    let suffix = resolve_anthropic_api_key_suffix(&resolved);
    assert_eq!(suffix.as_deref(), Some("QLWn-dUnuwQ-hIhDiAAA"));
}

#[test]
#[serial_test::serial]
fn resolve_suffix_returns_none_for_short_key() {
    std::env::remove_var(ANTHROPIC_API_KEY_ENV);
    let resolved = HashMap::from([(OsString::from("ANTHROPIC_API_KEY"), OsString::from("short"))]);
    assert_eq!(resolve_anthropic_api_key_suffix(&resolved), None);
}

#[test]
#[serial_test::serial]
fn resolve_suffix_returns_none_when_empty() {
    std::env::remove_var(ANTHROPIC_API_KEY_ENV);
    assert_eq!(resolve_anthropic_api_key_suffix(&HashMap::new()), None);
}

#[test]
#[serial_test::serial]
fn prepare_local_wake_command_rehydrates_transcript_with_self_managed_listener() {
    let home_dir = TempDir::new().unwrap();
    let claude_config_dir = TempDir::new().unwrap();
    let bridge_state_root = TempDir::new().unwrap();
    let working_dir = home_dir.path().join("workspace/project");
    fs::create_dir_all(&working_dir).unwrap();

    std::env::set_var("HOME", home_dir.path());
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_config_dir.path());
    std::env::set_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV, bridge_state_root.path());

    let session_id = Uuid::new_v4();
    let remote = ClaudeWakeRemoteContext {
        session_id,
        envelope: ClaudeTranscriptEnvelope {
            cwd: Path::new("/stale/cwd").to_path_buf(),
            uuid: session_id,
            claude_version: None,
            entries: vec![serde_json::json!({"type": "assistant", "text": "done"})],
            subagents: HashMap::new(),
            todos: HashMap::new(),
        },
        wake_prompt: "resume prompt\n\nwake prompt".to_string(),
    };
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440010".parse().unwrap();
    let parent_run_id = "parent-run-456".to_string();

    let command = futures::executor::block_on(ClaudeHarness::prepare_local_wake_command(
        ServerApiProvider::new_for_test().get(),
        task_id,
        Some(parent_run_id.clone()),
        Some(working_dir.clone()),
        remote,
        None,
    ))
    .unwrap();

    let state_dir = bridge_state_root.path().join(session_id.to_string());
    let prompt_path = state_dir.join(CLAUDE_WAKE_PROMPT_FILE_NAME);

    assert!(command.contains("--resume"));
    assert!(command.starts_with("env "));
    assert!(command.contains(&session_id.to_string()));
    assert!(command.contains(CLAUDE_WAKE_PROMPT_FILE_NAME));
    assert!(command.contains(&format!(
        "{OZ_RUN_ID_ENV}={}",
        shell_quote(&task_id.to_string())
    )));
    assert!(command.contains(&format!(
        "{OZ_PARENT_RUN_ID_ENV}={}",
        shell_quote(&parent_run_id)
    )));
    assert!(command.contains(&format!("{OZ_HARNESS_ENV}={}", shell_quote("claude"))));
    assert!(!command.contains(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV));
    assert!(!command.contains("OZ_PARENT_LISTENER_MANAGED_EXTERNALLY"));
    assert_eq!(
        fs::read_to_string(&prompt_path).unwrap(),
        "resume prompt\n\nwake prompt"
    );
    assert!(!parent_bridge_hook_output_file(&state_dir).exists());

    let restored_envelope =
        read_envelope(session_id, &working_dir, claude_config_dir.path()).unwrap();
    assert_eq!(restored_envelope.cwd, working_dir);
    assert_eq!(
        restored_envelope.entries,
        vec![serde_json::json!({"type": "assistant", "text": "done"})]
    );
    assert!(claude_config_dir
        .path()
        .join(CLAUDE_JSON_FILE_NAME)
        .exists());
    assert!(claude_config_dir
        .path()
        .join(CLAUDE_SETTINGS_FILE_NAME)
        .exists());

    std::env::remove_var("HOME");
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV);
}

#[tokio::test]
async fn prime_parent_bridge_for_wake_clears_acked_output_and_surfaces_new_message() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("session-123");
    ensure_parent_bridge_state_dir(&state_dir).unwrap();

    let stale = sample_parent_bridge_message(
        41,
        "stale-msg",
        "Old direction",
        "This message should be acknowledged and cleared.",
    );
    write_surfaced_parent_bridge_message(&state_dir, &stale);
    fs::write(
        parent_bridge_hook_output_file(&state_dir),
        serde_json::to_vec(&MessageBridgeHookOutput {
            additional_context: "stale context".to_string(),
            remaining_staged_count: 0,
            surfaced_count: 1,
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(parent_bridge_hook_output_ack_file(&state_dir), "").unwrap();

    let wake_message = AgentMessageEventMetadata {
        sequence: 42,
        message_id: "msg-123".to_string(),
        occurred_at: "2026-04-17T15:47:00Z".to_string(),
    };
    let expected = sample_parent_bridge_message(
        42,
        "msg-123",
        "Please pivot",
        "Inspect the failing tests first.",
    );

    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_mark_message_delivered()
        .with(eq("stale-msg"))
        .times(1)
        .returning(|_| Ok(()));
    let expected_message = expected.clone();
    ai_client
        .expect_read_agent_message()
        .with(eq("msg-123"))
        .times(1)
        .returning(move |_| {
            Ok(ReadAgentMessageResponse {
                message_id: expected_message.message_id.clone(),
                sender_run_id: expected_message.sender_run_id.clone(),
                subject: expected_message.subject.clone(),
                body: expected_message.body.clone(),
                sent_at: "2026-04-17T15:46:00Z".to_string(),
                delivered_at: None,
                read_at: Some("2026-04-17T15:46:02Z".to_string()),
            })
        });
    let hydrator = MessageHydrator::new(Arc::new(ai_client) as Arc<dyn AIClient>);
    prime_parent_bridge_for_wake(&hydrator, &state_dir, Some(&wake_message))
        .await
        .unwrap();

    assert_eq!(read_parent_bridge_event_cursor(&state_dir).unwrap(), 42);
    assert!(!parent_bridge_hook_output_ack_file(&state_dir).exists());
    assert!(!parent_bridge_surfaced_message_path(&state_dir, 41, "stale-msg").exists());

    let surfaced_path = parent_bridge_surfaced_message_path(&state_dir, 42, "msg-123");
    assert!(surfaced_path.exists());
    assert!(!parent_bridge_staged_message_path(&state_dir, 42, "msg-123").exists());

    let hook_output: MessageBridgeHookOutput =
        serde_json::from_slice(&fs::read(parent_bridge_hook_output_file(&state_dir)).unwrap())
            .unwrap();
    assert_eq!(hook_output.surfaced_count, 1);
    assert_eq!(hook_output.remaining_staged_count, 0);
    assert!(hook_output.additional_context.contains("Please pivot"));

    let surfaced_record: MessageBridgeMessageRecord =
        serde_json::from_slice(&fs::read(&surfaced_path).unwrap()).unwrap();
    assert_eq!(surfaced_record.subject, expected.subject);
    assert_eq!(surfaced_record.body, expected.body);
    assert_eq!(surfaced_record.occurred_at, wake_message.occurred_at);
}

#[test]
#[serial_test::serial]
fn suffix_uses_worker_injected_env_when_present() {
    let worker_key = "sk-ant-api03-WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW-worker-suffix!";
    std::env::set_var(ANTHROPIC_API_KEY_ENV, worker_key);
    // Even when the resolved map has a different value, the worker env wins.
    let resolved = HashMap::from([(
        OsString::from("ANTHROPIC_API_KEY"),
        OsString::from(
            "sk-ant-api03-RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR-resolved-val!",
        ),
    )]);
    let suffix = resolve_anthropic_api_key_suffix(&resolved);
    let expected = &worker_key[worker_key.len() - 20..];
    assert_eq!(suffix.as_deref(), Some(expected));
    std::env::remove_var(ANTHROPIC_API_KEY_ENV);
}

#[test]
fn prepare_claude_settings_creates_settings_file() {
    let tmp = TempDir::new().unwrap();
    let claude_settings_path = tmp.path().join(".claude/settings.json");

    prepare_claude_settings(&claude_settings_path).unwrap();

    let claude_settings: Value =
        serde_json::from_slice(&fs::read(claude_settings_path).unwrap()).unwrap();
    assert_eq!(
        claude_settings["skipDangerousModePermissionPrompt"],
        Value::Bool(true)
    );
}

#[test]
fn prepare_claude_settings_merges_existing_settings() {
    let tmp = TempDir::new().unwrap();
    let claude_settings_path = tmp.path().join("settings.json");
    fs::write(
        &claude_settings_path,
        r#"{"editor":"vim","nested":{"value":1}}"#,
    )
    .unwrap();

    prepare_claude_settings(&claude_settings_path).unwrap();

    let claude_settings: Value =
        serde_json::from_slice(&fs::read(claude_settings_path).unwrap()).unwrap();
    assert_eq!(claude_settings["editor"], "vim");
    assert_eq!(claude_settings["nested"]["value"], 1);
    assert_eq!(
        claude_settings["skipDangerousModePermissionPrompt"],
        Value::Bool(true)
    );
}
