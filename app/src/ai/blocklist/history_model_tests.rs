use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use itertools::Itertools;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warpui::{App, EntityId};

use super::{
    convert_persisted_conversation_to_ai_conversation_with_metadata, AIConversationMetadata,
    AIQueryHistoryOutputStatus, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, PersistedAIInput,
    PersistedAIInputType,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversationId, ServerAIConversationMetadata,
};
use crate::ai::agent::{
    AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus, FinishedAIAgentOutput,
    Shared, UserQueryMode,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::controller::RequestInput;
use crate::ai::blocklist::ResponseStreamId;
use crate::ai::llms::LLMId;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions};
use crate::input_suggestions::HistoryInputSuggestion;
use crate::persistence::model::{
    AgentConversation, AgentConversationData, AgentConversationRecord, PersistedAutoexecuteMode,
};
use crate::persistence::ModelEvent;
use crate::server::ids::ServerId;
use crate::terminal::model::session::SessionId;
use crate::test_util::settings::{
    initialize_history_persistence_for_tests, initialize_settings_for_tests,
};
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

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

fn create_user_query_message(
    id: &str,
    task_id: &str,
    request_id: &str,
    query: &str,
) -> warp_multi_agent_api::Message {
    warp_multi_agent_api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(warp_multi_agent_api::message::Message::UserQuery(
            warp_multi_agent_api::message::UserQuery {
                query: query.to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            },
        )),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn persisted_agent_conversation(
    conversation_id: AIConversationId,
    conversation_data: AgentConversationData,
    last_modified_at: chrono::NaiveDateTime,
    initial_query: Option<&str>,
) -> AgentConversation {
    let task_id = format!("task-{conversation_id}");
    let tasks = initial_query
        .map(|query| {
            vec![warp_multi_agent_api::Task {
                id: task_id.clone(),
                messages: vec![create_user_query_message(
                    "message-1",
                    &task_id,
                    "request-1",
                    query,
                )],
                dependencies: None,
                description: query.to_string(),
                summary: String::new(),
                server_data: String::new(),
            }]
        })
        .unwrap_or_default();

    AgentConversation {
        conversation: AgentConversationRecord {
            id: 0,
            conversation_id: conversation_id.to_string(),
            conversation_data: serde_json::to_string(&conversation_data)
                .expect("conversation data should serialize"),
            last_modified_at,
        },
        tasks,
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

fn persisted_agent_conversation_from_update_event(event: ModelEvent) -> AgentConversation {
    let ModelEvent::UpdateMultiAgentConversation {
        conversation_id,
        updated_tasks,
        conversation_data,
    } = event
    else {
        panic!("expected UpdateMultiAgentConversation event");
    };

    AgentConversation {
        conversation: AgentConversationRecord {
            id: 0,
            conversation_id,
            conversation_data: serde_json::to_string(&conversation_data)
                .expect("conversation data should serialize"),
            last_modified_at: Utc::now().naive_utc(),
        },
        tasks: updated_tasks,
    }
}

#[test]
fn start_new_child_conversation_persists_harness_metadata() {
    App::test((), |mut app| async move {
        initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());

        // Pick a non-nil UUID for the parent run_id so the orchestration
        // capability gate (which now reads run_id() exclusively) sees a valid
        // agent identifier when seeding the child's parent_agent_id.
        const PARENT_RUN_ID: &str = "00000000-0000-0000-0000-000000000001";
        let (child_a, child_b, child_ids) = history_model.update(&mut app, |history_model, ctx| {
            let parent_conversation_id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            if let Some(parent) = history_model.conversation_mut(&parent_conversation_id) {
                parent.set_run_id(PARENT_RUN_ID.to_string());
            }
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
            assert_eq!(child_a_conversation.parent_agent_id(), Some(PARENT_RUN_ID));
            assert_eq!(child_b_conversation.parent_agent_id(), Some(PARENT_RUN_ID));
        });
    });
}

#[test]
fn test_initialize_historical_conversations_resolves_parent_agent_id_children_via_seeded_run_ids() {
    App::test((), |app| async move {
        let parent_id = AIConversationId::new();
        let child_id = AIConversationId::new();
        let parent_run_id = Uuid::new_v4().to_string();
        let now = Utc::now().naive_utc();

        let conversations = vec![
            persisted_agent_conversation(
                child_id,
                AgentConversationData {
                    server_conversation_token: Some("child-token".to_string()),
                    conversation_usage_metadata: None,
                    reverted_action_ids: None,
                    forked_from_server_conversation_token: None,
                    artifacts_json: None,
                    parent_agent_id: Some(parent_run_id.clone()),
                    agent_name: Some("Child agent".to_string()),
                    orchestration_harness_type: None,
                    parent_conversation_id: None,
                    is_remote_child: true,
                    root_task_is_optimistic: None,
                    run_id: None,
                    autoexecute_override: None,
                    last_event_sequence: None,
                    pinned: false,
                },
                now,
                None,
            ),
            persisted_agent_conversation(
                parent_id,
                AgentConversationData {
                    server_conversation_token: Some("parent-token".to_string()),
                    conversation_usage_metadata: None,
                    reverted_action_ids: None,
                    forked_from_server_conversation_token: None,
                    artifacts_json: None,
                    parent_agent_id: None,
                    agent_name: None,
                    orchestration_harness_type: None,
                    parent_conversation_id: None,
                    is_remote_child: false,
                    root_task_is_optimistic: None,
                    run_id: Some(parent_run_id.clone()),
                    autoexecute_override: None,
                    last_event_sequence: None,
                    pinned: false,
                },
                now - chrono::Duration::seconds(1),
                Some("Parent query"),
            ),
        ];

        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &conversations));

        history_model.read(&app, |model, _| {
            assert_eq!(
                model.conversation_id_for_agent_id(&parent_run_id),
                Some(parent_id),
                "startup hydration should seed the run-id lookup before linking children",
            );
            assert_eq!(
                model.child_conversation_ids_of(&parent_id),
                &[child_id],
                "parent_agent_id-only children should be indexed under their resolved parent",
            );
        });
    });
}

