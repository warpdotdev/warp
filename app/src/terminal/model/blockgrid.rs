use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::SizeInfo;

use crate::terminal::model::ansi::{
    self, Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, LineClearMode, Mode,
    PrecmdValue, PreexecValue, StandardCharset, TabulationClearMode,
};
use crate::terminal::model::grid::grid_handler::{GridHandler, PerformResetGridChecks, RegexIter};
use crate::terminal::model::index::{Point, VisibleRow};
use crate::terminal::model::iterm_image::ITermImage;

use crate::terminal::model::grid::Dimensions;
use crate::terminal::model::GridStorage;

use crate::terminal::model::secrets::ObfuscateSecrets;
use instant::Instant;
use pathfinder_color::ColorU;
use std::collections::HashMap;
use std::io;
use std::num::NonZeroUsize;
use std::ops::{Range, RangeInclusive};
use std::sync::{Arc, OnceLock};

use super::find::RegexDFAs;
use super::grid::RespectDisplayedOutput;
use super::image_map::StoredImageMetadata;
use super::kitty::{KittyAction, KittyResponse};
use super::secrets::RespectObfuscatedSecrets;
use super::selection::ScrollDelta;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};

#[derive(Clone)]
pub struct BlockGrid {
    pub(super) grid_handler: GridHandler,

    started: bool,

    /// Whether the grid is finished receiving input. For the command grid this
    /// // maps to when `preexec` gets called and for the output grid this maps
    /// to when the `precmd` gets called. Once finished is set, we consider the
    /// grid to be immutable, though the block may go through a new
    /// start/input/finish cycle.
    finished: bool,

    start_time: Option<Instant>,

    /// [`Grid::rightmost_visible_nonempty_cell`] is inefficient to compute anew, especially if it
    /// happens every frame, as it loops through the visible cells in the Grid. As the Grid itself
    /// doesn't know its own finished state, it can't memoize the result. That's why we put this
    /// state here where FinishedState is known.
    cached_rightmost_visible_nonempty_cell: OnceLock<Option<usize>>,

    /// Similar to [`Self::cached_rightmost_visible_nonempty_cell`].
    cached_has_visible_chars: OnceLock<bool>,

    /// Similar to [`Self::cached_rightmost_visible_nonempty_cell`].
    cached_starts_with_input_buffer_sequence: OnceLock<bool>,

    pub(super) should_scan_for_secrets: ObfuscateSecrets,

    /// When true, `len_displayed()` caps its return value at the bottommost
    /// visible content row to trim trailing blank rows.
    trim_trailing_blank_rows: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::terminal) enum CursorDisplayPoint {
    Visible(Point),
    HiddenCache(Point),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for BlockGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}",
            self.to_string(
                false, /* include_esc_sequences */
                None,
                RespectObfuscatedSecrets::No,
                false, /* force_secrets_obfuscated */
                RespectDisplayedOutput::Yes
            )
        )
    }
}

impl BlockGrid {
    pub fn new(
        size_info: SizeInfo,
        max_scroll_limit: usize,
        event_proxy: ChannelEventListener,
        should_scan_for_secrets: ObfuscateSecrets,
        perform_reset_grid_checks: PerformResetGridChecks,
    ) -> Self {
        let grid_handler = GridHandler::new(
            size_info,
            max_scroll_limit,
            event_proxy,
            false,
            should_scan_for_secrets,
            perform_reset_grid_checks,
        );

        BlockGrid {
            grid_handler,
            started: false,
            finished: false,
            start_time: None,
            cached_rightmost_visible_nonempty_cell: Default::default(),
            cached_has_visible_chars: Default::default(),
            should_scan_for_secrets,
            cached_starts_with_input_buffer_sequence: Default::default(),
            trim_trailing_blank_rows: false,
        }
    }

    pub fn num_secrets_obfuscated(&self) -> usize {
        self.grid_handler().num_secrets_obfuscated()
    }

    pub fn should_scan_for_secrets(&self) -> ObfuscateSecrets {
        self.should_scan_for_secrets
    }

