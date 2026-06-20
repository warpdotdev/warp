//! This module defines HeaderGrid, a struct which manages the prompt and command grid's within
//! Warp. This struct is abstracted away from Block, for the purposes of enabling same-line prompt,
//! utilizing a combined prompt/command grid, with helper methods to expose the prompt and command.
use std::cmp::max;
use std::io;

use instant::Instant;
use pathfinder_color::ColorU;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};
use warpui::units::{IntoLines as _, Lines};

use super::ansi::{self, Attr, Handler, PrecmdValue, PreexecValue, Processor, PromptMetadata};
use super::block::{BlockGridPoint, BlockSize};
use super::blockgrid::BlockGrid;
use super::bootstrap::BootstrapStage;
use super::find::RegexDFAs;
use super::grid::grid_handler::{PerformResetGridChecks, RegexIter};
use super::grid::{Cursor, Dimensions as _, RespectDisplayedOutput};
use super::index::{Point, VisibleRow};
use super::selection::ScrollDelta;
use super::{ObfuscateSecrets, RespectObfuscatedSecrets};
use crate::terminal::event::Event;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::SizeInfo;

macro_rules! delegate {
    ($self:ident.$method:ident( $( $arg:expr ),* )) => {
        match $self.receiving_chars_for_prompt {
            Some(ansi::PromptKind::Initial) => {
                let mut retval = None;
                if !$self.ignore_next_prompt_preview {
                    // Used for prompt preview (select PS1 modal) with combined grid,
                    // or used for all prompt content (pre-same line prompt).
                    retval = Some($self.prompt_grid.$method($( $arg ),*));
                }
                if $self.honor_ps1 {
                    retval = Some($self.prompt_and_command_grid.$method($( $arg ),*));
                }

                retval.unwrap_or_default()
            },
            _ => {
                $self.prompt_and_command_grid.$method($( $arg ),*)
            }
        }
    };
}

/// Any methods which write responses back to the shell process cannot have double delegation, since
/// that would result in extra responses being sent back to the shell.
macro_rules! delegate_with_writer {
    ($self:ident.$method:ident( $( $arg:expr ),* )) => {
        match $self.receiving_chars_for_prompt {
            Some(ansi::PromptKind::Initial) => {
                if $self.honor_ps1 {
                    $self.prompt_and_command_grid.$method($( $arg ),*);
                } else if !$self.ignore_next_prompt_preview {
                    // Used for prompt preview (select PS1 modal) with combined grid,
                    // or used for all prompt content (pre-same line prompt).
                    $self.prompt_grid.$method($( $arg ),*);
                }
            },
            _ => {
                $self.prompt_and_command_grid.$method($( $arg ),*);
            }
        }
    };
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(super) enum CommandStartPoint {
    /// NOTE: this should ideally have a `has_extra_leading_newline` field which tracks a leading newline in the case of a
    /// hard-wrap overwriting a soft-wrap (which isn't captured in the "Point" data). However, we don't currently use
    /// the # of rows of command content (from combined grid) anywhere. And for all other purposes, removing the leading
    /// newline in that instance makes sense (product-wise) e.g. copying command content. Hence, we treat this as a known
    /// limitation of these markers.
    /// This field is difficult to implement since you need to "peek ahead" to the first character of the command to see
    /// if it has a newline that would overwrite a prior soft-wrap.
    /// Concrete example: Grid is only 3 columns. Prompt is "foo" and command is "\nbar". We get
    /// foo
    /// bar
    /// The command start point is (1, 0), however the command's contents technically have a leading newline. This example
    /// illustrates a hard-wrap "overwriting" a soft-wrap.
    CommandStart {
        /// Note: this is INCLUSIVE of the printable character at the Point.
        point: Point,
    },
    /// Stale indicates the cached marker is no longer valid (e.g. due to a prompt being overwritten).
    Stale,
}

/// Cached version of the underlying end of prompt marker stored as a `CellExtra`. We cache this to prevent re-scanning the Grid
/// whenever we need to find the demarcation between the prompt/command. We update this cached value when the prompt
/// is being written to or when we have a resize event.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(super) enum PromptEndPoint {
    EmptyPrompt,
    PromptEnd {
        /// Note: this is INCLUSIVE of the printable character at the Point.
        point: Point,
        /// Used to indicate whether the prompt has a trailing newline that isn't covered by the marker (since the marker
        /// Point only covers printable characters).
        has_extra_trailing_newline: bool,
    },
    /// Stale indicates the cached marker is no longer valid (e.g. due to a prompt being overwritten).
    Stale,
}

pub struct HeaderGrid {
    prompt_grid: BlockGrid,
    prompt_and_command_grid: BlockGrid,
    /// If Some, received characters should be directed to the specified prompt
    /// grid instead of the active grid.
    pub(super) receiving_chars_for_prompt: Option<ansi::PromptKind>,
    /// If true, we should discard the next left prompt data we receive
    /// (whether it comes from a precmd hook or from a marked prompt
    /// printed by the shell). Note that we only ignore it for the prompt grid (we do NOT ignore it for the
    /// combined grid, which needs to get the updated prompt bytes from the shell, to support in-band generators
    /// with remote subshells correctly).
    /// TODO(CORE-2403): Rename this field to should_populate_prompt_preview_grid.
    ignore_next_prompt_preview: bool,
    /// The height of the Warp prompt in lines (non-PS1).
    warp_prompt_height_lines: f32,
    // whether to honor users ps1 and rprompt values, can be changed by a user in the settings
    // note that the change will only apply to the active block; historical blocks will keep the
    // previous setting
    pub(super) honor_ps1: bool,
    command_started: bool,
    command_start_time: Option<Instant>,
    /// Cached Point at which the prompt content ends,
    /// Use prompt_end_point() to access this value safely, do not use the raw value.
    cached_prompt_end_point: Option<PromptEndPoint>,
    /// Cached Point at which the command content starts.
    /// Use command_start_point() to access this value safely, do not use the raw value.
    cached_command_start_point: Option<CommandStartPoint>,

