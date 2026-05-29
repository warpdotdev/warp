use std::collections::HashMap;

use super::{ConversationDetailsData, PanelMode, UNKNOWN_CREATOR_DISPLAY_NAME};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversation, AIConversationId, ServerAIConversationMetadata,
};
use crate::ai::agent_conversations_model::entry::{
    AgentConversationBackingData, AgentConversationCapabilities, AgentConversationDisplayData,
    AgentConversationEntry, AgentConversationEntryId, AgentConversationIdentity,
    AgentConversationPrincipal, AgentConversationProvenance, PrincipalType,
};
use crate::ai::agent_conversations_model::AgentRunDisplayStatus;
use crate::ai::ambient_agents::task::{AgentConfigSnapshot, HarnessConfig, TaskPrincipalInfo};
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::auth::UserUid;
use crate::cloud_object::{Revision, ServerMetadata, ServerPermissions};
use crate::server::ids::ServerId;
use crate::workspaces::user_profiles::{UserProfileWithUID, UserProfiles};
use chrono::{Local, Utc};
use persistence::model::{AgentConversationData, ConversationUsageMetadata};
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;
use warpui::{App, EntityId, SingletonEntity};

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
        run_time: Some("PT1S".parse().unwrap()),
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

fn create_server_metadata_with_creator(
    server_token: &str,
    creator_uid: &str,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "test conversation".to_string(),
        working_directory: None,
        harness: AIAgentHarness::Oz,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            credits_spent_for_last_block: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
        },
        metadata: ServerMetadata {
            uid: ServerId::default(),
            revision: Revision::now(),
            metadata_last_updated_ts: Utc::now().into(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: Some(creator_uid.to_string()),
            last_editor_uid: None,
            current_editor_uid: None,
        },
        permissions: ServerPermissions::mock_personal(),
        ambient_agent_task_id: None,
        server_conversation_token: ServerConversationToken::new(server_token.to_string()),
        artifacts: vec![],
    }
}

fn user_profile(
    uid: &str,
    display_name: Option<&str>,
    email: &str,
    photo_url: &str,
) -> UserProfileWithUID {
    UserProfileWithUID {
        firebase_uid: UserUid::new(uid),
        display_name: display_name.map(str::to_string),
        email: email.to_string(),
        photo_url: photo_url.to_string(),
    }
}

fn create_entry_with_creator_uid(creator_uid: &str) -> AgentConversationEntry {
    let conversation_id = AIConversationId::new();
    AgentConversationEntry {
        id: AgentConversationEntryId::Conversation(conversation_id),
        identity: AgentConversationIdentity {
            local_conversation_id: Some(conversation_id),
            ambient_agent_task_id: None,
            server_conversation_token: None,
            session_id: None,
        },
        provenance: AgentConversationProvenance::CloudSyncedConversation,
        display: AgentConversationDisplayData {
            title: "Entry conversation".to_string(),
            initial_query: Some("test".to_string()),
            created_at: Utc::now(),
            last_updated: Utc::now(),
            status: AgentRunDisplayStatus::ConversationSucceeded,
            creator: AgentConversationPrincipal {
                name: None,
                uid: Some(creator_uid.to_string()),
                principal_type: Some(PrincipalType::User),
            },
            executor: None,
            request_usage: None,
            run_time: None,
            session_status: None,
            source: None,
            working_directory: None,
            environment_id: None,
            harness: None,
            artifacts: vec![],
        },
        backing: AgentConversationBackingData {
            has_loaded_conversation: false,
            has_local_persisted_data: false,
            has_cloud_data: true,
            has_ambient_run: false,
        },
        capabilities: AgentConversationCapabilities {
            can_open: false,
            can_copy_link: false,
            can_share: false,
            can_delete: false,
            can_fork_locally: false,
            can_cancel: false,
        },
    }
}

#[test]
fn test_from_conversation_uses_unknown_creator_when_profile_is_missing() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| UserProfiles::new(vec![]));

        let conversation_id = AIConversationId::new();
        let creator_uid = "B123456789";
        let mut conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            "/tmp/unknown-creator-conversation",
            AgentConversationData {
                server_conversation_token: Some("server-token-unknown-creator".to_string()),
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
            },
        );
        conversation.set_server_metadata(create_server_metadata_with_creator(
            "server-token-unknown-creator",
            creator_uid,
        ));

        app.update(|ctx| {
            let data = ConversationDetailsData::from_conversation(&conversation, ctx);
            let creator = data.creator.as_ref().expect("creator should be populated");

            assert_eq!(creator.display_name, UNKNOWN_CREATOR_DISPLAY_NAME);
            assert_eq!(creator.uid.as_deref(), Some(creator_uid));
        });
    });
}

