use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use itertools::Itertools;
use warp_cli::agent::Harness;
use warpui::{App, EntityId};

use crate::{
    ai::{
        agent::{
            api::ServerConversationToken,
            conversation::{AIAgentHarness, AIConversationId, ServerAIConversationMetadata},
            AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus,
            FinishedAIAgentOutput, Shared, UserQueryMode,
        },
        ambient_agents::AmbientAgentTaskId,
        blocklist::{controller::RequestInput, ResponseStreamId},
        llms::LLMId,
    },
    cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions},
    input_suggestions::HistoryInputSuggestion,
    persistence::{model::PersistedAutoexecuteMode, ModelEvent},
    server::ids::ServerId,
    terminal::model::session::SessionId,
    test_util::settings::initialize_settings_for_tests,
    GlobalResourceHandles, GlobalResourceHandlesProvider,
};

use super::{
    AIConversationMetadata, AIQueryHistoryOutputStatus, BlocklistAIHistoryModel, PersistedAIInput,
    PersistedAIInputType,
};

/// Helper function to create a PersistedAIInput for testing
fn create_persisted_query(
    query_text: &str,
    conversation_id: AIConversationId,
    start_time: DateTime<Local>,
) -> PersistedAIInput {
    PersistedAIInput {
        exchange_id: AIAgentExchangeId::new(),
        conversation_id,
        start_ts: start_time,
        inputs: vec![PersistedAIInputType::Query {
            text: query_text.to_string(),
            context: Default::default(),
            referenced_attachments: Default::default(),
        }],
        output_status: AIQueryHistoryOutputStatus::Completed,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        coding_model_id: LLMId::from("test-coding-model"),
    }
}

/// Helper function to create an AIAgentExchange for testing
fn create_exchange_with_query(
    query_text: &str,
    start_time: DateTime<Local>,
    working_directory: Option<String>,
) -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![AIAgentInput::UserQuery {
            query: query_text.to_string(),
            context: Default::default(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: UserQueryMode::default(),
            running_command: None,
            intended_agent: None,
        }],
        output_status: AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success {
                output: Shared::new(Default::default()),
            },
        },
        added_message_ids: HashSet::new(),
        start_time,
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

#[test]
fn start_new_child_conversation_persists_harness_metadata() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        let (child_a, child_b, child_ids) = history_model.update(&mut app, |history_model, ctx| {
            let parent_conversation_id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(
                parent_conversation_id,
                "parent-agent-id".to_string(),
            );
            let child_a = history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent 1".to_string(),
                parent_conversation_id,
                Some(Harness::Claude),
                ctx,
            );
            let child_b = history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent 2".to_string(),
                parent_conversation_id,
                Some(Harness::Codex),
                ctx,
            );
            (
                child_a,
                child_b,
                history_model
                    .child_conversation_ids_of(&parent_conversation_id)
                    .to_vec(),
            )
        });

        assert_eq!(child_ids, vec![child_a, child_b]);
        history_model.read(&app, |history_model, _| {
            let child_a_conversation = history_model
                .conversation(&child_a)
                .expect("child conversation should exist");
            let child_b_conversation = history_model
                .conversation(&child_b)
                .expect("child conversation should exist");
            assert_eq!(
                child_a_conversation.orchestration_harness_type(),
                Some(Harness::Claude.config_name())
            );
            assert_eq!(
                child_a_conversation.orchestration_harness(),
                Some(Harness::Claude)
            );
            assert_eq!(
                child_b_conversation.orchestration_harness_type(),
                Some(Harness::Codex.config_name())
            );
            assert_eq!(
                child_b_conversation.orchestration_harness(),
                Some(Harness::Codex)
            );
            assert_eq!(
                child_a_conversation.parent_agent_id(),
                Some("parent-agent-id")
            );
            assert_eq!(
                child_b_conversation.parent_agent_id(),
                Some("parent-agent-id")
            );
        });
    });
}