    event_proxy: ChannelEventListener,
}

impl HeaderGrid {
    pub fn new(
        sizes: BlockSize,
        event_proxy: ChannelEventListener,
        should_scan_for_secrets: ObfuscateSecrets,
        honor_ps1: bool,
        perform_reset_grid_checks: PerformResetGridChecks,
    ) -> Self {
        let prompt_grid = BlockGrid::new(
            sizes.size,
            // Even though prompt is most likely only 1-2 lines, we allow for the bigger
            // max_scroll_limit, to account for resizing the window/pane.
            sizes.max_block_scroll_limit,
            event_proxy.clone(),
            should_scan_for_secrets,
            // We ignore checking if we've received the Reset Grid OSC on the prompt grid.
            PerformResetGridChecks::No,
        );
        let prompt_and_command_grid = BlockGrid::new(
            sizes.size,
            sizes.max_block_scroll_limit,
            event_proxy.clone(),
            should_scan_for_secrets,
            perform_reset_grid_checks,
        );
        Self {
            prompt_grid,
            prompt_and_command_grid,
            receiving_chars_for_prompt: None,
            ignore_next_prompt_preview: false,
            warp_prompt_height_lines: sizes.warp_prompt_height_lines,
            honor_ps1,
            command_started: false,
            command_start_time: None,
            cached_prompt_end_point: None,
            cached_command_start_point: None,
            event_proxy,
        }
    }

    pub fn honor_ps1(&self) -> bool {
        self.honor_ps1
    }

    pub fn set_honor_ps1(&mut self, honor_ps1: bool) {
        self.honor_ps1 = honor_ps1;
        if !self.honor_ps1 {
            // If we are switching to Warp prompt (from PS1), we need to clear the cached prompt end point
            // and update the command start point appropriately!
            self.cached_prompt_end_point = Some(PromptEndPoint::EmptyPrompt);
            self.cached_command_start_point = Some(CommandStartPoint::CommandStart {
                point: Point::new(0, 0),
            });
        }
    }

    pub fn set_supports_emoji_presentation_selector(
        &mut self,
        supports_emoji_presentation_selector: bool,
    ) {
        self.prompt_grid
            .grid_handler
            .set_supports_emoji_presentation_selector(supports_emoji_presentation_selector);
        self.prompt_and_command_grid
            .grid_handler
            .set_supports_emoji_presentation_selector(supports_emoji_presentation_selector);
    }

    #[cfg(test)]
    pub(super) fn set_prompt_grid(&mut self, grid: BlockGrid) {
        self.prompt_grid = grid;
    }

    #[cfg(test)]
    pub(super) fn set_prompt_and_command_grid(&mut self, grid: BlockGrid) {
        self.prompt_and_command_grid = grid;
    }

    #[cfg(test)]
    pub(super) fn set_raw_prompt_end_point(&mut self, point: Option<PromptEndPoint>) {
        self.cached_prompt_end_point = point;
    }

    fn command_to_string_internal(
        &self,
        include_esc_sequences: bool,
        max_rows: Option<usize>,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
        force_obfuscated_secrets: bool,
    ) -> String {
        if self.prompt_and_command_grid.is_empty() {
            return "".to_owned();
        }

        let Some(CommandStartPoint::CommandStart {
            point: command_start_point,
        }) = self.command_start_point()
        else {
            return "".to_owned();
        };

        // Calculate the row start bound similarly to the `BlockGrid::to_string` function (from the bottom).
        let row_start_bound = match max_rows {
            None => 0,
            Some(max) => (self.prompt_and_command_grid.len() - 1).saturating_sub(max),
        };
        let start_point_bound = Point::new(row_start_bound, 0);
        // We should not go above the command start point, since we don't want to enter prompt territory.
        let start_point = command_start_point.max_point(
            &start_point_bound,
            self.prompt_and_command_grid.grid_handler().columns(),
        );
        self.prompt_and_command_grid.grid_handler.bounds_to_string(
            *start_point,
            self.prompt_and_command_grid.end_point(),
            include_esc_sequences,
            respect_obfuscated_secrets,
            force_obfuscated_secrets,
            RespectDisplayedOutput::Yes,
        )
    }

    pub fn command_to_string(&self) -> String {
        self.command_to_string_internal(
            false, /* include_esc_sequences */
            None,  /* max_rows */
            RespectObfuscatedSecrets::Yes,
            false, /* force_obfuscated_secrets */
        )
    }

    pub fn command_with_secrets_obfuscated(&self, include_escape_sequences: bool) -> String {
        self.command_to_string_internal(
            include_escape_sequences,
            None, /* max_rows */
            RespectObfuscatedSecrets::No,
            true, /* force_obfuscated_secrets */
        )
    }

    pub fn command_with_secrets_unobfuscated(&self, include_escape_sequences: bool) -> String {
        self.command_to_string_internal(
            include_escape_sequences,
            None, /* max_rows */
            RespectObfuscatedSecrets::No,
            false, /* force_obfuscated_secrets */
        )
    }

