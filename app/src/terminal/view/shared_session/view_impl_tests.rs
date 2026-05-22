use std::collections::HashMap;

use chrono::Utc;
use pathfinder_geometry::vector::vec2f;
use persistence::model::ConversationUsageMetadata;
use session_sharing_protocol::sharer::SessionSourceType;
use warp_multi_agent_api::{self as api, client_action as api_client_action};
use warpui::platform::WindowStyle;
use warpui::{App, EntityId, TypedActionView, ViewHandle};

use super::*;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversation, ConversationStatus, ServerAIConversationMetadata,
};
use crate::ai::agent::AIAgentInput;
use crate::ai::agent_conversations_model::{
    AgentConversationsModel, AgentConversationsModelEvent, AgentRunDisplayStatus,
};
use crate::ai::ambient_agents::task::{TaskPrincipalInfo, TaskStatusErrorCode, TaskStatusMessage};
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::auth::user::TEST_USER_UID;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions};
use crate::context_chips::prompt_type::PromptType;
use crate::editor::InteractionState;
use crate::server::ids::ServerId;
use crate::terminal::model::blocks::{ToTotalIndex as _, INLINE_BANNER_HEIGHT};
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::terminal::view::ambient_agent::{
    HandoffSubmissionState, PendingHandoff, SnapshotUploadStatus,
};
use crate::terminal::view::shared_session::test_utils::terminal_view_for_viewer;
use crate::terminal::view::TerminalAction;
use crate::terminal::TerminalView;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::{assert_lines_approx_eq, FeatureFlag};

#[test]
fn test_prompt_context_menu_items_shared_session_viewer_no_edit_prompt() {
    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
                    role: Default::default(),
                });

                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx)
                });
            })
        });

        let session_settings = SessionSettings::handle(&app);
        session_settings.update(&mut app, |settings, ctx| {
            let _ = settings.honor_ps1.set_value(false, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items: Vec<MenuItem<TerminalAction>> = view.prompt_context_menu_items(ctx);
            assert_eq!(items.len(), 3);

            // We expect the prompt menu items to be something like the following when no context chips exist:
            // Copy prompt
            // ------------
            // Edit prompt (disabled for shared-session viewers)
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(items[2].fields().unwrap().label(), "Edit prompt");
            assert!(items[2].fields().unwrap().is_disabled());
        });
    })
}

#[test]
fn test_on_ambient_agent_execution_ended_enables_followup_input_for_editable_non_owner_finished_view(
) {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::Oz,
            current_user_owner_permissions(),
        );
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
            drop(model);

            view.on_ambient_agent_execution_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert_eq!(
                model.block_list().block_heights().items().len(),
                initial_block_height_items
            );
            assert!(matches!(
                model.shared_session_status(),
                SharedSessionStatus::NotShared
            ));
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert!(view.is_input_box_visible(&model, ctx));
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_shared_session_banners() {
    let _flag = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let mut expected_block_heights_len = terminal.read(&app, |view, _| {
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::None
            ));
            view.model.lock().block_list().block_heights().items().len()
        });

        // Make a block and then insert the shared session starter banner.
        terminal.update(&mut app, |view, ctx| {
            view.model.lock().simulate_block("ls", "foo");
            view.insert_shared_session_started_banner(
                SharedSessionScrollbackType::All,
                false,
                Local::now(),
                ctx,
            );
            expected_block_heights_len += 2;
        });

        terminal.read(&app, |view, _ctx| {
            let model = view.model.lock();

            // Make sure the state has changed.
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::ActiveShare { .. }
            ));

            // We should have inserted a block and a banner.
            let block_height_items = model.block_list().block_heights().items();
            assert_eq!(block_height_items.len(), expected_block_heights_len);

            // The banner should have been inserted before the first visible block.
            let first_block_total_index = model
                .block_list()
                .first_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[first_block_total_index.0 - 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );
        });

        // Insert another block and then the shared session ended banner.
        terminal.update(&mut app, |view, ctx| {
            view.model.lock().simulate_block("ls", "foo");
            view.insert_shared_session_ended_banner(ctx);
            expected_block_heights_len += 2;
        });

        terminal.read(&app, |view, _ctx| {
            let model = view.model.lock();

            // Make sure the state has changed.
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::LastShared { .. }
            ));

            // by now, we've inserted two new blocks and two new banners since the initialization of the view.
            let block_height_items = model.block_list().block_heights().items();
            assert_eq!(block_height_items.len(), expected_block_heights_len);

            // The first banner should continue to be at the start of the blocklist.
            let first_block_total_index = model
                .block_list()
                .first_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[first_block_total_index.0 - 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );

            // The second banner should be at the end of the blocklist, before the active block.
            let last_block_total_index = model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[last_block_total_index.0 + 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );
        });

        // Mimic starting a shared session again in the same view.
        terminal.update(&mut app, |view, ctx| {
            view.insert_shared_session_started_banner(
                SharedSessionScrollbackType::None,
                false,
                Local::now(),
                ctx,
            );

            // We should have removed two banners and inserted one. So overall,
            // we lost one item in the blocklist since the last time.
            expected_block_heights_len -= 1;
        });

        terminal.read(&app, |view, _ctx| {
            let model = view.model.lock();

            // Make sure the state has changed.
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::ActiveShare { .. }
            ));

            // We should have removed two banners and inserted one. So overall,
            // we lost one item in the blocklist since the last time.
            let block_height_items = model.block_list().block_heights().items();
            assert_eq!(block_height_items.len(), expected_block_heights_len);

            // The banner should have been inserted at the end of the blocklist, before the active block.
            let last_block_total_index = model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[last_block_total_index.0 + 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );
        });
    })
}

