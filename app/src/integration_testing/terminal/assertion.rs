use pathfinder_geometry::rect::RectF;
use regex::Regex;
use settings::Setting as _;
use warp_util::path::user_friendly_path;
use warpui::integration::{AssertionCallback, AssertionOutcome};
use warpui::units::Lines;
use warpui::windowing::WindowManager;
use warpui::{App, SingletonEntity, ViewHandle, WindowId, async_assert, async_assert_eq};

use super::util::ExpectedOutput;
use crate::integration_testing::view_getters::{
    single_input_view_for_tab, single_terminal_view, single_terminal_view_for_tab, terminal_view,
};
use crate::settings::InputModeSettings;
use crate::terminal::History;
use crate::terminal::block_list_viewport::{InputMode, ScrollPosition};
use crate::terminal::model::block::BlockState;
use crate::terminal::model::blocks::BlockFilter;
use crate::terminal::model::bootstrap::BootstrapStage;
use crate::terminal::model::grid::grid_handler::TermMode;
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::view::TerminalViewState;
use crate::workspace::{ActiveSession, Workspace};

lazy_static::lazy_static! {
    /// When a python interpreter is ready for user input,
    /// the '>>>' prompt is displayed at the end of the REPL.
    pub static ref PYTHON_PROMPT_READY: Regex = Regex::new(">>> $").expect("python prompt regex should not fail to compile");
}

pub fn validate_block_output<T>(
    expected_output: &T,
    tab_idx: usize,
    pane_idx: usize,
    window_id: WindowId,
    app: &App,
) -> AssertionOutcome
where
    T: ExpectedOutput + ?Sized,
{
    let terminal_view = terminal_view(app, window_id, tab_idx, pane_idx);
    terminal_view.read(app, |view, _ctx| {
        let model = view.model.lock();
        let last_index = model
            .block_list()
            .last_matching_block_by_index(BlockFilter::commands());
        // After the last test step, there should always be a block here, but for
        // some reason, it sometimes doesn't exist.
        match last_index {
            Some(last_index) => {
                let block = model
                    .block_list()
                    .block_at(last_index)
                    .expect("Block should exist");
                let last_output = block
                    .output_grid()
                    .contents_to_string_with_secrets_unobfuscated(
                        false, /*include_escape_sequences*/
                        None,  /*max_rows*/
                    );
                async_assert!(
                    expected_output.matches(&last_output),
                    "The output should be {:?}, but got \"{}\"",
                    expected_output,
                    last_output
                )
            }
            None => AssertionOutcome::failure("No block yet".to_string()),
        }
    })
}

/// Assumes that the block is finished and its contents are now immutable.
/// Fails fast if the block contents don't match the expected output.
pub fn validate_block_output_on_finished_block<T>(
    expected_output: &T,
    tab_idx: usize,
    pane_idx: usize,
    window_id: WindowId,
    app: &App,
) -> AssertionOutcome
where
    T: ExpectedOutput + ?Sized,
{
    let terminal_view = terminal_view(app, window_id, tab_idx, pane_idx);
    terminal_view.read(app, |view, _ctx| {
        let model = view.model.lock();
        let last_index = model
            .block_list()
            .last_matching_block_by_index(BlockFilter::commands());
        // After the last test step, there should always be a block here, but for
        // some reason, it sometimes doesn't exist.
        match last_index {
            Some(last_index) => {
                let block = model
                    .block_list()
                    .block_at(last_index)
                    .expect("Block should exist");
                let last_output = block
                    .output_grid()
                    .contents_to_string_with_secrets_unobfuscated(
                        false, /*include_escape_sequences*/
                        None,  /*max_rows*/
                    );
                if expected_output.matches(&last_output) {
                    AssertionOutcome::Success
                } else {
                    AssertionOutcome::immediate_failure(format!(
                        "The output should be {expected_output:?}, but got \"{last_output}\""
                    ))
                }
            }
            None => AssertionOutcome::failure("No block yet".to_string()),
        }
    })
}

pub fn assert_input_mode(expected_input_mode: InputMode) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |_, ctx| {
            let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
            async_assert_eq!(input_mode, expected_input_mode, "input mode doesn't match")
        })
    })
}

pub fn assert_gap_exists(gap_exists: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        app.update(|ctx| {
            assert!(
                ctx.presenter(window_id)
                    .expect("should exist")
                    .borrow()
                    .scene()
                    .is_some()
            );
        });
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            let has_gap = model.block_list().active_gap().is_some();
            async_assert_eq!(
                gap_exists,
                has_gap,
                "Expected gap {} but was gap {}",
                gap_exists,
                has_gap
            )
        })
    })
}