#[test]
fn test_initialize_historical_conversations_eagerly_hydrates_orchestration_children() {
    // Fix C: orchestration children should be inserted into `conversations_by_id`
    // eagerly during `initialize_historical_conversations` so the pill bar and
    // orchestration transcript name resolution can find them before the parent's
    // hidden child pane materializes lazily. Non-orchestration historical rows
    // must stay on the lazy path.
    App::test((), |app| async move {
        let parent_id = AIConversationId::new();
        let child_id = AIConversationId::new();
        let parent_run_id = Uuid::new_v4().to_string();
        let child_run_id = Uuid::new_v4().to_string();
        let now = Utc::now().naive_utc();

        let conversations = vec![
            persisted_agent_conversation(
                child_id,
                AgentConversationData {
                    server_conversation_token: Some("child-token".to_string()),
                    conversation_usage_metadata: None,
                    reverted_action_ids: None,
                    forked_from_server_conversation_token: None,
                    artifacts_json: None,
                    parent_agent_id: Some(parent_run_id.clone()),
                    agent_name: Some("Agent 1".to_string()),
                    orchestration_harness_type: None,
                    parent_conversation_id: Some(parent_id.to_string()),
                    is_remote_child: false,
                    root_task_is_optimistic: None,
                    run_id: Some(child_run_id.clone()),
                    autoexecute_override: None,
                    last_event_sequence: None,
                    pinned: false,
                },
                now,
                // Child needs at least one root task so `AIConversation::new_restored` succeeds.
                Some("Child query"),
            ),
            persisted_agent_conversation(
                parent_id,
                AgentConversationData {
                    server_conversation_token: Some("parent-token".to_string()),
                    conversation_usage_metadata: None,
                    reverted_action_ids: None,
                    forked_from_server_conversation_token: None,
                    artifacts_json: None,
                    parent_agent_id: None,
                    agent_name: None,
                    orchestration_harness_type: None,
                    parent_conversation_id: None,
                    is_remote_child: false,
                    root_task_is_optimistic: None,
                    run_id: Some(parent_run_id.clone()),
                    autoexecute_override: None,
                    last_event_sequence: None,
                    pinned: false,
                },
                now - chrono::Duration::seconds(1),
                Some("Parent query"),
            ),
        ];

        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &conversations));

        history_model.read(&app, |model, _| {
            // Child is hydrated into conversations_by_id eagerly so the pill
            // bar / transcript name resolution can find it.
            assert!(
                model.conversation(&child_id).is_some(),
                "Fix C: orchestration child should be eagerly hydrated into conversations_by_id",
            );
            // children_by_parent still gets populated as before.
            assert_eq!(
                model.child_conversation_ids_of(&parent_id),
                &[child_id],
                "orchestration children should still be indexed in children_by_parent",
            );
            // run_id index seeded for the child so name resolution succeeds.
            assert_eq!(
                model.conversation_id_for_agent_id(&child_run_id),
                Some(child_id),
                "child run_id should be indexed in agent_id_to_conversation_id",
            );
            // Parent run_id index is also seeded (matches existing behavior).
            assert_eq!(
                model.conversation_id_for_agent_id(&parent_run_id),
                Some(parent_id),
                "parent run_id should still be indexed in agent_id_to_conversation_id",
            );
            // Parent must NOT be in conversations_by_id yet; it remains on the
            // existing lazy path via `restore_conversations`.
            assert!(
                model.conversation(&parent_id).is_none(),
                "Fix C: parent conversation should NOT be eagerly loaded into conversations_by_id",
            );
            // Parent metadata is still recorded in all_conversations_metadata.
            assert!(
                model.get_conversation_metadata(&parent_id).is_some(),
                "parent metadata should be recorded in all_conversations_metadata",
            );
            // Child metadata must NOT be recorded in all_conversations_metadata
            // (orchestration children are managed by their parent and excluded from navigation).
            assert!(
                model.get_conversation_metadata(&child_id).is_none(),
                "child metadata should NOT be recorded in all_conversations_metadata",
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
        platform_credits_spent: 0.0,
        credits_spent_for_last_block: None,
        token_usage: vec![],
        tool_usage_metadata: Default::default(),
    };

    ServerAIConversationMetadata {
        title: title.to_string(),
        usage,
        metadata: create_mock_server_metadata(),
        creator: None,
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
    use chrono::NaiveDateTime;

    use crate::persistence::model::{AgentConversation, AgentConversationRecord};

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
fn test_restore_conversations_indexes_child_by_parent_agent_id() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let parent_run_id = Uuid::new_v4().to_string();

        let mut parent_conversation = AIConversation::new(false, false);
        parent_conversation.set_run_id(parent_run_id.clone());
        let parent_id = parent_conversation.id();

        let mut child_conversation = AIConversation::new(false, false);
        child_conversation.set_parent_agent_id(parent_run_id);
        let child_id = child_conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![parent_conversation], ctx);
            model.restore_conversations(terminal_view_id, vec![child_conversation], ctx);
        });

        history_model.read(&app, |model, _| {
            assert_eq!(
                model.child_conversation_ids_of(&parent_id),
                &[child_id],
                "runtime restoration should index parent_agent_id-only children under their parent",
            );
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
fn test_start_new_child_conversation_persists_child_metadata_for_restore() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let parent_run_id = Uuid::new_v4().to_string();

        let (parent_conversation_id, child_conversation_id, expected_parent_agent_id) =
            history_model.update(&mut app, |history_model, ctx| {
                let parent_conversation_id = history_model.start_new_conversation(
                    terminal_view_id,
                    false,
                    false,
                    false,
                    ctx,
                );
                history_model.set_server_conversation_token_for_conversation(
                    parent_conversation_id,
                    "parent-server-token".to_string(),
                );
                history_model
                    .conversation_mut(&parent_conversation_id)
                    .expect("parent conversation should exist")
                    .set_run_id(parent_run_id.clone());
                let expected_parent_agent_id = history_model
                    .conversation(&parent_conversation_id)
                    .and_then(|conversation| conversation.orchestration_agent_id())
                    .expect("parent conversation should expose an orchestration agent id");
                let child_conversation_id = history_model.start_new_child_conversation(
                    terminal_view_id,
                    "Agent 1".to_string(),
                    parent_conversation_id,
                    Some(Harness::Claude),
                    ctx,
                );
                (
                    parent_conversation_id,
                    child_conversation_id,
                    expected_parent_agent_id,
                )
            });

        let persisted_conversation = persisted_agent_conversation_from_update_event(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("child creation should persist conversation state"),
        );
        let restored =
            convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
                .expect("persisted child conversation should be restorable");

        assert_eq!(restored.id(), child_conversation_id);
        assert_eq!(
            restored.parent_conversation_id(),
            Some(parent_conversation_id)
        );
        assert_eq!(
            restored.parent_agent_id(),
            Some(expected_parent_agent_id.as_str())
        );
        assert_eq!(restored.agent_name(), Some("Agent 1"));
        assert_eq!(restored.orchestration_harness(), Some(Harness::Claude));
    });
}

