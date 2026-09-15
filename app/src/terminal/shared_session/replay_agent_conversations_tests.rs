use std::collections::HashMap;

use warp_multi_agent_api::{self as api, client_action as api_client_action, response_event};

use super::reconstruct_response_events_from_conversations;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::persistence::model::AgentConversationData;

const TOKENLESS_TASK_ID: &str = "tokenless-task";
const TOKENED_TASK_ID: &str = "tokened-task";
const REMOTE_CHILD_TASK_ID: &str = "remote-child-task";

fn conversation_data(
    server_token: Option<&str>,
    forked_from_token: Option<&str>,
) -> AgentConversationData {
    AgentConversationData {
        server_conversation_token: server_token.map(ToString::to_string),
        forked_from_server_conversation_token: forked_from_token.map(ToString::to_string),
        conversation_usage_metadata: None,
        reverted_action_ids: None,
        artifacts_json: None,
        parent_agent_id: None,
        agent_name: None,
        orchestration_harness_type: None,
        parent_conversation_id: None,
        is_remote_child: false,
        root_task_is_optimistic: None,
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: None,
        pinned: false,
    }
}

/// A single-exchange conversation whose root task holds one user query.
fn conversation_with_user_query(
    conversation_data: AgentConversationData,
    task_id: &str,
    query: &str,
) -> AIConversation {
    let message = api::Message {
        fetched_memories: vec![],
        id: format!("{task_id}-message"),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: query.to_string(),
            context: None,
            referenced_attachments: HashMap::new(),
            mode: None,
            intended_agent: Default::default(),
        })),
        request_id: format!("{task_id}-request"),
        timestamp: None,
    };
    let task = api::Task {
        id: task_id.to_string(),
        messages: vec![message],
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    };

    AIConversation::new_restored(AIConversationId::new(), vec![task], Some(conversation_data))
        .expect("conversation with a root task restores")
}

fn replayed_conversation_tokens(events: &[api::ResponseEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.r#type {
            Some(response_event::Type::Init(init)) => Some(init.conversation_id.clone()),
            _ => None,
        })
        .collect()
}

fn replayed_message_task_ids(events: &[api::ResponseEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.r#type {
            Some(response_event::Type::ClientActions(actions)) => Some(actions),
            _ => None,
        })
        .flat_map(|actions| actions.actions.iter())
        .filter_map(|action| match &action.action {
            Some(api_client_action::Action::AddMessagesToTask(add)) => Some(add.task_id.clone()),
            _ => None,
        })
        .collect()
}

/// QUALITY-1676: a conversation with neither a server token nor a forked-from
/// token cannot be addressed by a viewer, so replaying it as an `Init` with an
/// empty `conversation_id` makes the viewer mint an empty conversation. It must
/// be left out of the replay entirely.
#[test]
fn test_replay_skips_conversations_with_no_server_token() {
    let conversations = vec![
        conversation_with_user_query(
            conversation_data(None, None),
            TOKENLESS_TASK_ID,
            "child prompt",
        ),
        conversation_with_user_query(
            conversation_data(Some("orchestrator-token"), None),
            TOKENED_TASK_ID,
            "orchestrator prompt",
        ),
    ];

    let events = reconstruct_response_events_from_conversations(&conversations);

    assert_eq!(
        replayed_conversation_tokens(&events),
        vec!["orchestrator-token".to_string()],
        "only the conversation the viewer can address may be replayed"
    );
    assert_eq!(
        replayed_message_task_ids(&events),
        vec![TOKENED_TASK_ID.to_string()],
        "the tokenless conversation's messages must not be replayed either"
    );
}

/// QUALITY-1676: a remote-child placeholder lives on the orchestrator's own
/// terminal surface but mirrors another run, which a viewer materializes in its
/// own child pane from that run's session. Its token is real, so only the
/// placeholder check keeps it out of this session's replay.
#[test]
fn test_replay_skips_remote_child_placeholders() {
    let mut remote_child_data = conversation_data(Some("remote-child-token"), None);
    remote_child_data.is_remote_child = true;
    let conversations = vec![
        conversation_with_user_query(remote_child_data, REMOTE_CHILD_TASK_ID, "child prompt"),
        conversation_with_user_query(
            conversation_data(Some("orchestrator-token"), None),
            TOKENED_TASK_ID,
            "orchestrator prompt",
        ),
    ];

    let events = reconstruct_response_events_from_conversations(&conversations);

    assert_eq!(
        replayed_conversation_tokens(&events),
        vec!["orchestrator-token".to_string()],
        "another run's placeholder is not part of this session's replay"
    );
    assert_eq!(
        replayed_message_task_ids(&events),
        vec![TOKENED_TASK_ID.to_string()]
    );
}

/// The forked-from token is still a usable address, so a conversation carrying
/// only that one keeps being replayed under it.
#[test]
fn test_replay_uses_forked_from_token_when_server_token_is_absent() {
    let conversation = conversation_with_user_query(
        conversation_data(None, Some("forked-from-token")),
        TOKENED_TASK_ID,
        "forked prompt",
    );

    let events = reconstruct_response_events_from_conversations(&[conversation]);

    assert_eq!(
        replayed_conversation_tokens(&events),
        vec!["forked-from-token".to_string()]
    );
}
