use warp::appearance::Appearance;
use warp::tui_export::{
    AmbientAgentTaskId, BlocklistAIHistoryModel, CloudAgentStartupBlocker, ConversationStatus,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, SingletonEntity as _};
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, TuiView as _, TypedActionView as _};

use super::{TuiCloudRunAction, TuiCloudRunView};
use crate::cloud_run::TuiCloudRunState;
use crate::terminal_session_view::CTRL_C_KILL_CHILD_HINT;
use crate::test_fixtures::TestHostView;
use crate::tui_builder::TuiUiBuilder;

const RUN_URL: &str = "https://oz.staging.warp.dev/runs/019f71ef-6285-7480-90f6-3ad84d8e0d1e";
const TASK_ID: &str = "11111111-1111-1111-1111-111111111111";

#[test]
fn lightweight_cloud_view_renders_startup_and_blocker_without_terminal_state() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let window_id = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            )
            .0
        });
        let state = app.add_model(|_| TuiCloudRunState::new());
        let view = app.update(|ctx| {
            ctx.add_typed_action_tui_view(window_id, |ctx| TuiCloudRunView::new(state.clone(), ctx))
        });
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 80, 24),
                ctx,
            );
            assert!(
                frame
                    .buffer
                    .to_lines()
                    .iter()
                    .any(|line| line.contains("Starting cloud run…"))
            );
        });

        app.update(|ctx| {
            state.update(ctx, |state, ctx| {
                state.set_blocked(
                    CloudAgentStartupBlocker::GitHubAuthRequired {
                        message: "GitHub authentication required".to_string(),
                        auth_url: "https://example.com/auth".to_string(),
                    },
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 80, 24),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("GitHub Authentication Required"))
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("https://example.com/auth"))
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Authenticate with GitHub"))
            );
            assert!(
                !lines
                    .iter()
                    .any(|line| line.contains("GitHub authentication required Authenticate"))
            );
        });
    });
}

#[test]
fn spawned_cloud_view_matches_figma_in_progress_and_succeeded_states() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let window_id = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            )
            .0
        });
        let state = app.add_model(|_| TuiCloudRunState::new());
        let view = app.update(|ctx| {
            ctx.add_typed_action_tui_view(window_id, |ctx| TuiCloudRunView::new(state.clone(), ctx))
        });
        let conversation_id = app.update(|ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id =
                        history.start_new_conversation(view.id(), false, false, false, ctx);
                    history.set_active_conversation_id(conversation_id, view.id(), ctx);
                    conversation_id
                });
            state.update(ctx, |state, ctx| {
                state.set_conversation_id(conversation_id, ctx);
                state.set_spawned(
                    TASK_ID
                        .parse::<AmbientAgentTaskId>()
                        .expect("hardcoded task id parses"),
                    "019f71ef-6285-7480-90f6-3ad84d8e0d1e".to_string(),
                    RUN_URL.to_string(),
                    ctx,
                );
            });
            conversation_id
        });

        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 112, 24),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            let visible_lines = lines
                .iter()
                .enumerate()
                .filter_map(|(row, line)| (!line.trim().is_empty()).then_some((row, line.trim())))
                .collect::<Vec<_>>();
            assert_eq!(
                visible_lines,
                vec![
                    (7, "*****⟡○○*"),
                    (8, "*******⚬⚬⚬⚬⚬*****"),
                    (9, "****○○*⚬⚬⚬◌⟡◌⚬⚬⚬*○○****"),
                    (10, "**◌◌*○○⚬⚬⚬○○⚬⚬⚬○○⟡◌◌**"),
                    (11, "*○○⟡*******"),
                    (14, "● Cloud run in progress"),
                    (15, "Press enter to view or click the link below"),
                    (17, RUN_URL),
                ]
            );

            let builder = TuiUiBuilder::from_app(ctx);
            let mark_start = lines[7].find("*****⟡○○*").expect("mark is visible");
            assert_eq!(
                Some(frame.buffer[(mark_start as u16, 7)].fg),
                builder.cloud_run_mark_styles().brightest.fg
            );
            assert_eq!(
                Some(frame.buffer[((mark_start + 3) as u16, 7)].fg),
                builder.cloud_run_mark_styles().ansi_bright.fg
            );
            let status_start = lines[14]
                .find("● Cloud run in progress")
                .expect("status is visible");
            assert_eq!(
                Some(frame.buffer[(status_start as u16, 14)].fg),
                builder.attention_glyph_style().fg
            );
            let instruction_start = lines[15].find("Press ").expect("instruction is visible");
            assert!(
                frame.buffer[((instruction_start + "Press ".len()) as u16, 15)]
                    .modifier
                    .contains(Modifier::BOLD)
            );
            let url_start = lines[17].find(RUN_URL).expect("URL is visible");
            assert!(
                frame.buffer[(url_start as u16, 17)]
                    .modifier
                    .contains(Modifier::UNDERLINED)
            );
            assert_eq!(
                Some(frame.buffer[(url_start as u16, 17)].fg),
                builder.muted_text_style().fg
            );
        });

        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.update_conversation_status(
                    view.id(),
                    conversation_id,
                    ConversationStatus::Success,
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 112, 24),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            assert_eq!(lines[14].trim(), "✓ Cloud run succeeded");
            let status_start = lines[14]
                .find("✓ Cloud run succeeded")
                .expect("success status is visible");
            assert_eq!(
                Some(frame.buffer[(status_start as u16, 14)].fg),
                TuiUiBuilder::from_app(ctx).success_glyph_style().fg
            );
        });
    });
}

