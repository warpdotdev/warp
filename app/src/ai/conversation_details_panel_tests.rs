use std::collections::HashMap;

use chrono::{Local, Utc};
use persistence::model::AgentConversationData;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;
use warpui::{App, EntityId, SingletonEntity};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::ambient_agents::task::{AgentConfigSnapshot, HarnessConfig, TaskPrincipalInfo};
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;

use super::{ConversationDetailsData, PanelMode};

fn create_test_task(task_id: &str) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id: task_id.parse().unwrap(),
        parent_run_id: None,
        title: "Task".to_string(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: now,
        started_at: None,
        updated_at: now,
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: "user-1".to_string(),
            display_name: Some("User 1".to_string()),
        }),
        executor: None,
        conversation_id: None,
        request_usage: None,
        agent_config_snapshot: None,
        artifacts: vec![],
        is_sandbox_running: false,
        last_event_sequence: None,
        children: vec![],
    }
}

fn create_message_with_directory(id: &str, task_id: &str, directory: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: "test query".to_string(),
            context: Some(api::InputContext {
                directory: Some(api::input_context::Directory {
                    pwd: directory.to_string(),
                    home: String::new(),
                    pwd_file_symbols_indexed: false,
                }),
                ..Default::default()
            }),
            referenced_attachments: HashMap::new(),
            mode: None,
            intended_agent: Default::default(),
        })),
        request_id: "request-1".to_string(),
        timestamp: None,
    }
}

fn create_agent_output_message(id: &str, task_id: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: "done".to_string(),
            },
        )),
        request_id: "request-1".to_string(),
        timestamp: None,
    }
}

fn create_restored_conversation(
    conversation_id: AIConversationId,
    root_task_id: &str,
    directory: &str,
    conversation_data: AgentConversationData,
) -> AIConversation {
    let task = api::Task {
        id: root_task_id.to_string(),
        messages: vec![
            create_message_with_directory("message-1", root_task_id, directory),
            create_agent_output_message("message-2", root_task_id),
        ],
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    };

    AIConversation::new_restored(conversation_id, vec![task], Some(conversation_data))
        .expect("restored conversation should build")
}

