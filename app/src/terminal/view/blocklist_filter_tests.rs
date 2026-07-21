use warp_multi_agent_api as api;

use super::{exchanges_for_blocklist, should_show_task_in_blocklist};
use crate::ai::agent::MessageId;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::test_util::ai_agent_tasks::{
    create_api_subtask, create_api_task, create_message, create_subagent_tool_call_message,
};

const ROOT_TASK_ID: &str = "root_task";
const SUBTASK_ID: &str = "subtask";
const SUBTASK_OUTPUT_MESSAGE_ID: &str = "subtask_output";

fn create_restored_conversation(
    subagent_metadata: api::message::tool_call::subagent::Metadata,
) -> AIConversation {
    let root_task = create_api_task(
        ROOT_TASK_ID,
        vec![create_subagent_tool_call_message(
            "subagent_call",
            ROOT_TASK_ID,
            SUBTASK_ID,
            Some(subagent_metadata),
        )],
    );
    let subtask = create_api_subtask(
        SUBTASK_ID,
        ROOT_TASK_ID,
        vec![create_message(SUBTASK_OUTPUT_MESSAGE_ID, SUBTASK_ID)],
    );

    AIConversation::new_restored(AIConversationId::new(), vec![root_task, subtask], None)
        .expect("restored conversation should build")
}

#[test]
fn test_should_show_task_in_blocklist_hides_warp_docs_subagent_task() {
    let conversation = create_restored_conversation(
        api::message::tool_call::subagent::Metadata::WarpDocumentationSearch(()),
    );

    let subtask = conversation
        .all_tasks()
        .find(|task| task.is_warp_documentation_search_subagent())
        .expect("warp docs subagent task should exist");

    assert!(!should_show_task_in_blocklist(subtask));
}

#[test]
fn test_should_show_task_in_blocklist_hides_conversation_search_subagent_task() {
    let conversation = create_restored_conversation(
        api::message::tool_call::subagent::Metadata::ConversationSearch(Default::default()),
    );

    let subtask = conversation
        .all_tasks()
        .find(|task| task.is_conversation_search_subagent())
        .expect("conversation search subagent task should exist");

    assert!(!should_show_task_in_blocklist(subtask));
}

#[test]
fn test_exchanges_for_blocklist_excludes_warp_docs_subagent_exchanges() {
    let conversation = create_restored_conversation(
        api::message::tool_call::subagent::Metadata::WarpDocumentationSearch(()),
    );

    let exchanges = exchanges_for_blocklist(&conversation);

    assert_eq!(exchanges.len(), 1);
    assert!(exchanges.iter().all(|exchange| {
        !exchange
            .added_message_ids
            .contains(&MessageId::new(SUBTASK_OUTPUT_MESSAGE_ID.to_string()))
    }));
}