    /// Splits the [`BlockGrid`] into two at the specified Grid row.
    ///
    /// The top grid contains all rows up to but not including the split row.
    /// The bottom grid contains all rows including and following the split row.
    /// If the split row is out of bounds, returns [`None`] for the bottom grid.
    ///
    /// The row index here is the index into the full grid, not the visible
    /// region (i.e.: includes any rows in the scrollback buffer).
    pub fn split(&self, row_to_split_on: NonZeroUsize) -> (BlockGrid, Option<BlockGrid>) {
        let (top_grid, bottom_grid) = self.grid_handler.split(row_to_split_on);

        (
            self.new_for_split(top_grid),
            bottom_grid.map(|b| self.new_for_split(b)),
        )
    }

    /// Constructs a new [`BlockGrid`] from a [`GridHandler`] that holds a
    /// subset of rows in `self`.
    fn new_for_split(&self, grid_handler: GridHandler) -> BlockGrid {
        BlockGrid {
            grid_handler,
            started: self.started,
            finished: self.finished,
            start_time: self.start_time,
            cached_rightmost_visible_nonempty_cell: Default::default(),
            cached_has_visible_chars: Default::default(),
            should_scan_for_secrets: self.should_scan_for_secrets,
            cached_starts_with_input_buffer_sequence: Default::default(),
            trim_trailing_blank_rows: false,
        }
    }

    /// Returns the length of the entire blockgrid, including rows that are not
    /// displayed.
    pub fn len(&self) -> usize {
        if self.finished() {
            self.finished_len()
        } else if self.started {
            self.grid_storage().max_cursor_point.row.0 + self.grid_handler.history_size() + 1
        } else {
            0
        }
    }

    /// Returns the length of the blockgrid containing only displayed rows.
    /// When `trim_trailing_blank_rows` is set, caps at the bottommost visible
    /// content row to exclude trailing blank rows from the displayed height.
    pub fn len_displayed(&self) -> usize {
        let base = if let Some(len_displayed) = self.grid_handler().len_displayed() {
            len_displayed
        } else {
            self.len()
        };
        if self.trim_trailing_blank_rows {
            let content_len = self
                .grid_handler()
                .visible_content_len_for_trimming()
                .unwrap_or(usize::from(self.started));
            base.min(content_len)
        } else {
            base
        }
    }

    pub(in crate::terminal) fn cursor_display_point(&self) -> Option<CursorDisplayPoint> {
        let cursor_point = self.grid_handler().cursor_render_point();
        let cursor_display_point = self
            .grid_handler()
            .maybe_translate_point_from_original_to_displayed(cursor_point);
        let len_displayed = self.len_displayed();

        if !self.trim_trailing_blank_rows || cursor_display_point.row < len_displayed {
            return Some(CursorDisplayPoint::Visible(cursor_display_point));
        }

        if len_displayed == 0 {
            return None;
        }
        Some(CursorDisplayPoint::HiddenCache(Point::new(
            cursor_display_point.row.min(len_displayed - 1),
            cursor_display_point
                .col
                .min(self.grid_handler().columns().saturating_sub(1)),
        )))
    }

    /// Determine whether the block is currently empty
    ///
    /// Note: Depending on the state of the block, it can be _currently_ empty even when it has
    /// received contents
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Calculate the length of the blockgrid as if it were finished
    ///
    /// This doesn't look at the current state, but rather represents the state of the block if
    /// it were already finished
    fn finished_len(&self) -> usize {
        // Only include the row with the cursor if the cursor is not on the first column.
        self.grid_handler.rows_to_cursor() + (self.grid_handler.cursor_point().col != 0) as usize
    }

    /// Determine whether the grid has received content.
    /// This is a simple check for whether the grid_cursor has been moved at all by the shell.
    /// I.e., is the max grid_cursor past position (0,0)? If the cursor position has been modified,
    /// then it's safe to assume we've received output contents from the shell.
    ///
    /// An example where has_received_content() == true but should_show_as_empty_when_finished == false
    /// are full-screen apps like mitmproxy. Here, the program has an output that spans the size
    /// of the terminal view; however, the grid_cursor is positioned at (0,0), indicating that this
    /// content should be truncated away once the program is finished -- hence, pre-finish content
    /// but no post-finish content.
    pub fn has_received_content(&self) -> bool {
        !(self.grid_storage().max_cursor_point.row.0 == 0
            && self.grid_storage().max_cursor_point.col == 0)
    }

