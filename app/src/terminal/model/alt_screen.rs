use std::collections::HashMap;
use std::io;
use std::ops::{Range, RangeInclusive};

use super::find::RegexDFAs;
use super::grid::RespectDisplayedOutput;
use super::image_map::StoredImageMetadata;
use super::index::Direction;
use super::kitty::{KittyAction, KittyResponse};
use super::secrets::RespectObfuscatedSecrets;
use super::selection::{ExpandedSelectionRange, ScrollDelta};
use crate::terminal::event::Event as TerminalEvent;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::grid_renderer::ColorSampler;
use crate::terminal::model::ansi;
use crate::terminal::model::ansi::{
    Attr, CharsetIndex, ClearMode, CommandFinishedValue, CursorShape, CursorStyle, LineClearMode,
    Mode, PrecmdValue, PreexecValue, StandardCharset, TabulationClearMode,
};
use crate::terminal::model::grid::grid_handler::{
    FragmentBoundary, GridHandler, Link, PerformResetGridChecks, PossiblePath, TermMode,
};
use crate::terminal::model::grid::{Dimensions, GridStorage};
use crate::terminal::model::index::{Point, Side, VisibleRow};
use crate::terminal::model::iterm_image::ITermImage;
use crate::terminal::model::secrets::ObfuscateSecrets;
use crate::terminal::model::selection::{Selection, SelectionRange};
use crate::terminal::{SizeInfo, SizeUpdate};
use itertools::Itertools;
use num_traits::Float as _;
use parking_lot::Mutex;
use pathfinder_color::ColorU;
use std::sync::Arc;
use vec1::Vec1;
use warp_core::semantic_selection::SemanticSelection;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};
use warpui::text::SelectionType;
use warpui::units::Lines;

pub struct AltScreen {
    grid_handler: GridHandler,
    // Number of fractional lines that have yet to scroll the alt grid.
    // This number is always between 1. and -1, since the alt grid only supports
    // line-based scrolling.
    pending_lines_to_scroll: Lines,
    /// The current alt screen selection.
    /// Do not set this value directly - use [`Self::set_selection`] and [`Self::clear_selection`] instead.
    selection: Option<Selection>,
    /// If this is Some, and if smart-select is enabled, double-clicking within this range will
    /// select this range instead of the normal smart-select logic. The purpose of this is to
    /// allow double-click selection to work on the TerminalView::highlighted_link even when it
    /// contains spaces. Smart-select never traverses across whitespace.
    smart_select_override: Option<RangeInclusive<Point>>,

    /// 'Sampler' that samples background color of output grid cells as the alt screen is rendered.
    ///
    /// In some instances, we color-match the background of other UI elements outside the altscreen
    /// against the altscreen background.
    ///
    /// This has interior mutability because it's updated at render time, as we render each cell in
    /// the output grid.
    pub bg_color_sampler: Arc<Mutex<ColorSampler>>,

    event_proxy: ChannelEventListener,
}

impl AltScreen {
    pub fn new(
        size_info: SizeInfo,
        max_scroll_limit: usize,
        event_proxy: ChannelEventListener,
        obfuscate_secrets: ObfuscateSecrets,
    ) -> Self {
        let grid_handler = GridHandler::new(
            size_info,
            max_scroll_limit,
            event_proxy.clone(),
            true,
            obfuscate_secrets,
            PerformResetGridChecks::default(),
        );

        AltScreen {
            grid_handler,
            pending_lines_to_scroll: Lines::zero(),
            selection: None,
            smart_select_override: None,
            bg_color_sampler: Arc::new(Mutex::new(ColorSampler::new())),
            event_proxy,
        }
    }

    pub fn set_smart_select_override(&mut self, smart_select_override: RangeInclusive<Point>) {
        self.smart_select_override = Some(smart_select_override);
    }

    pub fn clear_smart_select_override(&mut self) {
        self.smart_select_override = None;
    }

    pub(super) fn grid_storage(&self) -> &GridStorage {
        self.grid_handler.grid_storage()
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.grid_handler.cursor_style()
    }

    pub(super) fn grid_storage_mut(&mut self) -> &mut GridStorage {
        self.grid_handler.grid_storage_mut()
    }

    pub fn grid_handler(&self) -> &GridHandler {
        &self.grid_handler
    }

    pub fn grid_handler_mut(&mut self) -> &mut GridHandler {
        &mut self.grid_handler
    }

    /// Resize terminal to new dimensions.
    pub fn resize(&mut self, size_update: &SizeUpdate) {
        // Clear any selection on screen resize, to prevent stale anchors
        if size_update.rows_or_columns_changed() {
            self.clear_selection();
        }
        self.grid_handler.resize(size_update.new_size);
    }