#[derive(Debug)]
pub enum InputPosition {
    TopOfTerminal,
    BottomOfTerminal,
    NotAtEitherEdge,
}

const ROUNDING_ERROR_PX: f32 = 0.1;

impl InputPosition {
    fn assert_position(&self, terminal_rect: RectF, input_rect: RectF) -> bool {
        match *self {
            InputPosition::TopOfTerminal => {
                (terminal_rect.origin_y() - input_rect.origin_y()).abs() < ROUNDING_ERROR_PX
            }
            InputPosition::BottomOfTerminal => {
                (terminal_rect.max_y() - input_rect.max_y()).abs() < ROUNDING_ERROR_PX
            }
            InputPosition::NotAtEitherEdge => {
                terminal_rect.contains_rect(input_rect)
                    && !InputPosition::TopOfTerminal.assert_position(terminal_rect, input_rect)
                    && !InputPosition::BottomOfTerminal.assert_position(terminal_rect, input_rect)
            }
        }
    }
}

pub fn assert_input_position(input_position: InputPosition) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, ctx| {
            let terminal_rect = ctx
                .element_position_by_id_at_last_frame(window_id, view.terminal_position_id())
                .expect("terminal position should be set");
            let input_id = view.input().as_ref(ctx).save_position_id();
            let input_rect = ctx
                .element_position_by_id_at_last_frame(window_id, input_id)
                .expect("input position should be set");
            async_assert!(
                input_position.assert_position(terminal_rect, input_rect),
                "Input should be {:?} but it isn't.  Terminal rect {:?} and input rect {:?}",
                input_position,
                terminal_rect,
                input_rect
            )
        })
    })
}

pub fn assert_input_at_top_of_terminal() -> AssertionCallback {
    assert_input_position(InputPosition::TopOfTerminal)
}

pub fn assert_input_at_bottom_of_terminal() -> AssertionCallback {
    assert_input_position(InputPosition::BottomOfTerminal)
}

pub fn assert_input_not_at_either_edge_of_terminal() -> AssertionCallback {
    assert_input_position(InputPosition::NotAtEitherEdge)
}

pub fn assert_view_has_text_selection(has_text_selection: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _ctx| {
            let view_is_selecting = view.is_selecting();
            async_assert_eq!(
                view_is_selecting,
                has_text_selection,
                "Expected view to have text selection {} but it was {}",
                has_text_selection,
                view_is_selecting
            )
        })
    })
}

/// Asserts whether the waterfall gap empty state element is rendered or not
pub fn assert_waterfall_gap_empty_background_rendered(is_showing: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, ctx| {
            let element_showing = ctx
                .element_position_by_id_at_last_frame(
                    window_id,
                    view.waterfall_background_position_id(),
                )
                .is_some();
            async_assert_eq!(
                element_showing,
                is_showing,
                "Expected gap element to be showing {} but it was {}",
                is_showing,
                element_showing
            )
        })
    })
}

pub fn assert_model_term_mode(mode: TermMode, expected_value: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _| {
            let model = view.model.lock();
            async_assert_eq!(model.is_term_mode_set(mode), expected_value)
        })
    })
}

pub fn assert_no_block_executing(tab_index: usize, pane_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            // Note: When the user presses enter, we "start" the block and send the newline to the
            // shell, however we don't update the state of the block until the shell responds with
            // a preexec message. As a result, we need to check _both_ the state and whether the
            // block has started to ensure that we don't think a recently executed block is
            // actually waiting for a command.
            let block = model.block_list().active_block();
            let block_is_ready =
                !block.started() && matches!(block.state(), BlockState::BeforeExecution);
            async_assert!(
                block_is_ready,
                "Should not be a command active. Block output is:\n{}\n",
                block.output_with_secrets_unobfuscated()
            )
        })
    })
}

pub fn assert_alt_grid_active(
    tab_index: usize,
    pane_index: usize,
    should_be_active: bool,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            let is_alt_grid_active = model.is_alt_screen_active();
            async_assert_eq!(
                should_be_active,
                is_alt_grid_active,
                "Expected alt grid active to be {} but it was {}",
                should_be_active,
                is_alt_grid_active
            )
        })
    })
}