#[test]
fn test_ai_queries_for_terminal_view_up_arrow_history() {
    App::test((), |mut app| async move {
        let now = Local::now();
        let terminal_view_id = EntityId::new();
        let current_session_id = SessionId::from(0);
        let all_live_session_ids = HashSet::from([current_session_id]);

        // Create initial persisted queries
        let conversation_id_1 = AIConversationId::new();
        let conversation_id_2 = AIConversationId::new();

        let persisted_queries = vec![
            create_persisted_query(
                "restored query 1",
                conversation_id_1,
                now - chrono::Duration::seconds(10),
            ),
            create_persisted_query(
                "restored query 2",
                conversation_id_2,
                now - chrono::Duration::seconds(5),
            ),
        ];

        // Create history model with persisted queries as a singleton
        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(persisted_queries, &[]));

        // Helper function to get and sort AI queries using the same logic as Input
        let get_sorted_queries = |model: &BlocklistAIHistoryModel| -> Vec<String> {
            model
                .all_ai_queries(Some(terminal_view_id))
                .map(|query| HistoryInputSuggestion::AIQuery { entry: query })
                .sorted_by(|a, b| a.cmp(b, Some(current_session_id), &all_live_session_ids))
                .map(|suggestion| suggestion.text().to_string())
                .collect()
        };

        // Test initial state with just persisted queries
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");

        // Start a new conversation and add "live query 1"
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        let stream_id = ResponseStreamId::new_for_test();
        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("live query 1", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id,
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Test state after adding live query 1
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 3);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");

        // Start another new conversation and add "live query 2"
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query(
                "live query 2",
                now + chrono::Duration::seconds(1),
                None,
            );
            let stream_id = ResponseStreamId::new_for_test();
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id,
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Test state after adding live query 2
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 4);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");
        assert_eq!(queries[3], "live query 2");

        // Clear the blocklist
        history_model.update(&mut app, |history_model, ctx| {
            history_model.clear_conversations_in_terminal_view(terminal_view_id, ctx);
        });

        // Test state after clearing - should remain the same
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 4);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");
        assert_eq!(queries[3], "live query 2");

        // Start a new conversation after clearing and add "new query after clear"
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let stream_id = ResponseStreamId::new_for_test();
            let exchange = create_exchange_with_query(
                "new query after clear",
                now + chrono::Duration::seconds(2),
                None,
            );
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id,
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Test final state
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 5);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");
        assert_eq!(queries[3], "live query 2");
        assert_eq!(queries[4], "new query after clear");
    });
}

/// Helper function to create ServerMetadata for testing
fn create_mock_server_metadata() -> ServerMetadata {
    ServerMetadata {
        uid: ServerId::default(),
        revision: Revision::now(),
        metadata_last_updated_ts: Utc::now().into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid: None,
        last_editor_uid: None,
        current_editor_uid: None,
    }
}

/// Helper function to create ServerPermissions for testing
fn create_mock_server_permissions() -> ServerPermissions {
    ServerPermissions {
        space: Owner::mock_current_user(),
        guests: Vec::new(),
        anyone_link_sharing: None,
        permissions_last_updated_ts: Utc::now().into(),
    }
}

/// Helper function to create ServerAIConversationMetadata for testing
fn create_server_metadata(
    title: &str,
    server_token: &str,
    credits_spent: f32,
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
) -> ServerAIConversationMetadata {
    use crate::persistence::model::ConversationUsageMetadata;

    // Create ConversationUsageMetadata from persistence model
    let usage = ConversationUsageMetadata {
        was_summarized: false,
        context_window_usage: 0.0,
        credits_spent,
        credits_spent_for_last_block: None,
        token_usage: vec![],
        tool_usage_metadata: Default::default(),
    };

    ServerAIConversationMetadata {
        title: title.to_string(),
        usage,
        metadata: create_mock_server_metadata(),
        permissions: create_mock_server_permissions(),
        ambient_agent_task_id,
        server_conversation_token: ServerConversationToken::new(server_token.to_string()),
        artifacts: Vec::new(),
        working_directory: None,
        harness: AIAgentHarness::Oz,
    }
}

#[test]
fn test_merge_cloud_conversation_metadata() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // Set up local metadata: some with server tokens, some without
        history_model.update(&mut app, |model, _| {
            let cloud_metadata = vec![
                create_server_metadata("Local Conversation 1", "token-1", 10.0, None),
                create_server_metadata("Local Conversation 2", "token-2", 20.0, None),
                create_server_metadata("Local Conversation 3", "token-3", 30.0, None),
            ];
            model.merge_cloud_conversation_metadata(cloud_metadata);
        });

        // Fetch server metadata where:
        // - token-1 and token-2 match existing local (should update)
        // - token-4 and token-5 are net new (should add)
        // - token-3 is not in server response (local should remain)
        history_model.update(&mut app, |model, _| {
            let cloud_metadata = vec![
                create_server_metadata("Updated Conversation 1", "token-1", 15.0, None),
                create_server_metadata("Updated Conversation 2", "token-2", 25.0, None),
                create_server_metadata("New Conversation 4", "token-4", 40.0, None),
                create_server_metadata("New Conversation 5", "token-5", 50.0, None),
            ];
            model.merge_cloud_conversation_metadata(cloud_metadata);
        });

        // Verify end state
        let (titles, token_map): (Vec<String>, HashMap<String, f32>) =
            history_model.read(&app, |model, _| {
                let mut titles = Vec::new();
                let mut token_map = HashMap::new();
                for meta in model.get_local_conversations_metadata() {
                    titles.push(meta.title.clone());
                    if let (Some(token), Some(credits)) =
                        (meta.server_conversation_token.as_ref(), meta.credits_spent)
                    {
                        token_map.insert(token.as_str().to_string(), credits);
                    }
                }
                (titles, token_map)
            });

        // Should have 5 total: 3 original (token-1, token-2, token-3) + 2 new (token-4, token-5)
        assert_eq!(titles.len(), 5);

        // token-1 and token-2 should be updated
        assert_eq!(token_map.get("token-1"), Some(&15.0));
        assert_eq!(token_map.get("token-2"), Some(&25.0));
        assert!(titles.contains(&"Updated Conversation 1".to_string()));
        assert!(titles.contains(&"Updated Conversation 2".to_string()));

        // token-3 should remain unchanged (not in server response)
        assert_eq!(token_map.get("token-3"), Some(&30.0));
        assert!(titles.contains(&"Local Conversation 3".to_string()));

        // token-4 and token-5 should be new
        assert_eq!(token_map.get("token-4"), Some(&40.0));
        assert_eq!(token_map.get("token-5"), Some(&50.0));
        assert!(titles.contains(&"New Conversation 4".to_string()));
        assert!(titles.contains(&"New Conversation 5".to_string()));
    });
}

