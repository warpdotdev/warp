use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{StartAgentExecutionMode, StartAgentResult};
use crate::BlocklistAIHistoryModel;
use ai::agent::action_result::StartAgentVersion;
use warp_cli::agent::Harness;
use warp_core::ui::appearance::Appearance;
use warpui::elements::MouseStateHandle;
use warpui::{App, EntityId};

use super::{
    agent_display_name_from_id, child_conversation_card_data_for_result, participant_for_agent_id,
    render_conversation_navigation_card_row, start_agent_cancelled_prefix,
    start_agent_error_prefix, start_agent_in_progress_prefix, start_agent_success_suffix,
    transcript_metadata, ChildConversationCardData, OrchestrationAvatar, OrchestrationParticipant,
};

#[test]
fn child_conversation_card_data_for_success_result_returns_conversation_id_and_title() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(EntityId::new(), false, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(
                conversation_id,
                "child-agent-id".to_string(),
            );
            history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist")
                .set_fallback_display_title("Generated child title".to_string());
            conversation_id
        });
        let result = StartAgentResult::Success {
            agent_id: "child-agent-id".to_string(),
            version: StartAgentVersion::V1,
        };
        let actual = app.read(|ctx| child_conversation_card_data_for_result(&result, ctx));
        assert_eq!(
            actual,
            Some(ChildConversationCardData {
                conversation_id,
                agent_name: "Agent".to_string(),
                title: "Generated child title".to_string(),
                status: ConversationStatus::InProgress,
            })
        );
    });
}

#[test]
fn start_agent_copy_uses_local_labels_for_local_children() {
    let execution_mode = StartAgentExecutionMode::local_harness("claude-code".to_string());

    assert_eq!(start_agent_success_suffix(&execution_mode), " locally.");
    assert_eq!(
        start_agent_error_prefix(&execution_mode),
        "Failed to start agent "
    );
    assert_eq!(
        start_agent_cancelled_prefix(&execution_mode),
        "Start agent "
    );
    assert_eq!(
        start_agent_in_progress_prefix(&execution_mode),
        "Starting agent "
    );
}

#[test]
fn start_agent_copy_uses_remote_labels_for_remote_children() {
    let execution_mode = StartAgentExecutionMode::Remote {
        environment_id: "env-123".to_string(),
        skill_references: vec![],
        model_id: String::new(),
        computer_use_enabled: false,
        worker_host: String::new(),
        harness_type: String::new(),
        title: String::new(),
        auth_secret_name: None,
    };

    assert_eq!(start_agent_success_suffix(&execution_mode), " remotely.");
    assert_eq!(
        start_agent_error_prefix(&execution_mode),
        "Failed to start remote agent "
    );
    assert_eq!(
        start_agent_cancelled_prefix(&execution_mode),
        "Start remote agent "
    );
    assert_eq!(
        start_agent_in_progress_prefix(&execution_mode),
        "Starting remote agent "
    );
}

#[test]
fn child_conversation_card_data_for_success_result_without_available_title_uses_placeholder() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(EntityId::new(), false, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(
                conversation_id,
                "child-agent-id".to_string(),
            );
            conversation_id
        });
        let result = StartAgentResult::Success {
            agent_id: "child-agent-id".to_string(),
            version: StartAgentVersion::V1,
        };
        let actual = app.read(|ctx| child_conversation_card_data_for_result(&result, ctx));
        assert_eq!(
            actual,
            Some(ChildConversationCardData {
                conversation_id,
                agent_name: "Agent".to_string(),
                title: "Generating title...".to_string(),
                status: ConversationStatus::InProgress,
            })
        );
    });
}

#[test]
fn child_conversation_card_data_for_non_success_result_returns_none() {
    App::test((), |app| async move {
        let error_result = StartAgentResult::Error {
            error: "boom".to_string(),
            version: StartAgentVersion::V1,
        };
        let error_actual =
            app.read(|ctx| child_conversation_card_data_for_result(&error_result, ctx));
        assert_eq!(error_actual, None);
        let cancelled_actual = app.read(|ctx| {
            child_conversation_card_data_for_result(
                &StartAgentResult::Cancelled {
                    version: StartAgentVersion::V1,
                },
                ctx,
            )
        });
        assert_eq!(cancelled_actual, None);
    });
}

#[test]
fn child_conversation_card_data_returns_none_for_unknown_agent_id() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let result = StartAgentResult::Success {
            agent_id: "missing-agent-id".to_string(),
            version: StartAgentVersion::V1,
        };
        let actual = app.read(|ctx| child_conversation_card_data_for_result(&result, ctx));
        assert_eq!(actual, None);
    });
}