    /// Determine whether the grid will have content once it's been finished.
    /// This is a simple check for whether the grid_cursor is currently at coordinates (0,0).
    /// If it is, this means any possible content in the grid will be truncated away at
    /// grid-finish time (precmd or preexec), leaving the grid contentless. If the grid_cursor
    /// is not at (0,0), we can assume the grid contains content that will persist through finish.
    pub fn should_show_as_empty_when_finished(&self) -> bool {
        self.finished_len() == 0
            || !self.has_visible_chars()
            || self.contains_only_input_buffer_sequence()
    }

    pub(super) fn all_bytes_scanned_for_secrets(&self) -> bool {
        self.grid_handler().all_bytes_scanned_for_secrets()
    }

    /// Disables secret obfuscation unconditionally.
    pub(super) fn disable_secret_obfuscation(&mut self) {
        self.should_scan_for_secrets = ObfuscateSecrets::No;
        self.grid_handler
            .set_obfuscate_secrets(ObfuscateSecrets::No);
    }

    /// Sets the grid to obfuscate secrets except when both of the following are true:
    /// (1) The grid is finished.
    /// (2) All bytes have not been scanned for secrets already.
    pub(super) fn maybe_enable_secret_obfuscation(&mut self, obfuscate_secrets: ObfuscateSecrets) {
        if self.finished() && !self.all_bytes_scanned_for_secrets() {
            // This grid finished with secrets disabled and has not been
            // scanned for secrets but we are attempting to obfuscate secrets now.
            return;
        }

        self.should_scan_for_secrets = obfuscate_secrets;
        self.grid_handler.set_obfuscate_secrets(obfuscate_secrets);
    }

    pub(super) fn scan_full_grid_for_secrets(&mut self) {
        self.grid_handler.scan_full_grid_for_secrets();
    }

    /// Mark the grid as finished and update the length of the grid accordingly. Once finished, the
    /// block is considered immutable and new content is no longer fed to it from the PTY. Any
    /// unused rows are truncated from the grid.
    pub fn finish(&mut self) {
        self.started = true;
        self.finished = true;
        self.trim_trailing_blank_rows = false;

        self.grid_handler_mut().finish();
    }

    pub fn set_trim_trailing_blank_rows(&mut self, trim: bool) {
        self.trim_trailing_blank_rows = trim;
        self.grid_handler.set_track_content_length(trim);
    }

    pub(in crate::terminal) fn enable_full_grid_clear_behavior(&mut self) {
        self.grid_handler.enable_full_grid_clear_behavior();
    }

    /// Returns a freshly-computed value of rightmost_nonempty_cell if this grid isn't
    /// finished yet. Otherwise, return a memoized value since the grid won't be mutated anymore.
    pub fn rightmost_visible_nonempty_cell(&self) -> Option<usize> {
        if self.finished {
            self.cached_rightmost_visible_nonempty_cell
                .get_or_init(|| self.grid_handler().rightmost_nonempty_cell(None))
                .to_owned()
        } else {
            self.grid_handler().rightmost_nonempty_cell(None)
        }
    }

    fn has_visible_chars(&self) -> bool {
        if self.finished {
            *self
                .cached_has_visible_chars
                .get_or_init(|| self.grid_handler().has_visible_chars())
        } else {
            self.grid_handler().has_visible_chars()
        }
    }

    fn calculate_if_grid_contains_only_input_buffer_sequence(&self) -> bool {
        if self.len_displayed() > 2 {
            return false;
        }

        let contents = self.to_string(
            false, /* include_esc_sequences */
            None,  /* max_rows */
            RespectObfuscatedSecrets::No,
            false, /* force_secrets_obfuscated */
            RespectDisplayedOutput::No,
        );
        // We ignore any content that only contains line editor bindkeys that Warp sends to the
        // shell. Specifically `^[i` (requests the shell's input buffer) and `^P` (clears the
        // shell's input buffer). There are instances where these bindkeys will show up in
        // background blocks when PTY throughput is reduced.
        let contents_trimmed = contents.trim();
        contents_trimmed == "^[i" || contents_trimmed == "^P" || contents_trimmed == "^[i^P"
    }

