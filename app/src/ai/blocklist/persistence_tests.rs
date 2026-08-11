use std::collections::HashMap;

use warpui::{App, EntityId};

use super::{PersistedAIInputType, maybe_build_ai_query_upsert_event};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::persistence::ModelEvent;

fn user_query_message(task_id: &str, query: &str) -> warp_multi_agent_api::Message {
    warp_multi_agent_api::Message {
        fetched_memories: Vec::new(),
        id: "message-id".to_owned(),
        task_id: task_id.to_owned(),
        server_message_data: String::new(),
        citations: Vec::new(),
        message: Some(warp_multi_agent_api::message::Message::UserQuery(
            warp_multi_agent_api::message::UserQuery {
                query: query.to_owned(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            },
        )),
        request_id: "request-id".to_owned(),
        timestamp: None,
    }
}

#[test]
fn query_exchange_event_builds_persistence_upsert() {
    App::test((), |mut app| async move {
        let terminal_surface_id = EntityId::new();
        let conversation_id = AIConversationId::new();
        let task_id = "task-id";
        let conversation = AIConversation::new_restored(
            conversation_id,
            vec![warp_multi_agent_api::Task {
                id: task_id.to_owned(),
                messages: vec![user_query_message(task_id, "persist this prompt")],
                dependencies: None,
                description: String::new(),
                summary: String::new(),
                server_data: String::new(),
            }],
            None,
        )
        .expect("conversation should restore");
        let exchange_id = conversation
            .root_task_exchanges()
            .next()
            .expect("conversation should contain the query exchange")
            .id;
        let task_id = conversation.get_root_task_id().clone();

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_surface_id, vec![conversation], ctx);
        });

        let event = BlocklistAIHistoryEvent::AppendedExchange {
            exchange_id,
            task_id,
            terminal_surface_id,
            conversation_id,
            is_hidden: false,
            response_stream_id: None,
        };
        let persistence_event = app
            .read(|ctx| maybe_build_ai_query_upsert_event(&event, terminal_surface_id, false, ctx))
            .expect("query exchange should produce a persistence event");
        let ModelEvent::UpsertAIQuery { query } = persistence_event else {
            panic!("query exchange should produce an AI-query upsert");
        };
        assert_eq!(query.conversation_id, conversation_id);
        assert_eq!(query.exchange_id, exchange_id);
        assert_eq!(
            query.inputs,
            vec![PersistedAIInputType::Query {
                text: "persist this prompt".to_owned(),
                context: Default::default(),
                referenced_attachments: Default::default(),
            }]
        );
    });
}