    pub fn command_should_show_as_empty_when_finished(&self) -> bool {
        match self.command_start_point() {
            Some(CommandStartPoint::CommandStart {
                point: command_start_point,
            }) => {
                // If the cursor is still at the "start" of the command, there is no command content,
                // assuming we truncate the grid (when we "finish" it).
                self.prompt_and_command_grid.grid_handler().cursor_point() == command_start_point
            }
            // Cannot make assertions about the "finished state" of the command if in stale state. Specifically,
            // the prompt is being reprinted, which means we don't have the command contents reflowed correctly yet.
            Some(CommandStartPoint::Stale) => false,
            None => true,
        }
    }

    pub fn prompt_and_command_number_of_rows(&self) -> usize {
        self.prompt_and_command_grid.len()
    }

    /// Checks if the command is marked as finished AND it is empty (note that we truncate
    /// finished commands to their cursor point). This differs from below since it requires
    /// the command to be finished.
    fn is_command_finished_and_empty(&self) -> bool {
        if !self.honor_ps1 {
            // If we are using Warp prompt, we expect the combined grid cursor to be at the start, if
            // the command is truly empty.
            return self.prompt_and_command_grid.finished()
                && self.prompt_and_command_grid.grid_handler().cursor_point() == Point::new(0, 0);
        }
        // Otherwise, we need to compare the end of the command content to the end of the prompt content,
        // to determine if the command is empty.
        self.prompt_and_command_grid.finished()
            && self.prompt_grid.grid_handler().cursor_point()
                == self.prompt_and_command_grid.grid_handler().cursor_point()
    }

    pub fn is_command_empty(&self) -> bool {
        // The only 2 possible cases where we consider the command to be "empty" are:
        // 1. Command has not been "started" yet.
        // 2. Command has been "finished" and is empty.
        // If the command is currently being written, it is NOT empty.
        !self.command_started || self.is_command_finished_and_empty()
    }

    pub fn prompt_grid(&self) -> &BlockGrid {
        &self.prompt_grid
    }

    /// Returns the cached start of the command marker, in the case of a combined command/prompt grid.
    pub(super) fn command_start_point(&self) -> Option<CommandStartPoint> {
        self.cached_command_start_point
    }

    /// Returns the cached end of the prompt marker, in the case of a combined command/prompt grid.
    pub(super) fn prompt_end_point(&self) -> Option<PromptEndPoint> {
        self.cached_prompt_end_point
    }

    fn prompt_to_string_internal(
        &self,
        include_esc_sequences: bool,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
        force_obfuscated_secrets: bool,
    ) -> String {
        // If prompt is not finished, we can use the entire grid (till the end).
        let prompt_end_point = match self.prompt_end_point() {
            Some(PromptEndPoint::EmptyPrompt) => return "".to_owned(),
            // NOTE: we explicitly choose NOT to use `has_trailing_newline` in this context since we believe
            // that the trailing newline should NOT be included in the "string version" of the prompt, from a
            // product POV. We only use the trailing newline to count the # of rows of prompt content (used
            // in context of rprompt offset).
            Some(PromptEndPoint::PromptEnd {
                point: prompt_end_point,
                ..
            }) => prompt_end_point,
            // We consider the prompt to be empty if it's being reprinted.
            Some(PromptEndPoint::Stale) => return "".to_owned(),
            None => self.prompt_and_command_grid.end_point(),
        };
        self.prompt_and_command_grid.grid_handler.bounds_to_string(
            self.prompt_and_command_grid.start_point(),
            prompt_end_point,
            include_esc_sequences,
            respect_obfuscated_secrets,
            force_obfuscated_secrets,
            RespectDisplayedOutput::Yes,
        )
    }

    pub fn prompt_contents_to_string(&self, include_escape_sequences: bool) -> String {
        self.prompt_to_string_internal(
            include_escape_sequences,
            RespectObfuscatedSecrets::Yes,
            false, /* force_obfuscated_secrets */
        )
    }

    pub fn prompt_with_secrets_obfuscated(&self, include_escape_sequences: bool) -> String {
        self.prompt_to_string_internal(
            include_escape_sequences,
            RespectObfuscatedSecrets::No,
            true, /* force_obfuscated_secrets */
        )
    }

    pub fn prompt_with_secrets_unobfuscated(&self, include_escape_sequences: bool) -> String {
        self.prompt_to_string_internal(
            include_escape_sequences,
            RespectObfuscatedSecrets::No,
            false, /* force_obfuscated_secrets */
        )
    }

    pub fn prompt_and_command_with_secrets_obfuscated(
        &self,
        include_escape_sequences: bool,
    ) -> String {
        self.prompt_and_command_grid.to_string(
            include_escape_sequences,
            None,
            RespectObfuscatedSecrets::No,
            true, /* force_obfuscated_secrets */
            RespectDisplayedOutput::Yes,
        )
    }

    pub fn prompt_and_command_with_secrets_unobfuscated(
        &self,
        include_escape_sequences: bool,
    ) -> String {
        self.prompt_and_command_grid.to_string(
            include_escape_sequences,
            None,
            RespectObfuscatedSecrets::No,
            false, /* force_obfuscated_secrets */
            RespectDisplayedOutput::Yes,
        )
    }

    pub fn prompt_number_of_rows(&self) -> usize {
        if !self.prompt_and_command_grid.started() {
            return 0;
        }
        match self.prompt_end_point() {
            Some(PromptEndPoint::EmptyPrompt) => 0,
            // We add 1 since row indices are zero-indexed.
            // We include the trailing newline for this calculation of # of rows for
            // prompt content (can be used downstream for instances such as rprompt
            // offset calculations).
            Some(PromptEndPoint::PromptEnd {
                point: prompt_end_point,
                has_extra_trailing_newline,
            }) => prompt_end_point.row + 1 + has_extra_trailing_newline as usize,
            // If the prompt is being reprinted, we can use the current cursor.
            Some(PromptEndPoint::Stale) => {
                self.prompt_and_command_grid
                    .grid_handler()
                    .cursor_point()
                    .row
                    + 1
            }
            // The command hasn't started yet, so we simply have the prompt in combined grid.
            None => self.prompt_and_command_grid.len(),
        }
    }