#[test]
fn test_mark_conversation_as_remote_child_persists_updated_conversation_state() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.mark_conversation_as_remote_child(conversation_id, ctx);
        });

        let persisted_conversation = persisted_agent_conversation_from_update_event(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("remote child mutation should persist conversation state"),
        );
        let restored =
            convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
                .expect("persisted remote child conversation should be restorable");

        assert_eq!(restored.id(), conversation_id);
        assert!(restored.is_remote_child());
    });
}

/// Persisting a conversation whose root is still `Optimistic(Root)` (i.e.
/// the server has not yet upgraded it via a `CreateTask` action) must NOT
/// emit a stub `api::Task` in `updated_tasks`.
///
/// Previously, `Task::source_for_persistence` returned a synthetic empty
/// `api::Task` keyed by the client-generated optimistic UUID, which
/// accumulated as an orphan row in `agent_tasks` and broke later restores
/// via `HashMap` iteration non-determinism in `AIConversation::new_restored`
/// (when two parentless tasks — the stub and the real server root —
/// co-existed and the stub randomly won).
#[test]
fn test_persist_with_optimistic_root_emits_event_with_no_task_rows() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();

        // Create a fresh conversation. Its root is `Optimistic(Root)` with a
        // client-generated UUID; no server response has been received.
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // Force a persist while the root is still optimistic.
        // `mark_conversation_as_remote_child` is one of several early-persist
        // sites; any of them would exhibit the same writer behavior.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.mark_conversation_as_remote_child(conversation_id, ctx);
        });

        let event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("optimistic-root persist should emit an UpdateMultiAgentConversation event");

        let ModelEvent::UpdateMultiAgentConversation {
            updated_tasks,
            conversation_data,
            ..
        } = event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };

        // The fix: optimistic-root tasks must not produce any persisted task rows.
        assert!(
            updated_tasks.is_empty(),
            "Persisting a conversation whose root is still Optimistic(Root) must emit zero \
             task rows; got {} task(s) with ids: {:?}",
            updated_tasks.len(),
            updated_tasks
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
        );

        // The legacy `root_task_is_optimistic` flag must no longer be written.
        assert!(
            conversation_data.root_task_is_optimistic.is_none(),
            "conversation_data.root_task_is_optimistic must not be written (legacy field); \
             got {:?}",
            conversation_data.root_task_is_optimistic,
        );
    });
}

