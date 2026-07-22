use chrono::{Duration, Utc};
use serde_json::{Value, json};

use super::{
    AgentConfigSnapshot, AmbientAgentTask, AmbientAgentTaskState, TaskStatusErrorCode,
    TaskStatusMessage,
};

fn make_task(snapshot_name: Option<&str>, title: &str) -> AmbientAgentTask {
    let now = Utc::now();
    let agent_config_snapshot = snapshot_name.map(|name| AgentConfigSnapshot {
        name: Some(name.to_string()),
        ..Default::default()
    });
    AmbientAgentTask {
        task_id: "11111111-1111-1111-1111-111111111111".parse().unwrap(),
        parent_run_id: None,
        title: title.to_string(),
        state: AmbientAgentTaskState::InProgress,
        prompt: String::new(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

fn task_json_with_run_time(run_time_key: &str, run_time: Value) -> Value {
    let now = Utc::now().to_rfc3339();
    let mut task = json!({
        "task_id": "11111111-1111-1111-1111-111111111111",
        "title": "Task",
        "state": "SUCCEEDED",
        "prompt": "test",
        "created_at": now,
        "started_at": now,
        "updated_at": now,
        "status_message": null,
        "session_id": null,
        "session_link": null,
        "creator": null,
        "conversation_id": null,
        "request_usage": null,
        "is_sandbox_running": false
    });
    task[run_time_key] = run_time;
    task
}

#[test]
fn display_name_prefers_agent_config_snapshot_name_over_title() {
    let task = make_task(Some("frontend-tests"), "Long descriptive task title");
    assert_eq!(task.display_name(), "frontend-tests");
}

#[test]
fn display_name_falls_back_to_title_when_snapshot_name_is_missing() {
    let task = make_task(None, "Long descriptive task title");
    assert_eq!(task.display_name(), "Long descriptive task title");
}

#[test]
fn display_name_falls_back_to_title_when_snapshot_name_is_whitespace() {
    let task = make_task(Some("   "), "Long descriptive task title");
    assert_eq!(task.display_name(), "Long descriptive task title");
}

#[test]
fn display_name_returns_literal_agent_when_both_sources_are_empty() {
    let task = make_task(None, "");
    assert_eq!(task.display_name(), "Agent");
}

#[test]
fn display_name_returns_literal_agent_for_whitespace_only_title() {
    let task = make_task(None, "   \t\n  ");
    assert_eq!(task.display_name(), "Agent");
}

#[test]
fn display_name_trims_whitespace_at_each_layer() {
    let task = make_task(Some("  frontend-tests  "), "  Long descriptive title  ");
    assert_eq!(task.display_name(), "frontend-tests");

    let task = make_task(None, "  Long descriptive title  ");
    assert_eq!(task.display_name(), "Long descriptive title");
}

#[test]
fn task_status_error_code_deserializes_public_api_casing() {
    let message: TaskStatusMessage = serde_json::from_str(
        "{\"message\":\"setup failed\",\"error_code\":\"environment_setup_failed\"}",
    )
    .unwrap();

    assert_eq!(
        message.error_code,
        Some(TaskStatusErrorCode::EnvironmentSetupFailed)
    );
    assert!(message.is_environment_setup_failure());
}

#[test]
fn task_status_error_code_deserializes_graphql_casing() {
    let message: TaskStatusMessage = serde_json::from_str(
        "{\"message\":\"setup failed\",\"errorCode\":\"ENVIRONMENT_SETUP_FAILED\"}",
    )
    .unwrap();

    assert_eq!(
        message.error_code,
        Some(TaskStatusErrorCode::EnvironmentSetupFailed)
    );
    assert!(message.is_environment_setup_failure());
}

#[test]
fn task_status_error_code_deserializes_unknown_codes() {
    let message: TaskStatusMessage =
        serde_json::from_str("{\"message\":\"failed\",\"error_code\":\"new_error\"}").unwrap();

    assert_eq!(message.error_code, Some(TaskStatusErrorCode::Unknown));
    assert!(!message.is_environment_setup_failure());
}

#[test]
fn ambient_agent_task_deserializes_run_time_iso8601() {
    let task: AmbientAgentTask =
        serde_json::from_value(task_json_with_run_time("run_time", json!("PT2M30S"))).unwrap();

    assert_eq!(task.run_time(), Some(Duration::seconds(150)));
}