    /// The block only starts with a control sequence related to the Input Buffer hook.
    fn contains_only_input_buffer_sequence(&self) -> bool {
        if self.finished {
            *self
                .cached_starts_with_input_buffer_sequence
                .get_or_init(|| self.calculate_if_grid_contains_only_input_buffer_sequence())
        } else {
            self.calculate_if_grid_contains_only_input_buffer_sequence()
        }
    }

    /// Returns a freshly-computed value of rightmost_nonempty_cell, taking into account `max_rows` if given.
    pub fn rightmost_visible_nonempty_cell_with_max_row(&self, max_row: usize) -> Option<usize> {
        self.grid_handler().rightmost_nonempty_cell(Some(max_row))
    }

    /// Resizes the grid based on the incoming `SizeInfo` that represents the
    /// size of the entire terminal.
    pub fn resize(&mut self, size_info: SizeInfo) {
        self.grid_handler.resize(size_info);
    }

    pub fn start(&mut self) {
        self.started = true;
        self.start_time = Some(Instant::now());
    }

    pub(in crate::terminal) fn grid_storage(&self) -> &GridStorage {
        self.grid_handler.grid_storage()
    }

    pub fn grid_handler(&self) -> &GridHandler {
        &self.grid_handler
    }

    // TODO(vorporeal): Fix code location of blockgrid test utils and make this
    // pub(in crate::terminal).
    pub fn grid_storage_mut(&mut self) -> &mut GridStorage {
        self.grid_handler.grid_storage_mut()
    }

    pub fn grid_handler_mut(&mut self) -> &mut GridHandler {
        &mut self.grid_handler
    }

    pub fn find<'a>(&'a self, dfas: &'a RegexDFAs) -> RegexIter<'a> {
        self.grid_handler.find(dfas)
    }

    pub(super) fn to_string(
        &self,
        include_esc_sequences: bool,
        max_rows: Option<usize>,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
        force_secrets_obfuscated: bool,
        respect_displayed_output: RespectDisplayedOutput,
    ) -> String {
        if self.is_empty() {
            String::new()
        } else {
            let row_start_bound = match max_rows {
                None => 0,
                Some(max) => (self.len() - 1).saturating_sub(max),
            };
            self.grid_handler.bounds_to_string(
                Point::new(row_start_bound, 0),
                self.end_point(),
                include_esc_sequences,
                respect_obfuscated_secrets,
                force_secrets_obfuscated,
                respect_displayed_output,
            )
        }
    }

    /// Converts the grid into a string.
    /// If the include_escape_sequences field is true, we include the ansi escape sequences.
    /// Otherwise, we have a plaintext string.
    /// max_rows is an optional limit, if it is none, we return everything. Otherwise, we return
    /// the last max_rows rows.
    pub fn contents_to_string(
        &self,
        include_escape_sequences: bool,
        max_rows: Option<usize>,
    ) -> String {
        self.to_string(
            include_escape_sequences,
            max_rows,
            RespectObfuscatedSecrets::Yes,
            false,
            RespectDisplayedOutput::Yes,
        )
    }