/// Once the in-memory root has been upgraded from `Optimistic(Root)` to a
/// server-backed `Task`, the next `persist_conversation_state` must emit
/// exactly one task row with the server-assigned id and no dependencies.
/// Previously, the persist also retained the original optimistic stub row,
/// producing two parentless rows that broke restore.
#[test]
fn test_optimistic_root_upgrade_then_persist_emits_event_with_single_server_task_row() {
    use crate::test_util::ai_agent_tasks::create_api_task;

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(2);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // First persist: while the root is still Optimistic(Root).
        history_model.update(&mut app, |history_model, ctx| {
            history_model.mark_conversation_as_remote_child(conversation_id, ctx);
        });
        let first_event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("first persist event must arrive");
        let ModelEvent::UpdateMultiAgentConversation {
            updated_tasks: first_updated_tasks,
            ..
        } = first_event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };
        assert!(
            first_updated_tasks.is_empty(),
            "precondition: optimistic-root persist must emit zero task rows",
        );

        // Drive the optimistic→server upgrade in-place and trigger another
        // persist via mark_conversation_as_remote_child (idempotent setter +
        // unconditional persist) to keep this test isolated from the full
        // response-stream/CreateTask plumbing.
        let server_root_id = "server-root-task-id".to_string();
        history_model.update(&mut app, |history_model, ctx| {
            let conversation = history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should still exist");
            conversation.upgrade_optimistic_root_to_server_task_for_test(create_api_task(
                &server_root_id,
                vec![],
            ));
            history_model.mark_conversation_as_remote_child(conversation_id, ctx);
        });

        let second_event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("post-upgrade persist event must arrive");
        let ModelEvent::UpdateMultiAgentConversation {
            updated_tasks: second_updated_tasks,
            ..
        } = second_event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };

        assert_eq!(
            second_updated_tasks.len(),
            1,
            "post-upgrade persist must emit exactly one task row (the server root); got {} task(s) with ids {:?}",
            second_updated_tasks.len(),
            second_updated_tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        );
        let only_task = &second_updated_tasks[0];
        assert_eq!(
            only_task.id, server_root_id,
            "post-upgrade persist row id must match the server-assigned id",
        );
        assert!(
            only_task.dependencies.is_none(),
            "the server root must be parentless (no dependencies); got {:?}",
            only_task.dependencies,
        );
    });
}

/// Round-trip: take the persist event emitted while the root is still
/// optimistic, build an `AgentConversation` from it (with the expected empty
/// `tasks` list), feed it through the local-DB restore path, and confirm we
/// get back an `InProgress` conversation with a fresh optimistic root and all
/// linkage metadata preserved.
#[test]
fn test_optimistic_root_restore_round_trip_yields_in_progress_optimistic_root() {
    use crate::ai::agent::conversation::ConversationStatus;

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(2);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();

        // Set up a parent conversation so the child has a real parent_agent_id.
        let (child_conversation_id, expected_parent_agent_id) =
            history_model.update(&mut app, |history_model, ctx| {
                let parent_id = history_model.start_new_conversation(
                    terminal_view_id,
                    false,
                    false,
                    false,
                    ctx,
                );
                let parent_run_id = Uuid::new_v4().to_string();
                history_model
                    .conversation_mut(&parent_id)
                    .expect("parent conversation should exist")
                    .set_run_id(parent_run_id.clone());
                // Drain any persist event from parent setup. start_new_conversation
                // itself does not persist; nothing should be enqueued yet.
                let child_id = history_model.start_new_child_conversation(
                    terminal_view_id,
                    "Round-trip child".to_string(),
                    parent_id,
                    Some(Harness::Claude),
                    ctx,
                );
                let expected_parent_agent_id = history_model
                    .conversation(&child_id)
                    .and_then(|c| c.parent_agent_id().map(|s| s.to_string()))
                    .expect("child conversation should have its parent_agent_id stamped");
                (child_id, expected_parent_agent_id)
            });

        // The child-creation call site is itself one of the early-persist
        // sites; consume that first event for the assertion below.
        let first_event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("child creation should persist conversation state");
        let ModelEvent::UpdateMultiAgentConversation {
            conversation_id: child_id_str,
            updated_tasks,
            conversation_data,
        } = first_event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };
        assert_eq!(child_id_str, child_conversation_id.to_string());
        assert!(
            updated_tasks.is_empty(),
            "child conversation persisted while root is optimistic must emit zero task rows",
        );

        // Round-trip via the local-DB loader.
        let persisted = AgentConversation {
            conversation: AgentConversationRecord {
                id: 0,
                conversation_id: child_id_str.clone(),
                conversation_data: serde_json::to_string(&conversation_data)
                    .expect("conversation data should serialize"),
                last_modified_at: Utc::now().naive_utc(),
            },
            tasks: updated_tasks,
        };
        let restored = convert_persisted_conversation_to_ai_conversation_with_metadata(persisted)
            .expect("empty-tasks restore must succeed");

        assert_eq!(restored.id(), child_conversation_id);
        let root_task = restored.get_root_task().expect("root task should exist");
        assert!(root_task.is_root_task());
        assert!(
            root_task.source().is_none(),
            "the synthesized restored root must be optimistic (no api::Task source)",
        );
        assert_eq!(restored.status(), &ConversationStatus::InProgress);
        assert!(restored.status_error_message().is_none());

        // All persisted linkage metadata must round-trip.
        assert_eq!(
            restored.parent_agent_id(),
            Some(expected_parent_agent_id.as_str()),
        );
        assert_eq!(restored.agent_name(), Some("Round-trip child"));
        assert_eq!(restored.orchestration_harness(), Some(Harness::Claude));
    });
}