#[test]
fn agent_display_name_from_id_returns_child_agent_name() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        history_model.update(&mut app, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(EntityId::new(), false, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(
                conversation_id,
                "child-agent-id".to_string(),
            );
            history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist")
                .set_agent_name("Agent 1".to_string());
        });

        let actual = app.read(|ctx| {
            agent_display_name_from_id("child-agent-id", Some("orchestrator-agent-id"), ctx)
        });
        assert_eq!(actual, "Agent 1");
    });
}

#[test]
fn agent_display_name_from_id_returns_orchestrator_label() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        history_model.update(&mut app, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(EntityId::new(), false, false, false, ctx);
            let conversation = history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist");
            conversation.set_server_conversation_token("orchestrator-agent-id".to_string());
            conversation.set_agent_name("Agent 0".to_string());
        });

        let actual = app.read(|ctx| {
            agent_display_name_from_id("orchestrator-agent-id", Some("orchestrator-agent-id"), ctx)
        });
        assert_eq!(actual, "Orchestrator");
    });
}

#[test]
fn agent_display_name_from_id_returns_unknown_fallback() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let actual =
            app.read(|ctx| agent_display_name_from_id("missing-agent-id", Some("other-id"), ctx));
        assert_eq!(actual, "Unknown agent");
    });
}
#[test]
fn participant_for_agent_id_uses_pill_style_child_agent_avatar() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        history_model.update(&mut app, |history_model, ctx| {
            let terminal_view_id = EntityId::new();
            let parent_conversation_id =
                history_model.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(
                parent_conversation_id,
                "orchestrator-agent-id".to_string(),
            );
            let child_conversation_id = history_model.start_new_child_conversation(
                terminal_view_id,
                "Agent 1".to_string(),
                parent_conversation_id,
                Some(Harness::Claude),
                ctx,
            );
            history_model.set_server_conversation_token_for_conversation(
                child_conversation_id,
                "child-agent-id".to_string(),
            );
        });

        let actual = app.read(|ctx| {
            participant_for_agent_id("child-agent-id", Some("orchestrator-agent-id"), ctx)
        });
        assert_eq!(actual.display_name, "Agent 1");
        assert_eq!(
            actual.avatar,
            OrchestrationAvatar::agent("Agent 1".to_string())
        );
    });
}

#[test]
fn transcript_metadata_uses_transcript_copy_without_technical_labels() {
    let recipients = vec![OrchestrationParticipant {
        display_name: "Agent 1".to_string(),
        avatar: OrchestrationAvatar::agent("Agent 1".to_string()),
        conversation_id: None,
    }];

    let metadata = transcript_metadata(&recipients, "Fix tests").expect("metadata");

    assert_eq!(metadata, "to Agent 1 • Fix tests");
    for legacy_label in ["Messages received", "From:", "To:", "Subject:"] {
        assert!(
            !metadata.contains(legacy_label),
            "Transcript metadata should not contain old technical label {legacy_label}: {metadata}"
        );
    }
}

#[test]
fn transcript_metadata_omits_orchestrator_recipients() {
    let recipients = vec![OrchestrationParticipant::orchestrator()];

    assert_eq!(
        transcript_metadata(&recipients, "Status update"),
        Some("Status update".to_string())
    );
    assert_eq!(transcript_metadata(&recipients, ""), None);
}

#[test]
fn transcript_metadata_preserves_non_orchestrator_recipients() {
    let recipients = vec![
        OrchestrationParticipant::orchestrator(),
        OrchestrationParticipant {
            display_name: "Agent 1".to_string(),
            avatar: OrchestrationAvatar::agent("Agent 1".to_string()),
            conversation_id: None,
        },
    ];

    assert_eq!(
        transcript_metadata(&recipients, "Fix tests"),
        Some("to Agent 1 • Fix tests".to_string())
    );
}

#[test]
fn conversation_navigation_card_row_renders_title_without_legacy_subtitle() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let element = app.read(|ctx| {
            render_conversation_navigation_card_row(
                "Child conversation",
                None,
                None,
                AIConversationId::new(),
                MouseStateHandle::default(),
                false,
                ctx,
            )
        });
        let text_content = element.debug_text_content().unwrap_or_default();
        assert!(
            text_content.contains("Child conversation"),
            "Expected child conversation title in rendered text: {text_content}",
        );
        assert!(
            !text_content.contains("Open in agent mode"),
            "Legacy subtitle should not appear in rendered card text: {text_content}",
        );
    });
}