#[test]
fn test_resize_shared_session_viewer_from_server() {
    let _flag = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        terminal.update(&mut app, |view, ctx| {
            // Refresh the size at the start of the test to make sure
            // we're using a consistent size throughout.
            view.refresh_size(ctx);
        });

        let model = terminal.read(&app, |view, _| view.model.clone());
        model
            .lock()
            .set_shared_session_status(SharedSessionStatus::ActiveViewer {
                role: Default::default(),
            });

        // The viewer's current size info.
        let original_size_info = *model.lock().block_list().size();
        let original_num_rows = original_size_info.rows();
        let original_num_cols = original_size_info.columns();

        // Case 1: suppose the sharer has a larger size.
        // The size info we expect is the old one with the greater
        // number of rows and columns (nothing else changed).
        let new_num_rows = original_num_rows + 1;
        let new_num_cols = original_num_cols + 1;
        let expected_size_info =
            original_size_info.with_rows_and_columns(new_num_rows, new_num_cols);

        terminal.update(&mut app, |view, ctx| {
            view.resize_from_sharer_update(
                WindowSize {
                    num_rows: new_num_rows,
                    num_cols: new_num_cols,
                },
                ctx,
            );
        });

        // Make sure the view and model reflect the new, expected size info.
        terminal.read(&app, |view, _ctx| {
            assert_eq!(*view.size_info(), expected_size_info);
            assert_eq!(*view.model.lock().block_list().size(), expected_size_info);
        });

        // Case 2: suppose the sharer has a smaller size.
        // The size info we expect is our old, larger one; nothing changed.
        let new_num_rows = original_num_rows - 1;
        let new_num_cols = original_num_cols - 1;
        let expected_size_info = original_size_info;

        terminal.update(&mut app, |view, ctx| {
            view.resize_from_sharer_update(
                WindowSize {
                    num_rows: new_num_rows,
                    num_cols: new_num_cols,
                },
                ctx,
            );
        });

        // Make sure the view and model reflect the old, expected size info.
        terminal.read(&app, |view, _ctx| {
            assert_eq!(*view.size_info(), expected_size_info);
            assert_eq!(*view.model.lock().block_list().size(), expected_size_info);
        });
    })
}

