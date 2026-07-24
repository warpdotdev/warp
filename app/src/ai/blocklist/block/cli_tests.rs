use std::collections::HashMap;

use warp_multi_agent_api as api;
use warpui::{App, EntityId};

use super::CLISubagentView;
use crate::BlocklistAIHistoryModel;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::task::TaskId;

/// Builds a `UserQuery` `api::Message` with a non-empty `request_id` so that
/// `api::Task::into_exchanges` produces a restored exchange from it.
fn user_query_message(id: &str, task_id: &str, request_id: &str, query: &str) -> api::Message {
    api::Message {
        fetched_memories: vec![],
        id: id.to_string(),
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
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn root_api_task(task_id: &str, messages: Vec<api::Message>) -> api::Task {
    api::Task {
        id: task_id.to_string(),
        messages,
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    }
}

/// Regression guard for APP-4912: a `SpawnedSubagent` event whose agent
/// exchange is already absent from history must not crash the app. Previously
/// `CLISubagentView::new` `.expect`ed the exchange lookup and panicked; the
/// caller now skips view registration when this helper returns `None`.
#[test]
fn latest_exchange_id_for_task_returns_none_when_conversation_missing() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let conversation_id = AIConversationId::new();
        let task_id = TaskId::new("stale-task".to_owned());
        let result = app.read(|app| {
            CLISubagentView::latest_exchange_id_for_task(&conversation_id, &task_id, app)
        });
        assert!(result.is_none());
    });
}

#[test]
fn latest_exchange_id_for_task_returns_none_when_task_missing() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let conversation_id = AIConversationId::new();
        let conversation = AIConversation::new_restored(
            conversation_id,
            vec![root_api_task("root-task", vec![])],
            None,
        )
        .expect("conversation should restore");

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let missing_task_id = TaskId::new("nonexistent-task".to_owned());
        let result = app.read(|app| {
            CLISubagentView::latest_exchange_id_for_task(&conversation_id, &missing_task_id, app)
        });
        assert!(result.is_none());
    });
}

#[test]
fn latest_exchange_id_for_task_returns_none_when_task_has_no_exchanges() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let conversation_id = AIConversationId::new();
        let conversation = AIConversation::new_restored(
            conversation_id,
            vec![root_api_task("root-task", vec![])],
            None,
        )
        .expect("conversation should restore");

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let root_task_id = TaskId::new("root-task".to_owned());
        let result = app.read(|app| {
            CLISubagentView::latest_exchange_id_for_task(&conversation_id, &root_task_id, app)
        });
        assert!(result.is_none());
    });
}

#[test]
fn latest_exchange_id_for_task_returns_some_when_exchange_present() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let conversation_id = AIConversationId::new();
        let conversation = AIConversation::new_restored(
            conversation_id,
            vec![root_api_task(
                "root-task",
                vec![user_query_message("m1", "root-task", "req-1", "hello")],
            )],
            None,
        )
        .expect("conversation should restore");

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let root_task_id = TaskId::new("root-task".to_owned());
        let result = app.read(|app| {
            CLISubagentView::latest_exchange_id_for_task(&conversation_id, &root_task_id, app)
        });
        assert!(
            result.is_some(),
            "expected an exchange id for the root task"
        );
    });
}