    /// Returns a string containing a summary of the block's contents.
    ///
    /// The summary is guaranteed to contain the first `start_rows` rows and
    /// the last `end_rows` rows.  If the block is longer than
    /// `start_rows + end_rows`, a line will be added in the middle to denote
    /// the truncated section.
    pub fn content_summary(
        &self,
        start_rows: usize,
        end_rows: usize,
        force_secrets_obfuscated: bool,
    ) -> String {
        let include_esc_sequences = false;
        let respect_obfuscated_secrets = RespectObfuscatedSecrets::Yes;
        let respect_displayed_output = RespectDisplayedOutput::No;

        // If the block is shorter than the combined number of rows requested, return the full block.
        let total_rows = self.finished_len();
        if start_rows + end_rows >= total_rows {
            let max_rows = None;
            return self.to_string(
                include_esc_sequences,
                max_rows,
                respect_obfuscated_secrets,
                force_secrets_obfuscated,
                respect_displayed_output,
            );
        }

        // Otherwise, get the first `start_rows` rows and the last `end_rows` rows, and join them
        // with some text in the middle to denote the truncated section.
        let cols = self.grid_handler().columns();
        let mut summary = self.grid_handler.bounds_to_string(
            self.start_point(),
            Point::new(start_rows - 1, cols - 1),
            include_esc_sequences,
            respect_obfuscated_secrets,
            force_secrets_obfuscated,
            respect_displayed_output,
        );
        const TRUNCATION_MESSAGE: &str = "\n...(truncated)...\n";
        let summary_suffix = self.grid_handler.bounds_to_string(
            Point::new(total_rows - end_rows, 0),
            self.end_point(),
            include_esc_sequences,
            respect_obfuscated_secrets,
            force_secrets_obfuscated,
            respect_displayed_output,
        );

        // Now that we know all of the pieces of the final string, we can grow the
        // initial string by exactly the amount we need, then append the additional
        // content.
        summary.reserve_exact(TRUNCATION_MESSAGE.len() + summary_suffix.len());
        summary.push_str(TRUNCATION_MESSAGE);
        summary.push_str(&summary_suffix);

        summary
    }

    /// Converts the grid into a string.
    /// If the include_escape_sequences field is true, we include the ansi escape sequences.
    /// Otherwise, we have a plaintext string.
    /// max_rows is an optional limit, if it is none, we return everything. Otherwise, we return
    /// the last max_rows rows.
    ///
    /// NOTE: This function does not respect displayed/filtered content and returns all
    /// contents in the original grid.
    pub fn contents_to_string_force_full_grid_contents(
        &self,
        include_escape_sequences: bool,
        max_rows: Option<usize>,
    ) -> String {
        self.to_string(
            include_escape_sequences,
            max_rows,
            RespectObfuscatedSecrets::Yes,
            false,
            RespectDisplayedOutput::No,
        )
    }

    /// Converts the grid into a string.
    /// If the include_escape_sequences field is true, we include the ansi escape sequences.
    /// Otherwise, we have a plaintext string.
    /// max_rows is an optional limit, if it is none, we return everything. Otherwise, we return
    /// the last max_rows rows.
    ///
    /// NOTE: This function does not obfuscate secrets.
    pub fn contents_to_string_with_secrets_unobfuscated(
        &self,
        include_escape_sequences: bool,
        max_rows: Option<usize>,
    ) -> String {
        self.to_string(
            include_escape_sequences,
            max_rows,
            RespectObfuscatedSecrets::No,
            false,
            RespectDisplayedOutput::No,
        )
    }

    /// Converts the grid into a string.
    /// If the include_escape_sequences field is true, we include the ansi escape sequences.
    /// Otherwise, we have a plaintext string.
    /// max_rows is an optional limit, if it is none, we return everything. Otherwise, we return
    /// the last max_rows rows.
    ///
    /// NOTE: This function forces all secrets in grids to be obfuscated. We need this
    /// to respect the value when serializing a block for sharing.
    pub fn contents_to_string_force_secrets_obfuscated(
        &self,
        include_escape_sequences: bool,
        max_rows: Option<usize>,
    ) -> String {
        self.to_string(
            include_escape_sequences,
            max_rows,
            RespectObfuscatedSecrets::No,
            true,
            RespectDisplayedOutput::No,
        )
    }

    /// Converts all the points from (0.0) to `point` to a string.
    pub fn start_to_point_as_string(&self, point: Point) -> String {
        if self.is_empty() {
            String::new()
        } else {
            self.grid_handler.bounds_to_string(
                self.start_point(),
                point,
                false, /* include_esc_sequences */
                RespectObfuscatedSecrets::Yes,
                false, /* force_obfuscated_secrets */
                RespectDisplayedOutput::Yes,
            )
        }
    }