    pub fn reset_pending_lines_to_scroll(&mut self) {
        self.pending_lines_to_scroll = Lines::zero();
    }

    /// Accumulates scroll amounts, returning the integral number of whole lines
    /// which should be scrolled and retaining any remaining fractional
    /// component.
    pub fn accumulate_lines_to_scroll(&mut self, delta: Lines) -> i32 {
        self.pending_lines_to_scroll += delta;
        let whole_lines = self.pending_lines_to_scroll.trunc();
        self.pending_lines_to_scroll = self.pending_lines_to_scroll.fract();
        whole_lines.as_f64() as i32
    }

    #[cfg(test)]
    pub fn pending_lines_to_scroll(&self) -> Lines {
        self.pending_lines_to_scroll
    }

    pub fn selection_range(
        &self,
        semantic_selection: &SemanticSelection,
    ) -> Option<ExpandedSelectionRange<Point>> {
        let selection = self.selection().as_ref()?;
        let SelectionRange {
            start,
            end,
            is_reversed,
        } = selection.to_range(&self.grid_handler, semantic_selection)?;

        if start == end {
            return Some(ExpandedSelectionRange::Regular {
                start,
                end,
                reversed: is_reversed,
            });
        }

        Some(match selection.ty {
            SelectionType::Rect => {
                let start_col = start.col.min(end.col);
                let end_col = end.col.max(start.col);

                // Skip rect selections that will have zero-width.
                if start_col == end_col {
                    return None;
                }

                // Iterate over each row and create a vec of (start, end) points.
                let rows = (start.row..=end.row)
                    .map(|row| {
                        (
                            Point {
                                row,
                                col: start_col,
                            },
                            Point { row, col: end_col },
                        )
                    })
                    .collect();

                ExpandedSelectionRange::Rect {
                    rows: Vec1::try_from_vec(rows).ok()?,
                }
            }
            _ => ExpandedSelectionRange::Regular {
                start,
                end,
                reversed: is_reversed,
            },
        })
    }

    pub fn selection(&self) -> &Option<Selection> {
        &self.selection
    }

    pub fn start_selection(&mut self, point: Point, selection_type: SelectionType, side: Side) {
        let mut selection = Selection::new(selection_type, point, side);
        selection.set_smart_select_side(Direction::Left);

        if let Some(smart_select_override) = &self.smart_select_override {
            // We only want to accept this override if it actually wraps around the cursor, so we
            // do this comparison to make sure the point of the cursor is between the ends of the
            // range of the potential override text.
            if smart_select_override.contains(&point) {
                selection.set_smart_select_override(smart_select_override.clone());
            }
        }
        self.set_selection(selection);
    }

    pub fn update_selection(&mut self, point: Point, side: Side) {
        let mut selection = match self.selection.take() {
            None => return,
            Some(selection) => selection,
        };

        selection.update(point, side);
        if selection.is_tail_before_head() {
            selection.set_smart_select_side(Direction::Right);
        } else {
            selection.set_smart_select_side(Direction::Left);
        }
        self.set_selection(selection);
    }

    fn set_selection(&mut self, value: Selection) {
        self.selection = Some(value);
        self.event_proxy
            .send_terminal_event(TerminalEvent::TextSelectionChanged);
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.event_proxy
            .send_terminal_event(TerminalEvent::TextSelectionChanged);
    }

    pub fn selection_to_string(&self, semantic_selection: &SemanticSelection) -> Option<String> {
        let selection_range = self.selection_range(semantic_selection)?;
        Some(match selection_range {
            ExpandedSelectionRange::Regular { start, end, .. } => {
                self.grid_handler.bounds_to_string(
                    start,
                    end,
                    false, /* include_esc_sequences */
                    RespectObfuscatedSecrets::Yes,
                    false, /* force_obfuscated_secrets */
                    RespectDisplayedOutput::No,
                )
            }
            ExpandedSelectionRange::Rect { rows } => {
                rows.into_iter()
                    .map(|(start, end)| {
                        self.grid_handler.bounds_to_string(
                            start,
                            end,
                            false, /* include_esc_sequences */
                            RespectObfuscatedSecrets::Yes,
                            false, /* force_obfuscated_secrets */
                            RespectDisplayedOutput::No,
                        )
                    })
                    .join("\n")
            }
        })
    }

    pub fn possible_file_paths_at_point(&self, point: Point) -> impl Iterator<Item = PossiblePath> {
        self.grid_handler
            .possible_file_paths_at_point(point)
            .into_iter()
    }

    pub fn url_at_point(&self, point: &Point) -> Option<Link> {
        self.grid_handler.url_at_point(*point)
    }

    pub fn fragment_boundary_at_point(&self, point: &Point) -> FragmentBoundary {
        self.grid_handler.fragment_boundary_at_point(point)
    }