#[test]
fn test_resize_shared_session_viewer_independent_of_sharer() {
    let _create_flag = FeatureFlag::CreatingSharedSessions.override_enabled(true);
    let _view_flag = FeatureFlag::ViewingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        terminal.update(&mut app, |view, ctx| {
            // Refresh the size at the start of the test to make sure
            // we're using a consistent size throughout.
            view.after_terminal_view_layout(vec2f(100., 100.), ctx);

            // Set the sharer's size.
            let num_rows = view.size_info().rows();
            let num_cols = view.size_info().columns();
            view.resize_from_sharer_update(WindowSize { num_rows, num_cols }, ctx);
        });

        let original_size_info = terminal.read(&app, |view, _| *view.size_info());
        let original_num_rows = original_size_info.rows();
        let original_num_cols = original_size_info.columns();

        // Case 1: make the viewer winsize smaller by making the pane narrower.
        terminal.update(&mut app, |view, ctx| {
            let narrower = vec2f(
                original_size_info.pane_width_px().as_f32() - 10.,
                original_size_info.pane_height_px().as_f32(),
            );
            view.after_terminal_view_layout(narrower, ctx);
        });

        // Make sure the overall size info was changed but the rows, columns
        // were unchanged because we're respecting the sharer's larger size.
        terminal.read(&app, |view, _ctx| {
            let new_size_info = *view.size_info();
            assert_ne!(original_size_info, new_size_info);

            let expected_size_info =
                new_size_info.with_rows_and_columns(original_num_rows, original_num_cols);
            assert_eq!(*view.size_info(), expected_size_info);
            assert_eq!(*view.model.lock().block_list().size(), expected_size_info);
        });

        // Case 2: make the viewer winsize larger by making the pane wider.
        terminal.update(&mut app, |view, ctx| {
            let wider = vec2f(
                original_size_info.pane_width_px().as_f32() + 10.,
                original_size_info.pane_height_px().as_f32(),
            );
            view.after_terminal_view_layout(wider, ctx);
        });

        // Make sure the overall size info was changed, and that the rows, columns
        // were updated because we're respecting the viewer's larger size.
        terminal.read(&app, |view, _ctx| {
            let new_size_info = *view.size_info();
            assert_ne!(original_size_info, new_size_info);

            assert!(new_size_info.columns() > original_num_cols);
            assert!(view.model.lock().block_list().size().columns() > original_num_cols);
        });
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_on_session_share_ended_restores_size_after_viewer_driven_resize() {
    let _flag = FeatureFlag::CreatingSharedSessions.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            // Refresh the size at the start of the test to make sure
            // we're using a consistent size throughout.
            view.after_terminal_view_layout(vec2f(100., 100.), ctx);
        });

        let original_size = terminal.read(&app, |view, _| *view.size_info());
        let viewer_rows = original_size.rows().saturating_sub(2).max(1);
        let viewer_cols = original_size.columns().saturating_sub(4).max(1);
        assert!(viewer_rows < original_size.rows() || viewer_cols < original_size.columns());

        // Resize the view as if a viewer with a smaller winsize has joined the session.
        terminal.update(&mut app, |view, ctx| {
            view.resize_from_viewer_report(
                WindowSize {
                    num_rows: viewer_rows,
                    num_cols: viewer_cols,
                },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert_eq!(view.size_info().rows(), viewer_rows);
            assert_eq!(view.size_info().columns(), viewer_cols);
            assert_eq!(
                view.active_viewer_driven_size,
                Some((viewer_rows, viewer_cols))
            );
            assert_eq!(*view.model.lock().block_list().size(), *view.size_info());
        });

        // End the session, assert that the winsize was restored to the original.
        terminal.update(&mut app, |view, ctx| {
            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, _| {
            assert_eq!(view.size_info().rows(), original_size.rows());
            assert_eq!(view.size_info().columns(), original_size.columns());
            assert_eq!(view.active_viewer_driven_size, None);
            assert_eq!(*view.model.lock().block_list().size(), original_size);
        });
    })
}

#[test]
fn test_on_session_share_ended_does_not_insert_tombstone_for_ambient_session_under_cloud_mode_setup_v2(
) {
    let _flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(None));
            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, _| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            // Only shared session ended banner. Ended-state continuation UI is
            // driven by the task resolver path under CloudModeSetupV2.
            assert_eq!(final_block_height_items, initial_block_height_items + 1);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
        });
    });
}

fn create_cloud_mode_task_for_user(creator_uid: &str) -> AmbientAgentTask {
    let now = chrono::Utc::now();
    AmbientAgentTask {
        task_id: uuid::Uuid::new_v4().to_string().parse().unwrap(),
        parent_run_id: None,
        title: "Owned task".to_string(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        status_message: None,
        source: Some(AgentSource::CloudMode),
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: None,
        }),
        executor: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

fn insert_cloud_mode_task_with_server_metadata(
    app: &mut App,
    terminal_view_id: EntityId,
    mut task: AmbientAgentTask,
    harness: AIAgentHarness,
    permissions: ServerPermissions,
) {
    let task_id = task.task_id;
    let conversation_token = task_id.to_string();
    task.conversation_id = Some(conversation_token.clone());

    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });
    BlocklistAIHistoryModel::handle(app).update(app, |model, ctx| {
        let conversation_id =
            model.start_new_conversation(terminal_view_id, false, false, false, ctx);
        model.set_server_conversation_token_for_conversation(
            conversation_id,
            conversation_token.clone(),
        );
        model.set_server_metadata_for_conversation(
            conversation_id,
            server_conversation_metadata(harness, task_id, conversation_token, permissions),
            ctx,
        );
    });
}

