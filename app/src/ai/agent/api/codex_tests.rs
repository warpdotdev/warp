use std::sync::Arc;

use serde_json::json;
use warp_multi_agent_api as api;
use warp_multi_agent_api::client_action as api_client_action;
use warp_multi_agent_api::response_event as api_response_event;

use super::*;

#[test]
fn codex_conversation_tokens_round_trip_thread_ids() {
    let token = conversation_token_for_thread("thread-123");

    assert!(is_codex_conversation_token(&token));
    assert_eq!(
        thread_id_from_conversation_token(&token),
        Some("thread-123")
    );
    assert_eq!(
        thread_id_from_conversation_token(&ServerConversationToken::new("hosted-id".to_owned())),
        None
    );
}

#[test]
fn prompt_for_resume_uses_a_local_continuation_instruction() {
    let mut params = RequestParams::new_for_test();
    params.input = vec![AIAgentInput::ResumeConversation {
        context: Arc::from([]),
    }];

    assert_eq!(
        prompt_for_request(&params).unwrap(),
        "Continue the current task."
    );
}

#[test]
fn provider_switch_carries_the_visible_warp_transcript_once() {
    let mut params = RequestParams::new_for_test();
    params.conversation_token = Some(ServerConversationToken::new("hosted-thread".to_owned()));
    params.tasks = vec![api::Task {
        id: "task-1".to_owned(),
        messages: vec![
            api::Message {
                id: "user-1".to_owned(),
                task_id: "task-1".to_owned(),
                message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                    query: "Inspect this repository".to_owned(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            api::Message {
                id: "assistant-1".to_owned(),
                task_id: "task-1".to_owned(),
                message: Some(api::message::Message::AgentOutput(
                    api::message::AgentOutput {
                        text: "I found the entry point.".to_owned(),
                    },
                )),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    params.input = vec![AIAgentInput::ResumeConversation {
        context: Arc::from([]),
    }];

    let prompt = prompt_for_request(&params).unwrap();
    assert!(prompt.contains("USER: Inspect this repository"));
    assert!(prompt.contains("ASSISTANT: I found the entry point."));
    assert!(prompt.contains("Continue the current task."));
}

#[test]
fn agent_message_delta_maps_to_native_warp_append_action() {
    let mut accumulated = String::new();
    let mut terminal_error = None;
    let event = translate_notification(
        &Notification {
            method: "item/agentMessage/delta".to_owned(),
            params: json!({ "delta": "hello" }),
        },
        "task-1",
        "message-1",
        "request-1",
        &mut accumulated,
        &mut terminal_error,
    )
    .unwrap();

    let Some(api_response_event::Type::ClientActions(actions)) = event.r#type else {
        panic!("expected ClientActions event");
    };
    let Some(api_client_action::Action::AppendToMessageContent(append)) =
        actions.actions[0].action.as_ref()
    else {
        panic!("expected AppendToMessageContent action");
    };
    assert_eq!(append.task_id, "task-1");
    assert_eq!(
        append.mask.as_ref().unwrap().paths,
        vec!["agent_output.text".to_owned()]
    );
    let Some(api::message::Message::AgentOutput(output)) = append
        .message
        .as_ref()
        .and_then(|message| message.message.as_ref())
    else {
        panic!("expected AgentOutput message");
    };
    assert_eq!(output.text, "hello");
    assert_eq!(accumulated, "hello");
    assert!(terminal_error.is_none());
}

#[test]
fn embedded_permissions_fail_closed_except_for_explicit_unsandboxed_autonomy() {
    let mut params = RequestParams::new_for_test();
    let mut options = ThreadOptions::new(".");

    params.autonomy_level = api::AutonomyLevel::Supervised;
    params.isolation_level = api::IsolationLevel::None;
    configure_permissions(&params, &mut options);
    assert_eq!(options.approval_policy, ApprovalPolicy::Never);
    assert_eq!(options.sandbox, SandboxMode::WorkspaceWrite);

    params.autonomy_level = api::AutonomyLevel::Unsupervised;
    params.isolation_level = api::IsolationLevel::None;
    configure_permissions(&params, &mut options);
    assert_eq!(options.sandbox, SandboxMode::DangerFullAccess);
}

#[test]
fn finished_event_matches_warp_agent_success_protocol() {
    let event = stream_finished_event();
    let Some(api_response_event::Type::Finished(finished)) = event.r#type else {
        panic!("expected Finished event");
    };
    assert!(matches!(
        finished.reason,
        Some(stream_finished_event::Reason::Done(_))
    ));
}