    pub fn is_prompt_empty(&self) -> bool {
        // Prompt is empty if the combined grid is entirely empty, or if the command starts at (0, 0) (meaning no prompt content before
        // then).
        self.prompt_and_command_grid.is_empty()
            || self.prompt_end_point() == Some(PromptEndPoint::EmptyPrompt)
    }

    pub fn prompt_rightmost_visible_nonempty_cell(&self) -> Option<usize> {
        match self.prompt_end_point() {
            Some(PromptEndPoint::PromptEnd {
                point: prompt_end_point,
                has_extra_trailing_newline,
            }) => {
                // We compare the rightmost cell from the n-1 lines of the prompt to the rightmost cell in the nth line of the prompt
                // and return the maximum. Note that the nth line of the prompt can include the start of the command, hence we need
                // to consider it separately.
                let second_last_line_prompt = if !has_extra_trailing_newline {
                    prompt_end_point.row.saturating_sub(1)
                } else {
                    prompt_end_point.row
                };
                let upper_rows_prompt_rightmost = self
                    .prompt_and_command_grid
                    .rightmost_visible_nonempty_cell_with_max_row(second_last_line_prompt);
                max(upper_rows_prompt_rightmost, Some(prompt_end_point.col))
            }
            // No nonempty cell in the prompt.
            Some(PromptEndPoint::EmptyPrompt) => None,
            // Cannot make assertions about the "rightmost nonempty prompt cell" if in stale state.
            Some(PromptEndPoint::Stale) => None,
            // If the command has not started yet, we can simply return the overall rightmost visible nonempty cell.
            None => self
                .prompt_and_command_grid
                .rightmost_visible_nonempty_cell(),
        }
    }

    pub fn prompt_grid_columns(&self) -> usize {
        self.prompt_and_command_grid.grid_handler().columns()
    }

    pub fn prompt_grid_cell_height(&self) -> usize {
        self.prompt_and_command_grid.grid_handler.cell_height()
    }

    pub fn prompt_and_command_grid(&self) -> &BlockGrid {
        &self.prompt_and_command_grid
    }

    pub fn prompt_and_command_grid_mut(&mut self) -> &mut BlockGrid {
        &mut self.prompt_and_command_grid
    }

    pub fn prompt_grid_mut(&mut self) -> &mut BlockGrid {
        &mut self.prompt_grid
    }

    // TODO(advait): remove once we remove GridType::Prompt.
    pub fn set_saved_cursor_for_prompt(&mut self, cursor: Cursor) {
        self.prompt_grid.grid_storage_mut().saved_cursor = cursor;
    }

    pub fn set_saved_cursor_for_prompt_and_command(&mut self, cursor: Cursor) {
        self.prompt_and_command_grid.grid_storage_mut().saved_cursor = cursor;
    }

    /// Returns the number of columns occupied by the last line of the lprompt (including command characters
    /// as relevant). This is used for space calculations with the rprompt, which lives on the same line.
    pub fn lprompt_last_line_width_cols(&self) -> usize {
        let lprompt_last_row_index = match self.prompt_end_point() {
            Some(PromptEndPoint::PromptEnd {
                point,
                has_extra_trailing_newline,
            }) => {
                if has_extra_trailing_newline {
                    point.row + 1
                } else {
                    point.row
                }
            }
            Some(PromptEndPoint::EmptyPrompt) => 0,
            // If the prompt is being reprinted, then we can simply get the row of the cursor position as the last line of the lprompt.
            Some(PromptEndPoint::Stale) => {
                self.prompt_and_command_grid
                    .grid_handler()
                    .cursor_point()
                    .row
            }
            // If the prompt hasn't finished yet, we can simply get the row of the cursor position as the last line of the lprompt.
            None => {
                self.prompt_and_command_grid
                    .grid_handler()
                    .cursor_point()
                    .row
            }
        };
        // Find the rightmost visible nonempty cell in the last line of the prompt. We use this for rprompt space calculations.
        self.prompt_and_command_grid
            .grid_handler()
            .rightmost_visible_nonempty_cell_in_row(lprompt_last_row_index)
            .unwrap_or(0)
    }

    pub fn command_to_string_with_max_rows(&self, max_rows: Option<usize>) -> String {
        self.command_to_string_internal(false, max_rows, RespectObfuscatedSecrets::Yes, false)
    }

    pub fn prompt_start_blockgrid_point(&self) -> BlockGridPoint {
        BlockGridPoint::PromptAndCommand(self.prompt_and_command_grid.start_point())
    }