#[test]
fn test_from_conversation_uses_profile_info_when_available() {
    App::test((), |mut app| async move {
        let creator_uid = "profile-user-conversation";
        app.add_singleton_model(|_| {
            UserProfiles::new(vec![user_profile(
                creator_uid,
                Some("ZL"),
                "zl@example.com",
                "https://example.com/zl.png",
            )])
        });

        let conversation_id = AIConversationId::new();
        let mut conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            "/tmp/profile-creator-conversation",
            AgentConversationData {
                server_conversation_token: Some("server-token-profile-creator".to_string()),
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
            },
        );
        conversation.set_server_metadata(create_server_metadata_with_creator(
            "server-token-profile-creator",
            creator_uid,
        ));

        app.update(|ctx| {
            let data = ConversationDetailsData::from_conversation(&conversation, ctx);
            let creator = data.creator.as_ref().expect("creator should be populated");

            assert_eq!(creator.display_name, "ZL");
            assert_eq!(
                creator.photo_url.as_deref(),
                Some("https://example.com/zl.png")
            );
            assert_eq!(creator.uid.as_deref(), Some(creator_uid));
        });
    });
}

#[test]
fn test_from_task_uses_unknown_creator_when_only_uid_is_available() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        app.add_singleton_model(|_| UserProfiles::new(vec![]));

        let mut task = create_test_task("550e8400-e29b-41d4-a716-000000004040");
        let creator_uid = "B987654321";
        task.creator = Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: None,
        });

        app.update(|ctx| {
            let data = ConversationDetailsData::from_task(&task, None, None, ctx);
            let creator = data.creator.as_ref().expect("creator should be populated");

            assert_eq!(creator.display_name, UNKNOWN_CREATOR_DISPLAY_NAME);
            assert_eq!(creator.uid.as_deref(), Some(creator_uid));
        });
    });
}

#[test]
fn test_from_task_uses_profile_info_when_display_name_is_missing() {
    App::test((), |mut app| async move {
        let creator_uid = "profile-user-task";
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        app.add_singleton_model(|_| {
            UserProfiles::new(vec![user_profile(
                creator_uid,
                Some("Profile User"),
                "profile@example.com",
                "https://example.com/profile.png",
            )])
        });

        let mut task = create_test_task("550e8400-e29b-41d4-a716-000000004041");
        task.creator = Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: None,
        });

        app.update(|ctx| {
            let data = ConversationDetailsData::from_task(&task, None, None, ctx);
            let creator = data.creator.as_ref().expect("creator should be populated");

            assert_eq!(creator.display_name, "Profile User");
            assert_eq!(
                creator.photo_url.as_deref(),
                Some("https://example.com/profile.png")
            );
            assert_eq!(creator.uid.as_deref(), Some(creator_uid));
        });
    });
}

#[test]
fn test_from_entry_uses_profile_info_when_name_is_missing() {
    App::test((), |mut app| async move {
        let creator_uid = "profile-user-entry";
        app.add_singleton_model(|_| {
            UserProfiles::new(vec![user_profile(
                creator_uid,
                Some("Entry Profile"),
                "entry@example.com",
                "https://example.com/entry.png",
            )])
        });

        let entry = create_entry_with_creator_uid(creator_uid);

        app.update(|ctx| {
            let data = ConversationDetailsData::from_agent_conversation_entry(
                &entry, None, None, None, ctx,
            );
            let creator = data.creator.as_ref().expect("creator should be populated");

            assert_eq!(creator.display_name, "Entry Profile");
            assert_eq!(
                creator.photo_url.as_deref(),
                Some("https://example.com/entry.png")
            );
            assert_eq!(creator.uid.as_deref(), Some(creator_uid));
        });
    });
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
                root_task_is_optimistic: None,
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
                root_task_is_optimistic: None,
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
                root_task_is_optimistic: None,
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
