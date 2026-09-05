use chrono::Utc;
use persistence::model::ConversationUsageMetadata;
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIAgentHarness, ServerAIConversationMetadata};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::task::{AmbientAgentTask, AmbientAgentTaskState, TaskPrincipalInfo};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::blocklist::{InputConfig, InputType};
use crate::auth::user::TEST_USER_UID;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerObjectGuest, ServerPermissions};
use crate::server::ids::ServerId;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

const CONVERSATION_TOKEN: &str = "server-conversation-token";

fn ambient_task_id(index: usize) -> AmbientAgentTaskId {
    format!("550e8400-e29b-41d4-a716-{index:012}")
        .parse()
        .unwrap()
}

fn claude_session() -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext::default(),
        input_state: CLIAgentInputState::Open {
            entrypoint: CLIAgentInputEntrypoint::FooterButton,
            previous_input_config: InputConfig {
                input_type: InputType::AI,
                is_locked: true,
            },
            previous_was_lock_set_with_empty_buffer: true,
        },
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
        should_auto_toggle_input: false,
    }
}

fn start_cli_session(
    view: &mut crate::terminal::TerminalView,
    ctx: &mut ViewContext<crate::terminal::TerminalView>,
) {
    let view_id = view.id();
    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        sessions.set_session(view_id, claude_session(), ctx);
    });
}

fn rendered_cli_footer_child_ids(
    view: &crate::terminal::TerminalView,
    ctx: &AppContext,
) -> Vec<EntityId> {
    let footer = view.input().as_ref(ctx).agent_input_footer().as_ref(ctx);
    assert!(
        footer.is_cli_agent_session_active(ctx),
        "CLI session must be active so render uses the CLI footer",
    );
    footer.render(ctx).debug_child_view_ids()
}

fn ambient_agent_task(
    task_id: AmbientAgentTaskId,
    state: AmbientAgentTaskState,
) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id,
        parent_run_id: None,
        title: "Task".to_string(),
        state,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: TEST_USER_UID.to_string(),
            display_name: None,
        }),
        executor: None,
        conversation_id: Some(CONVERSATION_TOKEN.to_string()),
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
        debug_agent_available: false,
        scope: None,
    }
}

fn claude_conversation_metadata(task_id: AmbientAgentTaskId) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "Conversation".to_string(),
        working_directory: None,
        harness: AIAgentHarness::ClaudeCode,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            total_provider_cost_in_cents: None,
            credits_spent_for_last_block: None,
            charged_usage_for_last_block: None,
            total_charged_usage: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
            context_window_segments: Vec::new(),
        },
        metadata: ServerMetadata {
            uid: ServerId::default(),
            revision: Revision::now(),
            metadata_last_updated_ts: Utc::now().into(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: Some(TEST_USER_UID.to_string()),
            last_editor_uid: None,
            current_editor_uid: None,
        },
        creator: None,
        permissions: ServerPermissions {
            space: Owner::mock_current_user(),
            guests: Vec::<ServerObjectGuest>::new(),
            anyone_link_sharing: None,
            permissions_last_updated_ts: Utc::now().into(),
        },
        ambient_agent_task_id: Some(task_id),
        server_conversation_token: ServerConversationToken::new(CONVERSATION_TOKEN.to_string()),
        artifacts: vec![],
    }
}

#[test]
fn cli_footer_shows_live_indicator_for_third_party_cloud_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let task_id = ambient_task_id(1);

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::executor());
            start_cli_session(view, ctx);

            let footer = view.input().as_ref(ctx).agent_input_footer().as_ref(ctx);
            let live_id = footer.live_session_indicator_id();
            let new_vm_id = footer.new_cloud_vm_indicator_id();
            let child_ids = rendered_cli_footer_child_ids(view, ctx);
            assert!(
                child_ids.contains(&live_id),
                "CLI footer should embed the live-session indicator, got {child_ids:?}"
            );
            assert!(
                !child_ids.contains(&new_vm_id),
                "CLI footer should not embed the new-VM indicator, got {child_ids:?}"
            );
        });
    });
}

#[test]
fn cli_footer_shows_new_vm_indicator_for_disconnected_third_party_cloud_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let task_id = ambient_task_id(1);

        terminal.update(&mut app, |view, ctx| {
            AgentConversationsModel::handle(ctx).update(ctx, |model, _| {
                model.insert_task_for_test(ambient_agent_task(
                    task_id,
                    AmbientAgentTaskState::Succeeded,
                ));
            });
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                let conversation_id =
                    model.start_new_conversation(view.id(), false, false, false, ctx);
                model.set_server_conversation_token_for_conversation(
                    conversation_id,
                    CONVERSATION_TOKEN.to_string(),
                );
                model.set_server_metadata_for_conversation(
                    conversation_id,
                    claude_conversation_metadata(task_id),
                    ctx,
                );
            });
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::NotShared);
            start_cli_session(view, ctx);

            let footer = view.input().as_ref(ctx).agent_input_footer().as_ref(ctx);
            let live_id = footer.live_session_indicator_id();
            let new_vm_id = footer.new_cloud_vm_indicator_id();
            let child_ids = rendered_cli_footer_child_ids(view, ctx);
            assert!(
                child_ids.contains(&new_vm_id),
                "CLI footer should embed the new-VM indicator, got {child_ids:?}"
            );
            assert!(
                !child_ids.contains(&live_id),
                "CLI footer should not embed the live-session indicator, got {child_ids:?}"
            );
        });
    });
}

#[test]
fn cli_footer_omits_cloud_indicator_for_local_cli_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            start_cli_session(view, ctx);
            let footer = view.input().as_ref(ctx).agent_input_footer().as_ref(ctx);
            let live_id = footer.live_session_indicator_id();
            let new_vm_id = footer.new_cloud_vm_indicator_id();
            let child_ids = rendered_cli_footer_child_ids(view, ctx);
            assert!(
                !child_ids.contains(&live_id) && !child_ids.contains(&new_vm_id),
                "local CLI footer should omit both cloud indicators, got {child_ids:?}"
            );
        });
    });
}

#[test]
fn cli_footer_omits_cloud_indicator_for_shared_local_viewer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::executor());
            start_cli_session(view, ctx);
            let footer = view.input().as_ref(ctx).agent_input_footer().as_ref(ctx);
            let live_id = footer.live_session_indicator_id();
            let new_vm_id = footer.new_cloud_vm_indicator_id();
            let child_ids = rendered_cli_footer_child_ids(view, ctx);
            assert!(
                !child_ids.contains(&live_id) && !child_ids.contains(&new_vm_id),
                "shared local viewer CLI footer should omit both cloud indicators, got {child_ids:?}"
            );
        });
    });
}