    /// Marks the EndOfPromptMarker at the prior cursor position. Also caches the "end of prompt" and "start of command" markers.
    /// The cursor position is generally expected to be at the command start point, as this function is called when the end prompt marker
    /// is received. The notable exception is when the prompt ends exactly at the end of the grid (`input_needs_wrap` is true), in which case
    /// we do NOT need to go back 1 cell to find the end of the prompt.
    fn mark_and_cache_end_of_prompt(&mut self) {
        // NOTE: we specifically mark the end of the prompt and NOT the start of the command since it's more resilient. For example,
        // p10k's transient prompt feature will issue a clear_line command which will destroy the marker in the "start of command" cell
        // since it does this after overwriting the prompt.
        let cursor_point = self.prompt_and_command_grid.grid_handler().cursor_point();
        // Special case: we need to differentiate between an empty prompt and a prompt with a single char i.e. " ".
        if cursor_point == Point::new(0, 0) {
            self.cached_prompt_end_point = Some(PromptEndPoint::EmptyPrompt);
            self.cached_command_start_point = Some(CommandStartPoint::CommandStart {
                point: Point::new(0, 0),
            });
            return;
        }
        let cursor = self.prompt_and_command_grid.grid_storage().cursor();
        let cols = self.prompt_and_command_grid.grid_handler().columns();
        let prompt_end_point = if cursor.input_needs_wrap {
            // If the cursor is at the end of the grid and needs wrapping, we haven't advanced it yet! We don't need to subtract
            // 1 cell here!
            cursor_point
        } else {
            // Otherwise, we go back 1 cell from the current cursor position to find the last printable character in the prompt.
            cursor_point.wrapping_sub(cols, 1)
        };

        let mut has_trailing_newline = false;
        // We track whether the prompt has a trailing newline since it may be required for calculations such as # of rows in the prompt.
        // Note that we DO NOT move the `Point` since we DO NOT want to include any printable "command" characters.
        if cursor_point.col == 0 && cursor_point.row > 0 {
            has_trailing_newline = !self
                .prompt_and_command_grid
                .grid_handler()
                .row_wraps(cursor_point.row - 1);
        }
        // Mark the cell we consider to be the "end of the prompt". We can track this cell through resizes.
        self.prompt_and_command_grid
            .grid_handler_mut()
            .mark_end_of_prompt(prompt_end_point, has_trailing_newline);

        // Update the cached values for demarcation of prompt/command.
        self.cached_prompt_end_point = Some(PromptEndPoint::PromptEnd {
            point: prompt_end_point,
            has_extra_trailing_newline: has_trailing_newline,
        });
        self.cached_command_start_point = Some(CommandStartPoint::CommandStart {
            point: cursor_point,
        });
    }

    pub fn start_command_grid(&mut self) {
        self.command_started = true;
        self.command_start_time = Some(Instant::now());

        // This applies in the case of shell output from user rcfiles i.e. before
        // the first prompt/user-inputted command. In this case, we do not
        // receive prompt markers, hence, we need to start the combined grid
        // manually here!
        if !self.prompt_and_command_grid.started() {
            self.prompt_and_command_grid.start();
        }
        self.prompt_and_command_grid.terminal_attribute(Attr::Bold);
        // Note that we still need to mark demarcation here, even though we do it when receiving the end prompt marker since
        // we DO NOT receive an end prompt marker when restoring blocks.
        // We do NOT want to overwrite the end of prompt marker if it's already been set (specifically, in the case
        // of input-matching typeahead, where the command has already been populated into the combined grid).
        if self.cached_prompt_end_point.is_none() {
            self.mark_and_cache_end_of_prompt();
        }
    }

    pub fn init_command(&mut self, command: impl AsRef<[u8]>) {
        let mut processor = Processor::new();
        // If we haven't marked the end of the prompt yet (possible in the case of input-matching
        // typeahead, in older shells), we do so now.
        if self.cached_prompt_end_point.is_none() {
            self.mark_and_cache_end_of_prompt();
        }
        processor.parse_bytes(
            &mut self.prompt_and_command_grid,
            command.as_ref(),
            &mut io::sink(),
        );

        // If the given command didn't include a newline, make sure to add one here.
        if self.command_should_show_as_empty_when_finished() {
            self.linefeed();
        }
    }

    pub fn is_command_finished(&self) -> bool {
        self.prompt_and_command_grid.finished()
    }

    pub fn finish_command_grid(&mut self) {
        // If the command grid is finished, the prompt grid necessarily should
        // also be finished.
        self.prompt_grid.finish();
        self.prompt_and_command_grid.finish();
    }

    pub fn finish_command(&mut self, bootstrap_stage: BootstrapStage) {
        // We need to invoke linefeed directly to get an empty block to be visible in two
        // circumstances:
        // 1. Newer versions of bash don't echo the `\n` character when executing an empty command
        // To ensure that empty blocks are still rendered, we should push that character manually
        // 2. When we restore blocks, there is no shell yet to echo a linefeed, so we need to
        // do so manually.
        if !self.is_command_finished()
            && self.command_should_show_as_empty_when_finished()
            && bootstrap_stage.is_empty_block_allowed()
        {
            // Need to force a linefeed and skip the prompt check!
            self.linefeed();
        }

        self.finish_command_grid();
    }

    pub fn command_needs_bracketed_paste(&self) -> bool {
        self.prompt_and_command_grid.needs_bracketed_paste()
    }

    pub fn resize(&mut self, size: SizeInfo) {
        self.prompt_grid.resize(size);
        self.prompt_and_command_grid.resize(size);

        match self.prompt_end_point() {
            Some(PromptEndPoint::PromptEnd {
                has_extra_trailing_newline: old_has_trailing_newline,
                ..
            }) => match self
                .prompt_and_command_grid()
                .grid_handler()
                .prompt_end_point()
            {
                // We cache the newly computed position of our "end of prompt" marker, post-resize.
                Some(new_prompt_end_point) => {
                    self.cached_prompt_end_point = Some(PromptEndPoint::PromptEnd {
                        point: new_prompt_end_point,
                        has_extra_trailing_newline: old_has_trailing_newline,
                    });
                    self.cached_command_start_point = Some(CommandStartPoint::CommandStart {
                        point: new_prompt_end_point
                            .wrapping_add(self.prompt_and_command_grid.grid_handler().columns(), 1),
                    });
                }
                None => {
                    log::warn!(
                        "Prompt end point should not be None after resize, if Some previously!"
                    );
                    // TODO(CORE-2241): Root-cause why we're ever reaching this code block.
                    self.cached_prompt_end_point = None;
                    // To be on the cautious side: reset the command start point to the start too!
                    self.cached_command_start_point = Some(CommandStartPoint::CommandStart {
                        point: Point::new(0, 0),
                    });
                }
            },
            Some(PromptEndPoint::EmptyPrompt) | Some(PromptEndPoint::Stale) | None => {}
        }
    }

