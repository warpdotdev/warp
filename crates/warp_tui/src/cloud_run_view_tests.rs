use std::cell::RefCell;
use std::rc::Rc;

use warp::appearance::Appearance;
use warp::tui_export::{
    AmbientAgentTaskId, BlocklistAIHistoryModel, CloudAgentStartupBlocker, ConversationStatus,
    FactoryAccess, FactoryAccessModel,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, SingletonEntity as _};
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiEvent, TuiPoint, TuiRect};
use warpui_core::event::{KeyEventDetails, ModifiersState};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, TuiView as _, TypedActionView as _};

use super::{TuiCloudRunAction, TuiCloudRunView};
use crate::cloud_run::TuiCloudRunState;
use crate::terminal_session_view::CTRL_C_KILL_CHILD_HINT;
use crate::test_fixtures::TestHostView;
use crate::tui_builder::TuiUiBuilder;

// Resolved from the default (production) channel config: no `FactoryAccessModel` singleton is
// registered in these tests, so the viewer's Factory access is `Unknown` and the link stays on
// Oz per `cloud_run_web_url` (APP-5583).
const RUN_URL: &str = "https://oz.warp.dev/runs/019f71ef-6285-7480-90f6-3ad84d8e0d1e";
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

fn key_event(key: &str) -> TuiEvent {
    TuiEvent::KeyDown {
        keystroke: Keystroke {
            key: key.to_owned(),
            ..Default::default()
        },
        chars: key.to_owned(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

fn left_click(x: u16, y: u16) -> (TuiEvent, TuiEvent) {
    (
        TuiEvent::LeftMouseDown {
            position: TuiPoint::new(x, y),
            modifiers: ModifiersState::default(),
            click_count: 1,
            is_first_mouse: false,
        },
        TuiEvent::LeftMouseUp {
            position: TuiPoint::new(x, y),
            modifiers: ModifiersState::default(),
        },
    )
}

#[test]
fn cloud_run_link_activation_resolves_fresh_on_a_stale_rendered_frame() {
    // The regression this guards: the rendered frame's click/Enter handlers used to close over
    // whatever destination `display_state` produced at render time, so a Factory access probe
    // that resolved after the frame was drawn had no effect until something re-rendered it.
    // Deliberately do NOT re-render after the access change below, and dispatch the actual
    // mouse and key events against the retained (stale) frame rather than calling
    // `primary_url`/`handle_action` directly — calling either of those would only prove the
    // resolver works, not that the rendered element's own handlers still reach it.
    const RUN_ID: &str = "019f71ef-6285-7480-90f6-3ad84d8e0d1e";

    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
        app.add_singleton_model(|_| FactoryAccessModel::new_for_test(FactoryAccess::Unknown));

        let opened_urls = Rc::new(RefCell::new(Vec::<String>::new()));
        let opened_urls_for_hook = opened_urls.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _ctx| {
                opened_urls_for_hook.borrow_mut().push(url.to_owned());
                url.to_owned()
            });
        });

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
        app.update(|ctx| {
            state.update(ctx, |state, ctx| {
                state.set_spawned(
                    TASK_ID
                        .parse::<AmbientAgentTaskId>()
                        .expect("hardcoded task id parses"),
                    RUN_ID.to_string(),
                    ctx,
                );
            });
        });

        // Render once, retained by `presenter`, while access is still Unknown: the frame shows
        // an Oz link. This is the "stale frame" the fix must not depend on ever being replaced.
        let mut presenter = TuiPresenter::new();
        let link_position = app.update(|ctx| {
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 112, 24),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            let row = lines
                .iter()
                .position(|line| line.contains(&format!("oz.warp.dev/runs/{RUN_ID}")))
                .unwrap_or_else(|| {
                    panic!(
                        "expected an Oz link while Factory access is still Unknown; rendered: {}",
                        lines.join("\n")
                    )
                });
            let col = lines[row]
                .find(&format!("oz.warp.dev/runs/{RUN_ID}"))
                .expect("column of the located line must contain the match");
            (col as u16, row as u16)
        });

        // The probe resolves to Allowed after the run was already spawned and rendered
        // (simulating it landing between spawn and the viewer activating the link). No render
        // happens after this point; `presenter` still holds the stale, Oz-rendered tree.
        app.update(|ctx| {
            FactoryAccessModel::handle(ctx).update(ctx, |model, _| {
                model.set_access_for_test(FactoryAccess::Allowed)
            });
        });

        // Click the link on the stale frame: TuiHoverable's press-then-release dispatches
        // OpenPrimaryUrl, which must resolve to Platform, not the Oz destination baked into the
        // frame the click was dispatched against.
        let (mouse_down, mouse_up) = left_click(link_position.0, link_position.1);
        app.update(|ctx| {
            presenter.dispatch_event(ctx, window_id, view.id(), &mouse_down);
            presenter.dispatch_event(ctx, window_id, view.id(), &mouse_up);
        });
        assert_eq!(
            opened_urls.borrow().as_slice(),
            [format!("https://platform.warp.dev/runs/{RUN_ID}")],
            "clicking the stale Oz-rendered link must open Platform once access resolved to \
             Allowed"
        );
        opened_urls.borrow_mut().clear();

        // Pressing Enter on the same stale frame must resolve the same way.
        app.update(|ctx| {
            presenter.dispatch_event(ctx, window_id, view.id(), &key_event("enter"));
        });
        assert_eq!(
            opened_urls.borrow().as_slice(),
            [format!("https://platform.warp.dev/runs/{RUN_ID}")],
            "pressing enter on the stale Oz-rendered frame must also open Platform"
        );
    });
}