/// Asserts that a long running block is currently executing.
pub fn assert_long_running_block_executing_for_single_terminal_in_tab(
    assert_output_grid_active: bool,
    tab_index: usize,
) -> AssertionCallback {
    assert_long_running_block_executing(assert_output_grid_active, tab_index, 0)
}

pub fn assert_long_running_block_executing(
    assert_output_grid_active: bool,
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            let is_editor_focused = view
                .input()
                .read(app, |input, ctx| input.editor().is_focused(ctx));
            let active_block = model.block_list().active_block();
            // Note that we check the output grid is active to ensure the
            // command has actually started executing.
            async_assert!(
                !is_editor_focused
                    && (!assert_output_grid_active || active_block.is_executing())
                    && active_block.is_active_and_long_running(),
                "Check that it's a long running process/command"
            )
        })
    })
}

pub fn assert_single_terminal_in_tab_bootstrapped(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
) -> AssertionOutcome {
    assert_bootstrapping_result(
        app, window_id, tab_index, 0, true, /* expect_bootstrapped */
    )
}

pub fn assert_terminal_bootstrapped(tab_index: usize, pane_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        assert_bootstrapping_result(app, window_id, tab_index, pane_index, true)
    })
}

pub fn assert_terminal_bootstrapping(tab_index: usize, pane_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        assert_bootstrapping_result(app, window_id, tab_index, pane_index, false)
    })
}

pub fn assert_bootstrapping_result(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
    pane_index: usize,
    expect_bootstrapped: bool,
) -> AssertionOutcome {
    let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
    let bootstrapped = terminal_view.read(app, |view, ctx| {
        let model = view.model.lock();
        let input_visible = view.is_input_box_visible(&model, ctx);
        let history_bootstrapped = model
            .block_list()
            .active_block()
            .session_id()
            .is_some_and(|session_id| History::as_ref(ctx).is_session_initialized(&session_id));
        input_visible
            && history_bootstrapped

            // Note that we check whether the precmd that follows bootstrapping is done rather than
            // just checking bootstrapping is done. In tests it can cause indeterminancy to have
            // this precmd come in later (it increases the number of blocks), whereas in the actual
            // running of the app we don't care about these blocks and it's a slight performance hit
            // to wait for the precmd so we can just check is_bootstrapped.
            && model.block_list().is_bootstrapping_precmd_done()
    });

    async_assert_eq!(
        expect_bootstrapped,
        bootstrapped,
        "terminal should be bootstrapped ({})",
        expect_bootstrapped
    )
}

pub fn assert_selected_block_index_is_first_renderable() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _| {
            let selected_block_index = view
                .selected_blocks_tail_index()
                .expect("Selection should not be none");
            let model = view.model.lock();
            let block = model
                .block_list()
                .block_at(selected_block_index)
                .expect("Block should exist");
            assert!(
                block.height(&crate::terminal::model::block::TranscriptScope::Terminal)
                    != Lines::zero(),
                "The selected block should be rendered"
            );
            // Previous index either doesn't exist or isn't renderable
            if selected_block_index > BlockIndex::zero() {
                let prev_block = model.block_list().block_at(selected_block_index - 1.into());
                if let Some(prev_block) = prev_block {
                    assert!(
                        prev_block
                            .is_empty(&crate::terminal::model::block::TranscriptScope::Terminal),
                        "Prev index should be hidden"
                    );
                }
            }
            AssertionOutcome::Success
        })
    })
}

pub fn assert_selected_block_index_is_last_renderable() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _| {
            let selected_block_index = view
                .selected_blocks_tail_index()
                .expect("Selection should not be none");
            let model = view.model.lock();
            let block = model
                .block_list()
                .block_at(selected_block_index)
                .expect("Block should exist");
            assert!(
                block.height(&crate::terminal::model::block::TranscriptScope::Terminal)
                    != Lines::zero(),
                "The selected block should be rendered"
            );

            assert_eq!(
                model.block_list().last_non_hidden_block_by_index(),
                Some(selected_block_index)
            );
            AssertionOutcome::Success
        })
    })
}

pub fn assert_focused_editor_in_tab(tab_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input_view = single_input_view_for_tab(app, window_id, tab_index);
        input_view.read(app, |view, ctx| {
            async_assert!(view.editor().is_focused(ctx), "Editor should be focused")
        })
    })
}

pub fn assert_command_executed_for_single_terminal_in_tab(
    tab_index: usize,
    command: String,
) -> AssertionCallback {
    assert_command_executed(tab_index, 0, command)
}

