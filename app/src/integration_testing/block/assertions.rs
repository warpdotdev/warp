use settings::Setting as _;
use warpui::{
    async_assert, async_assert_eq,
    integration::{AssertionCallback, AssertionOutcome},
    units::{IntoPixels, Lines},
    AppContext, SingletonEntity, WindowId,
};

use crate::{
    integration_testing::view_getters::single_terminal_view,
    terminal::block_list_viewport::ViewportState,
};
use crate::{
    integration_testing::{
        terminal::util::ExpectedOutput, view_getters::single_terminal_view_for_tab,
    },
    terminal::view::BlockVisibilityMode,
};
use crate::{
    settings::InputModeSettings,
    terminal::{heights_approx_eq, model::terminal_model::BlockIndex, TerminalModel, TerminalView},
};

/// Specifies a block position either directly by index, or by whether it's first or
/// last
#[derive(Debug, Copy, Clone)]
pub enum BlockPosition {
    /// The block at the given index
    AtIndex(BlockIndex),

    /// The first block in the blocklist
    FirstBlock,

    /// The last block in the blocklist
    LastBlock,
}

impl BlockPosition {
    fn block_index(&self, model: &TerminalModel) -> BlockIndex {
        match *self {
            BlockPosition::AtIndex(index) => index,
            BlockPosition::FirstBlock => model
                .block_list()
                .first_non_hidden_block_by_index()
                .expect("no first block"),
            BlockPosition::LastBlock => model
                .block_list()
                .last_non_hidden_block_by_index()
                .expect("no last block"),
        }
    }
}

/// Specifies a line position either directly by number of lines, or by where it is
/// in the viewport
#[derive(Debug, Copy, Clone)]
pub enum LinePosition {
    /// At a specific scroll top in lines
    AtLines(Lines),

    /// At the scroll top of the viewport
    AtScrollTop,

    /// Directly above the input
    AtTopOfInput,
}

impl LinePosition {
    fn calculate_lines(
        &self,
        window_id: WindowId,
        view: &TerminalView,
        viewport: &ViewportState,
        ctx: &AppContext,
    ) -> Lines {
        let top_of_view_in_lines = ctx
            .element_position_by_id_at_last_frame(window_id, view.terminal_position_id())
            .expect("terminal rendered")
            .min_y()
            .into_pixels()
            .to_lines(view.size_info().cell_height_px());
        match *self {
            LinePosition::AtLines(lines) => lines,
            LinePosition::AtScrollTop => viewport.scroll_top_in_lines(),
            LinePosition::AtTopOfInput => {
                ctx.element_position_by_id_at_last_frame(
                    window_id,
                    view.input().as_ref(ctx).save_position_id(),
                )
                .expect("input rendered")
                .min_y()
                .into_pixels()
                .to_lines(view.size_info().cell_height_px())
                    + viewport.scroll_top_in_lines()
                    - top_of_view_in_lines
            }
        }
    }
}

/// Asserts there are exactly num_blocks in the model
pub fn assert_num_blocks_in_model(num_blocks: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            async_assert_eq!(
                num_blocks,
                model.block_list().blocks().len(),
                "Block list should have {} block but it has {}",
                num_blocks,
                model.block_list().blocks().len()
            )
        })
    })
}

/// Asserts a block is visible with the given position and visibility mode
pub fn assert_block_visible(
    block_position: BlockPosition,
    visibility_mode: BlockVisibilityMode,
    visible: bool,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, move |view, ctx| {
            let model = view.model.lock();
            let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
            let viewport = view.viewport_state(model.block_list(), input_mode, ctx);
            let block_index = block_position.block_index(&model);
            let is_block_visible = viewport.is_block_in_view(block_index, visibility_mode);
            async_assert_eq!(
                visible,
                is_block_visible,
                "Expected block to be visible {} at index {:?} but was {}",
                visible,
                block_index,
                is_block_visible
            )
        })
    })
}

/// Asserts the top of a block is at the given line position
pub fn assert_top_of_block_approx_at(
    block_position: BlockPosition,
    line_position: LinePosition,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, move |view, ctx| {
            let model = view.model.lock();
            let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
            let viewport = view.viewport_state(model.block_list(), input_mode, ctx);
            let block_index = block_position.block_index(&model);
            let top_of_block_in_lines = viewport.top_of_block_in_lines(block_index);
            let lines = line_position.calculate_lines(window_id, view, &viewport, ctx);
            async_assert!(
                heights_approx_eq(top_of_block_in_lines, lines),
                "Expected top of block at {:?} ({}) but was {}",
                line_position,
                lines,
                top_of_block_in_lines,
            )
        })
    })
}

/// Asserts the bottom of a block is at the given line position
pub fn assert_bottom_of_block_approx_at(
    block_position: BlockPosition,
    line_position: LinePosition,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, move |view, ctx| {
            let model = view.model.lock();
            let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
            let viewport = view.viewport_state(model.block_list(), input_mode, ctx);
            let block_index = block_position.block_index(&model);
            let bottom_of_block_in_lines = viewport.bottom_of_block_in_lines(block_index);
            let lines = line_position.calculate_lines(window_id, view, &viewport, ctx);
            async_assert!(
                heights_approx_eq(bottom_of_block_in_lines, lines),
                "Expected bottom of block {:?} ({}) but was {}",
                line_position,
                lines,
                bottom_of_block_in_lines,
            )
        })
    })
}

/// Assertion that the last background output block exists and contains the
/// expected output.
pub fn assert_background_output(
    tab_index: usize,
    expected_output: impl ExpectedOutput + 'static,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal = single_terminal_view_for_tab(app, window_id, tab_index);
        terminal.read(app, |view, _ctx| {
            let model = view.model.lock();
            let background_block = model
                .block_list()
                .blocks()
                .iter()
                .rev()
                .find(|block| block.is_background() && !block.finished());
            match background_block {
                Some(block) => {
                    let output = block.output_to_string();
                    async_assert!(
                        expected_output.matches(&output),
                        "The background output should be {expected_output:?}, but got {output:?}"
                    )
                }
                None => AssertionOutcome::failure("No active background block".into()),
            }
        })
    })
}