fn server_conversation_metadata(
    harness: AIAgentHarness,
    ambient_agent_task_id: AmbientAgentTaskId,
    server_conversation_token: String,
    permissions: ServerPermissions,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "Conversation".to_string(),
        working_directory: None,
        harness,
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
            creator_uid: None,
            last_editor_uid: None,
            current_editor_uid: None,
        },
        permissions,
        ambient_agent_task_id: Some(ambient_agent_task_id),
        server_conversation_token: ServerConversationToken::new(server_conversation_token),
        artifacts: vec![],
    }
}

fn current_user_owner_permissions() -> ServerPermissions {
    server_permissions(Owner::mock_current_user())
}

fn server_permissions(space: Owner) -> ServerPermissions {
    ServerPermissions {
        space,
        guests: vec![],
        anyone_link_sharing: None,
        permissions_last_updated_ts: Utc::now().into(),
    }
}

fn configure_ambient_details_panel_test(
    app: &mut App,
    terminal: &ViewHandle<TerminalView>,
    task: AmbientAgentTask,
) -> AmbientAgentTaskId {
    let task_id = task.task_id;
    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });
    terminal.update(app, |view, _| {
        view.model
            .lock()
            .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
    });
    task_id
}

#[test]
fn test_conversation_details_auto_open_policy_defaults_to_open_for_ambient_shared_session() {
    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task_id = configure_ambient_details_panel_test(
            &mut app,
            &terminal,
            create_cloud_mode_task_for_user(TEST_USER_UID),
        );

        terminal.update(&mut app, |view, ctx| {
            assert_eq!(
                view.ambient_agent_task_id_for_details_panel(ctx),
                Some(task_id)
            );
            assert!(!view.is_conversation_details_panel_open);
            assert!(!view.has_auto_opened_conversation_details_panel);

            view.maybe_auto_open_conversation_details_panel(ctx);

            assert!(view.is_conversation_details_panel_open);
            assert!(view.has_auto_opened_conversation_details_panel);
            assert!(
                view.conversation_details_panel
                    .as_ref(ctx)
                    .task_display_status_for_test()
                    .is_some(),
                "auto-open should fetch details when details are available"
            );
        });
    });
}

#[test]
fn test_suppressed_conversation_details_auto_open_consumes_initial_open_but_manual_toggle_works() {
    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        configure_ambient_details_panel_test(
            &mut app,
            &terminal,
            create_cloud_mode_task_for_user(TEST_USER_UID),
        );

        terminal.update(&mut app, |view, ctx| {
            view.suppress_initial_conversation_details_panel_auto_open();

            view.maybe_auto_open_conversation_details_panel(ctx);

            assert!(!view.is_conversation_details_panel_open);
            assert!(view.has_auto_opened_conversation_details_panel);
            assert!(
                view.conversation_details_panel
                    .as_ref(ctx)
                    .task_display_status_for_test()
                    .is_none(),
                "suppressed auto-open should not fetch details"
            );

            view.handle_action(&TerminalAction::ToggleConversationDetailsPanel, ctx);

            assert!(view.is_conversation_details_panel_open);
            assert!(
                view.conversation_details_panel
                    .as_ref(ctx)
                    .task_display_status_for_test()
                    .is_some(),
                "manual toggle should fetch details after suppressed auto-open"
            );
        });
    });
}

#[test]
fn test_child_shared_session_link_keeps_default_conversation_details_auto_open() {
    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task_id = configure_ambient_details_panel_test(
            &mut app,
            &terminal,
            create_cloud_mode_task_for_user(TEST_USER_UID),
        );

        terminal.update(&mut app, |view, ctx| {
            let parent_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(view.id(), false, false, false, ctx)
                });
            let mut child_conversation = AIConversation::new(true, false);
            child_conversation.set_parent_conversation_id(parent_conversation_id);
            child_conversation.set_task_id(task_id);
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.restore_conversations(view.id(), vec![child_conversation], ctx);
            });

            view.maybe_auto_open_conversation_details_panel(ctx);

            assert!(view.is_conversation_details_panel_open);
            assert!(view.has_auto_opened_conversation_details_panel);
            assert!(
                view.conversation_details_panel
                    .as_ref(ctx)
                    .task_display_status_for_test()
                    .is_some(),
                "child task metadata alone should not suppress direct-link auto-open"
            );
        });
    });
}

fn cloud_mode_terminal_for_test(app: &mut App) -> ViewHandle<TerminalView> {
    initialize_app_for_terminal_view(app);
    let tips_model = app.add_model(|_| Default::default());
    let (_, terminal) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        TerminalView::new_for_test_with_cloud_mode(tips_model, None, true, ctx)
    });
    terminal
}