    /// Converts all the points from `point` to the end to a string.
    pub fn point_to_end_as_string(&self, point: Point) -> String {
        if self.is_empty() {
            String::new()
        } else {
            self.grid_handler.bounds_to_string(
                point,
                self.end_point(),
                false, /* include_esc_sequences */
                RespectObfuscatedSecrets::Yes,
                false, /* force_obfuscated_secrets */
                RespectDisplayedOutput::Yes,
            )
        }
    }

    /// Returns the Point position of the last character/element in the grid.
    pub fn end_point(&self) -> Point {
        Point::new(
            self.len().saturating_sub(1),
            self.grid_handler.columns().saturating_sub(1),
        )
    }

    /// Returns the Point position of the first character/element in the grid.
    pub fn start_point(&self) -> Point {
        Point::new(0, 0)
    }

    /// Returns the total number of rows printed to the grid, including any lines that were
    /// truncated due maximum block line limits.
    pub fn total_row_count(&self) -> u64 {
        (self.end_point().row as u64) + self.grid_handler().num_lines_truncated() + 1
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.grid_handler.cursor_style()
    }

    pub fn start_time(&self) -> Option<Instant> {
        self.start_time
    }

    pub fn needs_bracketed_paste(&self) -> bool {
        self.grid_handler.needs_bracketed_paste()
    }

    pub fn filter_matches(&self) -> Option<&[RangeInclusive<Point>]> {
        self.grid_handler().filter_matches()
    }

    /// Apply a filter to this blockgrid. The logical lines containing the
    /// matches will be shown in the blockgrid's visible output, while
    /// non-matching lines will be hidden.
    pub fn filter_lines(
        &mut self,
        dfas: Arc<RegexDFAs>,
        num_context_lines: usize,
        invert_filter: bool,
    ) {
        self.grid_handler_mut()
            .filter_lines(dfas, num_context_lines, invert_filter);
    }

    /// Re-applies the filter to the blockgrid if it has truncated rows.
    pub fn maybe_refilter_lines(&mut self) {
        if self.grid_handler().num_lines_truncated() > 0 {
            self.grid_handler_mut().refilter_lines();
        }
    }

    /// Clear the applied filter. Will reset the displayed output so all rows in
    /// the blockgrid will be visible.
    pub fn clear_filter(&mut self) {
        self.grid_handler_mut().clear_filter();
    }

    pub fn estimated_heap_usage_bytes(&self) -> usize {
        self.grid_storage().estimated_heap_usage_bytes()
    }

    pub fn estimated_memory_usage_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.grid_storage().estimated_heap_usage_bytes()
    }

    pub fn flat_storage_lines(&self) -> usize {
        self.grid_handler().flat_storage.total_rows()
    }

    pub fn flat_storage_bytes(&self) -> usize {
        self.grid_handler()
            .flat_storage
            .estimated_memory_usage_bytes()
    }

    pub(super) fn disable_reset_grid_checks(&mut self) {
        self.grid_handler.disable_reset_grid_checks();
    }

    pub(super) fn reset_received_osc(&mut self) {
        self.grid_handler.reset_received_osc();
    }

    fn ansi_handler(&mut self) -> &mut impl ansi::Handler {
        self.grid_handler.ansi_handler()
    }

    pub(super) fn set_marked_text(&mut self, marked_text: &str, selected_range: &Range<usize>) {
        self.grid_handler
            .set_marked_text(marked_text, selected_range);
    }

    pub(super) fn clear_marked_text(&mut self) {
        self.grid_handler.clear_marked_text();
    }
}

impl ansi::Handler for BlockGrid {
    fn set_title(&mut self, _: Option<String>) {
        // Ignore: This should be handled at model layer. However, the PS1 may set the title to
        // trigger this, so we want to gracefully... ignore their request.
    }

    fn set_cursor_style(&mut self, style: Option<CursorStyle>) {
        self.ansi_handler().set_cursor_style(style);
    }

    fn set_cursor_shape(&mut self, shape: CursorShape) {
        self.ansi_handler().set_cursor_shape(shape);
    }

    fn input(&mut self, c: char) {
        self.ansi_handler().input(c);
    }