    pub fn set_prompt_from_cached_data(&mut self, prompt_grid: BlockGrid) {
        self.prompt_grid = prompt_grid.clone();
        // Note that the combined prompt/command grid still receives the UPDATED prompt, not the CACHED prompt.
        self.ignore_next_prompt_preview = true;
    }

    /// Used for copying over content from background output blocks to the command content for the active block,
    /// specifically for typeahead purposes (when the user has entered in a command early + pressed ENTER early,
    /// which executes the command).
    pub fn clone_command_from_blockgrid(&mut self, other: &BlockGrid) {
        // Need to mark the fact that the command content has been started! We use this for empty checks!
        self.command_started = true;

        // If the combined grid (which we are copying content INTO) is finished, then we cannot
        // copy content into it!
        if self.prompt_and_command_grid.finished() {
            return;
        }

        // Make sure to mark the end of the prompt before copying over the command content.
        self.mark_and_cache_end_of_prompt();

        self.prompt_and_command_grid
            .grid_handler_mut()
            .append_cells_from_grid(other.grid_handler());

        self.prompt_and_command_grid.finish();
    }

    pub fn command_started(&self) -> bool {
        self.command_started
    }

    pub fn command_start_time(&self) -> Option<Instant> {
        self.command_start_time
    }

    pub fn num_secrets_obfuscated(&self) -> usize {
        self.prompt_and_command_grid.num_secrets_obfuscated()
    }

    pub fn prompt_and_command_find<'a>(&'a self, dfas: &'a RegexDFAs) -> RegexIter<'a> {
        self.prompt_and_command_grid.find(dfas)
    }

    pub fn prompt_has_received_content(&self) -> bool {
        self.prompt_grid.has_received_content()
    }

    pub fn prompt_height(&self) -> Lines {
        // The logic below applies to both the legacy and SLP (same line prompt) cases since we write the prompt
        // to both the prompt grid AND the combined grid in the SLP case.
        if self.is_command_empty() {
            // Example cases: background output blocks or blocks with completely truncated command that we want
            // to "skip" rendering. Note that user-entered "empty" blocks are still rendered (the cursor moves with linefeed).
            Lines::zero()
        } else if self.honor_ps1 {
            self.prompt_grid.len().into_lines()
        } else {
            self.warp_prompt_height_lines.into_lines()
        }
    }

    pub fn prompt_and_command_height(&self) -> Lines {
        if self.is_command_empty() {
            Lines::zero()
        } else {
            self.prompt_and_command_grid.len().into_lines()
        }
    }

    pub fn ignore_next_prompt_preview(&self) -> bool {
        self.ignore_next_prompt_preview
    }

    #[cfg(feature = "integration_tests")]
    pub fn prompt_to_string(&self) -> String {
        self.prompt_to_string_internal(false, RespectObfuscatedSecrets::Yes, false)
    }

    pub(super) fn disable_reset_grid_checks(&mut self) {
        self.prompt_grid.disable_reset_grid_checks();
        self.prompt_and_command_grid.disable_reset_grid_checks();
    }
}

// Utilities used in tests for HeaderGrid.
#[cfg(test)]
impl HeaderGrid {
    pub fn command_cursor_flags(&self) -> &super::cell::Flags {
        self.prompt_and_command_grid
            .grid_storage()
            .cursor()
            .template
            .flags()
    }

    pub fn command_grid_linefeed(&mut self) {
        self.prompt_and_command_grid.linefeed();
        self.prompt_and_command_grid
            .on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    }
}

impl ansi::Handler for HeaderGrid {
    fn set_title(&mut self, _: Option<String>) {
        log::error!("Handler method HeaderGrid::set_title should never be called. This should be handled by TerminalModel.");
    }

    fn set_cursor_style(&mut self, style: Option<ansi::CursorStyle>) {
        delegate!(self.set_cursor_style(style));
    }

    fn set_cursor_shape(&mut self, shape: ansi::CursorShape) {
        delegate!(self.set_cursor_shape(shape));
    }

    fn input(&mut self, c: char) {
        delegate!(self.input(c));
    }

    fn goto(&mut self, row: VisibleRow, column: usize) {
        delegate!(self.goto(row, column));
    }

    fn goto_line(&mut self, row: VisibleRow) {
        delegate!(self.goto_line(row));
    }

    fn goto_col(&mut self, column: usize) {
        delegate!(self.goto_col(column));
    }

    fn insert_blank(&mut self, count: usize) {
        delegate!(self.insert_blank(count));
    }

    fn move_up(&mut self, rows: usize) {
        delegate!(self.move_up(rows));
    }

    fn move_down(&mut self, rows: usize) {
        delegate!(self.move_down(rows));
    }

    fn identify_terminal<W: std::io::Write>(&mut self, writer: &mut W, intermediate: Option<char>) {
        delegate_with_writer!(self.identify_terminal(writer, intermediate));
    }