#[test]
fn test_ambient_session_join_auto_opens_details_panel() {
    let _cloud_mode_flag = FeatureFlag::CloudMode.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let firebase_uid = UserUid::new("mock_firebase_uid");

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ViewPending);
            view.on_session_share_joined(
                ParticipantId::new(),
                firebase_uid,
                ReplicaId::random(),
                Box::new(ParticipantList::default()),
                SessionId::new(),
                SessionSourceType::AmbientAgent { task_id: None },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(view.is_conversation_details_panel_open);
            assert!(view.has_auto_opened_conversation_details_panel);
        });
    });
}

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
#[test]
fn test_local_to_cloud_handoff_session_join_keeps_details_panel_hidden() {
    let _cloud_mode_flag = FeatureFlag::CloudMode.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let firebase_uid = UserUid::new("mock_firebase_uid");

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ViewPending);
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have an ambient agent view model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.set_pending_handoff(
                    Some(PendingHandoff {
                        forked_conversation_id: None,
                        title: None,
                        touched_workspace: None,
                        snapshot_upload: SnapshotUploadStatus::Pending,
                        submission_state: HandoffSubmissionState::Idle,
                        auto_submit: None,
                    }),
                    ctx,
                );
            });

            view.on_session_share_joined(
                ParticipantId::new(),
                firebase_uid,
                ReplicaId::random(),
                Box::new(ParticipantList::default()),
                SessionId::new(),
                SessionSourceType::AmbientAgent { task_id: None },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(!view.is_conversation_details_panel_open);
            assert!(!view.has_auto_opened_conversation_details_panel);
        });
    });
}

#[test]
fn test_cloud_cloud_handoff_session_join_keeps_closed_details_panel_hidden() {
    let _cloud_mode_flag = FeatureFlag::CloudMode.override_enabled(true);
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;
        let firebase_uid = UserUid::new("mock_firebase_uid");

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        terminal.update(&mut app, |view, ctx| {
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            assert!(!view.is_conversation_details_panel_open);
            assert!(!view.has_auto_opened_conversation_details_panel);

            view.enable_cloud_followup_input(task_id, ctx);
            view.handle_ambient_agent_event(
                &crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent::ExecutionSessionReady {
                    session_id: SessionId::new(),
                },
                ctx,
            );

            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ViewPending);
            view.on_session_share_joined(
                ParticipantId::new(),
                firebase_uid,
                ReplicaId::random(),
                Box::new(ParticipantList::default()),
                SessionId::new(),
                SessionSourceType::AmbientAgent {
                    task_id: Some(task_id.to_string()),
                },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(!view.is_conversation_details_panel_open);
            assert!(view.has_auto_opened_conversation_details_panel);
        });
    });
}

#[test]
fn test_cloud_cloud_handoff_session_join_respects_details_panel_closed_after_followup_input() {
    let _cloud_mode_flag = FeatureFlag::CloudMode.override_enabled(true);
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;
        let firebase_uid = UserUid::new("mock_firebase_uid");

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        terminal.update(&mut app, |view, ctx| {
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            view.is_conversation_details_panel_open = true;
            view.fetch_and_update_conversation_details_panel(ctx);
            assert!(view.is_conversation_details_panel_open);

            view.enable_cloud_followup_input(task_id, ctx);
            view.handle_action(&TerminalAction::ToggleConversationDetailsPanel, ctx);
            assert!(!view.is_conversation_details_panel_open);
            assert!(!view.has_auto_opened_conversation_details_panel);

            view.handle_ambient_agent_event(
                &crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent::ExecutionSessionReady {
                    session_id: SessionId::new(),
                },
                ctx,
            );

            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ViewPending);
            view.on_session_share_joined(
                ParticipantId::new(),
                firebase_uid,
                ReplicaId::random(),
                Box::new(ParticipantList::default()),
                SessionId::new(),
                SessionSourceType::AmbientAgent {
                    task_id: Some(task_id.to_string()),
                },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(!view.is_conversation_details_panel_open);
            assert!(view.has_auto_opened_conversation_details_panel);
        });
    });
}
#[test]
fn test_restored_ambient_view_resolves_cta_from_view_model_task_id() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::ClaudeCode,
            current_user_owner_permissions(),
        );

        terminal.update(&mut app, |view, ctx| {
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            {
                let model = view.model.lock();
                assert!(!model.is_shared_ambient_agent_session());
                assert!(model.conversation_transcript_viewer_status().is_none());
            }

            let state = view
                .cloud_conversation_continuation_ui_state(ctx)
                .expect("ambient view model task ID should pass the continuation gate");
            assert!(matches!(
                state,
                CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id: resolved_task_id })
                } if resolved_task_id == task_id
            ));
        });
    });
}