    pub fn bounds_to_string(
        &self,
        start: Point,
        end: Point,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
    ) -> String {
        self.grid_handler.bounds_to_string(
            start,
            end,
            false, /* include_esc_sequences */
            respect_obfuscated_secrets,
            false, /* force_obfuscated_secrets */
            RespectDisplayedOutput::No,
        )
    }

    pub fn output_to_string(&self) -> String {
        self.grid_handler.bounds_to_string(
            Point::new(0, 0),
            Point::new(
                self.grid_handler.total_rows() - 1,
                self.grid_handler.columns() - 1,
            ),
            false, /* include_esc_sequences */
            RespectObfuscatedSecrets::No,
            false, /* force_obfuscated_secrets */
            RespectDisplayedOutput::No,
        )
    }

    pub fn needs_bracketed_paste(&self) -> bool {
        self.grid_handler.needs_bracketed_paste()
    }

    pub fn is_mode_set(&self, mode: TermMode) -> bool {
        self.grid_handler.is_mode_set(mode)
    }

    pub fn find(&self, dfas: &RegexDFAs) -> Vec<RangeInclusive<Point>> {
        self.grid_handler.find(dfas).collect()
    }

    /// Sets whether any content within a grid that is "secret-like" should be obfuscated.
    pub(super) fn set_obfuscate_secrets(&mut self, obfuscate_secrets: ObfuscateSecrets) {
        self.grid_handler.set_obfuscate_secrets(obfuscate_secrets);
    }

    fn rotate_selection(&mut self, delta: ScrollDelta) {
        self.selection = self.selection.take().and_then(|s| {
            s.rotate(
                self.grid_handler.scroll_region(),
                delta,
                self.grid_storage().columns(),
            )
        });
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

    pub fn inferred_bg_color(&self) -> Option<ColorU> {
        self.bg_color_sampler
            .lock()
            .most_common()
            .filter(|color| !color.is_fully_transparent())
    }
}

impl ansi::Handler for AltScreen {
    fn set_title(&mut self, _: Option<String>) {
        log::error!("Handler method AltScreen::set_title should never be called. This should be handled by TerminalModel.");
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
        let lines_scrolled = self.ansi_handler().linefeed();
        self.rotate_selection(lines_scrolled);
        lines_scrolled
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
        let lines_scrolled = self.ansi_handler().scroll_up(lines);
        self.rotate_selection(lines_scrolled);
        lines_scrolled
    }

    fn scroll_down(&mut self, lines: usize) -> ScrollDelta {
        let lines_scrolled = self.ansi_handler().scroll_down(lines);
        self.rotate_selection(lines_scrolled);
        lines_scrolled
    }

    fn insert_blank_lines(&mut self, lines: usize) -> ScrollDelta {
        let lines_scrolled = self.ansi_handler().insert_blank_lines(lines);
        self.rotate_selection(lines_scrolled);
        lines_scrolled
    }

    fn delete_lines(&mut self, lines: usize) -> ScrollDelta {
        let lines_scrolled = self.ansi_handler().delete_lines(lines);
        self.rotate_selection(lines_scrolled);
        lines_scrolled
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
        let lines_scrolled = self.ansi_handler().reverse_index();
        self.rotate_selection(lines_scrolled);
        lines_scrolled
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

    fn set_color(&mut self, index: usize, color: ColorU) {
        self.ansi_handler().set_color(index, color)
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

    fn reset_color(&mut self, index: usize) {
        self.ansi_handler().reset_color(index);
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
        log::error!("Handler method AltScreen::push_title should never be called. This should be handled by TerminalModel.");
    }

    fn pop_title(&mut self) {
        log::error!("Handler method AltScreen::pop_title should never be called. This should be handled by TerminalModel.");
    }

    fn text_area_size_pixels<W: io::Write>(&mut self, writer: &mut W) {
        self.ansi_handler().text_area_size_pixels(writer);
    }

    fn text_area_size_chars<W: io::Write>(&mut self, writer: &mut W) {
        self.ansi_handler().text_area_size_chars(writer);
    }

    fn command_finished(&mut self, _: CommandFinishedValue) {}

    fn precmd(&mut self, _: PrecmdValue) {}

    fn preexec(&mut self, _: PreexecValue) {}

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

impl Dimensions for AltScreen {
    #[inline]
    fn total_rows(&self) -> usize {
        self.grid_handler.total_rows()
    }

    #[inline]
    fn visible_rows(&self) -> usize {
        self.grid_handler.visible_rows()
    }

    #[inline]
    fn columns(&self) -> usize {
        self.grid_handler.columns()
    }
}

#[cfg(test)]
#[path = "alt_screen_tests.rs"]
mod tests;