/// Test that when a conversation is restored BEFORE cloud metadata is fetched,
/// the server_metadata is populated when merge_cloud_conversation_metadata is called.
#[test]
fn test_merge_cloud_metadata_updates_already_restored_conversations() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        // Create a conversation with a server token and restore it
        let mut conversation = AIConversation::new(false, false);
        conversation.set_server_conversation_token("token-1".to_string());
        let conversation_id = conversation.id();

        // Verify conversation has no server_metadata initially
        assert!(conversation.server_metadata().is_none());

        // Restore the conversation (simulating app startup restoration)
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        // Verify the conversation is still without server_metadata
        let has_metadata = history_model.read(&app, |model, _| {
            model
                .conversation(&conversation_id)
                .map(|c| c.server_metadata().is_some())
                .unwrap_or(false)
        });
        assert!(
            !has_metadata,
            "Conversation should not have server_metadata before merge"
        );

        // Now merge cloud metadata - this should update the restored conversation
        history_model.update(&mut app, |model, _| {
            let cloud_metadata = vec![create_server_metadata(
                "Conversation from Server",
                "token-1",
                42.0,
                None,
            )];
            model.merge_cloud_conversation_metadata(cloud_metadata);
        });

        // Verify that the restored conversation now has server_metadata
        let (has_metadata, title) = history_model.read(&app, |model, _| {
            let conv = model.conversation(&conversation_id).unwrap();
            let has_metadata = conv.server_metadata().is_some();
            let title = conv
                .server_metadata()
                .map(|m| m.title.clone())
                .unwrap_or_default();
            (has_metadata, title)
        });
        assert!(
            has_metadata,
            "Conversation should have server_metadata after merge"
        );
        assert_eq!(title, "Conversation from Server");
    });
}

#[test]
fn test_merge_cloud_metadata_refreshes_stale_restored_conversation_metadata() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();
        let token = "stale-metadata-token";

        let mut conversation = AIConversation::new(false, false);
        conversation.set_server_conversation_token(token.to_string());
        conversation.set_server_metadata(create_server_metadata(
            "Stale Conversation",
            token,
            1.0,
            None,
        ));
        let conversation_id = conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        history_model.update(&mut app, |model, _| {
            model.merge_cloud_conversation_metadata(vec![create_server_metadata(
                "Refreshed Conversation",
                token,
                2.0,
                None,
            )]);
        });

        history_model.read(&app, |model, _| {
            let token = ServerConversationToken::new(token.to_string());
            let metadata = model
                .get_server_conversation_metadata_by_server_token(&token)
                .expect("metadata should be available by server token");
            assert_eq!(metadata.title, "Refreshed Conversation");
            assert_eq!(metadata.usage.credits_spent, 2.0);

            let conversation_metadata = model
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.server_metadata())
                .expect("restored conversation metadata should be refreshed");
            assert_eq!(conversation_metadata.title, "Refreshed Conversation");
        });
    });
}

#[test]
fn test_merge_cloud_metadata_reuses_restored_conversation_id_for_token() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();
        let token = ServerConversationToken::new("restored-canonical-token".to_string());

        let mut conversation = AIConversation::new(false, false);
        conversation.set_server_conversation_token(token.as_str().to_string());
        let conversation_id = conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        history_model.update(&mut app, |model, _| {
            model.server_token_to_conversation_id.remove(&token);
            model.merge_cloud_conversation_metadata(vec![create_server_metadata(
                "Restored canonical conversation",
                token.as_str(),
                12.0,
                None,
            )]);
        });

        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
            assert_eq!(
                model
                    .conversation(&conversation_id)
                    .and_then(|conversation| conversation.server_metadata())
                    .map(|metadata| metadata.title.as_str()),
                Some("Restored canonical conversation"),
            );

            let metadata = model
                .get_conversation_metadata(&conversation_id)
                .expect("metadata should be inserted under the restored conversation id");
            assert_eq!(metadata.server_conversation_token.as_ref(), Some(&token));
            assert!(
                metadata.has_local_data,
                "restored conversation metadata should preserve local data"
            );
            assert_eq!(
                model
                    .all_conversations_metadata
                    .values()
                    .filter(|metadata| metadata.server_conversation_token.as_ref() == Some(&token))
                    .count(),
                1,
            );
        });
    });
}