/// `AIConversation::truncate_from_exchange` resets the root to
/// `Optimistic(Root)` when all exchanges are removed and then calls
/// `write_updated_conversation_state`. That persist must emit zero task rows
/// (the synthesized optimistic root no longer produces a stub).
#[test]
fn test_truncate_from_exchange_to_empty_persist_event_has_empty_updated_tasks() {
    use crate::test_util::ai_agent_tasks::create_api_task;

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let now = Local::now();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // Upgrade the root to a server-backed task so the truncate path
        // ("all exchanges removed → reset to optimistic") actually involves a
        // real server root being torn down.
        let server_root_id = "truncate-server-root".to_string();
        history_model.update(&mut app, |history_model, _ctx| {
            let conversation = history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist");
            conversation.upgrade_optimistic_root_to_server_task_for_test(create_api_task(
                &server_root_id,
                vec![],
            ));
        });

        // Append an exchange tied to the now server-backed root, then
        // truncate from it. The exchange add path does not persist; the
        // truncate call does. `update_for_new_request_input` allocates a
        // fresh exchange id internally, so we look the freshly-assigned id
        // up on the conversation rather than reusing the dummy exchange's
        // id from `create_exchange_with_query`.
        let stream_id = ResponseStreamId::new_for_test();
        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("truncate me", now, None);
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(
                    crate::ai::agent::task::TaskId::new(server_root_id.clone()),
                    exchange.input,
                )]),
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
                .expect("update_for_new_request_input must succeed on server-backed root");
        });
        let exchange_id = history_model.read(&app, |model, _| {
            model
                .conversation(&conversation_id)
                .expect("conversation should exist")
                .get_root_task()
                .expect("root task should exist")
                .exchanges()
                .last()
                .map(|e| e.id)
                .expect("a freshly-appended exchange must exist on the root task")
        });

        history_model.update(&mut app, |history_model, ctx| {
            let conversation = history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist");
            conversation
                .truncate_from_exchange(exchange_id, ctx)
                .expect("truncating from an existing exchange must succeed");
        });

        let event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("truncate-to-empty must emit an UpdateMultiAgentConversation event");
        let ModelEvent::UpdateMultiAgentConversation { updated_tasks, .. } = event else {
            panic!("expected UpdateMultiAgentConversation event");
        };
        assert!(
            updated_tasks.is_empty(),
            "truncate-to-empty resets the root to optimistic; the persist must emit zero task rows, got {} row(s) with ids {:?}",
            updated_tasks.len(),
            updated_tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        );
    });
}