    fn goto(&mut self, row: VisibleRow, col: usize) {
        self.ansi_handler().goto(row, col);
    }

    fn goto_line(&mut self, row: VisibleRow) {
        self.ansi_handler().goto_line(row);
    }

    fn goto_col(&mut self, col: usize) {
        self.ansi_handler().goto_col(col);
    }

    fn insert_blank(&mut self, count: usize) {
        self.ansi_handler().insert_blank(count);
    }

    fn move_up(&mut self, lines: usize) {
        self.ansi_handler().move_up(lines);
    }

    fn move_down(&mut self, lines: usize) {
        self.ansi_handler().move_down(lines);
    }

    fn identify_terminal<W: io::Write>(&mut self, writer: &mut W, intermediate: Option<char>) {
        self.ansi_handler().identify_terminal(writer, intermediate);
    }

    fn report_xtversion<W: io::Write>(&mut self, writer: &mut W) {
        self.ansi_handler().report_xtversion(writer);
    }

    fn device_status<W: io::Write>(&mut self, writer: &mut W, arg: usize) {
        self.ansi_handler().device_status(writer, arg);
    }

    fn move_forward(&mut self, columns: usize) {
        self.ansi_handler().move_forward(columns);
    }

    fn move_backward(&mut self, columns: usize) {
        self.ansi_handler().move_backward(columns);
    }

    fn move_down_and_cr(&mut self, lines: usize) {
        self.ansi_handler().move_down_and_cr(lines);
    }

    fn move_up_and_cr(&mut self, lines: usize) {
        self.ansi_handler().move_up_and_cr(lines);
    }

    fn put_tab(&mut self, count: u16) {
        self.ansi_handler().put_tab(count);
    }

    fn backspace(&mut self) {
        self.ansi_handler().backspace();
    }

    fn carriage_return(&mut self) {
        self.ansi_handler().carriage_return();
    }

    fn linefeed(&mut self) -> ScrollDelta {
        self.ansi_handler().linefeed()
    }

    fn bell(&mut self) {
        self.ansi_handler().bell();
    }

    fn substitute(&mut self) {
        self.ansi_handler().substitute();
    }

    fn newline(&mut self) {
        self.ansi_handler().newline();
    }

    fn set_horizontal_tabstop(&mut self) {
        self.ansi_handler().set_horizontal_tabstop();
    }

    fn scroll_up(&mut self, lines: usize) -> ScrollDelta {
        self.ansi_handler().scroll_up(lines)
    }

    fn scroll_down(&mut self, lines: usize) -> ScrollDelta {
        self.ansi_handler().scroll_down(lines)
    }

    fn insert_blank_lines(&mut self, lines: usize) -> ScrollDelta {
        self.ansi_handler().insert_blank_lines(lines)
    }

    fn delete_lines(&mut self, lines: usize) -> ScrollDelta {
        self.ansi_handler().delete_lines(lines)
    }

    fn erase_chars(&mut self, count: usize) {
        self.ansi_handler().erase_chars(count);
    }

    fn delete_chars(&mut self, count: usize) {
        self.ansi_handler().delete_chars(count)
    }

    fn move_backward_tabs(&mut self, count: u16) {
        self.ansi_handler().move_backward_tabs(count);
    }

    fn move_forward_tabs(&mut self, count: u16) {
        self.ansi_handler().move_forward_tabs(count);
    }

    fn save_cursor_position(&mut self) {
        self.ansi_handler().save_cursor_position();
    }

    fn restore_cursor_position(&mut self) {
        self.ansi_handler().restore_cursor_position();
    }

    fn clear_line(&mut self, mode: LineClearMode) {
        self.ansi_handler().clear_line(mode);
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        self.ansi_handler().clear_screen(mode);
    }

    fn clear_tabs(&mut self, mode: TabulationClearMode) {
        self.ansi_handler().clear_tabs(mode);
    }

    fn reset_state(&mut self) {
        self.ansi_handler().reset_state();
    }

    fn reverse_index(&mut self) -> ScrollDelta {
        self.ansi_handler().reverse_index()
    }