#[test]
fn test_merge_cloud_metadata_removes_stale_duplicate_metadata_ids_for_token() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let token = ServerConversationToken::new("duplicate-metadata-token".to_string());

        let (canonical_conversation_id, stale_conversation_id) =
            history_model.update(&mut app, |model, _| {
                let canonical_conversation_id =
                    model.get_or_set_canonical_conversation_id_for_server_token(&token);
                let stale_conversation_id = AIConversationId::new();
                let stale_metadata = AIConversationMetadata::from_server_metadata(
                    stale_conversation_id,
                    create_server_metadata("Stale duplicate", token.as_str(), 1.0, None),
                );
                model
                    .all_conversations_metadata
                    .insert(stale_conversation_id, stale_metadata);

                model.merge_cloud_conversation_metadata(vec![create_server_metadata(
                    "Canonical metadata",
                    token.as_str(),
                    2.0,
                    None,
                )]);

                (canonical_conversation_id, stale_conversation_id)
            });

        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(canonical_conversation_id),
            );
            assert!(
                model
                    .get_conversation_metadata(&stale_conversation_id)
                    .is_none(),
                "stale metadata under a duplicate id should be removed",
            );
            assert_eq!(
                model
                    .get_conversation_metadata(&canonical_conversation_id)
                    .map(|metadata| metadata.title.as_str()),
                Some("Canonical metadata"),
            );
            assert_eq!(
                model
                    .all_conversations_metadata
                    .values()
                    .filter(|metadata| metadata.server_conversation_token.as_ref() == Some(&token))
                    .count(),
                1,
            );
        });
    });
}

#[test]
fn test_reserved_canonical_conversation_id_reused_by_later_metadata_merge() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let token = ServerConversationToken::new("reserved-fallback-token".to_string());

        let reserved_conversation_id = history_model.update(&mut app, |model, _| {
            model.get_or_set_canonical_conversation_id_for_server_token(&token)
        });

        history_model.update(&mut app, |model, _| {
            model.merge_cloud_conversation_metadata(vec![create_server_metadata(
                "Reserved fallback conversation",
                token.as_str(),
                9.0,
                None,
            )]);
        });

        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(reserved_conversation_id),
            );
            let metadata = model
                .get_conversation_metadata(&reserved_conversation_id)
                .expect("metadata should be inserted under the reserved id");
            assert_eq!(metadata.title, "Reserved fallback conversation");
            assert_eq!(metadata.server_conversation_token.as_ref(), Some(&token));
            assert_eq!(metadata.credits_spent, Some(9.0));
        });
    });
}

#[test]
fn test_transcript_viewer_terminal_view_is_not_marked_historical() {
    App::test((), |mut app| async move {
        let now = Local::now();
        let terminal_view_id = EntityId::new();

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();

            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };

            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    ResponseStreamId::new_for_test(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        history_model.update(&mut app, |history_model, _| {
            history_model.mark_terminal_view_as_conversation_transcript_viewer(terminal_view_id);
            history_model.mark_conversations_historical_for_terminal_view(terminal_view_id);
        });

        let historical_count = history_model.read(&app, |history_model, _| {
            history_model.get_local_conversations_metadata().count()
        });
        assert_eq!(historical_count, 0);
    });
}

#[test]
fn test_ambient_agent_conversations_excluded_from_list_but_accessible_by_id() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let regular_id = AIConversationId::new();
        let ambient_id = AIConversationId::new();

        let ambient_task_id: AmbientAgentTaskId = uuid::Uuid::new_v4().to_string().parse().unwrap();

        history_model.update(&mut app, |model, _| {
            let regular_metadata = AIConversationMetadata::from_server_metadata(
                regular_id,
                create_server_metadata("Regular Conversation", "token-regular", 5.0, None),
            );
            model
                .all_conversations_metadata
                .insert(regular_id, regular_metadata);

            let ambient_metadata = AIConversationMetadata::from_server_metadata(
                ambient_id,
                create_server_metadata(
                    "Ambient Conversation",
                    "token-ambient",
                    3.0,
                    Some(ambient_task_id),
                ),
            );
            model
                .all_conversations_metadata
                .insert(ambient_id, ambient_metadata);
        });

        history_model.read(&app, |model, _| {
            // get_local_conversations_metadata should exclude the ambient conversation
            let listed: Vec<&AIConversationMetadata> =
                model.get_local_conversations_metadata().collect();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, regular_id);

            // get_conversation_metadata should return both by ID
            assert!(model.get_conversation_metadata(&regular_id).is_some());
            assert!(model.get_conversation_metadata(&ambient_id).is_some());
            assert_eq!(
                model.get_conversation_metadata(&ambient_id).unwrap().title,
                "Ambient Conversation"
            );
        });
    });
}