/// End-to-end happy path: start → early persist → upgrade → persist → restart
/// → post-restore persist → restart. After two restart cycles, the final
/// restored conversation must contain exactly one server-backed root task
/// with the server id and no orphan optimistic tasks.
#[test]
fn test_two_restart_cycles_keep_exactly_one_server_root_task_row() {
    use crate::test_util::ai_agent_tasks::create_api_task;

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        // Early persist while the root is still optimistic.
        history_model.update(&mut app, |history_model, ctx| {
            history_model.mark_conversation_as_remote_child(conversation_id, ctx);
        });
        let early_event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("early persist event must arrive");
        let ModelEvent::UpdateMultiAgentConversation {
            updated_tasks: early_updated_tasks,
            ..
        } = early_event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };
        assert!(
            early_updated_tasks.is_empty(),
            "early persist must not write any optimistic-stub task rows",
        );

        // Drive the optimistic→server upgrade and trigger another persist.
        let server_root_id = "server-root".to_string();
        history_model.update(&mut app, |history_model, ctx| {
            let conversation = history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist");
            conversation.upgrade_optimistic_root_to_server_task_for_test(create_api_task(
                &server_root_id,
                vec![],
            ));
            history_model.mark_conversation_as_remote_child(conversation_id, ctx);
        });
        let post_upgrade_event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("post-upgrade persist event must arrive");
        let post_upgrade_persisted =
            persisted_agent_conversation_from_update_event(post_upgrade_event);
        assert_eq!(
            post_upgrade_persisted.tasks.len(),
            1,
            "post-upgrade persist must emit exactly one task row (the real server root)",
        );
        assert_eq!(post_upgrade_persisted.tasks[0].id, server_root_id);

        // Simulate quit/restart #1: feed the persisted event through the
        // local-DB restore helper.
        let restored_after_restart_1 =
            convert_persisted_conversation_to_ai_conversation_with_metadata(post_upgrade_persisted)
                .expect("first simulated restart must restore cleanly");
        let restart_1_root = restored_after_restart_1
            .get_root_task()
            .expect("root task must exist after restart 1");
        assert!(
            restart_1_root.source().is_some(),
            "restart 1 root must be server-backed"
        );
        assert_eq!(
            restart_1_root.id().to_string(),
            server_root_id,
            "restart 1 root must use the server-assigned id",
        );
        assert_eq!(
            restored_after_restart_1.all_tasks().count(),
            1,
            "restart 1 must produce exactly one task (no orphan optimistic stub)",
        );

        // "Reload" the restored conversation into the in-memory model and
        // trigger another post-restore persist site. `restore_conversations`
        // uses `conversations_by_id.insert(...)` which overwrites the existing
        // in-memory entry under the same id, so we do NOT delete first
        // (`delete_conversation` would enqueue two model events that would
        // race the persist event we want to recv below).
        let restart_1_terminal_view_id = EntityId::new();
        history_model.update(&mut app, |history_model, ctx| {
            history_model.restore_conversations(
                restart_1_terminal_view_id,
                vec![restored_after_restart_1],
                ctx,
            );
            history_model.mark_conversation_as_remote_child(conversation_id, ctx);
        });

        let post_restart_event = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("post-restore persist event must arrive");
        let post_restart_persisted =
            persisted_agent_conversation_from_update_event(post_restart_event);
        assert_eq!(
            post_restart_persisted.tasks.len(),
            1,
            "post-restore persist must still emit exactly one task row (no accumulated stubs)",
        );
        assert_eq!(post_restart_persisted.tasks[0].id, server_root_id);

        // Simulate quit/restart #2.
        let restored_after_restart_2 =
            convert_persisted_conversation_to_ai_conversation_with_metadata(post_restart_persisted)
                .expect("second simulated restart must restore cleanly");

        // Still exactly one server-backed root with the server id, no orphan
        // optimistic tasks anywhere in the task store.
        let restart_2_tasks: Vec<_> = restored_after_restart_2.all_tasks().collect();
        assert_eq!(
            restart_2_tasks.len(),
            1,
            "final restored conversation must have exactly one task; got {}",
            restart_2_tasks.len(),
        );
        let restart_2_root = restored_after_restart_2
            .get_root_task()
            .expect("root task must exist after restart 2");
        assert!(
            restart_2_root.source().is_some(),
            "restart 2 root must be server-backed",
        );
        assert_eq!(
            restart_2_root.id().to_string(),
            server_root_id,
            "restart 2 root id must still match the server-assigned id",
        );
    });
}

#[test]
fn test_initialize_output_for_response_stream_persists_updated_conversation_state() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();
        let now = Local::now();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });

        let stream_id = ResponseStreamId::new_for_test();
        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist")
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

        let server_token = "stream-init-token".to_string();
        let run_id = Uuid::new_v4().to_string();
        history_model.update(&mut app, |history_model, ctx| {
            history_model.initialize_output_for_response_stream(
                &stream_id,
                conversation_id,
                terminal_view_id,
                warp_multi_agent_api::response_event::StreamInit {
                    request_id: "request-1".to_string(),
                    conversation_id: server_token.clone(),
                    run_id: run_id.clone(),
                },
                ctx,
            );
        });

        let persisted_conversation = persisted_agent_conversation_from_update_event(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("stream init should persist conversation state"),
        );
        let restored =
            convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
                .expect("persisted StreamInit conversation should be restorable");

        assert_eq!(
            restored
                .server_conversation_token()
                .map(|token| token.as_str()),
            Some(server_token.as_str())
        );
        assert_eq!(restored.run_id().as_deref(), Some(run_id.as_str()));
    });
}

#[test]
fn test_assign_run_id_for_conversation_persists_updated_conversation_state() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(
                conversation_id,
                "assigned-run-token".to_string(),
            );
            conversation_id
        });

        let task_id: AmbientAgentTaskId = Uuid::new_v4().to_string().parse().unwrap();
        history_model.update(&mut app, |history_model, ctx| {
            history_model.assign_run_id_for_conversation(
                conversation_id,
                task_id.to_string(),
                Some(task_id),
                terminal_view_id,
                ctx,
            );
        });

        let persisted_conversation = persisted_agent_conversation_from_update_event(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("run id assignment should persist conversation state"),
        );
        let restored =
            convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
                .expect("persisted run id assignment should be restorable");

        assert_eq!(
            restored
                .server_conversation_token()
                .map(|token| token.as_str()),
            Some("assigned-run-token")
        );
        assert_eq!(restored.task_id(), Some(task_id));
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
        initialize_history_persistence_for_tests(&mut app);
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
        initialize_history_persistence_for_tests(&mut app);
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
            root_task_is_optimistic: None,
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
                root_task_is_optimistic: None,
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