#[test]
fn test_restored_oz_edit_access_non_owner_finished_view_uses_followup_input_without_tombstone() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::Oz,
            current_user_owner_permissions(),
        );

        terminal.update(&mut app, |view, ctx| {
            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
            });
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });
            let initial_block_height_items = {
                let mut model = view.model.lock();
                model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
                model.block_list().block_heights().items().len()
            };

            view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);

            assert_eq!(
                view.model.lock().block_list().block_heights().items().len(),
                initial_block_height_items
            );
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            {
                let model = view.model.lock();
                assert!(matches!(
                    model.shared_session_status(),
                    SharedSessionStatus::NotShared
                ));
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_on_session_share_ended_enables_followup_input_without_tombstone_for_owned_ambient_session()
{
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::Oz,
            current_user_owner_permissions(),
        );
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            assert_eq!(final_block_height_items, initial_block_height_items + 1);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_on_session_share_ended_hides_input_for_no_cta_tombstone() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let mut task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;
        task.state = AmbientAgentTaskState::Failed;
        task.status_message = Some(TaskStatusMessage {
            message: "Environment setup failed: Failed to run setup command".to_string(),
            error_code: Some(TaskStatusErrorCode::EnvironmentSetupFailed),
        });

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Editable, ctx);
                });
            });
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));

            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert_eq!(
                model.block_list().block_heights().items().len(),
                initial_block_height_items + 2
            );
            assert!(view.conversation_ended_tombstone_view_id.is_some());
            assert!(!view.is_input_box_visible(&model, ctx));
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Selectable
            );
        });
    });
}

#[test]
fn test_on_session_share_ended_does_not_insert_tombstone_for_owned_ambient_session_without_handoff()
{
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(false);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::Oz,
            current_user_owner_permissions(),
        );
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            assert_eq!(final_block_height_items, initial_block_height_items + 1);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, None);
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Selectable
            );
            let model = view.model.lock();
            assert!(view.should_publish_shared_session_input_editor_update(&model, ctx));
        });
    });
}

#[test]
fn test_on_session_share_ended_clears_frozen_followup_input_for_owned_ambient_session() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::Oz,
            current_user_owner_permissions(),
        );

        terminal.update(&mut app, |view, ctx| {
            view.input().update(ctx, |input, ctx| {
                input.replace_buffer_content("can you put it in a file", ctx);
                input.freeze_input_in_loading_state(ctx);
            });
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert_eq!(view.input().as_ref(ctx).buffer_text(ctx), "");
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_on_session_share_ended_does_not_insert_tombstone_for_non_ambient_session_under_cloud_mode_setup_v2(
) {
    let _flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::user(None));
            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, _| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            // Only shared session ended banner.
            assert_eq!(final_block_height_items, initial_block_height_items + 1);
        });
    });
}

#[test]
fn test_on_ambient_agent_execution_ended_inserts_tombstone_when_handoff_enabled() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
            view.on_ambient_agent_execution_ended(ctx);
            view.on_ambient_agent_execution_ended(ctx);
        });

        terminal.read(&app, |view, _| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            assert_eq!(final_block_height_items, initial_block_height_items + 1);
            assert!(view.conversation_ended_tombstone_view_id.is_some());
        });
    });
}