    fn terminal_attribute(&mut self, attr: Attr) {
        self.ansi_handler().terminal_attribute(attr);
    }

    fn set_mode(&mut self, mode: Mode) {
        self.ansi_handler().set_mode(mode);
    }

    fn unset_mode(&mut self, mode: Mode) {
        self.ansi_handler().unset_mode(mode);
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        self.ansi_handler().set_scrolling_region(top, bottom);
    }

    fn set_keypad_application_mode(&mut self) {
        self.ansi_handler().set_keypad_application_mode();
    }

    fn unset_keypad_application_mode(&mut self) {
        self.ansi_handler().unset_keypad_application_mode();
    }

    fn set_active_charset(&mut self, index: CharsetIndex) {
        self.ansi_handler().set_active_charset(index);
    }

    fn configure_charset(&mut self, index: CharsetIndex, charset: StandardCharset) {
        self.ansi_handler().configure_charset(index, charset);
    }

    fn set_color(&mut self, _index: usize, _color: ColorU) {
        // Ignore. This needs to be handled by the TerminalModel. Users should emit this escape
        // code in their RC files if they want to override colors
    }

    fn dynamic_color_sequence<W: io::Write>(
        &mut self,
        writer: &mut W,
        code: u8,
        index: usize,
        terminator: &str,
    ) {
        self.ansi_handler()
            .dynamic_color_sequence(writer, code, index, terminator);
    }

    fn reset_color(&mut self, _index: usize) {
        // Ignore. See the Self::set_color method for explanation
    }

    fn clipboard_store(&mut self, clipboard: u8, base64: &[u8]) {
        self.ansi_handler().clipboard_store(clipboard, base64);
    }

    fn clipboard_load(&mut self, clipboard: u8, terminator: &str) {
        self.ansi_handler().clipboard_load(clipboard, terminator);
    }

    fn decaln(&mut self) {
        self.ansi_handler().decaln();
    }

    fn push_title(&mut self) {
        // Ignore: see Self::set_title for explanation
    }

    fn pop_title(&mut self) {
        // Ignore: see Self::set_title for explanation
    }

    fn text_area_size_pixels<W: io::Write>(&mut self, writer: &mut W) {
        self.ansi_handler().text_area_size_pixels(writer);
    }

    fn text_area_size_chars<W: io::Write>(&mut self, writer: &mut W) {
        self.ansi_handler().text_area_size_chars(writer);
    }

    fn precmd(&mut self, _: PrecmdValue) {
        unreachable!("Handled at block layer");
    }

    fn preexec(&mut self, _: PreexecValue) {
        unreachable!("Handled at block layer");
    }

    fn on_finish_byte_processing(&mut self, input: &ansi::ProcessorInput<'_>) {
        self.ansi_handler().on_finish_byte_processing(input);
    }

    fn on_reset_grid(&mut self) {
        self.ansi_handler().on_reset_grid();
    }

    fn handle_completed_iterm_image(&mut self, image: ITermImage) {
        self.ansi_handler().handle_completed_iterm_image(image)
    }

    fn handle_completed_kitty_action(
        &mut self,
        action: KittyAction,
        metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Option<KittyResponse> {
        self.ansi_handler()
            .handle_completed_kitty_action(action, metadata)
    }

    fn set_keyboard_enhancement_flags(
        &mut self,
        mode: KeyboardModes,
        apply: KeyboardModesApplyBehavior,
    ) {
        self.ansi_handler()
            .set_keyboard_enhancement_flags(mode, apply);
    }

    fn push_keyboard_enhancement_flags(&mut self, mode: KeyboardModes) {
        self.ansi_handler().push_keyboard_enhancement_flags(mode);
    }

    fn pop_keyboard_enhancement_flags(&mut self, count: u16) {
        self.ansi_handler().pop_keyboard_enhancement_flags(count);
    }

    fn query_keyboard_enhancement_flags<W: io::Write>(&mut self, writer: &mut W) {
        self.ansi_handler().query_keyboard_enhancement_flags(writer);
    }
}

#[cfg(test)]
#[path = "blockgrid_tests.rs"]
mod tests;