#[test]
fn test_fork_then_bind_handoff_token_persists_to_restored_conversation() {
    use crate::ai::agent::conversation::AIConversation;
    use crate::persistence::model::AgentConversationData;
    use crate::test_util::ai_agent_tasks::{create_api_task, create_message};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

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
                root_task_is_optimistic: None,
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

        let forked_id = history_model.update(&mut app, |model, ctx| {
            let source = model
                .conversation(&source_id)
                .expect("source conversation must be in memory after restore")
                .clone();
            model
                .fork_conversation(&source, "[Fork] ", false, None, ctx)
                .expect("fork must succeed when sqlite sender is wired up")
                .id()
        });

        history_model.update(&mut app, |model, ctx| {
            model.set_server_conversation_token_for_conversation_and_persist(
                forked_id,
                "cloud-T".to_string(),
                ctx,
            );
        });

        let mut persisted_fork = None;
        for _ in 0..2 {
            let event = receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("fork creation and token bind should both persist");
            let persisted = persisted_agent_conversation_from_update_event(event);
            if persisted.conversation.conversation_id == forked_id.to_string()
                && persisted
                    .conversation
                    .conversation_data
                    .contains("\"server_conversation_token\":\"cloud-T\"")
            {
                persisted_fork = Some(persisted);
                break;
            }
        }

        let restored = convert_persisted_conversation_to_ai_conversation_with_metadata(
            persisted_fork.expect("token-bound fork should be persisted"),
        )
        .expect("persisted token-bound fork should be restorable");

        assert_eq!(
            restored
                .server_conversation_token()
                .map(|token| token.as_str()),
            Some("cloud-T")
        );
        assert_eq!(
            restored
                .forked_from_server_conversation_token()
                .map(|token| token.as_str()),
            Some("src-token"),
        );
    });
}

#[test]
fn test_fork_then_bind_handoff_token_updates_cached_metadata_and_emits_refresh_events() {
    use crate::ai::agent::conversation::AIConversation;
    use crate::persistence::model::AgentConversationData;
    use crate::test_util::ai_agent_tasks::{create_api_task, create_message};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, _receiver) = std::sync::mpsc::sync_channel(4);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();
        let captured_events = Arc::new(Mutex::new(Vec::new()));

        app.update(|ctx| {
            let captured_events = captured_events.clone();
            ctx.subscribe_to_model(&history_model, move |_, event, _| {
                captured_events.lock().unwrap().push(event.clone());
            });
        });

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
                root_task_is_optimistic: None,
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

        let forked_conversation = history_model.update(&mut app, |model, ctx| {
            let source = model
                .conversation(&source_id)
                .expect("source conversation must be in memory after restore")
                .clone();
            model
                .fork_conversation(&source, "[Fork] ", false, None, ctx)
                .expect("fork must succeed when sqlite sender is wired up")
        });
        let forked_id = forked_conversation.id();
        let fork_terminal_view_id = EntityId::new();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(
                fork_terminal_view_id,
                vec![forked_conversation.clone()],
                ctx,
            );
        });

        history_model.update(&mut app, |model, ctx| {
            model.set_server_conversation_token_for_conversation_and_persist(
                forked_id,
                "cloud-T".to_string(),
                ctx,
            );
        });

        history_model.read(&app, |model, _| {
            let metadata = model
                .get_conversation_metadata(&forked_id)
                .expect("forked conversation should keep a cached metadata entry");
            assert_eq!(
                metadata
                    .server_conversation_token
                    .as_ref()
                    .map(ServerConversationToken::as_str),
                Some("cloud-T"),
            );
            assert!(
                metadata.has_cloud_data,
                "a token-bound fork should be treated as cloud-backed in cached metadata",
            );
        });

        let events = captured_events.lock().unwrap().clone();
        assert!(
            events.iter().any(|event| matches!(
                event,
                BlocklistAIHistoryEvent::UpdatedConversationMetadata {
                    terminal_view_id: Some(id),
                    conversation_id,
                } if *id == fork_terminal_view_id && *conversation_id == forked_id
            )),
            "token binding should emit UpdatedConversationMetadata so metadata-driven UI refreshes",
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                    terminal_view_id: id,
                    conversation_id,
                } if *id == fork_terminal_view_id && *conversation_id == forked_id
            )),
            "token binding should emit ConversationServerTokenAssigned so conversation-management UI refreshes",
        );
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
                root_task_is_optimistic: None,
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
                root_task_is_optimistic: None,
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