#[test]
fn test_initialize_historical_conversations_indexes_child_conversations() {
    use crate::persistence::model::{AgentConversation, AgentConversationRecord};
    use chrono::NaiveDateTime;

    App::test((), |app| async move {
        let parent_id = AIConversationId::new();
        let child_id = AIConversationId::new();

        // Build a child AgentConversation whose conversation_data contains
        // a parent_conversation_id.  The child needs no tasks because
        // initialize_historical_conversations returns None (filters it out)
        // before inspecting tasks.
        let child_conversation_data = format!(r#"{{"parent_conversation_id":"{parent_id}"}}"#);

        let conversations = vec![AgentConversation {
            conversation: AgentConversationRecord {
                id: 1,
                conversation_id: child_id.to_string(),
                conversation_data: child_conversation_data,
                last_modified_at: NaiveDateTime::default(),
            },
            tasks: vec![],
        }];

        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &conversations));

        history_model.read(&app, |model, _| {
            // The child conversation should be indexed under its parent.
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);

            // The child should NOT appear in navigable conversation metadata.
            let metadata_ids: Vec<AIConversationId> = model
                .get_local_conversations_metadata()
                .map(|m| m.id)
                .collect();
            assert!(
                !metadata_ids.contains(&child_id),
                "child conversation should be excluded from metadata"
            );
        });
    });
}

#[test]
fn test_set_parent_for_conversation_populates_index() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // Create parent and child conversations via start_new_conversation.
        let parent_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // Set the parent-child relationship.
        history_model.update(&mut app, |model, _| {
            model.set_parent_for_conversation(child_id, parent_id);
        });

        // Verify the index is populated and the conversation has the parent set.
        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
            assert_eq!(model.child_conversations_of(parent_id).len(), 1);
            assert_eq!(model.child_conversations_of(parent_id)[0].id(), child_id);
            assert!(
                model
                    .conversation(&child_id)
                    .unwrap()
                    .parent_conversation_id()
                    == Some(parent_id)
            );
        });
    });
}

#[test]
fn test_set_parent_for_conversation_dedup() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // Set the same parent-child relationship twice.
        history_model.update(&mut app, |model, _| {
            model.set_parent_for_conversation(child_id, parent_id);
            model.set_parent_for_conversation(child_id, parent_id);
        });

        // Should have exactly one entry, not two.
        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
        });
    });
}

#[test]
fn test_set_parent_multiple_children() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_a = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let child_b = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |model, _| {
            model.set_parent_for_conversation(child_a, parent_id);
            model.set_parent_for_conversation(child_b, parent_id);
        });

        history_model.read(&app, |model, _| {
            let children = model.child_conversation_ids_of(&parent_id);
            assert_eq!(children.len(), 2);
            assert!(children.contains(&child_a));
            assert!(children.contains(&child_b));
            assert_eq!(model.child_conversations_of(parent_id).len(), 2);
        });
    });
}

#[test]
fn test_child_conversation_ids_of_unknown_parent() {
    App::test((), |app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let unknown_id = AIConversationId::new();

        history_model.read(&app, |model, _| {
            assert!(model.child_conversation_ids_of(&unknown_id).is_empty());
            assert!(model.child_conversations_of(unknown_id).is_empty());
        });
    });
}

#[test]
fn test_restore_conversations_maintains_children_by_parent() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversationId::new();
        let mut child_conv = AIConversation::new(false, false);
        child_conv.set_parent_conversation_id(parent_id);
        let child_id = child_conv.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child_conv], ctx);
        });

        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
        });
    });
}

#[test]
fn test_restore_conversations_dedup_children_by_parent() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversationId::new();
        let mut child_conv_a = AIConversation::new(false, false);
        child_conv_a.set_parent_conversation_id(parent_id);
        let child_id = child_conv_a.id();
        let child_conv_b = child_conv_a.clone();

        // Restore the same child conversation twice (simulates close + reopen).
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child_conv_a], ctx);
        });
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child_conv_b], ctx);
        });

        // Should have exactly one entry, not two.
        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
        });
    });
}

#[test]
fn test_all_cleared_conversations_includes_terminal_view_id() {
    App::test((), |mut app| async move {
        let now = Local::now();
        let terminal_view_id = EntityId::new();

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();

            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };

            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    ResponseStreamId::new_for_test(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.clear_conversations_in_terminal_view(terminal_view_id, ctx);
        });

        let has_cleared = history_model.read(&app, |history_model, _| {
            history_model
                .all_cleared_conversations()
                .iter()
                .any(|(id, convo)| *id == terminal_view_id && convo.id() == conversation_id)
        });

        assert!(has_cleared);
    });
}

