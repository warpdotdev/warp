use super::*;
use crate::persistence::model::{AgentConversation, AgentConversationRecord};

fn persisted_conversation(conversation_id: AIConversationId) -> AgentConversation {
    let task_id = format!("task-{conversation_id}");
    AgentConversation {
        conversation: AgentConversationRecord {
            id: 0,
            conversation_id: conversation_id.to_string(),
            conversation_data: r#"{"server_conversation_token":null}"#.to_string(),
            last_modified_at: chrono::NaiveDateTime::default(),
            summary: None,
        },
        tasks: vec![warp_multi_agent_api::Task {
            id: task_id,
            messages: vec![],
            dependencies: None,
            description: "Test conversation".to_string(),
            summary: String::new(),
            server_data: String::new(),
        }],
    }
}

fn ai_conversation(conversation_id: AIConversationId) -> AIConversation {
    convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation(
        conversation_id,
    ))
    .expect("test conversation should convert")
}

#[test]
fn take_conversation_hands_out_each_conversation_at_most_once() {
    let conversation_id = AIConversationId::new();
    let mut store =
        RestoredAgentConversations::new_seeded(vec![persisted_conversation(conversation_id)]);

    assert!(store.take_conversation(&conversation_id).is_some());
    assert!(
        store.take_conversation(&conversation_id).is_none(),
        "a taken conversation must not be handed out again"
    );
    assert!(
        store.get_conversation(&conversation_id).is_none(),
        "a taken conversation must not be readable either"
    );
}

#[test]
fn failed_take_does_not_consume_the_restore_opportunity() {
    let conversation_id = AIConversationId::new();
    // No seed and no backing database: the first take fails to load.
    let mut store = RestoredAgentConversations::new_seeded(vec![]);
    assert!(store.take_conversation(&conversation_id).is_none());

    // Once the conversation becomes available (e.g. the earlier failure was
    // transient), a retry must still succeed — a failed load must not have
    // marked the ID as taken.
    store
        .conversations
        .insert(conversation_id, ai_conversation(conversation_id));
    assert!(
        store.take_conversation(&conversation_id).is_some(),
        "a failed load must not permanently consume the restore"
    );
    assert!(store.take_conversation(&conversation_id).is_none());
}
