use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use chrono::Utc;
use session_sharing_protocol::common::SessionId;

use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskState};
use crate::server::server_api::ai::{MockAIClient, SpawnAgentResponse};
use crate::terminal::shared_session;

use super::{spawn_task, AmbientAgentEvent, SessionJoinInfo};

fn task_with(
    state: AmbientAgentTaskState,
    session_id: Option<String>,
    session_link: Option<String>,
) -> AmbientAgentTask {
    AmbientAgentTask {
        task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        parent_run_id: None,
        title: "title".to_string(),
        state,
        prompt: "prompt".to_string(),
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        updated_at: Utc::now(),
        status_message: None,
        source: None,
        session_id,
        session_link,
        creator: None,
        conversation_id: None,
        request_usage: None,
        agent_config_snapshot: None,
        artifacts: vec![],
        is_sandbox_running: true,
        last_event_sequence: None,
        children: vec![],
    }
}

#[tokio::test]
async fn poll_stops_on_terminal_failure_like_state() {
    use futures::StreamExt;

    let mut mock = MockAIClient::new();

    mock.expect_spawn_agent().returning(|_| {
        Ok(SpawnAgentResponse {
            task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
            run_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            at_capacity: false,
        })
    });

    mock.expect_get_ambient_agent_task()
        .times(1)
        .returning(|_task_id| Ok(task_with(AmbientAgentTaskState::Error, None, None)));

    let ai_client = Arc::new(mock);
    let request = crate::server::server_api::ai::SpawnAgentRequest {
        prompt: "test".to_string(),
        config: None,
        title: None,
        team: None,
        skill: None,
        attachments: vec![],
        interactive: None,
        parent_run_id: None,
        runtime_skills: vec![],
        referenced_attachments: vec![],
    };

    let mut stream = Box::pin(spawn_task(request, ai_client, None));

    let event = stream
        .next()
        .await
        .expect("expected event")
        .expect("expected ok");
    assert!(matches!(event, AmbientAgentEvent::TaskSpawned { .. }));

    let event = stream
        .next()
        .await
        .expect("expected event")
        .expect("expected ok");
    assert!(matches!(
        event,
        AmbientAgentEvent::StateChanged {
            state: AmbientAgentTaskState::Error,
            ..
        }
    ));

    assert!(stream.next().await.is_none());
}

#[test]
fn session_join_info_prefers_session_link_and_tolerates_missing_session_id() {
    let task = task_with(
        AmbientAgentTaskState::InProgress,
        None,
        Some("https://example.com/session/abc".to_string()),
    );

    let join_info = SessionJoinInfo::from_task(&task).expect("expected join info");
    assert_eq!(join_info.session_link, "https://example.com/session/abc");
    assert!(join_info.session_id.is_none());
}

#[test]
fn session_join_info_falls_back_to_session_id() {
    let session_id = SessionId::new();
    let task = task_with(
        AmbientAgentTaskState::InProgress,
        Some(session_id.to_string()),
        None,
    );

    let join_info = SessionJoinInfo::from_task(&task).expect("expected join info");

    assert_eq!(join_info.session_id, Some(session_id));
    assert_eq!(
        join_info.session_link,
        shared_session::join_link(&session_id)
    );
}

#[test]
fn session_join_info_ignores_empty_link_and_invalid_session_id() {
    let task = task_with(
        AmbientAgentTaskState::InProgress,
        Some("not-a-session-id".to_string()),
        Some(String::new()),
    );

    assert_eq!(SessionJoinInfo::from_task(&task), None);
}

#[tokio::test]
async fn poll_for_session_join_info_waits_until_link_is_available() {
    use futures::StreamExt;

    let mut mock = MockAIClient::new();

    let call_count = Arc::new(AtomicUsize::new(0));

    mock.expect_spawn_agent().returning(|_| {
        Ok(SpawnAgentResponse {
            task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
            run_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            at_capacity: false,
        })
    });

    mock.expect_get_ambient_agent_task().returning({
        let call_count = call_count.clone();
        move |_task_id| {
            let idx = call_count.fetch_add(1, Ordering::SeqCst);
            let task = if idx == 0 {
                task_with(AmbientAgentTaskState::InProgress, None, None)
            } else {
                task_with(
                    AmbientAgentTaskState::InProgress,
                    None,
                    Some("https://example.com/session/ready".to_string()),
                )
            };

            Ok(task)
        }
    });

    let ai_client = Arc::new(mock);
    let request = crate::server::server_api::ai::SpawnAgentRequest {
        prompt: "test".to_string(),
        config: None,
        title: None,
        team: None,
        skill: None,
        attachments: vec![],
        interactive: None,
        parent_run_id: None,
        runtime_skills: vec![],
        referenced_attachments: vec![],
    };

    let mut stream = Box::pin(spawn_task(request, ai_client, None));

    // First event should be TaskSpawned
    let event = stream
        .next()
        .await
        .expect("expected event")
        .expect("expected ok");
    assert!(matches!(event, AmbientAgentEvent::TaskSpawned { .. }));

    // Second event should be StateChanged
    let event = stream
        .next()
        .await
        .expect("expected event")
        .expect("expected ok");
    assert!(matches!(event, AmbientAgentEvent::StateChanged { .. }));

    // Third event should be SessionStarted
    let event = stream
        .next()
        .await
        .expect("expected event")
        .expect("expected ok");
    if let AmbientAgentEvent::SessionStarted { session_join_info } = event {
        assert_eq!(
            session_join_info.session_link,
            "https://example.com/session/ready"
        );
    } else {
        panic!("Expected SessionStarted event");
    }
}