#[test]
fn cloud_child_first_interrupt_arms_kill_window_not_exit_window() {
    // AC 9 / cloud child path: when the cloud run view has a conversation_id
    // (i.e. it is a spawned child), the first Ctrl+C must arm the kill window
    // (child_kill_armed = true) and show the kill hint — not arm the normal
    // double-Ctrl+C TUI-exit window.
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        let window_id = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            )
            .0
        });
        let state = app.add_model(|_| TuiCloudRunState::new());
        let view = app.update(|ctx| {
            ctx.add_typed_action_tui_view(window_id, |ctx| TuiCloudRunView::new(state.clone(), ctx))
        });
        // Set a conversation_id so the view knows it is a spawned child.
        let conversation_id = app.update(|ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id =
                        history.start_new_conversation(view.id(), false, false, false, ctx);
                    history.set_active_conversation_id(conversation_id, view.id(), ctx);
                    conversation_id
                });
            state.update(ctx, |state, ctx| {
                state.set_conversation_id(conversation_id, ctx);
                state.set_spawned(
                    TASK_ID
                        .parse::<AmbientAgentTaskId>()
                        .expect("hardcoded task id parses"),
                    "019f71ef-6285-7480-90f6-3ad84d8e0d1e".to_string(),
                    RUN_URL.to_string(),
                    ctx,
                );
            });
            conversation_id
        });

        // First Ctrl+C on a spawned cloud view.
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiCloudRunAction::Interrupt, ctx);
        });

        view.read(&app, |view, _| {
            assert!(
                view.child_kill_armed,
                "first interrupt on a spawned cloud view must arm child_kill_armed"
            );
            assert!(
                view.exit_confirmation.is_armed(),
                "the kill timing window must be armed"
            );
        });

        // The footer (where tabs are absent) should show the kill hint.
        // Re-render and check the last visible line.
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 80, 24),
                ctx,
            );
            let visible = frame
                .buffer
                .to_lines()
                .into_iter()
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>();
            // At least one line should contain the kill hint.
            let hint_visible = visible.iter().any(|l| l.contains(CTRL_C_KILL_CHILD_HINT));
            assert!(
                hint_visible,
                "kill-child hint must be visible in the footer after first interrupt; \n\
                 rendered: {}",
                visible.join("\n")
            );
        });
        let _ = conversation_id; // suppress unused warning
    });
}
