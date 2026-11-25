use super::UpdateAgentTaskInput;

#[test]
fn absent_session_timestamp_is_omitted() {
    let input = UpdateAgentTaskInput {
        task_id: "task-id".into(),
        task_state: None,
        session_id: Some("session-id".into()),
        session_started_at: None,
        conversation_id: None,
        status_message: None,
        session_debug_until: None,
        debug_agent_active: None,
    };

    let serialized = serde_json::to_value(input).unwrap();

    assert_eq!(
        serialized,
        serde_json::json!({"taskId": "task-id", "sessionId": "session-id"})
    );
}