pub fn assert_command_executed(
    tab_index: usize,
    pane_index: usize,
    command: String,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            let last_index = model
                .block_list()
                .last_matching_block_by_index(BlockFilter::commands());

            if let Some(last_index) = last_index {
                let block = model
                    .block_list()
                    .block_at(last_index)
                    .expect("block should exist");
                let block_is_done = matches!(
                    block.state(),
                    BlockState::DoneWithExecution | BlockState::DoneWithNoExecution
                );
                let last_command = block
                    .command_with_secrets_unobfuscated(false /*include_escape_sequences*/);
                // We send an escape sequence once the line editor is active to fetch
                // typeahead. Currently, this is racy in integration tests because
                // they send the queued command more quickly than a real user could
                // type. For the time being, we handle this by cleaning up the command,
                // but ongoing work to consolidate PTY writes should be a more robust
                // solution.
                let cleaned_last_command = last_command.trim_end_matches("^[i").trim_end();
                let cleaned_command = command.trim_end();

                async_assert!(
                    block_is_done && cleaned_last_command == cleaned_command,
                    "Previous command should be {}, instead got {}",
                    command,
                    last_command,
                )
            } else {
                AssertionOutcome::failure("No block yet".to_string())
            }
        })
    })
}

pub fn assert_active_block_received_precmd(
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            if active_block.has_received_precmd() {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::failure("Precmd has not been received yet".to_string())
            }
        })
    })
}

pub fn assert_active_block_input_is_empty(
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, ctx| {
            view.input().read(ctx, |input, ctx| {
                let text = input.buffer_text(ctx);
                async_assert!(
                    text.is_empty(),
                    "Input buffer is not empty after block finished. Input buffer contents: {}",
                    text
                )
            })
        })
    })
}

pub fn assert_bootstrapping_stage(
    tab_index: usize,
    pane_index: usize,
    stage: BootstrapStage,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            async_assert_eq!(active_block.bootstrap_stage(), stage)
        })
    })
}

pub fn assert_context_menu_is_open(should_be_open: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _ctx| {
            let open_or_closed_str = if should_be_open { "open" } else { "closed" };
            async_assert_eq!(
                view.is_context_menu_open(),
                should_be_open,
                "The context menu should be {open_or_closed_str}"
            )
        })
    })
}

pub fn assert_active_block_command_for_single_terminal_in_tab(
    expected_command: impl ExpectedOutput + 'static,
    tab_index: usize,
) -> AssertionCallback {
    assert_active_block_command(expected_command, tab_index, 0)
}

pub fn assert_active_block_command(
    expected_command: impl ExpectedOutput + 'static,
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _| {
            let model = view.model.lock();
            let command = model.block_list().active_block().command_to_string();
            async_assert!(
                expected_command.matches(&command),
                "The command should be {:?}, but got \"{}\"",
                expected_command,
                command
            )
        })
    })
}

pub fn assert_active_block_output_for_single_terminal_in_tab(
    expected_output: impl ExpectedOutput + 'static,
    tab_index: usize,
) -> AssertionCallback {
    assert_active_block_output(expected_output, tab_index, 0)
}

pub fn assert_active_block_output(
    expected_output: impl ExpectedOutput + 'static,
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _| {
            let model = view.model.lock();
            let output = model.block_list().active_block().output_to_string();
            async_assert!(
                expected_output.matches(&output),
                "The output should be {:?}, but got \"{}\"",
                expected_output,
                output
            )
        })
    })
}

pub fn assert_no_visible_background_blocks(
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);
        terminal_view.read(app, |view, _| {
            let model = view.model.lock();
            let count_nonempty_background_blocks = model
                .block_list()
                .blocks()
                .iter()
                .filter(|block| {
                    block.is_background()
                        && block
                            .is_visible(&crate::terminal::model::block::TranscriptScope::Terminal)
                })
                .count();
            async_assert_eq!(
                count_nonempty_background_blocks,
                0,
                "BlockList should have no non-empty background blocks."
            )
        })
    })
}

/// Asserts that the output of the alt screen matches `expected_output`.
pub fn assert_alt_screen_output(
    expected_output: impl ExpectedOutput + 'static,
    tab_index: usize,
    pane_index: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, tab_index, pane_index);

        terminal_view.read(app, |view, _| {
            let model = view.model.lock();
            let output = model.alt_screen().output_to_string();
            async_assert!(
                expected_output.matches(&output),
                "The output should be {:?}, but got \"{}\"",
                expected_output,
                output
            )
        })
    })
}