#[test]
fn test_toggle_autoexecute_override_persists_updated_conversation_state() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.toggle_autoexecute_override(&conversation_id, terminal_view_id, ctx);
        });

        let event = receiver.recv_timeout(Duration::from_secs(1)).unwrap();

        let ModelEvent::UpdateMultiAgentConversation {
            conversation_id: persisted_conversation_id,
            conversation_data,
            ..
        } = event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };

        assert_eq!(persisted_conversation_id, conversation_id.to_string());
        assert_eq!(
            conversation_data.autoexecute_override,
            Some(PersistedAutoexecuteMode::RunToCompletion)
        );
    });
}

#[test]
fn test_update_event_sequence_persists_updated_conversation_state() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.update_event_sequence(conversation_id, 42, ctx);
        });

        let event = receiver.recv_timeout(Duration::from_secs(1)).unwrap();

        let ModelEvent::UpdateMultiAgentConversation {
            conversation_id: persisted_conversation_id,
            conversation_data,
            ..
        } = event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };

        assert_eq!(persisted_conversation_id, conversation_id.to_string());
        assert_eq!(conversation_data.last_event_sequence, Some(42));

        history_model.read(&app, |history_model, _| {
            let conversation = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist");
            assert_eq!(conversation.last_event_sequence(), Some(42));
        });
    });
}

#[test]
fn test_find_by_token_after_merge_cloud_metadata() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        history_model.update(&mut app, |model, _| {
            model.merge_cloud_conversation_metadata(vec![create_server_metadata(
                "New cloud conversation",
                "cloud-token-1",
                12.0,
                None,
            )]);
        });

        let token = ServerConversationToken::new("cloud-token-1".to_string());
        history_model.read(&app, |model, _| {
            let id = model
                .find_conversation_id_by_server_token(&token)
                .expect("token should resolve after merge_cloud_conversation_metadata");
            let metadata = model
                .get_conversation_metadata(&id)
                .expect("metadata should exist for resolved id");
            assert_eq!(
                metadata.server_conversation_token.as_ref(),
                Some(&token),
                "reverse index must point at the same metadata entry as the forward map",
            );
        });
    });
}

#[test]
fn test_find_by_token_after_restore_conversations() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let mut conversation = AIConversation::new(false, false);
        conversation.set_server_conversation_token("restored-token".to_string());
        let conversation_id = conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let token = ServerConversationToken::new("restored-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_returns_none_after_remove_conversation() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        // `delete_conversation` publishes persistence events via
        // `GlobalResourceHandlesProvider`, so we need a mock sender wired up.
        let (sender, _receiver) = std::sync::mpsc::sync_channel(2);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        history_model.update(&mut app, |model, _| {
            model.merge_cloud_conversation_metadata(vec![create_server_metadata(
                "Cloud conversation to remove",
                "removable-token",
                1.0,
                None,
            )]);
        });

        let token = ServerConversationToken::new("removable-token".to_string());
        let conversation_id = history_model.read(&app, |model, _| {
            model
                .find_conversation_id_by_server_token(&token)
                .expect("token should resolve before removal")
        });

        history_model.update(&mut app, |model, ctx| {
            model.delete_conversation(conversation_id, None, ctx);
        });

        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                None,
                "reverse index must be cleared when the conversation is removed",
            );
        });
    });
}

#[test]
fn test_find_by_token_returns_none_after_reset() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        history_model.update(&mut app, |model, _| {
            model.merge_cloud_conversation_metadata(vec![create_server_metadata(
                "Cloud conversation",
                "reset-token",
                1.0,
                None,
            )]);
        });

        let token = ServerConversationToken::new("reset-token".to_string());

        history_model.read(&app, |model, _| {
            assert!(model.find_conversation_id_by_server_token(&token).is_some());
        });

        history_model.update(&mut app, |model, _| {
            model.reset();
        });

        history_model.read(&app, |model, _| {
            assert_eq!(model.find_conversation_id_by_server_token(&token), None);
        });
    });
}