    fn report_xtversion<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate_with_writer!(self.report_xtversion(writer));
    }

    fn device_status<W: std::io::Write>(&mut self, writer: &mut W, arg: usize) {
        delegate_with_writer!(self.device_status(writer, arg));
    }

    fn move_forward(&mut self, columns: usize) {
        delegate!(self.move_forward(columns));
    }

    fn move_backward(&mut self, columns: usize) {
        delegate!(self.move_backward(columns));
    }

    fn move_down_and_cr(&mut self, rows: usize) {
        delegate!(self.move_down_and_cr(rows));
    }

    fn move_up_and_cr(&mut self, rows: usize) {
        delegate!(self.move_up_and_cr(rows));
    }

    fn put_tab(&mut self, count: u16) {
        delegate!(self.put_tab(count));
    }

    fn backspace(&mut self) {
        delegate!(self.backspace());
    }

    fn carriage_return(&mut self) {
        delegate!(self.carriage_return());
    }

    fn linefeed(&mut self) -> ScrollDelta {
        delegate!(self.linefeed())
    }

    fn bell(&mut self) {
        delegate!(self.bell());
    }

    fn substitute(&mut self) {
        delegate!(self.substitute());
    }

    fn newline(&mut self) {
        delegate!(self.newline());
    }

    fn set_horizontal_tabstop(&mut self) {
        delegate!(self.set_horizontal_tabstop());
    }

    fn scroll_up(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.scroll_up(rows))
    }

    fn scroll_down(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.scroll_down(rows))
    }

    fn insert_blank_lines(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.insert_blank_lines(rows))
    }

    fn delete_lines(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.delete_lines(rows))
    }

    fn erase_chars(&mut self, count: usize) {
        delegate!(self.erase_chars(count));
    }

    fn delete_chars(&mut self, count: usize) {
        delegate!(self.delete_chars(count));
    }

    fn move_backward_tabs(&mut self, count: u16) {
        delegate!(self.move_backward_tabs(count));
    }

    fn move_forward_tabs(&mut self, count: u16) {
        delegate!(self.move_forward_tabs(count));
    }

    fn save_cursor_position(&mut self) {
        delegate!(self.save_cursor_position());
    }

    fn restore_cursor_position(&mut self) {
        delegate!(self.restore_cursor_position());
    }

    fn clear_line(&mut self, mode: ansi::LineClearMode) {
        delegate!(self.clear_line(mode));
    }

    fn clear_screen(&mut self, mode: ansi::ClearMode) {
        delegate!(self.clear_screen(mode));
    }

    fn clear_tabs(&mut self, mode: ansi::TabulationClearMode) {
        delegate!(self.clear_tabs(mode));
    }

    fn reset_state(&mut self) {
        delegate!(self.reset_state());
    }

    fn reverse_index(&mut self) -> ScrollDelta {
        delegate!(self.reverse_index())
    }

    fn terminal_attribute(&mut self, attribute: ansi::Attr) {
        delegate!(self.terminal_attribute(attribute));
        // We ALWAYS want to bold the command, so we need to re-send the bold attribute to the grid if there's
        // a RESET attribute sent by the shell (which wipes the BOLD attribute).
        if self.command_started && !self.is_command_finished() && attribute == Attr::Reset {
            delegate!(self.terminal_attribute(Attr::Bold));
        }
    }

    fn set_mode(&mut self, mode: ansi::Mode) {
        delegate!(self.set_mode(mode));
    }

    fn unset_mode(&mut self, mode: ansi::Mode) {
        delegate!(self.unset_mode(mode));
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        delegate!(self.set_scrolling_region(top, bottom));
    }

    fn set_keypad_application_mode(&mut self) {
        delegate!(self.set_keypad_application_mode());
    }

    fn unset_keypad_application_mode(&mut self) {
        delegate!(self.unset_keypad_application_mode());
    }

    fn set_active_charset(&mut self, index: ansi::CharsetIndex) {
        delegate!(self.set_active_charset(index));
    }

    fn configure_charset(&mut self, index: ansi::CharsetIndex, charset: ansi::StandardCharset) {
        delegate!(self.configure_charset(index, charset));
    }

    fn set_color(&mut self, index: usize, color: ColorU) {
        delegate!(self.set_color(index, color));
    }

    fn dynamic_color_sequence<W: std::io::Write>(
        &mut self,
        writer: &mut W,
        code: u8,
        index: usize,
        terminator: &str,
    ) {
        delegate_with_writer!(self.dynamic_color_sequence(writer, code, index, terminator));
    }

    fn reset_color(&mut self, index: usize) {
        delegate!(self.reset_color(index));
    }

    fn clipboard_store(&mut self, clipboard: u8, data: &[u8]) {
        delegate!(self.clipboard_store(clipboard, data));
    }

    fn clipboard_load(&mut self, clipboard: u8, terminator: &str) {
        delegate!(self.clipboard_load(clipboard, terminator));
    }

    fn decaln(&mut self) {
        delegate!(self.decaln());
    }

    fn push_title(&mut self) {
        log::error!("Handler method HeaderGrid::push_title should never be called. This should be handled by TerminalModel.");
    }

    fn pop_title(&mut self) {
        log::error!("Handler method HeaderGrid::pop_title should never be called. This should be handled by TerminalModel.");
    }

    fn prompt_marker(&mut self, marker: ansi::PromptMarker) {
        match marker {
            ansi::PromptMarker::StartPrompt { kind } => {
                match kind {
                    ansi::PromptKind::Initial => {
                        log::debug!("Received start prompt marker for initial prompt");
                        self.prompt_and_command_grid.reset_state();
                        self.prompt_and_command_grid.start();

                        // If we are rendering this as an active prompt, then it is going into the combined grid, hence we need to update
                        // any existing cached demarcation information.
                        if self.honor_ps1 {
                            // If these markers are Some, then the prompt is being reprinted (e.g. due to transient prompt),
                            // hence we need to consider our demarcation markers to be stale.
                            if self.cached_prompt_end_point.is_some() {
                                self.cached_prompt_end_point = Some(PromptEndPoint::Stale);
                            }
                            if self.cached_command_start_point.is_some() {
                                self.cached_command_start_point = Some(CommandStartPoint::Stale);
                            }
                        }

                        if !self.ignore_next_prompt_preview {
                            self.prompt_grid.reset_state();
                            self.prompt_grid.start();
                        }
                    }
                    ansi::PromptKind::Right => {
                        log::error!("Right prompt marker should be handled by Block.");
                    }
                };
                self.receiving_chars_for_prompt = Some(kind);
            }
            ansi::PromptMarker::EndPrompt => {
                let Some(kind) = self.receiving_chars_for_prompt else {
                    log::debug!("Received end prompt marker without a matching start marker");
                    return;
                };
                match kind {
                    ansi::PromptKind::Initial => {
                        if self.honor_ps1 {
                            self.mark_and_cache_end_of_prompt();
                        } else {
                            // We need to reset the cursor position via the Reset Grid OSC so that
                            // ConPTY doesn't think the cursor is after the prompt. Therefore, we expect
                            // to receive another OSC before the command is inputted.
                            self.prompt_and_command_grid.reset_received_osc();
                        }

                        if self.ignore_next_prompt_preview {
                            self.ignore_next_prompt_preview = false;
                        } else {
                            log::debug!("Received end prompt marker for initial prompt");
                            self.prompt_grid.finish();
                        }
                    }
                    ansi::PromptKind::Right => {
                        log::error!("Right prompt marker should be handled by Block.");
                    }
                }
            }
        }
    }

    fn text_area_size_pixels<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate_with_writer!(self.text_area_size_pixels(writer));
    }

    fn text_area_size_chars<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate_with_writer!(self.text_area_size_chars(writer));
    }

    fn precmd_with_completion_metadata(&mut self, data: PrecmdValue) {
        self.prompt_only_precmd(data.prompt_metadata);
    }

    fn prompt_only_precmd(&mut self, data: PromptMetadata) {
        if let Some(honor_ps1) = data.honor_ps1 {
            if honor_ps1 != self.honor_ps1 {
                log::debug!(
                    "Honor PS1 value changed from {} to {}",
                    self.honor_ps1,
                    honor_ps1
                );
                // We send a terminal event which will result in bindkeys being issued to the shell session, to
                // switch the prompt mode via the $WARP_HONOR_PS1 environment variable.
                self.event_proxy
                    .send_terminal_event(Event::HonorPS1OutOfSync);

                // We synchronize the state of our `honor_ps1` setting with the value passed from the shell.
                // Note that we ALWAYS want this to be synced properly since the shell determines the prompt
                // to be emitted. This may be de-synced from Warp settings in particular niche cases (which are
                // bugs), however, we still want consistent behavior for the prompt in the blocklist (we want to
                // avoid double prompt or empty prompt issues).
                self.honor_ps1 = honor_ps1;
            }
        }

        if let Some(ps1) = data.ps1 {
            if ps1.is_empty() {
                return;
            }

            let unescaped_ps1 = if data
                .ps1_is_encoded
                .is_some_and(|ps1_is_encoded| !ps1_is_encoded)
            {
                Ok(ps1.as_bytes().to_vec())
            } else {
                // Decoding the bytes passed from the shell
                hex::decode(ps1)
            };

            if let Ok(unescaped_ps1) = unescaped_ps1 {
                let mut processor = Processor::new();
                self.prompt_and_command_grid.start();

                // We purposefully ignore ignore_next_prompt here (to handle in-band generators
                // correctly with prompt caching).
                // Only put the PS1 into the combined grid if we are honoring the PS1.
                // Otherwise, we only put it into the prompt grid which is used solely for
                // prompt previews (Edit Prompt modal + onboarding block).
                if self.honor_ps1 {
                    processor.parse_bytes(
                        self.prompt_and_command_grid_mut(),
                        &unescaped_ps1,
                        &mut io::sink(),
                    );
                }

                if !self.ignore_next_prompt_preview {
                    self.prompt_grid.start();
                    processor.parse_bytes(self.prompt_grid_mut(), &unescaped_ps1, &mut io::sink());
                    self.prompt_grid.finish();
                }
            }

            if self.ignore_next_prompt_preview {
                self.ignore_next_prompt_preview = false;
            }
        }
    }

    fn preexec(&mut self, _data: PreexecValue) {
        self.finish_command_grid();
    }

    fn on_finish_byte_processing(&mut self, input: &ansi::ProcessorInput<'_>) {
        delegate!(self.on_finish_byte_processing(input));
    }

    fn on_reset_grid(&mut self) {
        delegate!(self.on_reset_grid());
    }

    fn set_keyboard_enhancement_flags(
        &mut self,
        mode: KeyboardModes,
        apply: KeyboardModesApplyBehavior,
    ) {
        delegate!(self.set_keyboard_enhancement_flags(mode, apply));
    }

    fn push_keyboard_enhancement_flags(&mut self, mode: KeyboardModes) {
        delegate!(self.push_keyboard_enhancement_flags(mode));
    }

    fn pop_keyboard_enhancement_flags(&mut self, count: u16) {
        delegate!(self.pop_keyboard_enhancement_flags(count));
    }

    fn query_keyboard_enhancement_flags<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.query_keyboard_enhancement_flags(writer));
    }
}