/// Builds an assertion that the input box for the given tab will contain the
/// expected text.
pub fn assert_input_editor_contents(
    tab_index: usize,
    expected_contents: impl AsRef<str> + 'static,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let expected_contents = expected_contents.as_ref();
        let input_view = single_input_view_for_tab(app, window_id, tab_index);
        input_view.read(app, |view, ctx| {
            let contents = view.buffer_text(ctx);
            async_assert_eq!(&contents, expected_contents, "Incorrect input box contents:\nExpected {expected_contents:?}\nActual: {contents:?}")
        })
    })
}

pub fn assert_pane_group_has_state(
    tab_index: usize,
    expected_state: TerminalViewState,
) -> AssertionCallback {
    Box::new(move |app, _| {
        let active_window_id = app.read(|ctx| {
            WindowManager::as_ref(ctx)
                .active_window()
                .expect("should have active window")
        });
        let views = app
            .views_of_type(active_window_id)
            .expect("Active window lacks a Workspace.");
        let workspace: &ViewHandle<Workspace> =
            views.first().expect("Window is missing Workspace view.");
        workspace.read(app, |workspace, ctx| {
            workspace
                .get_pane_group_view(tab_index)
                .expect("Workspace has no tab view.")
                .read(ctx, |pane_group, ctx| {
                    async_assert_eq!(pane_group.most_recent_pane_state(ctx), expected_state)
                })
        })
    })
}

fn assert_snackbar_visibility(tab_index: usize, is_visible: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal_view.update(app, |view, ctx| {
            let presenter = ctx.presenter(window_id).expect("window should exist");
            let snackbar_position = presenter
                .borrow()
                .position_cache()
                .get_position(format!("block_list_snackbar:{}", view.id()));

            async_assert_eq!(snackbar_position.is_some(), is_visible)
        })
    })
}

/// Asserts that the snackbar is visible.
pub fn assert_snackbar_is_visible(tab_index: usize) -> AssertionCallback {
    assert_snackbar_visibility(tab_index, true /* is_visible */)
}

/// Asserts that the snackbar is _not_ visible.
pub fn assert_snackbar_is_not_visible(tab_index: usize) -> AssertionCallback {
    assert_snackbar_visibility(tab_index, false /* is_visible */)
}

/// Asserts that the current scroll position is equal to `ScrollPosition`.
pub fn assert_scroll_position(
    tab_index: usize,
    scroll_position: ScrollPosition,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal_view.read(app, |view, _ctx| {
            let actual_scroll_position = view.scroll_position();
            async_assert_eq!(actual_scroll_position, scroll_position)
        })
    })
}

pub fn validate_git_branch(
    expected_git_branch: Option<String>,
    tab_idx: usize,
    window_id: warpui::WindowId,
    app: &warpui::App,
) -> AssertionOutcome {
    let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
    terminal_view.read(app, |view, _ctx| {
        let model = view.model.lock();
        let block = model.block_list().active_block();
        let actual_branch = block.git_branch();

        if actual_branch.map(Into::into) == expected_git_branch {
            AssertionOutcome::Success
        } else {
            AssertionOutcome::failure(format!(
                "Expected {expected_git_branch:?} as git branch but got {actual_branch:?}"
            ))
        }
    })
}

/// Asserts that the active session of the current window's workspace has the expected local path.
/// For convenience, the local path is converted to a user-friendly path, since it will generally
/// be a temporary directory.
pub fn assert_active_session_local_path(expected_path: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        ActiveSession::handle(app).read(app, |active_session, _| {
            let session = active_session.session(window_id);
            let pwd = active_session.path_if_local(window_id);
            match session.zip(pwd) {
                Some((session, pwd)) => {
                    let relative_path = user_friendly_path(
                        pwd.to_str().expect("Non-UTF8 path"),
                        session.home_dir(),
                    );
                    async_assert_eq!(expected_path, relative_path)
                }
                None => {
                    AssertionOutcome::failure("Expected a local active session path".to_string())
                }
            }
        })
    })
}

pub fn assert_input_is_focused() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |view, ctx| {
            let is_input_focused = view.input().as_ref(ctx).editor().as_ref(ctx).is_focused();
            async_assert!(is_input_focused)
        })
    })
}