#[test]
fn test_find_by_token_after_initialize_output_for_response_stream() {
    App::test((), |mut app| async move {
        let now = Local::now();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // Prime a pending request so StreamInit can install the token.
        let stream_id = ResponseStreamId::new_for_test();
        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id.clone(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        let server_token_str = "init-token".to_string();
        history_model.update(&mut app, |history_model, ctx| {
            history_model.initialize_output_for_response_stream(
                &stream_id,
                conversation_id,
                terminal_view_id,
                warp_multi_agent_api::response_event::StreamInit {
                    request_id: String::new(),
                    conversation_id: server_token_str.clone(),
                    run_id: String::new(),
                },
                ctx,
            );
        });

        let token = ServerConversationToken::new(server_token_str);
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_after_assign_run_id_for_conversation() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            // Seed a token so assign_run_id has one to forward into the index.
            history_model
                .conversation_mut(&id)
                .expect("conversation should exist")
                .set_server_conversation_token("run-id-token".to_string());
            id
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.assign_run_id_for_conversation(
                conversation_id,
                "run-1".to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });

        let token = ServerConversationToken::new("run-id-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_after_insert_forked_conversation_from_tasks() {
    use crate::persistence::model::AgentConversationData;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let forked_conversation_id = AIConversationId::new();
        let conversation_data = AgentConversationData {
            server_conversation_token: Some("forked-token".to_string()),
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        };
        let tasks = vec![warp_multi_agent_api::Task {
            id: "root-task".to_string(),
            messages: vec![],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }];

        history_model.update(&mut app, |model, _| {
            model
                .insert_forked_conversation_from_tasks(
                    forked_conversation_id,
                    tasks,
                    conversation_data,
                )
                .expect("forked conversation should insert");
        });

        let token = ServerConversationToken::new("forked-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(forked_conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_after_mark_conversations_historical_for_terminal_view() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let now = Local::now();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        // Needs a real exchange to pass `conversation_would_render_in_blocklist`.
        let mut conversation = AIConversation::new(false, false);
        conversation.set_server_conversation_token("historical-token".to_string());
        let conversation_id = conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("historical query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    ResponseStreamId::new_for_test(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Sanity check: token resolves after restore_conversations.
        let token = ServerConversationToken::new("historical-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });

        history_model.update(&mut app, |model, _| {
            model.mark_conversations_historical_for_terminal_view(terminal_view_id);
        });

        // Token still resolves via the metadata-side index entry.
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
            assert!(
                model.get_conversation_metadata(&conversation_id).is_some(),
                "metadata entry must exist so the reverse index is not dangling",
            );
        });
    });
}

#[test]
fn test_set_server_conversation_token_rebinds_reverse_index() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(id, "old".to_string());
            id
        });

        let old_token = ServerConversationToken::new("old".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&old_token),
                Some(conversation_id),
            );
        });

        history_model.update(&mut app, |history_model, _| {
            history_model
                .set_server_conversation_token_for_conversation(conversation_id, "new".to_string());
        });

        let new_token = ServerConversationToken::new("new".to_string());
        history_model.read(&app, |model, _| {
            // Stale lookups must not resolve to the rebound conversation.
            assert_eq!(model.find_conversation_id_by_server_token(&old_token), None);
            assert_eq!(
                model.find_conversation_id_by_server_token(&new_token),
                Some(conversation_id),
            );
        });
    });
}

/// REMOTE-1519 fork-on-chip-click flow.
/// Forking the local conversation must:
/// 1. carry the source's server token forward as `forked_from_*` (so the
/// cloud agent's response stream can be reconciled to the right local
/// conversation during replay), and
/// 2. accept a binding to the cloud T_C via
/// `set_server_conversation_token_for_conversation` such that the reverse
/// index resolves the cloud token to the forked conversation.
#[test]
fn test_fork_then_bind_handoff_token_resolves_to_forked_conversation() {
    use crate::ai::agent::conversation::AIConversation;
    use crate::persistence::model::AgentConversationData;
    use crate::test_util::ai_agent_tasks::{create_api_task, create_message};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        // `fork_conversation` writes the new conversation through the
        // sqlite sender, so a mock sender must be wired up.
        let (sender, _receiver) = std::sync::mpsc::sync_channel(2);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        // Build a source conversation with a real root task (so `fork_conversation`
        // has a `Task::source()` to copy forward) and the local-side server token T_L.
        let source_id = AIConversationId::new();
        let root_task = create_api_task(
            "root-task",
            vec![create_message("root-task-message", "root-task")],
        );
        let source = AIConversation::new_restored(
            source_id,
            vec![root_task],
            Some(AgentConversationData {
                server_conversation_token: Some("src-token".to_string()),
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                orchestration_harness_type: None,
                parent_conversation_id: None,
                is_remote_child: false,
                run_id: None,
                autoexecute_override: None,
                last_event_sequence: None,
                pinned: false,
            }),
        )
        .expect("restored source conversation should build");
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![source], ctx);
        });

        // Fork the local conversation (REMOTE-1519: fork-on-chip-click).
        let forked_id = history_model.update(&mut app, |model, ctx| {
            let source = model
                .conversation(&source_id)
                .expect("source conversation must be in memory after restore")
                .clone();
            let forked = model
                .fork_conversation(&source, "[Fork] ", false, None, ctx)
                .expect("fork must succeed when sqlite sender is wired up");
            assert_eq!(
                forked
                    .forked_from_server_conversation_token()
                    .map(|t| t.as_str()),
                Some("src-token"),
                "forked conversation must carry its source token for replay reconciliation",
            );
            assert!(
                forked.server_conversation_token().is_none(),
                "freshly forked conversation must not yet have a server token of its own",
            );
            forked.id()
        });

        // Bind the cloud T_C returned by the fork RPC to the forked conversation.
        history_model.update(&mut app, |model, _| {
            model.set_server_conversation_token_for_conversation(forked_id, "cloud-T".to_string());
        });

        let cloud_token = ServerConversationToken::new("cloud-T".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&cloud_token),
                Some(forked_id),
                "after binding, cloud T_C must resolve to the forked conversation",
            );
        });
    });
}