#[test]
fn test_from_task_includes_linked_directory_when_run_id_matches() {
    App::test((), |mut app| async move {
        let _orchestration_v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation_id = AIConversationId::new();
        let task_id = "550e8400-e29b-41d4-a716-000000004000";
        let directory = "/tmp/run-id-directory";

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            directory,
            AgentConversationData {
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
                run_id: Some(task_id.to_string()),
                autoexecute_override: None,
                last_event_sequence: None,
                pinned: false,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        let task = create_test_task(task_id);
        app.update(|ctx| {
            let data = ConversationDetailsData::from_task(&task, None, None, ctx);
            assert!(matches!(
                data.mode,
                PanelMode::Task {
                    directory: Some(ref task_directory),
                    ..
                } if task_directory == directory
            ));
        });
    });
}

#[test]
fn test_from_conversation_metadata_passes_harness_through() {
    for harness in [
        None,
        Some(Harness::Oz),
        Some(Harness::Claude),
        Some(Harness::Gemini),
        Some(Harness::Unknown),
    ] {
        let data = ConversationDetailsData::from_conversation_metadata(
            AIConversationId::new(),
            "Title".to_string(),
            None,
            Utc::now().with_timezone(&Local),
            None,
            None,
            None,
            vec![],
            None,
            None,
            None,
            None,
            harness,
        );
        assert_eq!(
            data.harness, harness,
            "harness {harness:?} should pass through"
        );
    }
}

#[test]
fn test_from_task_resolves_harness() {
    App::test((), |mut app| async move {
        let _history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // Base task has `agent_config_snapshot: None`; cloning lets us mutate per case.
        let base_task = create_test_task("550e8400-e29b-41d4-a716-000000004020");

        app.update(|ctx| {
            // No snapshot → harness unknown.
            let data = ConversationDetailsData::from_task(&base_task, None, None, ctx);
            assert_eq!(data.harness, None);

            // Snapshot without an explicit harness → default to Warp Agent.
            let mut task = base_task.clone();
            task.agent_config_snapshot = Some(AgentConfigSnapshot::default());
            let data = ConversationDetailsData::from_task(&task, None, None, ctx);
            assert_eq!(data.harness, Some(Harness::Oz));

            // Snapshot with explicit harness_type.
            for harness in [
                Harness::Oz,
                Harness::Claude,
                Harness::Gemini,
                Harness::Unknown,
            ] {
                let mut task = base_task.clone();
                task.agent_config_snapshot = Some(AgentConfigSnapshot {
                    harness: Some(HarnessConfig::from_harness_type(harness)),
                    ..Default::default()
                });
                let data = ConversationDetailsData::from_task(&task, None, None, ctx);
                assert_eq!(data.harness, Some(harness), "harness {harness:?}");
            }
        });
    });
}

#[test]
fn test_from_task_populates_executor() {
    App::test((), |mut app| async move {
        let _history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let mut task = create_test_task("550e8400-e29b-41d4-a716-000000004030");
        task.executor = Some(TaskPrincipalInfo {
            creator_type: "service_account".to_string(),
            uid: "agent-uid".to_string(),
            display_name: Some("Deploy Agent".to_string()),
        });

        app.update(|ctx| {
            let data = ConversationDetailsData::from_task(&task, None, None, ctx);
            assert_eq!(
                data.executor
                    .as_ref()
                    .map(|executor| executor.display_name.as_str()),
                Some("Deploy Agent")
            );
        });
    });
}

#[test]
fn test_from_conversation_populates_local_conversation_fields() {
    // Locks in that `ConversationDetailsData::from_conversation` works on native
    // and surfaces the conversation-derived fields the conversation details panel
    // renders for local Warp Agent runs (APP-3595).
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation_id = AIConversationId::new();
        let directory = "/tmp/local-conversation-directory";
        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            directory,
            AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                orchestration_harness_type: None,
                parent_conversation_id: None,
                run_id: None,
                autoexecute_override: None,
                last_event_sequence: None,
                is_remote_child: false,
                pinned: false,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        app.update(|ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation should be present");
            let data = ConversationDetailsData::from_conversation(conversation, ctx);

            // Mode should be Conversation with the working directory and no server-side
            // conversation id (since this conversation was restored without a server token).
            match &data.mode {
                PanelMode::Conversation {
                    directory: panel_directory,
                    server_conversation_id,
                    ai_conversation_id,
                    status,
                } => {
                    assert_eq!(panel_directory.as_deref(), Some(directory));
                    assert!(server_conversation_id.is_none());
                    // `from_conversation` does not have access to the in-memory
                    // AIConversationId; that field is populated only by the
                    // management view path (`from_conversation_metadata`).
                    assert!(ai_conversation_id.is_none());
                    assert!(status.is_some());
                }
                PanelMode::Task { .. } => {
                    panic!("expected Conversation mode for a local conversation")
                }
            }

            assert_eq!(data.title, "test query");
            assert_eq!(data.source_prompt.as_deref(), Some("test query"));
            assert!(data.credits.is_some());
        });
    });
}

#[test]
fn test_from_task_includes_linked_directory_when_server_token_matches() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation_id = AIConversationId::new();
        let server_token = "server-token-123";
        let directory = "/tmp/server-token-directory";

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            directory,
            AgentConversationData {
                server_conversation_token: Some(server_token.to_string()),
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
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        let mut task = create_test_task("550e8400-e29b-41d4-a716-000000004001");
        task.conversation_id = Some(server_token.to_string());

        app.update(|ctx| {
            let data = ConversationDetailsData::from_task(&task, None, None, ctx);
            assert!(matches!(
                data.mode,
                PanelMode::Task {
                    directory: Some(ref task_directory),
                    ..
                } if task_directory == directory
            ));
        });
    });
}