#[test]
fn test_on_ambient_agent_execution_ended_enables_followup_for_owned_task_without_metadata() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::NotShared);
            drop(model);

            view.on_ambient_agent_execution_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            assert_eq!(final_block_height_items, initial_block_height_items);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_on_ambient_agent_execution_ended_enables_followup_input_without_tombstone_for_owned_task() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::Oz,
            current_user_owner_permissions(),
        );
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::NotShared);
            drop(model);

            view.on_ambient_agent_execution_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            assert_eq!(final_block_height_items, initial_block_height_items);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_restored_owned_tombstone_hides_input_until_continue() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::NotShared);
            drop(model);

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });
            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
            });

            view.insert_conversation_ended_tombstone_with_cta(
                Some(TombstoneCta::ContinueInCloud { task_id }),
                ctx,
            );
            assert!(view.conversation_ended_tombstone_view_id.is_some());
            {
                let model = view.model.lock();
                assert!(!view.is_input_box_visible(&model, ctx));
            }

            view.start_cloud_followup_from_tombstone(task_id, ctx);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            {
                let model = view.model.lock();
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_deep_linked_ambient_continuation_refreshes_when_task_data_arrives() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        terminal.update(&mut app, |view, ctx| {
            // Mirrors opening a cloud conversation directly (for example, a
            // Warp-on-Web deep link) before AgentConversationsModel has loaded
            // the ambient task. The restored pane only has the task id from
            // conversation metadata, so it first renders the conservative
            // ended-session UI.
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
            drop(model);

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);

            assert!(view.conversation_ended_tombstone_view_id.is_some());
            assert_eq!(view.pending_cloud_followup_task_id, None);
            {
                let model = view.model.lock();
                assert!(matches!(
                    model.shared_session_status(),
                    SharedSessionStatus::FinishedViewer
                ));
                assert!(!view.is_input_box_visible(&model, ctx));
            }
        });

        AgentConversationsModel::handle(&app).update(&mut app, |model, ctx| {
            // Once the task fetch or initial task sync finishes, the terminal
            // subscription should re-resolve the continuation state and replace
            // the conservative tombstone with owned follow-up input.
            model.insert_task_for_test(task);
            ctx.emit(AgentConversationsModelEvent::TasksUpdated);
        });

        terminal.read(&app, |view, ctx| {
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            {
                let model = view.model.lock();
                assert!(matches!(
                    model.shared_session_status(),
                    SharedSessionStatus::NotShared
                ));
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}
#[test]
fn test_on_ambient_agent_execution_ended_keeps_live_owned_session_on_session_sharing_path() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::executor());
            drop(model);
            view.on_ambient_agent_execution_ended(ctx);
        });

        terminal.read(&app, |view, _| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            assert_eq!(final_block_height_items, initial_block_height_items);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, None);
        });
    });
}

#[test]
fn test_try_submit_pending_cloud_followup_allows_repeat_submission_for_owned_task() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::executor());
            drop(model);

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            view.enable_cloud_followup_input(task_id, ctx);
            assert!(view.try_submit_pending_cloud_followup("follow up".to_string(), ctx));
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert!(view.try_submit_pending_cloud_followup("second follow up".to_string(), ctx));
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert_eq!(
                ambient_agent_view_model
                    .as_ref(ctx)
                    .pending_followup_prompt(),
                Some("second follow up")
            );
        });
    });
}
#[test]
fn test_shared_followup_on_existing_conversation_converts_user_query_input() {
    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let terminal_view_id = terminal.id();
        let conversation_token = "restored-conversation-token";
        let request_id = "new-followup-request";
        let root_task_id = "root-task";
        let followup_query = "follow up";

        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |model, ctx| {
                let conversation_id =
                    model.start_new_conversation(terminal_view_id, false, false, false, ctx);
                model.set_server_conversation_token_for_conversation(
                    conversation_id,
                    conversation_token.to_string(),
                );
                conversation_id
            });

        terminal.update(&mut app, |view, ctx| {
            let init_event = api::ResponseEvent {
                r#type: Some(api::response_event::Type::Init(
                    api::response_event::StreamInit {
                        request_id: request_id.to_string(),
                        conversation_id: conversation_token.to_string(),
                        run_id: String::new(),
                    },
                )),
            };
            view.ai_controller.update(ctx, |controller, ctx| {
                controller.handle_shared_session_response_event(init_event, ctx);
            });

            let create_root_task_event = api::ResponseEvent {
                r#type: Some(api::response_event::Type::ClientActions(
                    api::response_event::ClientActions {
                        actions: vec![api::ClientAction {
                            action: Some(api_client_action::Action::CreateTask(
                                api_client_action::CreateTask {
                                    task: Some(api::Task {
                                        id: root_task_id.to_string(),
                                        messages: vec![],
                                        dependencies: None,
                                        description: String::new(),
                                        summary: String::new(),
                                        server_data: String::new(),
                                    }),
                                },
                            )),
                        }],
                    },
                )),
            };
            view.ai_controller.update(ctx, |controller, ctx| {
                controller.handle_shared_session_response_event(create_root_task_event, ctx);
            });

            let add_user_query_event = api::ResponseEvent {
                r#type: Some(api::response_event::Type::ClientActions(
                    api::response_event::ClientActions {
                        actions: vec![api::ClientAction {
                            action: Some(api_client_action::Action::AddMessagesToTask(
                                api_client_action::AddMessagesToTask {
                                    task_id: root_task_id.to_string(),
                                    messages: vec![api::Message {
                                        id: "user-message".to_string(),
                                        task_id: root_task_id.to_string(),
                                        server_message_data: String::new(),
                                        citations: vec![],
                                        message: Some(api::message::Message::UserQuery(
                                            api::message::UserQuery {
                                                query: followup_query.to_string(),
                                                context: None,
                                                referenced_attachments: HashMap::new(),
                                                mode: None,
                                                intended_agent: Default::default(),
                                            },
                                        )),
                                        request_id: request_id.to_string(),
                                        timestamp: None,
                                    }],
                                },
                            )),
                        }],
                    },
                )),
            };
            view.ai_controller.update(ctx, |controller, ctx| {
                controller.handle_shared_session_response_event(add_user_query_event, ctx);
            });
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |model, _| {
            let conversation = model
                .conversation(&conversation_id)
                .expect("conversation should exist");
            assert!(conversation.is_viewing_shared_session());

            let input = conversation
                .latest_exchange()
                .and_then(|exchange| exchange.input.first())
                .expect("shared-session replay should reconstruct the user query input");
            assert!(matches!(input, AIAgentInput::UserQuery { .. }));
            assert_eq!(input.user_query().as_deref(), Some(followup_query));
        });
    });
}

