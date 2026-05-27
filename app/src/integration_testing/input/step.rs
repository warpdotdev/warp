use pathfinder_geometry::vector::Vector2F;
use warpui::integration::TestStep;
use warpui::windowing::WindowManager;
use warpui::SingletonEntity;

use crate::ai::blocklist::{InputConfig, InputType};
use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::terminal::assert_context_menu_is_open;
use crate::integration_testing::view_getters::{
    single_input_view_for_tab, single_terminal_view, single_terminal_view_for_tab,
};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::terminal::view::TerminalAction;
use crate::terminal::CLIAgent;

/// Opens the CLI-agent Rich Input for the terminal view at the given tab index.
///
/// Replicates the `open_rich_input_for_terminal` helper used in `input_tests.rs`
/// so that integration tests can drive the full keybinding / event-dispatch path
/// without spawning a real CLI agent process.
///
/// The `tab_index` parameter selects the terminal pane (use `0` for the first
/// and only pane in a fresh test session).
pub fn open_cli_agent_rich_input(tab_index: usize) -> TestStep {
    new_step_with_default_assertions("Open CLI Agent Rich Input").with_action(
        move |app, window_id, _step_data| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
            terminal_view.update(app, |view, ctx| {
                let view_id = view.view_id();

                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.set_session(
                        view_id,
                        CLIAgentSession {
                            agent: CLIAgent::Claude,
                            status: CLIAgentSessionStatus::InProgress,
                            session_context: CLIAgentSessionContext::default(),
                            input_state: CLIAgentInputState::Closed,
                            should_auto_toggle_input: false,
                            listener: None,
                            remote_host: None,
                            plugin_version: None,
                            draft_text: None,
                            custom_command_prefix: None,
                        },
                        ctx,
                    );
                });

                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.open_input(
                        view_id,
                        CLIAgentInputEntrypoint::CtrlG,
                        InputConfig {
                            input_type: InputType::AI,
                            is_locked: true,
                        },
                        false,
                        false,
                        ctx,
                    );
                });
            });
        },
    )
}

/// Asserts that the Rich Input buffer text for `tab_index` matches the predicate.
///
/// Returns an [`AssertionCallback`] suitable for use with `add_assertion`.
pub fn rich_input_buffer_text_is_empty(tab_index: usize) -> warpui::integration::AssertionCallback {
    Box::new(move |app, window_id| {
        let input_view = single_input_view_for_tab(app, window_id, tab_index);
        input_view.read(app, |view, ctx| {
            let text = view.buffer_text(ctx);
            warpui::async_assert!(
                text.is_empty(),
                "Expected Rich Input buffer to be empty; got: {text:?}"
            )
        })
    })
}

/// Asserts that the Rich Input buffer text for `tab_index` contains a newline character.
pub fn rich_input_buffer_contains_newline(
    tab_index: usize,
) -> warpui::integration::AssertionCallback {
    Box::new(move |app, window_id| {
        let input_view = single_input_view_for_tab(app, window_id, tab_index);
        input_view.read(app, |view, ctx| {
            let text = view.buffer_text(ctx);
            warpui::async_assert!(
                text.contains('\n'),
                "Expected Rich Input buffer to contain a newline; got: {text:?}"
            )
        })
    })
}

/// Asserts that the Rich Input buffer text for `tab_index` is NOT empty.
pub fn rich_input_buffer_is_not_empty(tab_index: usize) -> warpui::integration::AssertionCallback {
    Box::new(move |app, window_id| {
        let input_view = single_input_view_for_tab(app, window_id, tab_index);
        input_view.read(app, |view, ctx| {
            let text = view.buffer_text(ctx);
            warpui::async_assert!(
                !text.is_empty(),
                "Expected Rich Input buffer to be non-empty; got: {text:?}"
            )
        })
    })
}

pub fn open_input_context_menu() -> TestStep {
    new_step_with_default_assertions("Open input context menu")
        .with_action(move |app, _, _| {
            let window_id = app.read(|ctx| {
                WindowManager::as_ref(ctx)
                    .active_window()
                    .expect("no active window")
            });
            let terminal_view_id = single_terminal_view(app, window_id).id();
            app.dispatch_typed_action(
                window_id,
                &[terminal_view_id],
                &TerminalAction::OpenInputContextMenu {
                    position: Vector2F::new(8.5, 8.5),
                },
            );
        })
        .add_assertion(assert_context_menu_is_open(true))
}