/// REMOTE-1519 local-to-cloud handoff requires `preserve_task_ids: true` so the local fork's
/// task store matches the cloud-side fork (a byte-for-byte GCS copy of the source). Verifies
/// that root and subtask ids are preserved across the fork, the subtask's `parent_task_id`
/// reference still points at the source's root id, and only the root task description is
/// prefixed.
#[test]
fn test_fork_conversation_preserves_task_ids_when_requested() {
    use crate::ai::agent::conversation::AIConversation;
    use crate::persistence::model::AgentConversationData;
    use crate::test_util::ai_agent_tasks::{create_api_subtask, create_api_task, create_message};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, _receiver) = std::sync::mpsc::sync_channel(2);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let source_id = AIConversationId::new();
        let mut root_task = create_api_task(
            "root-task-id",
            vec![create_message("root-msg", "root-task-id")],
        );
        root_task.description = "Original root".to_string();
        let mut subtask = create_api_subtask(
            "subtask-id",
            "root-task-id",
            vec![create_message("sub-msg", "subtask-id")],
        );
        subtask.description = "Original subtask".to_string();
        let source = AIConversation::new_restored(
            source_id,
            vec![root_task, subtask],
            Some(AgentConversationData {
                server_conversation_token: Some("src-token".to_string()),
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                orchestration_harness_type: None,
                parent_conversation_id: None,
                is_remote_child: false,
                run_id: None,
                autoexecute_override: None,
                last_event_sequence: None,
                pinned: false,
            }),
        )
        .expect("restored source conversation should build");
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![source], ctx);
        });

        history_model.update(&mut app, |model, ctx| {
            let source = model
                .conversation(&source_id)
                .expect("source conversation must be in memory after restore")
                .clone();
            let forked = model
                .fork_conversation(&source, "[Fork] ", true, None, ctx)
                .expect("fork must succeed when sqlite sender is wired up");

            let forked_tasks: Vec<&warp_multi_agent_api::Task> =
                forked.all_tasks().filter_map(|t| t.source()).collect();
            let forked_root = forked_tasks
                .iter()
                .find(|t| t.id == "root-task-id")
                .expect("root task id must be preserved across fork");
            let forked_subtask = forked_tasks
                .iter()
                .find(|t| t.id == "subtask-id")
                .expect("subtask id must be preserved across fork");
            assert_eq!(
                forked_subtask
                    .dependencies
                    .as_ref()
                    .map(|d| d.parent_task_id.as_str()),
                Some("root-task-id"),
                "subtask must still reference the original root task id",
            );
            assert_eq!(
                forked_root.description, "[Fork] Original root",
                "root task description must be prefixed",
            );
            assert_eq!(
                forked_subtask.description, "Original subtask",
                "subtask description must not be prefixed",
            );
        });
    });
}

#[test]
fn test_fork_conversation_title_override_replaces_prefix() {
    use crate::ai::agent::conversation::AIConversation;
    use crate::persistence::model::AgentConversationData;
    use crate::test_util::ai_agent_tasks::{create_api_task, create_message};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, _receiver) = std::sync::mpsc::sync_channel(2);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let source_id = AIConversationId::new();
        let mut root_task = create_api_task(
            "root-task-id",
            vec![create_message("root-msg", "root-task-id")],
        );
        root_task.description = "Original root".to_string();
        let source = AIConversation::new_restored(
            source_id,
            vec![root_task],
            Some(AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                orchestration_harness_type: None,
                parent_conversation_id: None,
                is_remote_child: false,
                run_id: None,
                autoexecute_override: None,
                last_event_sequence: None,
                pinned: false,
            }),
        )
        .expect("restored source conversation should build");
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![source], ctx);
        });

        history_model.update(&mut app, |model, ctx| {
            let source = model
                .conversation(&source_id)
                .expect("source must be in memory")
                .clone();
            let forked = model
                .fork_conversation(&source, "[Fork] ", false, Some("Custom title"), ctx)
                .expect("fork must succeed");

            let forked_root = forked
                .all_tasks()
                .find_map(|t| t.source())
                .expect("forked conversation must have a root task");
            assert_eq!(
                forked_root.description, "Custom title",
                "title_override must replace the prefix+description",
            );
        });
    });
}