#[test]
fn test_non_owned_tombstone_is_removed_for_followup_and_reinserted_after_completion() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;
        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::ClaudeCode,
            current_user_owner_permissions(),
        );

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
            drop(model);

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            let initial_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
            assert!(view.conversation_ended_tombstone_view_id.is_some());
            assert_eq!(
                view.model.lock().block_list().block_heights().items().len(),
                initial_block_height_items + 1
            );

            view.start_cloud_followup_from_tombstone(task_id, ctx);
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            {
                let model = view.model.lock();
                assert!(matches!(
                    model.shared_session_status(),
                    SharedSessionStatus::NotShared
                ));
                assert!(view.is_input_box_visible(&model, ctx));
                assert_eq!(
                    model.block_list().block_heights().items().len(),
                    initial_block_height_items
                );
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );

            view.handle_ambient_agent_event(
                &crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent::ExecutionSessionReady {
                    session_id: SessionId::new(),
                },
                ctx,
            );
            view.on_ambient_agent_execution_ended(ctx);
            assert!(view.conversation_ended_tombstone_view_id.is_some());
            assert_eq!(
                view.model.lock().block_list().block_heights().items().len(),
                initial_block_height_items + 1
            );
        });
    });
}

#[test]
fn test_on_ambient_agent_execution_ended_refreshes_open_details_panel_to_terminal_status() {
    let _cloud_mode_flag = FeatureFlag::CloudMode.override_enabled(true);
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _orchestration_v2_flag = FeatureFlag::OrchestrationV2.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let session_id = SessionId::new();
        let mut task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;
        task.state = AmbientAgentTaskState::InProgress;
        task.session_id = Some(session_id.to_string());
        task.session_link = Some("https://example.com/session/active".to_string());
        task.is_sandbox_running = true;

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |model, ctx| {
            let conversation_id =
                model.start_new_conversation(terminal.id(), false, false, false, ctx);
            model.assign_run_id_for_conversation(
                conversation_id,
                task_id.to_string(),
                Some(task_id),
                terminal.id(),
                ctx,
            );
            model.update_conversation_status(
                terminal.id(),
                conversation_id,
                ConversationStatus::Success,
                ctx,
            );
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::executor());
            drop(model);

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            view.is_conversation_details_panel_open = true;
            view.fetch_and_update_conversation_details_panel(ctx);
            assert_eq!(
                view.conversation_details_panel
                    .as_ref(ctx)
                    .task_display_status_for_test(),
                Some(AgentRunDisplayStatus::TaskInProgress)
            );

            view.on_ambient_agent_execution_ended(ctx);
            assert_eq!(
                view.conversation_details_panel
                    .as_ref(ctx)
                    .task_display_status_for_test(),
                Some(AgentRunDisplayStatus::ConversationSucceeded)
            );
        });

        let task = AgentConversationsModel::handle(&app).read(&app, |model, _| {
            model
                .get_task_data(&task_id)
                .expect("task should remain cached")
        });
        assert!(!task.is_sandbox_running);
    });
}

#[test]
fn test_on_ambient_agent_execution_ended_inserts_tombstone_without_handoff() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(false);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.on_ambient_agent_execution_ended(ctx);
        });

        terminal.read(&app, |view, _| {
            let final_block_height_items =
                view.model.lock().block_list().block_heights().items().len();
            assert_eq!(final_block_height_items, initial_block_height_items + 1);
            assert!(view.conversation_ended_tombstone_view_id.is_some());
        });
    });
}