/// LoadTranscript -> merge integration coverage for the orchestration
/// remote-child restore path.
///
/// Simulates the smaller seam that
/// `pane_group::hydrate_remote_child_transcript_in_place` reaches after a
/// successful `load_conversation_by_server_token` fetch: it hands the
/// fetched cloud transcript to
/// `hydrate_remote_child_placeholder_with_cloud_transcript` on the local
/// placeholder. Asserts the merged record:
///   1. retains the placeholder's local `AIConversationId` (so it remains the
///      canonical `child_agent_panes` key on the pane-group side),
///   2. carries the placeholder's orchestration linkage forward
///      (parent_conversation_id, agent_name, run_id, is_remote_child),
///   3. surfaces the cloud transcript content (non-empty title + at least
///      one exchange).
///
/// Also asserts the precondition guard: calling the merge against an
/// unknown placeholder returns `Err` so the caller's tombstone fallback
/// runs instead of silently constructing a detached conversation.
#[test]
fn hydrate_remote_child_placeholder_with_cloud_transcript_preserves_placeholder_identity() {
    use crate::ai::agent::conversation::AIConversation;
    use crate::ai::ambient_agents::AmbientAgentTaskId;
    use crate::persistence::model::AgentConversationData;
    use crate::test_util::ai_agent_tasks::create_api_task;

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        // Build a placeholder "remote child" conversation with the
        // orchestration linkage we want preserved across merge.
        let parent_id = AIConversationId::new();
        let placeholder_id = AIConversationId::new();
        let placeholder_task_id_str = Uuid::new_v4().to_string();
        let placeholder_task_id: AmbientAgentTaskId =
            placeholder_task_id_str.parse().expect("task id must parse");

        // The placeholder has no transcript yet — just a synthetic root
        // task so `new_restored` succeeds. Real placeholders go through the
        // optimistic-root construction path; for this test we just need a
        // record with the right local-only fields.
        let placeholder_root = create_api_task("placeholder-root", vec![]);
        let placeholder = AIConversation::new_restored(
            placeholder_id,
            vec![placeholder_root],
            Some(AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: Some("parent-agent-id".to_string()),
                agent_name: Some("worker".to_string()),
                orchestration_harness_type: None,
                parent_conversation_id: Some(parent_id.to_string()),
                is_remote_child: true,
                root_task_is_optimistic: Some(true),
                run_id: Some(placeholder_task_id_str.clone()),
                autoexecute_override: None,
                last_event_sequence: None,
                pinned: false,
            }),
        )
        .expect("placeholder conversation should build");
        // Sanity-check the placeholder before restore so a later regression
        // in `new_restored` doesn't pass this test silently.
        assert!(placeholder.is_remote_child());
        assert_eq!(placeholder.task_id(), Some(placeholder_task_id));

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![placeholder], ctx);
        });

        // Build a cloud-side AIConversation with a non-empty root task
        // description (so `title()` returns it) and a real user-query
        // message (so the merged conversation has ≥1 exchange).
        let cloud_id = AIConversationId::new();
        let mut cloud_root = create_api_task(
            "cloud-root-task",
            vec![create_user_query_message(
                "cloud-user-msg",
                "cloud-root-task",
                "cloud-request",
                "What's the status?",
            )],
        );
        cloud_root.description = "Cloud-side title".to_string();
        let cloud_tasks = vec![cloud_root];
        let cloud_conversation = AIConversation::new_restored(
            cloud_id,
            cloud_tasks.clone(),
            Some(AgentConversationData {
                server_conversation_token: Some("cloud-token".to_string()),
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
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
            }),
        )
        .expect("cloud conversation should build");

        let merged = history_model.update(&mut app, |model, _| {
            model
                .hydrate_remote_child_placeholder_with_cloud_transcript(
                    placeholder_id,
                    cloud_tasks,
                    cloud_conversation,
                )
                .expect("hydration must succeed when placeholder is loaded")
        });

        assert_eq!(
            merged.id(),
            placeholder_id,
            "merge must reuse the placeholder's local AIConversationId so child_agent_panes stays canonical",
        );
        assert_eq!(
            merged.title().as_deref(),
            Some("Cloud-side title"),
            "merged conversation must surface the cloud-side root task title",
        );
        assert!(
            merged.exchange_count() >= 1,
            "merged conversation must have at least one exchange from the cloud transcript; got {}",
            merged.exchange_count(),
        );
        assert!(
            merged.is_remote_child(),
            "merged conversation must retain the placeholder's is_remote_child flag",
        );
        assert_eq!(
            merged.parent_conversation_id(),
            Some(parent_id),
            "merged conversation must retain the placeholder's parent_conversation_id",
        );
        assert_eq!(
            merged.agent_name(),
            Some("worker"),
            "merged conversation must retain the placeholder's agent_name",
        );
        assert_eq!(
            merged.task_id(),
            Some(placeholder_task_id),
            "merged conversation must retain the placeholder's task_id (orchestration run id)",
        );

        // And the history model's view of placeholder_id now reflects the
        // merge — callers that look up the placeholder will see the cloud
        // transcript content.
        history_model.read(&app, |model, _| {
            let live = model
                .conversation(&placeholder_id)
                .expect("placeholder must still be in conversations_by_id after merge");
            assert_eq!(live.id(), placeholder_id);
            assert_eq!(live.title().as_deref(), Some("Cloud-side title"));
            assert!(live.exchange_count() >= 1);
            assert!(live.is_remote_child());
        });

        // Precondition guard: merging against an unknown placeholder must
        // return Err so the caller falls back instead of silently building a
        // detached conversation.
        let unknown_placeholder = AIConversationId::new();
        let mut cloud_root_again = create_api_task(
            "cloud-root-task-2",
            vec![create_user_query_message(
                "cloud-user-msg-2",
                "cloud-root-task-2",
                "cloud-request-2",
                "another",
            )],
        );
        cloud_root_again.description = "Cloud title 2".to_string();
        let cloud_again = AIConversation::new_restored(
            AIConversationId::new(),
            vec![cloud_root_again.clone()],
            None,
        )
        .expect("second cloud conversation should build");
        let err = history_model.update(&mut app, |model, _| {
            model
                .hydrate_remote_child_placeholder_with_cloud_transcript(
                    unknown_placeholder,
                    vec![cloud_root_again],
                    cloud_again,
                )
                .expect_err("hydration must error when placeholder is not loaded")
        });
        assert!(
            format!("{err:#}").contains("not found in conversations_by_id"),
            "error must surface the missing-placeholder reason; got: {err:#}",
        );
    });
}
