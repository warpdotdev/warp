// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

#[path = "ansi_handler.rs"]
mod ansi_handler;
#[path = "filtering.rs"]
mod filtering;
#[path = "image.rs"]
mod image;
#[path = "resize.rs"]
mod resize;
#[path = "secrets.rs"]
mod secrets;

use std::borrow::Cow;
use std::cmp::max;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::ops::{Range, RangeInclusive};
use std::{
    cmp::{min, Ordering},
    mem,
};

use bounded_vec_deque::BoundedVecDeque;
use itertools::Itertools;
use lazy_static::lazy_static;
use unicode_general_category::{get_general_category, GeneralCategory};
use unicode_width::UnicodeWidthChar;
use urlocator::{UrlLocation, UrlLocator};
use warp_core::features::FeatureFlag;
use warp_core::semantic_selection::{SemanticSelection, SMART_SELECT_MATCH_WINDOW_LIMIT};
use warp_core::{safe_assert, safe_assert_eq};
use warp_terminal::model::grid::CellType;
use warp_terminal::model::grid::FlatStorage;
pub use warp_terminal::model::TermMode;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};
use warp_util::path::CleanPathResult;
use warpui::color::ColorU;

use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{self, Color, CursorStyle, Handler, NamedColor};
use crate::terminal::model::cell::{Cell, Flags, LineLength, DEFAULT_CHAR};
use crate::terminal::model::char_or_str::{CharOrStr, PushCharOrStr};
use crate::terminal::model::grid::{Dimensions, GridStorage};
use crate::terminal::model::image_map::ImageMap;
use crate::terminal::model::index::{IndexRange, Point, VisibleRow};
use crate::terminal::model::secrets::{ObfuscateSecrets, SecretMap};
use crate::terminal::SizeInfo;
use crate::util::extensions::TrimStringExt;

use crate::terminal::model::grid::RespectDisplayedOutput;
use crate::terminal::model::secrets::RespectObfuscatedSecrets;
use crate::terminal::model::terminal_model::RangeInModel;
use crate::terminal::model::{
    find::{Match, RegexDFAs},
    index::Direction,
};
use crate::terminal::model::{Secret, SecretHandle};

use super::displayed_output::DisplayedOutput;
use super::grapheme_cursor::{self, GraphemeCursor};
use super::row::Row;
use super::{ConvertToAbsolute as _, Cursor, SelectionCursor};
use filtering::FilterState;
use string_offset::ByteOffset;

/// Used to match equal brackets, when performing a bracket-pair selection.
const BRACKET_PAIRS: [(char, char); 4] = [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];

/// Number of characters to scan on a different line for a link.
const LINK_NUM_CHARACTER_SCAN: usize = 50;

/// Max number of characters to scan for a URL.
const URL_SCAN_CHARACTER_MAX_COUNT: usize = 1000;

/// Max depth for the kitty keyboard mode stack.
/// Per the kitty keyboard protocol, this should be
/// bounded to prevent denial of service attacks.
const KEYBOARD_MODE_STACK_MAX_DEPTH: usize = 4096;

/// For escape sequences
const CSI_START: &str = "\x1b[";
const SGR_RESET_ATTRIBUTES: &str = "\x1b[0m";
const DEFAULT_FG_CODE: u8 = 39;
const DEFAULT_BG_CODE: u8 = 49;
/// the SGR (Select Graphic Rendition) parameters for 256-color and true-color
const FG_SGR_PARAM: u8 = 38;
const BG_SGR_PARAM: u8 = 48;

lazy_static! {
    pub static ref FILE_LINK_SEPARATORS: HashSet<char> =
        HashSet::from([
            '\0', '\t', ' ', '(', ')', ':', '\\', ',', '"', '\'', '[', ']', '{', '}', '<', '>',
            ';', '|', '`', '=',
            // Box-drawing characters used by tree-style directory listers.
            '│', '├', '└', '─', '┬', '┴', '┼', '║', '╠', '╚', '═', '╦', '╩', '╬',
        ]);

    /// The set of characters where, if we encounter them, we have a high degree of confidence that
    /// we're not in a valid URL. Other characters (e.g. '%') might be used in such a way that they
    /// result in invalid URLs, but we don't halt detection if we find them.
    /// See https://datatracker.ietf.org/doc/html/rfc3986 for more details.
    static ref URL_SEPARATORS: HashSet<char> = HashSet::from([' ', '<', '>', '"', '{', '}', '|', '\\', '^', '`']);
}

/// Returns true when `c` should terminate a clickable file path.
///
/// Beyond the explicit set in [`FILE_LINK_SEPARATORS`] (ASCII punctuation
/// plus the box-drawing glyphs used by tree-style listers), any non-ASCII
/// Unicode whitespace or punctuation also acts as a boundary. This lets paths
/// preceded by CJK / full-width punctuation such as `：` (U+FF1A) or `（`
/// (U+FF08) be detected when no ASCII whitespace separates them. Connectors
/// (`Pc`) and dashes (`Pd`) are excluded since they commonly appear inside
/// identifiers and filenames.
pub fn is_file_link_separator(c: char) -> bool {
    if FILE_LINK_SEPARATORS.contains(&c) {
        return true;
    }
    if c.is_ascii() {
        return false;
    }
    if c.is_whitespace() {
        return true;
    }
    matches!(
        get_general_category(c),
        GeneralCategory::OpenPunctuation
            | GeneralCategory::ClosePunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::OtherPunctuation
    )
}

/// Represents a range of cells with information on their combined content and total
/// cell width.
#[derive(Debug, PartialEq, Eq)]
struct Fragment {
    content: String,
    total_cell_width: usize,
}

impl Fragment {
    fn has_separator(&self) -> bool {
        self.content.chars().any(is_file_link_separator)
    }
}

#[derive(Debug)]
pub struct FragmentBoundary(pub Range<Point>);

impl ContainsPoint for FragmentBoundary {
    fn contains(&self, point: Point) -> bool {
        self.0.contains(&point)
    }
}

/// Whether to include the first wide char when parsing line into fragments.
enum IncludeFirstWideChar {
    Yes,
    No,
}

pub trait ContainsPoint {
    fn contains(&self, point: Point) -> bool;
}

#[derive(Debug, PartialEq, Eq)]
pub struct PossiblePath {
    pub path: CleanPathResult,
    pub range: RangeInclusive<Point>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub range: RangeInclusive<Point>,
    pub is_empty: bool,
}

impl RangeInModel for Link {
    fn range(&self) -> RangeInclusive<Point> {
        self.range.clone()
    }
}

impl Link {
    fn extend_link(&mut self, point: Point) {
        if self.is_empty {
            self.range = RangeInclusive::new(point, point);
            self.is_empty = false;
        } else {
            self.range = RangeInclusive::new(*self.range.start(), point);
        }
    }

    pub fn range(&self) -> &RangeInclusive<Point> {
        &self.range
    }
}

impl ContainsPoint for Link {
    fn contains(&self, point: Point) -> bool {
        self.range.contains(&point)
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
pub struct AbsolutePoint {
    pub row: u64,
    pub col: usize,
}

impl PartialOrd for AbsolutePoint {
    fn partial_cmp(&self, other: &AbsolutePoint) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AbsolutePoint {
    fn cmp(&self, other: &AbsolutePoint) -> Ordering {
        match (self.row.cmp(&other.row), self.col.cmp(&other.col)) {
            (Ordering::Equal, ord) | (ord, _) => ord,
        }
    }
}

impl AbsolutePoint {
    pub fn from_point(point: Point, grid: &GridHandler) -> AbsolutePoint {
        let original_point = if grid.has_displayed_output() {
            grid.maybe_translate_point_from_displayed_to_original(point)
        } else {
            point
        };

        AbsolutePoint {
            row: original_point.row as u64 + grid.num_lines_truncated(),
            col: original_point.col,
        }
    }

    pub fn to_point(&self, grid: &GridHandler) -> Option<Point> {
        let point = Point {
            row: self
                .row
                .checked_sub(grid.num_lines_truncated())?
                .try_into()
                .ok()?,
            col: self.col,
        };

        if grid.has_displayed_output() {
            Some(grid.maybe_translate_point_from_original_to_displayed(point))
        } else {
            Some(point)
        }
    }

    pub fn is_truncated(&self, num_lines_truncated: u64) -> bool {
        self.row < num_lines_truncated
    }

    pub fn add_rows(&self, rows: usize) -> AbsolutePoint {
        AbsolutePoint {
            row: self.row + (rows as u64),
            col: self.col,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
pub struct AbsoluteRectangle {
    pub start_row: u64,
    pub end_row: u64,
}

impl AbsoluteRectangle {
    pub fn start_row_to_point(&self, col: usize) -> AbsolutePoint {
        AbsolutePoint {
            row: self.start_row,
            col,
        }
    }

    pub fn end_row_to_point(&self, col: usize) -> AbsolutePoint {
        AbsolutePoint {
            row: self.end_row,
            col,
        }
    }

    pub fn from_range(start_row: usize, end_row: usize, grid: &GridHandler) -> AbsoluteRectangle {
        let temp_start_point = Point::new(start_row, 0);
        let original_start_row = if grid.has_displayed_output() {
            grid.maybe_translate_point_from_displayed_to_original(temp_start_point)
                .row
        } else {
            temp_start_point.row
        };

        let temp_end_point = Point::new(end_row, 0);
        let original_end_row = if grid.has_displayed_output() {
            grid.maybe_translate_point_from_displayed_to_original(temp_end_point)
                .row
        } else {
            temp_end_point.row
        };

        AbsoluteRectangle {
            start_row: original_start_row as u64 + grid.num_lines_truncated(),
            end_row: original_end_row as u64 + grid.num_lines_truncated(),
        }
    }

    pub fn overlaps(&self, rectangle: AbsoluteRectangle) -> bool {
        self.start_row <= rectangle.end_row && self.end_row >= rectangle.start_row
    }
}

/// Whether or not this Grid should keep track of a "Reset Grid" OSC. On Windows, ConPTY has an internal
/// grid that needs to be kept in sync with Warp's grids. We do this via clearing the ConPTY
/// grid before Warp starts populating a new grid.
///
/// See here for more: https://docs.google.com/document/d/11fU_vVW8CH72W92QUnFJ1Kl31fGWNGbjkQQCK3TUaYk/edit?usp=sharing
#[derive(Default, Clone, Copy)]
pub enum PerformResetGridChecks {
    /// Enable checks relating to the Reset Grid OSC.
    Yes,
    /// Disable checks related to the Reset Grid OSC.
    #[default]
    No,
}

/// Specifies a row index within a particular internal storage structure.
#[derive(Debug, Copy, Clone, PartialEq)]
enum StorageRow {
    GridStorage(usize),
    FlatStorage(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Controls whether full-grid operations on a primary-screen grid preserve the old visible rows as
/// scrollback or treat them as a mutable redraw surface.
pub(in crate::terminal) enum FullGridClearBehavior {
    /// Reset visible cells in place for full-grid clears and resizes, avoiding scrollback growth
    /// during TUI-style redraws on the primary grid.
    Clear,
    /// Preserve normal primary-grid behavior where full-grid clears and resizes move visible rows
    /// into scrollback.
    Scroll,
}

/// An implementation of `ansi::Handler` that writes to a `Grid`.
#[derive(Clone)]
pub struct GridHandler {
    grid: GridStorage,

    pub(crate) flat_storage: FlatStorage,

    finished: bool,

    ansi_handler_state: ansi_handler::State,

    /// Info about the subset of rows we want to show to the user. If None, we
    /// show the entire blockgrid to the user.
    displayed_output: Option<DisplayedOutput>,
    /// Info about the output filter applied to the blockgrid.
    filter_state: Option<FilterState>,

    pub(super) secrets: SecretMap,
    /// Given a plaintext, this map returns all secrets with the same plaintext.
    secrets_in_plaintext: HashMap<String, HashSet<SecretHandle>>,
    secret_obfuscation_mode: ObfuscateSecrets,
    /// Determines whether all bytes have been processed to detect secrets.
    all_bytes_scanned_for_secrets: bool,
    pub(super) images: ImageMap,
    marked_text: Option<String>,

    /// Bottommost row with content that should contribute to trimmed CLI agent
    /// block height, updated per PTY-read batch in `on_finish_byte_processing`
    /// when `track_content_length` is true.
    ///
    /// `None` = not yet computed or no visible content.
    /// `Some(row)` = bottommost visible content row index.
    bottommost_visible_content_row: Option<usize>,

    /// When true, `on_finish_byte_processing` computes
    /// `bottommost_visible_content_row` via backward scan. Set by the owning
    /// `BlockGrid` when `trim_trailing_blank_rows` is active.
    track_content_length: bool,

    full_grid_clear_behavior: FullGridClearBehavior,

    /// Accumulates dirty row ranges for find operations.
    ///
    /// Unlike `dirty_cells_range` in `ansi_handler_state` which is reset after each byte
    /// processing pass, this field accumulates dirty rows until find explicitly consumes them.
    /// This allows find to be updated less frequently than byte processing while still
    /// knowing exactly which rows have changed.
    find_dirty_rows_range: Option<RangeInclusive<usize>>,
}

impl GridHandler {
    pub fn new(
        size_info: SizeInfo,
        max_scroll_limit: usize,
        event_proxy: ChannelEventListener,
        is_alt_screen: bool,
        obfuscate_secrets: ObfuscateSecrets,
        perform_reset_grid_checks: PerformResetGridChecks,
    ) -> Self {
        // We set the maximum scrollback for grid storage to zero, as the
        // scrollback is stored in flat storage _instead_.  `GridHandler`
        // is responsible for moving lines from grid storage to flat storage
        // when they are about to be scrolled up out of the active region of
        // the grid.
        let grid_max_scroll_limit = 0;

        let grid = GridStorage::new(
            size_info.rows(),
            size_info.columns(),
            grid_max_scroll_limit,
            obfuscate_secrets,
        );

        let ansi_handler_state = ansi_handler::State::new(
            &size_info,
            event_proxy,
            is_alt_screen,
            obfuscate_secrets,
            perform_reset_grid_checks,
        );

        GridHandler {
            grid,
            flat_storage: FlatStorage::new(size_info.columns(), Some(max_scroll_limit), None),
            finished: false,
            ansi_handler_state,
            displayed_output: None,
            filter_state: None,
            secrets: Default::default(),
            secrets_in_plaintext: Default::default(),
            secret_obfuscation_mode: obfuscate_secrets,
            all_bytes_scanned_for_secrets: true,
            images: Default::default(),
            marked_text: None,
            bottommost_visible_content_row: None,
            track_content_length: false,
            full_grid_clear_behavior: FullGridClearBehavior::Scroll,
            find_dirty_rows_range: None,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(rows: usize, columns: usize) -> Self {
        Self::new_for_test_with_scroll_limit(rows, columns, 0)
    }

    #[cfg(test)]
    pub fn new_for_test_with_scroll_limit(
        rows: usize,
        columns: usize,
        max_scroll_limit: usize,
    ) -> Self {
        Self::new(
            SizeInfo::new_without_font_metrics(rows, columns),
            max_scroll_limit,
            ChannelEventListener::new_for_test(),
            false,
            ObfuscateSecrets::No,
            PerformResetGridChecks::No,
        )
    }

    #[cfg(test)]
    pub fn new_for_alt_screen_test(rows: usize, columns: usize) -> Self {
        Self::new(
            SizeInfo::new_without_font_metrics(rows, columns),
            0,
            ChannelEventListener::new_for_test(),
            true,
            ObfuscateSecrets::No,
            PerformResetGridChecks::No,
        )
    }

    pub(in crate::terminal::model) fn ansi_handler(&mut self) -> &mut impl ansi::Handler {
        self
    }

    pub fn set_track_content_length(&mut self, track: bool) {
        self.track_content_length = track;
        if track {
            // Eagerly compute so the very first render after enabling
            // trimming sees the correct content length instead of falling
            // back to the full max_cursor_point-based height.
            self.bottommost_visible_content_row = self.bottommost_visible_content_row_backward();
        }
    }

    pub(in crate::terminal) fn enable_full_grid_clear_behavior(&mut self) {
        self.full_grid_clear_behavior = FullGridClearBehavior::Clear;
    }

    pub(crate) fn set_supports_emoji_presentation_selector(
        &mut self,
        supports_emoji_presentation_selector: bool,
    ) {
        self.ansi_handler_state.supports_emoji_presentation_selector =
            supports_emoji_presentation_selector;
    }

    /// Splits the [`GridHandler`] into two at the specified Grid row.
    ///
    /// The top grid contains all rows up to but not including the split row.
    /// The bottom grid contains all rows including and following the split row.
    /// If the split row is out of bounds, returns [`None`] for the bottom grid.
    ///
    /// The row index here is the index into the full grid, not the visible
    /// region (i.e.: includes any rows in the scrollback buffer).
    pub fn split(&self, row_to_split_on: NonZeroUsize) -> (Self, Option<Self>) {
        debug_assert!(
            self.ansi_handler_state.dirty_cells_range.is_empty(),
            "should never be splitting a grid while it is receiving pty output"
        );

        let (top_rows, bottom_rows): (Vec<_>, Vec<_>) =
            (0..self.total_rows()).partition_map(|row_idx| {
                let row = self
                    .row(row_idx)
                    .expect("should not fail to get row within bounds")
                    .into_owned();

                if row_idx < row_to_split_on.get() {
                    itertools::Either::Left(row)
                } else {
                    itertools::Either::Right(row)
                }
            });

        let top_grid = self.new_for_split(top_rows, 0);
        let bottom_grid = if bottom_rows.is_empty() {
            None
        } else {
            Some(self.new_for_split(bottom_rows, top_grid.total_rows()))
        };

        (top_grid, bottom_grid)
    }

    /// Constructs a new [`GridHandler`] with a subset of rows from `self`.
    ///
    /// `num_preceding_rows` is the number of rows in `self` that come before
    /// the rows in `rows`.
    fn new_for_split(&self, rows: Vec<Row>, num_preceding_rows: usize) -> Self {
        let grid = self
            .grid
            .new_for_split(rows, num_preceding_rows, self.history_size());

        let mut ansi_handler_state = self.ansi_handler_state.clone();
        ansi_handler_state.scroll_region = VisibleRow(0)..VisibleRow(grid.visible_rows());

        // Create a new grid handler with the new grid.
        let mut grid = GridHandler {
            grid,
            flat_storage: FlatStorage::new(self.columns(), self.flat_storage.max_rows(), None),
            finished: self.finished,
            ansi_handler_state,
            displayed_output: None,
            filter_state: None,
            secrets: Default::default(),
            secrets_in_plaintext: Default::default(),
            secret_obfuscation_mode: self.secret_obfuscation_mode,
            all_bytes_scanned_for_secrets: false,
            images: Default::default(),
            // We do not support splitting marked text, though at the moment,
            // we never split an active grid, so it should not be an issue.
            marked_text: None,
            bottommost_visible_content_row: None,
            track_content_length: false,
            full_grid_clear_behavior: FullGridClearBehavior::Scroll,
            find_dirty_rows_range: None,
        };

        // Scan the full grid for secrets.  This is less performant than
        // actually splitting the secret map, but it's much easier.
        grid.scan_full_grid_for_secrets();

        grid
    }

    pub fn set_obfuscate_secrets(&mut self, obfuscate_secrets: ObfuscateSecrets) {
        self.ansi_handler_state.obfuscate_secrets = obfuscate_secrets;
        self.obfuscate_secrets(obfuscate_secrets);
    }

    pub(in crate::terminal::model) fn grid_storage(&self) -> &GridStorage {
        &self.grid
    }

    pub(in crate::terminal::model) fn grid_storage_mut(&mut self) -> &mut GridStorage {
        &mut self.grid
    }

    pub fn is_mode_set(&self, mode: TermMode) -> bool {
        self.ansi_handler_state.mode.intersects(mode)
    }

    fn color_to_bg_escape_sequence(color: Color) -> String {
        match color {
            Color::Named(named_color) => format!(
                "{}",
                named_color
                    .to_ansi_bg_escape_code()
                    .unwrap_or(DEFAULT_BG_CODE)
            ),
            Color::Indexed(v) => format!("{BG_SGR_PARAM};5;{v}"),
            Color::Spec(ColorU { r, g, b, .. }) => format!("{BG_SGR_PARAM};2;{r};{g};{b}"),
        }
    }

    fn color_to_fg_escape_sequence(color: Color) -> String {
        match color {
            Color::Named(named_color) => format!(
                "{}",
                named_color
                    .to_ansi_fg_escape_code()
                    .unwrap_or(DEFAULT_FG_CODE)
            ),
            Color::Indexed(v) => format!("{FG_SGR_PARAM};5;{v}"),
            Color::Spec(ColorU { r, g, b, .. }) => format!("{FG_SGR_PARAM};2;{r};{g};{b}"),
        }
    }

    pub fn grapheme_cursor_from(
        &self,
        point: Point,
        wrap: grapheme_cursor::Wrap,
    ) -> GraphemeCursor<'_> {
        GraphemeCursor::new(point, self, wrap)
    }

    #[inline]
    pub fn selection_cursor_from(&self, point: Point) -> SelectionCursor<'_> {
        SelectionCursor::new(self, point)
    }

    // TODO(vorporeal): Make this storage-agnostic.
    fn regex_iter<'a>(
        &'a self,
        start: Point,
        end: Point,
        direction: Direction,
        dfas: &'a RegexDFAs,
    ) -> RegexIter<'a> {
        RegexIter::new(start, end, direction, self, dfas)
    }

    pub fn url_at_point(&self, displayed_point: Point) -> Option<Link> {
        let original_point = self.maybe_translate_point_from_displayed_to_original(displayed_point);
        let row = original_point.row;
        let col = original_point.col;

        let grid_line = self.row(row)?;
        let line_length = grid_line.line_length();

        // Omit cases when user is hovering over blank spaces after the end of a line.
        if col >= line_length {
            return None;
        }

        let current_cell = grid_line.get(col)?;

        // Function to check if cell is on url boundary (invalid url characters).
        let is_at_boundary = |cell: &Cell| {
            !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                && URL_SEPARATORS.contains(&cell.c)
        };
        // If the point is on a separator, return directly because this can't be
        // part of a url.
        if is_at_boundary(current_cell) {
            return None;
        }

        // Scan backward until fragment boundary.
        let mut cursor = self.grapheme_cursor_from(original_point, grapheme_cursor::Wrap::Soft);
        cursor.move_backward();

        let mut starting_point = original_point;
        let mut total_characters_scanned = 0;

        while let Some(item) = cursor.current_item() {
            let current_point = item.point();

            // If we've hit a URL boundary, great; we can continue with our logic
            // by detecting a URL from that point
            if is_at_boundary(item.cell()) {
                break;
            }

            // Otherwise, if we've scanned behind the hovered point more than the max character
            // limit, we know scanning forward from there won't yield a URL
            if total_characters_scanned > URL_SCAN_CHARACTER_MAX_COUNT {
                return None;
            }

            starting_point = current_point;
            total_characters_scanned += 1;
            cursor.move_backward();
        }

        let mut cursor = self.grapheme_cursor_from(starting_point, grapheme_cursor::Wrap::Soft);
        let mut locator = UrlLocator::new();
        let mut state = UrlLocation::Reset;
        let mut scheme_buffer = Vec::new();
        let mut url = Link {
            range: RangeInclusive::new(Point::default(), Point::default()),
            is_empty: true,
        };

        // Whether we have scanned past the hovered point.
        let mut passed_point = false;

        total_characters_scanned = 0;
        while let Some(item) = cursor.current_item() {
            // If we maxed out the number of characters we're willing to scan, then one of
            // two scenarios happened:
            // 1. The URL is exactly of length URL_SCAN_CHARACTER_MAX_COUNT+1, in which case we
            //    should return it.
            // 2. The URL is larger than URL_SCAN_CHARACTER_MAX_COUNT+1, in which case we shouldn't
            //    return anything, because offering an incomplete link is a bad UX.
            // It's far more likely that (2) is what happened, so break and reset the link.
            if total_characters_scanned > URL_SCAN_CHARACTER_MAX_COUNT {
                url = Link {
                    range: RangeInclusive::new(Point::default(), Point::default()),
                    is_empty: true,
                };
                break;
            }
            let current_point = item.point();

            if current_point >= original_point {
                passed_point = true;
            }

            let last_state = mem::replace(&mut state, locator.advance(item.cell().c));
            let link_changed = match (state, last_state) {
                (UrlLocation::Url(_length, _num_illegal_end_chars), UrlLocation::Scheme) => {
                    // Create empty URL.
                    url = Link {
                        range: RangeInclusive::new(Point::default(), Point::default()),
                        is_empty: true,
                    };

                    // Push schemes into URL.
                    for scheme_point in &scheme_buffer {
                        url.extend_link(*scheme_point);
                    }

                    // Push the new char into URL.
                    url.extend_link(current_point);
                    true
                }
                (UrlLocation::Url(_length, num_illegal_end_chars), UrlLocation::Url(..)) => {
                    // If the last character processed is not considered an "illegal"
                    // trailing character for a URL, extend the link up to the current
                    // point.
                    if num_illegal_end_chars == 0 {
                        url.extend_link(current_point);
                    }
                    // Whether or not we actually extended the URL, continue processing
                    // characters, as we might find a valid end character, which would
                    // cause us to yank the end of the URL up to the current point.
                    true
                }
                (UrlLocation::Scheme, _) => {
                    scheme_buffer.push(current_point);
                    true
                }
                (UrlLocation::Reset, _) => {
                    locator = UrlLocator::new();
                    state = UrlLocation::Reset;
                    scheme_buffer.clear();
                    false
                }
                _ => false,
            };

            // We are at the hovered point and link has not updated -- the point is not
            // part of a url.
            if current_point == original_point && !link_changed {
                return None;
            // Passed the hovered point and link hasn't changed -- break because all the later
            // urls will not include the point.
            } else if passed_point && !link_changed {
                break;
            }

            cursor.move_forward();
            total_characters_scanned += 1;
        }

        if url.is_empty || !url.range.contains(&original_point) {
            None
        } else {
            if self.has_displayed_output() {
                let displayed_start =
                    self.maybe_translate_point_from_original_to_displayed(*url.range.start());
                let displayed_end =
                    self.maybe_translate_point_from_original_to_displayed(*url.range.end());
                if displayed_start > displayed_end {
                    log::error!(
                        "URL translation to displayed points failed. Displayed range start {displayed_start:?} is greater than displayed range end {displayed_end:?}"
                    );
                } else {
                    url.range = displayed_start..=displayed_end;
                }
            }
            Some(url)
        }
    }

    /// Converts a cell to a string, with ansi escape sequences
    fn cell_to_string(cell: &Cell) -> String {
        let cell_content = cell.content_for_display();

        let style_checker = Flags::BOLD
            | Flags::DIM
            | Flags::ITALIC
            | Flags::UNDERLINE
            | Flags::STRIKEOUT
            | Flags::INVERSE
            | Flags::HIDDEN
            | Flags::DOUBLE_UNDERLINE;
        let contains_style = cell.flags.intersects(style_checker);

        let color_sequence = match (cell.fg, cell.bg) {
            (Color::Named(NamedColor::Foreground), Color::Named(NamedColor::Background))
                if !contains_style =>
            {
                return cell_content.to_string();
            }
            (Color::Named(NamedColor::Foreground), Color::Named(NamedColor::Background)) => None,
            (Color::Named(NamedColor::Foreground), _) => {
                Some(Self::color_to_bg_escape_sequence(cell.bg))
            }
            (_, Color::Named(NamedColor::Background)) => {
                Some(Self::color_to_fg_escape_sequence(cell.fg))
            }
            (_, _) => Some(format!(
                "{};{}",
                Self::color_to_fg_escape_sequence(cell.fg),
                Self::color_to_bg_escape_sequence(cell.bg)
            )),
        };

        // Because this is a hot code path, we only want to allocate a vector
        // and check flags if we know styling (i.e., BOLD, ITALIC, UNDERLINE,
        // or STRIKEOUT) is needed.
        let style_sequence = if contains_style {
            let mut attributes = Vec::new();

            if cell.flags().contains(Flags::BOLD) {
                attributes.push("1");
            }
            if cell.flags().contains(Flags::DIM) {
                attributes.push("2");
            }
            if cell.flags().contains(Flags::ITALIC) {
                attributes.push("3");
            }
            if cell.flags().contains(Flags::UNDERLINE) {
                attributes.push("4");
            }
            if cell.flags().contains(Flags::DOUBLE_UNDERLINE) {
                attributes.push("4:2");
            }
            if cell.flags().contains(Flags::INVERSE) {
                attributes.push("7");
            }
            if cell.flags().contains(Flags::HIDDEN) {
                attributes.push("8");
            }
            if cell.flags().contains(Flags::STRIKEOUT) {
                attributes.push("9");
            }

            Some(attributes.join(";"))
        } else {
            None
        };

        match (color_sequence, style_sequence) {
            (Some(color_sequence), Some(style_sequence)) => {
                format!("{CSI_START}{color_sequence};{style_sequence}m{cell_content}{SGR_RESET_ATTRIBUTES}")
            }
            (color_sequence, style_sequence) => {
                let color_sequence = color_sequence.unwrap_or_default();
                let style_sequence = style_sequence.unwrap_or_default();

                format!(
                    "{CSI_START}{color_sequence}{style_sequence}m{cell_content}{SGR_RESET_ATTRIBUTES}",
                )
            }
        }
    }

    /// Convert a single line in the grid to a String.
    fn line_to_string(
        &self,
        row: usize,
        mut cols: Range<usize>,
        include_wrapped_wide: bool,
        include_esc_sequences: bool,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
        force_secrets_obfuscated: bool,
    ) -> Option<String> {
        let mut text = String::new();
        // If we have either 0 rows or 0 columns, we cannot have any content within the grid.
        // Thus, we return an empty string. This can happen when we have only newlines in a grid.
        if self.total_rows() == 0 || self.columns() == 0 {
            return None;
        }

        let grid_row = self.row(row)?;
        let row_length = min(grid_row.line_length(), cols.end + 1);

        // Include wide char when trailing spacer is selected.
        if grid_row
            .get(cols.start)
            .is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR_SPACER))
        {
            cols.start -= 1;
        }

        let mut tab_mode = false;
        let should_show_secrets = force_secrets_obfuscated
            || (respect_obfuscated_secrets == RespectObfuscatedSecrets::Yes
                && self.get_secret_obfuscation().is_visually_obfuscated());
        for col in IndexRange::from(cols.start..row_length) {
            let cell = grid_row.get(col);
            let Some(cell) = cell else {
                // If the cell doesn't exist for some reason, then we can break and
                // return a partially constructed string.
                break;
            };

            // Skip over cells until next tab-stop once a tab was found.
            if tab_mode {
                if self.ansi_handler_state.tabs[col] {
                    tab_mode = false;
                } else {
                    continue;
                }
            }

            if cell.c == '\t' {
                tab_mode = true;
            }

            if !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                // If this cell is part of an obfuscated secret, push the placeholder char '*'
                let mut obfuscated_char = false;
                if should_show_secrets {
                    if let Some((handle, _)) = self.secret_at_original_point(Point::new(row, col)) {
                        if self
                            .secret_by_handle(handle)
                            .is_some_and(Secret::is_obfuscated)
                        {
                            text.push('*');
                            obfuscated_char = true;
                        }
                    }
                }

                // If it's not obfuscated, push cell's primary character.
                if !obfuscated_char {
                    if include_esc_sequences {
                        text.push_str(&Self::cell_to_string(cell));
                    } else {
                        text.push_char_or_str(cell.content_for_display());
                    }
                }
            }
        }

        if cols.end >= self.columns() - 1
            && (row_length == 0
                || !grid_row
                    .get(row_length - 1)
                    .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE)))
        {
            // We need to include a carriage return specifically if we're encoding escape
            // sequences in addition to '\n' which is interpreted as a linefeed in the parser.
            // In most places (e.g. in any editor, OSX pasteboard, etc) this is unnecessary because
            // '\n' is interpreted as a newline. However, a terminal needs both a carriage return
            // and a linefeed to put the cursor on the first spot in a new line.
            if include_esc_sequences {
                text.push('\r');
            }
            text.push('\n');
        }

        // If wide char is not part of the selection, but leading spacer is, include it.
        if row_length == self.columns()
            && row_length >= 2
            && grid_row
                .get(row_length - 1)
                .is_some_and(|cell| cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER))
            && include_wrapped_wide
        {
            if let Some(row) = self.row(row - 1) {
                if let Some(cell) = row.get(0) {
                    text.push(cell.c);
                }
            }
        }

        Some(text)
    }

    /// Convert range between two points to a String.
    pub fn bounds_to_string(
        &self,
        start: Point,
        end: Point,
        include_esc_sequences: bool,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
        force_secrets_obfuscated: bool,
        respect_displayed_output: RespectDisplayedOutput,
    ) -> String {
        // If the selection rows are outside of the currently active grid,
        // just return an empty string.
        if end.row >= self.total_rows() {
            return String::new();
        }

        match (respect_displayed_output, self.displayed_output_rows()) {
            (RespectDisplayedOutput::Yes, Some(rows)) => {
                // The `start` and `end` objects are based on the displayed lines from the
                // grid. In other words, if the start/end point is at row i, then the bound
                // is at the i-th displayed line, not the i-th line of the full grid.
                let visible_rows = rows
                    .skip(start.row)
                    .take(end.row.saturating_sub(start.row) + 1);
                self.visible_rows_to_string(
                    start,
                    end,
                    visible_rows,
                    include_esc_sequences,
                    respect_obfuscated_secrets,
                    force_secrets_obfuscated,
                )
            }
            _ => self.visible_rows_to_string(
                start,
                end,
                start.row..(end.row + 1),
                include_esc_sequences,
                respect_obfuscated_secrets,
                force_secrets_obfuscated,
            ),
        }
    }

    fn visible_rows_to_string(
        &self,
        start: Point,
        end: Point,
        visible_rows: impl Iterator<Item = usize>,
        include_esc_sequences: bool,
        respect_obfuscated_secrets: RespectObfuscatedSecrets,
        force_secrets_obfuscated: bool,
    ) -> String {
        let mut res = String::new();

        for (offset, row) in visible_rows.enumerate() {
            let offset_row = start.row + offset;
            let start_col = if offset_row == start.row {
                start.col
            } else {
                0
            };
            let end_col = if offset_row == end.row {
                end.col
            } else {
                self.columns().saturating_sub(1)
            };

            res += &self
                .line_to_string(
                    row,
                    start_col..end_col,
                    offset_row == end.row,
                    include_esc_sequences,
                    respect_obfuscated_secrets,
                    force_secrets_obfuscated,
                )
                .unwrap_or_default();
        }

        // We only want to trim the newlines, if the output shouldn't include escape sequences.
        // Otherwise, we're keeping the newlines (partially, because our grid logic relies on
        // the newlines for resizing).
        if !include_esc_sequences {
            res.trim_trailing_newline();
        }

        res
    }

    /// Returns the boundary of the word at the given point.
    ///
    /// Words are separated by the file link separators.
    pub fn fragment_boundary_at_point(&self, point: &Point) -> FragmentBoundary {
        fn is_at_boundary(cell: &Cell) -> bool {
            is_file_link_separator(cell.c)
        }

        // Start by scanning backward.
        let mut cursor = self.grapheme_cursor_from(*point, grapheme_cursor::Wrap::None);
        // If the start point _is_ a boundary, it's the whole fragment.
        if matches!(cursor.current_item(), Some(item) if is_at_boundary(item.cell())) {
            let next_point = Point {
                row: point.row,
                col: point.col + 1,
            };
            return FragmentBoundary(*point..next_point);
        }
        while let Some(cursor_item) = cursor.current_item() {
            if is_at_boundary(cursor_item.cell()) {
                cursor.move_forward();
                break;
            }
            cursor.move_backward();
        }
        let fragment_start = cursor.last_valid_position();

        // Now, scan forwards.
        let mut cursor = self.grapheme_cursor_from(fragment_start, grapheme_cursor::Wrap::None);
        while let Some(cursor_item) = cursor.current_item() {
            if is_at_boundary(cursor_item.cell()) {
                cursor.move_backward();
                break;
            }
            cursor.move_forward();
        }
        let mut fragment_end = cursor.last_valid_position();
        fragment_end.col = min(fragment_end.col + 1, self.grid.columns());

        FragmentBoundary(fragment_start..fragment_end)
    }

    /// Return all possible file paths containing the grid point ordered from longest to shortest.
    pub fn possible_file_paths_at_point(&self, displayed_point: Point) -> Vec<PossiblePath> {
        let point = self.maybe_translate_point_from_displayed_to_original(displayed_point);
        let last_row_end_with_line_wrap = point.row > 0 && self.row_wraps(point.row - 1);
        let current_row_end_with_line_wrap =
            point.row + 1 < self.total_rows() && self.row_wraps(point.row);

        // All fragments in the row before the point (not including the point)
        // + Part of the fragments in the previous row if previous line ends with a line wrap.
        let mut prefix_chunks = match (point.col > 0, last_row_end_with_line_wrap) {
            // If the hovered point is not at column 0 and the last row ends with a linewrap,
            // we should take the fragments from the beginning of the line to current row
            // and the last couple of cells in the previous row. Note that we need to
            // concatenate the last fragment in the previous row with the first fragment
            // in the current row since they technically is one conherent fragment.
            (true, true) => {
                let mut prev_line_fragments = self.line_to_fragments(
                    point.row - 1,
                    self.columns().saturating_sub(LINK_NUM_CHARACTER_SCAN + 1)..self.columns() - 1,
                    IncludeFirstWideChar::Yes, /*should_scan_forward*/
                );

                let mut current_line_fragments =
                    self.line_to_fragments(point.row, 0..point.col - 1, IncludeFirstWideChar::Yes);

                match (prev_line_fragments.last(), current_line_fragments.first()) {
                    // Note that if any one of the two fragments has separator, we shouldn't
                    // concatenate them.
                    (Some(prev_line_fragment), Some(current_line_fragment))
                        if prev_line_fragment.has_separator()
                            || current_line_fragment.has_separator() =>
                    {
                        let mut fragment =
                            prev_line_fragments.pop().expect("Fragment should exist");

                        fragment.content.push_str(&current_line_fragment.content);
                        prev_line_fragments.push(Fragment {
                            content: fragment.content,
                            total_cell_width: fragment.total_cell_width
                                + current_line_fragment.total_cell_width,
                        });

                        current_line_fragments.remove(0);
                    }
                    _ => (),
                };

                prev_line_fragments.append(&mut current_line_fragments);
                prev_line_fragments
            }
            // If the previous line does not end with a linewrap, only parse for fragments in the current line.
            (true, false) => {
                self.line_to_fragments(point.row, 0..point.col - 1, IncludeFirstWideChar::Yes)
            }
            // If the point is at the start of the line and the previous line does end with a linewrap,
            // parse for fragments in the previous line.
            (false, true) => self.line_to_fragments(
                point.row - 1,
                self.columns().saturating_sub(LINK_NUM_CHARACTER_SCAN + 1)..self.columns() - 1,
                IncludeFirstWideChar::Yes, /*should_scan_forward*/
            ),
            (false, false) => Vec::new(),
        };

        // All fragments in the row after the point (including the point)
        // + Part of the fragments in the next row if the line ends with a line wrap.
        // Note that we set should_scan_forward here to false to prevent overlapping
        // width char characters between prefix and suffix.
        let suffix_chunks = match current_row_end_with_line_wrap {
            // If current line ends a line wrap, we parse for fragments in both the current and next line.
            true => {
                let mut current_line_fragments = self.line_to_fragments(
                    point.row,
                    point.col..self.columns() - 1,
                    IncludeFirstWideChar::No, /*should_scan_forward*/
                );

                let mut next_line_fragments = self.line_to_fragments(
                    point.row + 1,
                    0..(self.columns() - 1).min(LINK_NUM_CHARACTER_SCAN),
                    IncludeFirstWideChar::No,
                );

                match (current_line_fragments.last(), next_line_fragments.first()) {
                    (Some(current_line_fragment), Some(next_line_fragment))
                        if current_line_fragment.has_separator()
                            || next_line_fragment.has_separator() =>
                    {
                        let mut fragment =
                            current_line_fragments.pop().expect("Fragment should exist");

                        fragment.content.push_str(&next_line_fragment.content);
                        current_line_fragments.push(Fragment {
                            content: fragment.content,
                            total_cell_width: fragment.total_cell_width
                                + next_line_fragment.total_cell_width,
                        });

                        next_line_fragments.remove(0);
                    }
                    _ => (),
                };

                current_line_fragments.append(&mut next_line_fragments);
                current_line_fragments
            }
            false => self.line_to_fragments(
                point.row,
                point.col..self.columns() - 1,
                IncludeFirstWideChar::No, /*should_scan_forward*/
            ),
        };

        // This addresses the case when the file path starts from the point -- in this case
        // the valid path is entirely constructed from suffix chunks. Note that this is only possible
        // when the last fragment in prefix_chunks is a separator or prefix is empty. If it is true, pass
        // in a dummy fragment here so we could do one pass over all possible suffix fragment combinations first.
        let should_check_suffix = match prefix_chunks.last() {
            Some(last_prefix) => last_prefix
                .content
                .chars()
                .next()
                .map(is_file_link_separator)
                .unwrap_or(false),
            None => true,
        };

        if should_check_suffix {
            prefix_chunks.push(Fragment {
                content: "".to_string(),
                total_cell_width: 0,
            });
        }

        let mut left = String::new();
        let mut left_width = 0;
        let mut possible_paths = Vec::new();

        for prefix_chunk in prefix_chunks.into_iter().rev() {
            // Prepend a new fragment to left.
            left = format!("{}{}", prefix_chunk.content, left);
            left_width += prefix_chunk.total_cell_width;

            // Initialize a new right fragment.
            let mut right = String::new();
            let mut right_width = 0;

            for suffix_chunk in suffix_chunks.iter() {
                // Append a new fragment to right.
                right.push_str(suffix_chunk.content.as_str());
                right_width += suffix_chunk.total_cell_width;

                let possible_path = format!("{left}{right}");

                // On Windows, reject candidates with trailing whitespace
                // and candidates that are pure whitespace.
                // Both are accepted by the filesystem, so `PathBuf`'s
                // `is_dir()` returns true.
                #[cfg(windows)]
                {
                    let path_is_empty = possible_path.trim().is_empty();
                    let path_has_trailing_whitespace =
                        possible_path.trim_end().len() < possible_path.len();
                    if path_is_empty || path_has_trailing_whitespace {
                        continue;
                    }
                }

                // Need to expand the path here as built-in Path lib does not understand tilde.
                let expanded_path = shellexpand::tilde(&possible_path);

                // Scan for line and column number in the current fragment (left + right).
                let cleaned_path = CleanPathResult::with_line_and_column_number(&expanded_path);

                let starting_point =
                    self.advance_point_by_columns(point, left_width, false /*forward*/);
                let ending_point = self.advance_point_by_columns(
                    point,
                    right_width.saturating_sub(1),
                    true, /*forward*/
                );

                possible_paths.push(PossiblePath {
                    path: cleaned_path,
                    range: if self.has_displayed_output() {
                        let displayed_starting_point = self
                            .maybe_translate_point_from_original_to_displayed(starting_point);
                        let displayed_ending_point = self
                            .maybe_translate_point_from_original_to_displayed(ending_point);
                        if displayed_starting_point > displayed_ending_point {
                            log::error!(
                                "File path range translation to displayed points failed. Displayed range start {displayed_starting_point:?} is greater than displayed range end {displayed_ending_point:?}"
                            );
                            starting_point..=ending_point
                        } else {
                            displayed_starting_point..=displayed_ending_point
                        }
                    } else {
                        starting_point..=ending_point
                    },
                })
            }
        }

        possible_paths.reverse();
        possible_paths
    }

    fn advance_point_by_columns(&self, mut point: Point, count: usize, forward: bool) -> Point {
        if count == 0 {
            return point;
        }
        let mut total_count = 1;

        if forward {
            point = point.wrapping_add(self.columns(), 1);
            while self.cell_type(point).is_some() {
                if total_count == count {
                    break;
                }

                total_count += 1;
                point = point.wrapping_add(self.columns(), 1);
            }
        } else {
            point = point.wrapping_sub(self.columns(), 1);
            while self.cell_type(point).is_some() {
                if total_count == count {
                    break;
                }

                total_count += 1;
                point = point.wrapping_sub(self.columns(), 1);
            }
        }

        point
    }

    // Chunk line into an array of fragments with the separators.
    fn line_to_fragments(
        &self,
        row: usize,
        mut cols: Range<usize>,
        include_first_wide_char: IncludeFirstWideChar,
    ) -> Vec<Fragment> {
        let grid_line = match self.row(row) {
            Some(row) => row,
            None => return Vec::new(),
        };

        let mut fragments = Vec::new();
        let line_length = min(grid_line.line_length(), cols.end + 1);

        let mut last_fragment = String::new();
        let mut last_fragment_length = 0;

        // Include wide char when trailing spacer is selected and should_scan_forward is true.
        if matches!(include_first_wide_char, IncludeFirstWideChar::Yes)
            && grid_line
                .get(cols.start)
                .is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR_SPACER))
        {
            cols.start -= 1;
        }

        let mut tab_cell_count = 0;
        for col in IndexRange::from(cols.start..line_length) {
            let cell = match grid_line.get(col) {
                Some(cell) => cell,
                None => return Vec::new(),
            };

            // Skip over cells until next tab-stop once a tab was found.
            if tab_cell_count > 0 {
                if self.ansi_handler_state.tabs[col] {
                    if !last_fragment.is_empty() {
                        fragments.push(Fragment {
                            content: last_fragment.clone(),
                            total_cell_width: tab_cell_count,
                        });

                        last_fragment.clear();
                        last_fragment_length = 0;
                    }
                    tab_cell_count = 0;
                } else {
                    tab_cell_count += 1;
                    continue;
                }
            }

            if cell.c == '\t' {
                tab_cell_count = 1;
            }

            if !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                // If is a separator, we push the last fragment to the vector and push
                // the separator as its own fragment.
                if is_file_link_separator(cell.c) {
                    if !last_fragment.is_empty() {
                        let mut fragment_text = String::new();
                        mem::swap(&mut fragment_text, &mut last_fragment);

                        fragments.push(Fragment {
                            content: fragment_text,
                            total_cell_width: last_fragment_length,
                        });

                        last_fragment.clear();
                        last_fragment_length = 0;
                    }

                    fragments.push(Fragment {
                        content: cell.c.into(),
                        total_cell_width: UnicodeWidthChar::width(cell.c).unwrap_or(1),
                    });
                // Otherwise we append the current cell to the last fragment
                } else {
                    // Push zero-width characters, if any, otherwise simply push the Cell's character.
                    last_fragment.push_char_or_str(cell.content_for_display());
                    last_fragment_length += 1;
                }
            // The character_cells could be empty if the first cell is a WIDE_CHAR_SPACER
            // and should_scan_foward is set to false.
            } else if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
                || (cell.flags.contains(Flags::WIDE_CHAR_SPACER) && !last_fragment.is_empty())
            {
                last_fragment_length += 1
            }
        }

        // We need this to make sure the last cell's cell_width is correct. Line length
        // only gives us the length of cells that are non-empty and since WIDE_CHAR_SPACER
        // is empty, we will ignore the spacer length in the last cell without this check.
        if line_length < grid_line.len()
            && !last_fragment.is_empty()
            && grid_line
                .get(line_length)
                .is_some_and(|cell| cell.flags.contains(Flags::WIDE_CHAR_SPACER))
        {
            last_fragment_length += 1;
        }

        if !last_fragment.is_empty() {
            fragments.push(Fragment {
                content: last_fragment.clone(),
                total_cell_width: last_fragment_length,
            });
        }

        fragments
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.ansi_handler_state.cursor_style
    }

    pub(in crate::terminal::model) fn scroll_region(&self) -> &Range<VisibleRow> {
        &self.ansi_handler_state.scroll_region
    }

    pub fn cursor_point(&self) -> Point {
        let cursor_point = self.grid.cursor.point;
        Point::new(self.history_size() + cursor_point.row.0, cursor_point.col)
    }

    /// This should be used when you want to render the cursor, because it accounts for marked text.
    pub fn cursor_render_point(&self) -> Point {
        let cursor_point = self.cursor_point();
        cursor_point.wrapping_add(self.columns(), self.marked_text_cell_length())
    }

    /// Updates the cursor point to be at the provided row and column.
    #[cfg(test)]
    pub(super) fn set_cursor_point(&mut self, row: usize, col: usize) {
        self.update_cursor(|cursor| {
            cursor.point.row = VisibleRow(row);
            cursor.point.col = col;
        });
    }

    /// Determines if the rendered cursor is on a wide character, accounting for marked text.
    pub fn is_cursor_on_wide_char(&self) -> bool {
        let cursor_render_point = self.cursor_render_point();
        self.cell_type(cursor_render_point) == Some(CellType::WideChar)
    }

    /// Updates the active [`Cursor`] using the provided `update_cursor_fn`.
    pub fn update_cursor<T: FnOnce(&mut Cursor)>(&mut self, update_cursor_fn: T) {
        // Update state before updating the cursor, in case the cursor
        // moves backwards.
        self.grid.update_max_cursor();
        self.update_dirty_cells_range();

        (update_cursor_fn)(&mut self.grid.cursor);

        // Update the range again after moving the cursor, in case we need to
        // update the start point of the range.
        self.update_dirty_cells_range();
    }

    /// Updates the active [`Cursor`] using the provided `update_cursor_fn`,
    /// but must only be used to move the cursor forward.
    #[inline]
    fn move_cursor_forward<T: FnOnce(&mut Cursor)>(&mut self, update_cursor_fn: T) {
        #[cfg(debug_assertions)]
        let old_cursor_point = self.cursor_point();

        self.move_cursor_forward_unchecked(update_cursor_fn);

        #[cfg(debug_assertions)]
        assert!(
            self.cursor_point() >= old_cursor_point,
            "cursor should not move backwards in move_cursor_forward callback"
        );
    }

    /// Updates the active [`Cursor`] using the provided `update_cursor_fn`,
    /// but must only be used to move the cursor forward.
    ///
    /// This does not perform assertions that the cursor did not move
    /// backwards.
    #[inline]
    fn move_cursor_forward_unchecked<T: FnOnce(&mut Cursor)>(&mut self, update_cursor_fn: T) {
        (update_cursor_fn)(&mut self.grid.cursor);
    }

    #[cfg(test)]
    fn set_max_scroll_limit(&mut self, max_scroll_limit: usize) {
        self.flat_storage.set_max_rows(Some(max_scroll_limit));
    }

    /// Returns an inclusive range of cells that have been dirtied in the
    /// current pty output processing pass, or [`None`] if the range is
    /// empty.
    pub(in crate::terminal::model) fn dirty_cells_range(&self) -> Option<RangeInclusive<Point>> {
        let range = &self.ansi_handler_state.dirty_cells_range;
        if range.is_empty() {
            return None;
        }

        let start = range.start;
        // We need to make an inclusive range, and we know the existing range
        // is non-empty, so it's safe to subtract 1 from the end to get the
        // inclusive range points.
        let end = range.end.wrapping_sub(self.columns(), 1);

        Some(start..=end)
    }

    /// Updates the dirty cells range based on the current cursor position.
    fn update_dirty_cells_range(&mut self) {
        let mut cursor_point = self.cursor_point();
        // If the cursor is ready to wrap to the next line, the cell under it
        // should be included in the range.
        if self.grid.cursor.input_needs_wrap {
            cursor_point = cursor_point.wrapping_add(self.columns(), 1);
        }

        let dirty_cells_range = &mut self.ansi_handler_state.dirty_cells_range;

        let range_start = min(dirty_cells_range.start, cursor_point);
        let range_end = max(dirty_cells_range.end, cursor_point);
        *dirty_cells_range = range_start..range_end;

        // Also update the find dirty rows range.
        self.update_find_dirty_rows_range(cursor_point.row);
    }

    /// Updates the find dirty rows range to include the given row.
    ///
    /// This accumulates dirty rows across multiple byte processing passes
    /// until find explicitly consumes them.
    fn update_find_dirty_rows_range(&mut self, row: usize) {
        self.find_dirty_rows_range = Some(match &self.find_dirty_rows_range {
            Some(existing) => {
                let start = min(*existing.start(), row);
                let end = max(*existing.end(), row);
                start..=end
            }
            None => row..=row,
        });
    }

    /// Returns the accumulated dirty row range for find operations, if any.
    ///
    /// Unlike `dirty_cells_range()`, this range accumulates across multiple byte
    /// processing passes until explicitly consumed with `take_find_dirty_rows_range()`.
    pub fn find_dirty_rows_range(&self) -> Option<RangeInclusive<usize>> {
        self.find_dirty_rows_range.clone()
    }

    /// Returns and clears the accumulated dirty row range for find operations.
    ///
    /// This should be called when find has processed the dirty range and no longer
    /// needs to track those rows as dirty.
    pub fn take_find_dirty_rows_range(&mut self) -> Option<RangeInclusive<usize>> {
        self.find_dirty_rows_range.take()
    }

    fn reset_dirty_cells_range_to_cursor_point(&mut self) {
        let cursor_point_offset = self.cursor_point();
        // Initialize the dirty cells range to be the current point of the cursor, since this is
        // where the cursor will be upon the next run of the PTY event loop.
        self.ansi_handler_state.dirty_cells_range = cursor_point_offset..cursor_point_offset;
    }

    /// Returns `true` if the row has trailing whitespace or empty cells starting from `col`,
    /// inclusive.
    #[inline]
    pub fn row_has_trailing_whitespace(&self, row: usize, col: usize) -> bool {
        if row >= self.grid.total_rows() {
            return false;
        }

        let mut cursor =
            self.grapheme_cursor_from(Point::new(row, col), grapheme_cursor::Wrap::None);
        while let Some(item) = cursor.current_item() {
            if !item.content_char().is_whitespace() {
                return false;
            }

            cursor.move_forward()
        }
        true
    }

    /// Returns the index of the last row in the grid which received any output
    /// from the PTY.
    pub fn max_content_row(&self) -> usize {
        // We use the max cursor row as the end of the grid. If the max cursor
        // point is at the start of its line, we subtract 1 to avoid including
        // an empty line in the non-matching line ranges.
        let max_cursor_point = self.grid.max_cursor_point;
        let is_last_row_empty = max_cursor_point.col == 0;
        max_cursor_point
            .row
            .convert_to_absolute(self)
            .saturating_sub(is_last_row_empty as usize)
    }

    pub fn cell_height(&self) -> usize {
        self.ansi_handler_state.cell_height
    }

    /// The number of lines that have been truncated due to exceeding the
    /// grid's maximum scrollback limit.
    pub fn num_lines_truncated(&self) -> u64 {
        self.flat_storage.num_truncated_rows()
    }

    /// Finishes the grid.
    ///
    /// After this is called, the grid contents should be considered immutable.
    /// Note that the "layout" of the grid's content can change due to resizes.
    pub fn finish(&mut self) {
        self.finished = true;

        // Clear out all content in the grid following the cursor.
        self.truncate_to_cursor_rows();
        self.reset_trailing_cells_in_cursor_row();

        // Set the max cursor to equal the cursor, as we've dropped all content
        // after the cursor.
        self.grid.max_cursor_point = self.grid.cursor.point;

        // Make sure we finish processing any data that was added to this grid.
        // We can't rely on this being done by the PTY reader thread, as if a
        // new block is started as part of the same PTY read, we'll only end up
        // calling the finish byte processing hook on _that_ block.
        self.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

        // If we're using flat storage, push as many rows as possible into
        // flat storage to minimize memory consumption.
        if FeatureFlag::MaximizeFlatStorage.is_enabled() {
            self.resize_storage(1, self.columns());
        }
    }

    /// Returns the total number of rows that _precede_ the row containing the
    /// cursor.
    pub fn rows_to_cursor(&self) -> usize {
        self.grid.cursor.point.row.0 + self.history_size()
    }

    /// Truncate all rows after the cursor's row from the Grid
    ///
    /// This is primarily used when a block is finished, to drop the excess rows that are not part
    /// of the output.
    pub fn truncate_to_cursor_rows(&mut self) {
        // Normally, when using only grid storage, truncating rows after the
        // cursor naturally pulls some rows into the visible region of the
        // grid from grid storage scrollback.  When scrollback is stored in
        // flat storage, however, we need to move those extra rows into grid
        // storage pre-truncation, after which the grid storage truncation
        // logic will move them into the visible region of grid storage.
        safe_assert_eq!(
            self.grid.history_size(),
            0,
            "should not have any rows in grid storage scrollback"
        );

        let num_visible_rows_after_cursor = self
            .visible_rows()
            .saturating_sub(self.grid.cursor.point.row.0 + 1);
        // Move rows from flat storage into grid storage to fill
        // up as much of the below-cursor gap as possible.
        let rows_to_push = self.flat_storage.pop_rows(num_visible_rows_after_cursor);
        let num_rows_to_push = rows_to_push.len();
        if num_rows_to_push > 0 {
            safe_assert!(
                self.grid.total_rows() == self.grid.visible_rows(),
                "should have a full visible region if we had rows in flat storage"
            );
            self.grid.raw.push_from_scrollback(rows_to_push);
        }
        safe_assert_eq!(
            self.grid.history_size(),
            num_rows_to_push,
            "grid storage scrollback should contain the rows pulled from flat storage"
        );

        self.grid.truncate_to_cursor_rows();

        safe_assert_eq!(
            self.grid.history_size(),
            0,
            "should not have any rows in grid storage scrollback"
        );

        // We need to truncate the the rows that we have marked to be displayed.
        let cursor_row = self.rows_to_cursor();
        if let Some(displayed_output) = self.displayed_output.as_mut() {
            displayed_output.truncate_to_row(cursor_row);
        }
    }

    /// Truncate all columns to the right of the cursor (including the column of the cursor, if the cursor is
    /// not at the end of a line that needs to wrap for the next character).
    ///
    /// This is primarily used for the case of same-line prompt, where we want to truncate the
    /// grid, for creating the editor decorator elements that contain the prompts, however, we want
    /// to preserve user-inserted whitespace from their PS1. Note that this method largely only
    /// makes sense to call in the context of a 1 row grid (we are cutting off columns in every
    /// row).
    pub fn truncate_to_cursor_cols(&mut self) {
        self.grid.truncate_to_cursor_cols();
    }

    /// Resets all cells after the cursor in the cursor row to the default cell.
    fn reset_trailing_cells_in_cursor_row(&mut self) {
        let cursor_point = self.grid.cursor.point;
        let row = &mut self.grid[cursor_point.row];
        for col in (cursor_point.col + 1)..row.occ {
            row[col] = Cell::default();
        }
        row.occ = cursor_point.col;
    }

    /// Find next matching bracket.
    pub fn bracket_search(&self, point: Point) -> Option<Point> {
        let point = self.maybe_translate_point_from_displayed_to_original(point);
        let start_char = self
            .grid
            .get(point.row)
            .and_then(|row| row.get(point.col))?
            .c;

        // Find the matching bracket we're looking for
        let (forward, end_char) = BRACKET_PAIRS.iter().find_map(|(open, close)| {
            if open == &start_char {
                Some((true, *close))
            } else if close == &start_char {
                Some((false, *open))
            } else {
                None
            }
        })?;

        let mut cursor = self.grapheme_cursor_from(point, grapheme_cursor::Wrap::All);

        // For every character match that equals the starting bracket, we
        // ignore one bracket of the opposite type.
        let mut skip_pairs = 0;

        loop {
            // Check the next cell
            if forward {
                cursor.move_forward();
            } else {
                cursor.move_backward();
            };

            // Break if there are no more cells
            let cursor_item = match cursor.current_item() {
                Some(item) => item,
                None => break,
            };

            // Check if the bracket matches
            if cursor_item.cell().c == end_char && skip_pairs == 0 {
                return Some(cursor_item.point());
            } else if cursor_item.cell().c == start_char {
                skip_pairs += 1;
            } else if cursor_item.cell().c == end_char {
                skip_pairs -= 1;
            }
        }

        None
    }

    /// Connects the grid to the SemanticSelection model. There is also a EditorView::smart_select
    /// equivalent. The `cursor_point` is the Point on the grid where the cursor clicked to
    /// initiate the selection. Since smart-select operates on byte-indices, we need to convert
    /// from Point to byte-index and back.
    pub fn smart_search(
        &self,
        cursor_point: Point,
        selection: &SemanticSelection,
    ) -> Option<(Point, Point)> {
        let word_start = self.nonblank_word_bound_before_point(cursor_point).ok()?;
        let word_end = self.nonblank_word_bound_after_point(cursor_point).ok()?;
        let nonblank_word = self.bounds_to_string(
            word_start,
            word_end,
            false,
            RespectObfuscatedSecrets::No,
            false, /* force_obfuscated_secrets */
            RespectDisplayedOutput::Yes,
        );
        let offset = self.byte_offset_between_points(cursor_point, word_start);
        selection.smart_search(&nonblank_word, offset).map(|range| {
            (
                self.advance_point_by_bytes(word_start, range.start),
                // need to subtract 1 here because the terminal model selection range is inclusive
                // of the end point (unlike the editor which is exclusive)
                self.advance_point_by_bytes(word_start, range.end)
                    .wrapping_sub(self.columns(), 1),
            )
        })
    }

    /// How many bytes are traversed when advancing from one point to another. This is needed in
    /// order to correctly account for characters containing wide-spacers and for unicode multibyte
    /// characters
    fn byte_offset_between_points(&self, mut start: Point, mut end: Point) -> ByteOffset {
        if start > end {
            mem::swap(&mut start, &mut end);
        }

        let mut cursor = self.grapheme_cursor_from(start, grapheme_cursor::Wrap::All);
        let mut byte_offset = 0;

        while let Some(current_item) = cursor.current_item() {
            if current_item.point() >= end {
                break;
            }
            byte_offset += current_item.cell().c.len_utf8();
            cursor.move_forward();
        }
        ByteOffset::from(byte_offset)
    }

    /// The byte-offset equivalent of Self::advance_point_by_columns. This accounts for each byte
    /// in multibyte characters.
    /// TODO(andy): Update this to bias left or right. Currently it only biases right.
    /// TODO(andy): Update this to go forward or backward. Currently it only goes forward.
    fn advance_point_by_bytes(&self, start: Point, bytes: ByteOffset) -> Point {
        let mut bytes = bytes.as_usize();
        let mut cursor = self.grapheme_cursor_from(start, grapheme_cursor::Wrap::All);

        while let Some(current_item) = cursor.current_item() {
            if bytes == 0 {
                break;
            }
            bytes = bytes.saturating_sub(current_item.cell().c.len_utf8());
            cursor.move_forward();
        }

        cursor.last_valid_position()
    }

    /// Returns the start of the "word" containing the current point, using
    /// whitespace as a word delimiter.
    ///
    /// This will only walk backwards up to 500 cells, for performance reasons.
    fn nonblank_word_bound_before_point(&self, point: Point) -> anyhow::Result<Point> {
        let point = self.maybe_translate_point_from_displayed_to_original(point);

        let mut left_cursor = self.grapheme_cursor_from(point, grapheme_cursor::Wrap::All);
        if left_cursor.current_item().is_none() {
            anyhow::bail!("point is not valid");
        }
        let mut left_window_size = 0;
        let mut start = left_cursor.last_valid_position();
        while let Some(cursor_item) = left_cursor.current_item() {
            if char::is_whitespace(cursor_item.content_char())
                || left_window_size >= SMART_SELECT_MATCH_WINDOW_LIMIT
            {
                break;
            }
            start = left_cursor.last_valid_position();
            left_cursor.move_backward();
            left_window_size += 1;
        }

        Ok(self.maybe_translate_point_from_original_to_displayed(start))
    }

    /// Returns the end of the "word" containing the current point, using
    /// whitespace as a word delimiter.
    ///
    /// This will only walk forwards up to 500 cells, for performance reasons.
    fn nonblank_word_bound_after_point(&self, point: Point) -> anyhow::Result<Point> {
        let point = self.maybe_translate_point_from_displayed_to_original(point);

        let mut cursor = self.grapheme_cursor_from(point, grapheme_cursor::Wrap::All);
        let mut window_size = 0;
        let mut end = cursor.last_valid_position();
        while let Some(cursor_item) = cursor.current_item() {
            if char::is_whitespace(cursor_item.content_char())
                || window_size >= SMART_SELECT_MATCH_WINDOW_LIMIT
            {
                break;
            }
            end = cursor.last_valid_position();
            cursor.move_forward();
            window_size += 1;
        }

        Ok(self.maybe_translate_point_from_original_to_displayed(end))
    }

    /// Find left end of semantic block.
    pub fn semantic_search_left<F>(&self, point: Point, is_word_boundary_char: F) -> Point
    where
        F: Fn(char) -> bool,
    {
        let mut point = self.maybe_translate_point_from_displayed_to_original(point);
        // Limit the starting point to the last line in the history
        point.row = min(point.row, self.total_rows() - 1);

        let mut cursor = self.grapheme_cursor_from(point, grapheme_cursor::Wrap::Soft);

        cursor.move_backward();
        while let Some(cursor_item) = cursor.current_item() {
            if is_word_boundary_char(cursor_item.cell().c) {
                cursor.move_forward();
                break;
            }
            cursor.move_backward();
        }
        let point = cursor.last_valid_position();

        self.maybe_translate_point_from_original_to_displayed(point)
    }

    /// Find right end of semantic block.
    pub fn semantic_search_right<F>(&self, point: Point, is_word_boundary_char: F) -> Point
    where
        F: Fn(char) -> bool,
    {
        let mut point = self.maybe_translate_point_from_displayed_to_original(point);
        // Limit the starting point to the last line in the history
        point.row = min(point.row, self.total_rows() - 1);

        let mut cursor = self.grapheme_cursor_from(point, grapheme_cursor::Wrap::Soft);

        cursor.move_forward();
        while let Some(cursor_item) = cursor.current_item() {
            if is_word_boundary_char(cursor_item.cell().c) {
                cursor.move_backward();
                break;
            }
            cursor.move_forward();
        }
        let point = cursor.last_valid_position();

        self.maybe_translate_point_from_original_to_displayed(point)
    }

    /// Find the start of the current logical line across linewraps.
    pub fn line_search_left(&self, mut point: Point) -> Point {
        while point.row > 0 && self.row_wraps(point.row - 1) {
            point.row -= 1;
        }

        point.col = 0;

        point
    }

    /// Find the end of the current logical line across linewraps.
    pub fn line_search_right(&self, mut point: Point) -> Point {
        while point.row < self.total_rows() && self.row_wraps(point.row) {
            point.row += 1;
        }

        point.col = self.columns() - 1;

        point
    }

    /// Find all matches in this grid, starting from the last row, last column of the grid.
    pub fn find<'a>(&'a self, dfas: &'a RegexDFAs) -> RegexIter<'a> {
        let start = Point::new(0, 0);
        let end_point = Point::new(self.total_rows() - 1, self.columns() - 1);
        let end = self.line_search_right(end_point);

        self.regex_iter(end, start, Direction::Left, dfas)
    }

    /// Find matches in a specific range of the grid.
    ///
    /// This is used by async find to scan chunks of a grid without scanning the entire grid.
    pub(in crate::terminal) fn find_in_range<'a>(
        &'a self,
        dfas: &'a RegexDFAs,
        start: Point,
        end: Point,
    ) -> RegexIter<'a> {
        self.regex_iter(end, start, Direction::Left, dfas)
    }

    /// Find the next regex match to the left of the `right` Point by searching leftwards from
    /// `right` until the `left` Point is reached.
    ///
    /// The origin is always included in the regex.
    fn regex_search_leftwards(&self, dfas: &RegexDFAs, right: Point, left: Point) -> Option<Match> {
        dfas.regex_search_leftwards(right, left, self)
    }

    /// Find the next regex match to the right of the origin point by beginning at the `left` Point
    /// and searching until the `right` Point is reached, inclusive of both points.
    ///
    /// The origin is always included in the regex.
    fn regex_search_rightwards(
        &self,
        dfas: &RegexDFAs,
        left: Point,
        right: Point,
    ) -> Option<Match> {
        dfas.regex_search_rightwards(left, right, self)
    }

    /// Computes the [`Point`] that `point` would be if
    /// the grid had no wrapping. In essence, this computes
    /// a grid-agnostic point.
    ///
    /// See [`Grid::compatible_point`] for its counterpart.
    ///
    /// Example: suppose the grid was
    ///    0123456789
    ///   ┌──────────┐
    /// 0 │This is a │
    /// 1 │wrap.     │
    /// 2 |Short line│
    /// 3 │Another lo│
    /// 4 │ng line.  │
    ///   └──────────┘
    ///
    ///    0123456789 abcdefgh
    ///   ┌──────────┐
    /// 0 │This is a │wrap.
    /// 1 |Short line│
    /// 2 |Another lo│ng line.
    ///   └──────────┘
    ///
    /// The "unwrapped point" of (4, 0) would
    /// be (2, a).
    ///
    /// TODO: this is an O(r) operation, where r is the number
    /// of rows in the grid. Consider optimizing this.
    pub fn grid_agnostic_point(&self, point: Point) -> Point {
        // Find what line this point should be on if it's on a wrapped line.
        // This will also tell us the absolute column position.
        let mut start = self.line_search_left(point);
        start.col = point.col + (self.columns() * (point.row - start.row));

        // For any wrapped line, we need to adjust our row position
        // by 1, because we want to produce a point over an unwrapped grid.
        let mut row = start.row;
        while row > 0 {
            if self.row_wraps(row - 1) {
                start.row -= 1;
            }
            row -= 1;
        }

        start
    }

    /// Computes a [`Point`] that is compatible with
    /// the grid's size (rows and columns). This is useful
    /// when trying to adapt a point from a different
    /// grid to this grid.
    ///
    /// See [`Grid::grid_agnostic_point`] for its counterpart.
    ///
    /// Example: suppose the point was (2, a)
    /// and came from the following grid:
    ///
    ///    0123456789 abcdefgh
    ///   ┌──────────┐
    /// 0 │This is a │wrap.
    /// 1 |Short line│
    /// 2 |Another lo│ng line.
    ///   └──────────┘
    ///
    /// If our grid is:
    ///
    ///    0123456789
    ///   ┌──────────┐
    /// 0 │This is a │
    /// 1 │wrap.     │
    /// 2 |Short line│
    /// 3 │Another lo│
    /// 4 │ng line.  │
    ///   └──────────┘
    ///
    /// then the resultant point would be (4, 0).
    ///
    /// TODO: this is an O(r) operation, where r is the number
    /// of rows in the grid. Consider optimizing this.
    pub fn compatible_point(&self, point: Point) -> Point {
        // Find the corresponding row in the grid for the
        // unwrapped `point.row`.
        let mut row = 0;
        let mut num_unwrapped_rows_so_far = 0;
        while row < self.total_rows() {
            if point.row == num_unwrapped_rows_so_far {
                break;
            } else if !self.row_wraps(row) {
                num_unwrapped_rows_so_far += 1;
            }
            row += 1;
        }

        // Adjust the column to make sure it fits within the grid column limit.
        // If we're over by a whole column size, we need to move to the next row.
        let mut col = point.col;
        while col >= self.columns() && row < self.total_rows() && self.row_wraps(row) {
            row += 1;
            col -= self.columns();
        }

        Point { row, col }
    }

    /// Determine if bracketed paste is needed
    pub fn needs_bracketed_paste(&self) -> bool {
        self.ansi_handler_state
            .mode
            .contains(TermMode::BRACKETED_PASTE)
    }

    /// Set the active keyboard mode.
    ///
    /// This is the single entry point for changing which keyboard flags are
    /// active. It updates the `keyboard_mode` field and syncs the
    /// corresponding TermMode bitflags. It does **not** touch the
    /// push/pop stack — `push_keyboard_mode` and `pop_keyboard_modes`
    /// manage the stack and then delegate here.
    pub fn set_keyboard_mode(&mut self, mode: KeyboardModes, apply: KeyboardModesApplyBehavior) {
        let current = self.ansi_handler_state.keyboard_mode;
        let new_mode = match apply {
            KeyboardModesApplyBehavior::Replace => mode,
            KeyboardModesApplyBehavior::Union => current | mode,
            KeyboardModesApplyBehavior::Difference => current & !mode,
        };

        self.ansi_handler_state.keyboard_mode = new_mode;

        // Sync TermMode bitflags to match the new active mode.
        self.ansi_handler_state.mode &= !TermMode::KEYBOARD_PROTOCOL;
        self.ansi_handler_state.mode |= TermMode::from(new_mode);
    }

    /// Push a keyboard mode onto the stack and make it active.
    pub fn push_keyboard_mode(&mut self, mode: KeyboardModes) {
        self.ansi_handler_state.keyboard_mode_stack.push_back(mode);
        self.set_keyboard_mode(mode, KeyboardModesApplyBehavior::Replace);
    }

    /// Pop keyboard modes from the stack, restoring the previous mode.
    pub fn pop_keyboard_modes(&mut self, count: u16) {
        let removals = (count as usize).min(self.ansi_handler_state.keyboard_mode_stack.len());
        for _ in 0..removals {
            self.ansi_handler_state.keyboard_mode_stack.pop_back();
        }
        // Restore the mode at the new top of the stack, or NO_MODE if empty.
        let mode = self
            .ansi_handler_state
            .keyboard_mode_stack
            .back()
            .copied()
            .unwrap_or(KeyboardModes::NO_MODE);
        self.set_keyboard_mode(mode, KeyboardModesApplyBehavior::Replace);
    }

    /// Reset keyboard mode state to defaults.
    pub fn reset_keyboard_mode_state(&mut self) {
        self.ansi_handler_state.keyboard_mode_stack =
            BoundedVecDeque::new(KEYBOARD_MODE_STACK_MAX_DEPTH);
        self.set_keyboard_mode(KeyboardModes::NO_MODE, KeyboardModesApplyBehavior::Replace);
    }

    /// Converts the given grid row into a storage row.
    ///
    /// Returns [`None`] if the the provided row index is outside of the bounds
    /// of the grid.
    ///
    /// This should be invoked by any logic that needs to interact with the
    /// underlying storage, to know which row to request from within which
    /// storage backend.
    fn storage_row(&self, row_idx: usize) -> Option<StorageRow> {
        if row_idx >= self.total_rows() {
            return None;
        }

        if let Some(grid_row_idx) = row_idx.checked_sub(self.flat_storage.total_rows()) {
            Some(StorageRow::GridStorage(grid_row_idx))
        } else {
            Some(StorageRow::FlatStorage(row_idx))
        }
    }

    /// Returns the grid row at the given index, if the index is within the
    /// bounds of the grid.
    ///
    /// TODO(vorporeal): Unify this with self.grid() (requires modifications to
    /// GraphemeCursor).
    pub fn row(&self, index: usize) -> Option<Cow<'_, Row>> {
        match self.storage_row(index)? {
            StorageRow::GridStorage(index) => Some(Cow::Borrowed(self.grid.get(index)?)),
            StorageRow::FlatStorage(index) => {
                let mut row_iter = self.flat_storage.rows_from(index);
                let row = row_iter.next()?;
                // Drop the iterator so that we can take ownership of the data
                // from within the `Rc<Row>`.
                drop(row_iter);
                Some(Cow::Owned(std::rc::Rc::into_inner(row)?))
            }
        }
    }

    pub(in crate::terminal::model) fn row_wraps(&self, row_idx: usize) -> bool {
        match self.storage_row(row_idx) {
            Some(StorageRow::GridStorage(row_idx)) => self.grid.row_wraps(row_idx),
            Some(StorageRow::FlatStorage(row_idx)) => self.flat_storage.row_wraps(row_idx),
            None => false,
        }
    }

    /// Returns the type of the cell at the given point.
    ///
    /// Returns [`None`] if the given point is not within the bounds of the
    /// grid.
    pub(in crate::terminal::model) fn cell_type(&self, point: Point) -> Option<CellType> {
        match self.storage_row(point.row)? {
            StorageRow::GridStorage(row_idx) => {
                let row = self.grid.get(row_idx)?;
                row.get(point.col).map(CellType::from)
            }
            StorageRow::FlatStorage(row_idx) => self.flat_storage.cell_type(row_idx, point.col),
        }
    }

    /// Jump to the end of a wide cell.
    fn expand_wide(&self, mut point: Point, direction: Direction) -> Point {
        let cell_type = self.cell_type(point);

        match direction {
            Direction::Right if matches!(cell_type, Some(CellType::LeadingWideCharSpacer)) => {
                point.col = 1;
                point.row += 1;
            }
            Direction::Right if matches!(cell_type, Some(CellType::WideChar)) => point.col += 1,
            Direction::Left
                if matches!(
                    cell_type,
                    Some(CellType::WideChar) | Some(CellType::WideCharSpacer)
                ) =>
            {
                if matches!(cell_type, Some(CellType::WideCharSpacer)) {
                    point.col -= 1;
                }

                let prev = point.wrapping_sub(self.columns(), 1);
                let prev_cell_type = self.cell_type(prev);
                if matches!(prev_cell_type, Some(CellType::LeadingWideCharSpacer)) {
                    point = prev;
                }
            }
            _ => (),
        }

        point
    }

    pub(in crate::terminal::model) fn has_visible_chars(&self) -> bool {
        let mut cursor = self.grapheme_cursor_from(Point::new(0, 0), grapheme_cursor::Wrap::All);
        while let Some(current_item) = cursor.current_item() {
            if current_item.cell().is_visible() {
                return true;
            }
            cursor.move_forward();
        }
        false
    }

    /// Append the contents of another grid to this one, cell-by-cell.
    pub(in crate::terminal::model) fn append_cells_from_grid(&mut self, other: &GridHandler) {
        let max_point = other.grid.max_cursor_point.convert_to_absolute(other);

        // We ensure that we don't copy "extra" empty cells over - only copy till the end of the "real content".
        // This ensures the user does not see "extra lines" in the typeahead blocks.
        // TODO(CORE-1847): explore if we can remove this logic and simply rely on the cursor (need to fix
        // the cursor position for typeahead block first).
        let bottommost_nonempty_other = other.bottommost_nonempty_row();
        let rightmost_nonempty_other = other.rightmost_nonempty_cell(None);
        let max_row = min(max_point.row, bottommost_nonempty_other.unwrap_or(0));
        let max_col = min(
            other.columns().saturating_sub(1),
            rightmost_nonempty_other.unwrap_or(0),
        );

        // Iterate through the "other" Grid and copy over cells one-by-one.
        for row_idx in 0..=max_row {
            let row = other.row(row_idx).expect("row should exist");
            for col in 0..=max_col {
                // Move cursor to next line, if needed (in combined grid).
                if self.grid.cursor().input_needs_wrap {
                    self.wrapline();
                }
                // Copy over the current cell from the other grid to the current cursor position in the combined grid.
                let cur_cursor_point = self.grid.cursor_point();
                let cell_to_copy = &row[col];
                let mut cloned_cell = cell_to_copy.clone();
                // We want the command content to be bolded. Note that we cannot do this on the cursor-level, since
                // we are cloning cells NOT inputting characters with the cursor.
                cloned_cell.flags_mut().insert(Flags::BOLD);
                self.grid[cur_cursor_point.row][cur_cursor_point.col] = cloned_cell;

                // Move the cursor onto the next cell (in combined grid).
                if self.grid.cursor_point().col >= self.columns() - 1 {
                    // We need to move the cursor to the next line.
                    self.update_cursor(|cursor| {
                        cursor.input_needs_wrap = true;
                    });
                } else {
                    // We simply move the cursor onto the next cell in the current row.
                    self.update_cursor(|cursor| {
                        cursor.point.col += 1;
                    });
                    self.grid.update_max_cursor();
                }
            }
            // If we don't have a soft-wrap (from the "other" grid), then we need to hard-wrap the line
            // (in combined grid), to match the intended text layout.
            if !other.row_wraps(row_idx) && row_idx < max_row {
                self.newline();
            }
        }
    }

    /// Marks a location in the grid as being the end of a shell prompt (and
    /// the start of the user's command).
    pub(in crate::terminal::model) fn mark_end_of_prompt(
        &mut self,
        prompt_end_point: Point,
        has_extra_trailing_newline: bool,
    ) {
        match self.storage_row(prompt_end_point.row) {
            Some(StorageRow::GridStorage(row_idx)) => self.grid[row_idx][prompt_end_point.col]
                .mark_end_of_prompt(has_extra_trailing_newline),
            Some(StorageRow::FlatStorage(row_idx)) => {
                let rows_to_move = self.flat_storage.total_rows() - row_idx;
                let visible_rows = self.visible_rows();

                // Move the row from flat storage into grid storage so we can mark the end of
                // the prompt, then move the rows back.
                self.resize_storage(visible_rows + rows_to_move, self.columns());
                self.grid[0][prompt_end_point.col].mark_end_of_prompt(has_extra_trailing_newline);
                self.resize_storage(visible_rows, self.columns());
            }
            None => {}
        }
    }

    /// This method returns the absolute point representing the first cell which contains a
    /// "end of prompt" marker. This is used to demarcate the logical boundary between the
    /// prompt and command within a combined prompt/command grid. Returns None if no such valid
    /// marker exists.
    pub(in crate::terminal::model) fn prompt_end_point(&self) -> Option<Point> {
        for row_idx in 0..self.total_rows() {
            let row = self.row(row_idx)?;
            for col in (0..self.columns()).rev() {
                let cell = &row[col];
                if cell.is_end_of_prompt() {
                    return Some(Point::new(row_idx, col));
                }
            }
        }
        None
    }

    /// Returns the bottommost nonempty row index. If the Grid has no rows, returns None.
    fn bottommost_nonempty_row(&self) -> Option<usize> {
        let mut max_nonempty_row: Option<usize> = None;
        for row_idx in 0..self.total_rows() {
            let row = self.row(row_idx)?;
            for col in (0..self.columns()).rev() {
                let cell = &row[col];
                if !cell.is_empty() {
                    max_nonempty_row = max_nonempty_row.map(|y| y.max(row_idx)).or(Some(row_idx));
                    break;
                }
            }
        }
        max_nonempty_row
    }

    fn cell_has_visible_content_for_trimming(cell: &Cell) -> bool {
        match cell.raw_content() {
            CharOrStr::Char(c) => c != DEFAULT_CHAR && !c.is_ascii_whitespace(),
            CharOrStr::Str(s) => s
                .chars()
                .any(|c| c != DEFAULT_CHAR && !c.is_ascii_whitespace()),
        }
    }

    fn row_has_visible_content_for_trimming(&self, row_idx: usize) -> bool {
        let Some(row) = self.row(row_idx) else {
            return false;
        };
        row[..]
            .iter()
            .any(Self::cell_has_visible_content_for_trimming)
            || self.has_image_in_row(row_idx)
    }

    /// Scans backward from the max cursor row to find the bottommost row that
    /// should contribute to trimmed display height. O(trailing_blank_rows × cols)
    /// for the common cell-only case.
    fn bottommost_visible_content_row_backward(&self) -> Option<usize> {
        let max_row = self.grid.max_cursor_point.row.0 + self.history_size();
        (0..=max_row)
            .rev()
            .find(|&row_idx| self.row_has_visible_content_for_trimming(row_idx))
    }

    /// Returns the "content length" of the grid — the number of rows up to and
    /// including the bottommost row with visible content. Falls back to the
    /// full `max_cursor_point`-based length when not computed or grid is all
    /// blank (avoids trimming to 0).
    pub fn content_len(&self) -> usize {
        match self.bottommost_visible_content_row {
            Some(row) => row + 1,
            None => self.grid.max_cursor_point.row.0 + self.history_size() + 1,
        }
    }

    pub fn visible_content_len_for_trimming(&self) -> Option<usize> {
        self.bottommost_visible_content_row.map(|row| row + 1)
    }

    /// Returns the index of the right-most cell with visible contents in the
    /// given row.  Returns [`None`] if there are no such cells in the row.
    pub(in crate::terminal::model) fn rightmost_visible_nonempty_cell_in_row(
        &self,
        row_idx: usize,
    ) -> Option<usize> {
        let row = self.row(row_idx)?;
        let mut max_col: Option<usize> = None;
        for col in (0..self.columns()).rev() {
            let cell = &row[col];
            if cell.is_visible() {
                max_col = max_col.map(|x| x.max(col)).or(Some(col));
                break;
            }
        }
        max_col
    }

    /// Returns the column index of the rightmost non-empty cell in the grid.
    /// This can be used to determine the width of grid contents, excluding
    /// right-padded empty cells.
    ///
    /// This returns [`None`] if there are no non-empty cells in the grid.
    ///
    /// ┌──────────────────────────────│───┐
    /// │Rust is fun time.             │   │
    /// │Compile time is fun time.     │   │
    /// │Oncall is fun time.           ↓   │
    /// │Fixing bugs is also a fun time.   │ [HIDDEN ROW]
    /// └──────────────────────────────────┘
    pub fn rightmost_nonempty_cell(&self, max_row: Option<usize>) -> Option<usize> {
        // Compute the range of rows we want to iterate over.  `max_row` is
        // inclusive, so we add one, and default to the full range of rows if
        // no max is provided.
        let row_range = 0..max_row.map(|r| r + 1).unwrap_or(self.total_rows());

        let mut max_col: Option<usize> = None;
        for row_idx in row_range {
            if let Some(col) = self.rightmost_visible_nonempty_cell_in_row(row_idx) {
                max_col = max_col.map(|x| x.max(col)).or(Some(col));
            }
        }
        max_col
    }

    /// Inserts the characters within `text` into the grid starting at the
    /// cursor.
    ///
    /// Useful for tests that need to insert some text into the grid and then
    /// test things that expect correct cursor updates.
    ///
    /// If the written text wraps to a new line, the WRAPLINE flag will be set
    /// appropriately.
    #[cfg(test)]
    pub(super) fn input_at_cursor(&mut self, text: &str) {
        use warp_terminal::model::VisiblePoint;

        let columns = self.columns();
        let mut last_row = self.grid.cursor.point.row;
        for char in text.chars() {
            if self.grid.cursor.point.row != last_row {
                self.grid[last_row][columns - 1]
                    .flags_mut()
                    .insert(Flags::WRAPLINE);
            }
            last_row = self.grid.cursor.point.row;
            self.grid.cursor_cell().c = char;
            self.grid.cursor.point = self.grid.cursor.point.wrapping_add(columns, 1);
        }

        // Make sure we don't wrap the cursor beyond the end of the grid.
        if self.grid.cursor.point.row.0 >= self.visible_rows() {
            self.grid.cursor.point = VisiblePoint {
                row: VisibleRow(self.grid.rows - 1),
                col: self.columns() - 1,
            };
        }

        self.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    }

    pub fn set_marked_text(&mut self, marked_text: &str, _selected_range: &Range<usize>) {
        self.marked_text = Some(marked_text.to_string())
    }

    pub fn clear_marked_text(&mut self) {
        self.marked_text = None;
    }

    pub fn marked_text(&self) -> Option<&str> {
        self.marked_text.as_deref()
    }

    /// How many cells the marked text will occupy.
    fn marked_text_cell_length(&self) -> usize {
        self.marked_text
            .as_ref()
            .map(|s| s.chars().map(|c| c.width().unwrap_or(0)).sum())
            .unwrap_or(0)
    }

    pub fn evict_all_images(&mut self) {
        self.images.evict_all_images();
    }

    pub fn evict_image(&mut self, image_id: u32) {
        self.images.evict_image(image_id);
    }

    pub fn evict_placement(&mut self, image_id: u32, placement_id: u32) {
        self.images.evict_placement(image_id, placement_id);
    }
}

/// Iterator over regex matches.
pub struct RegexIter<'a> {
    /// The current point in the grid.
    point: Point,

    /// The last point in the grid that needs to be searched.
    end: Point,

    /// Left or right find direction.
    direction: Direction,

    dfas: &'a RegexDFAs,
    grid: &'a GridHandler,
    done: bool,
}

impl<'a> RegexIter<'a> {
    pub fn new(
        start: Point,
        end: Point,
        direction: Direction,
        grid: &'a GridHandler,
        dfas: &'a RegexDFAs,
    ) -> Self {
        Self {
            point: start,
            done: false,
            end,
            direction,
            grid,
            dfas,
        }
    }

    /// Skip one cell, advancing the origin point to the next one.
    fn skip(&mut self) {
        self.point = self.grid.expand_wide(self.point, self.direction);

        self.point = match self.direction {
            Direction::Right => self.point.wrapping_add(self.grid.columns(), 1),
            Direction::Left => self.point.wrapping_sub(self.grid.columns(), 1),
        };
    }

    /// Get the next match in the specified direction.
    fn next_match(&self) -> Option<Match> {
        match self.direction {
            Direction::Right => self
                .grid
                .regex_search_rightwards(self.dfas, self.point, self.end),
            Direction::Left => self
                .grid
                .regex_search_leftwards(self.dfas, self.point, self.end),
        }
    }
}

impl Iterator for RegexIter<'_> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        // Since the end itself might be a single cell match, we search one more time.
        if self.point == self.end {
            self.done = true;
        }

        let regex_match = self.next_match()?;

        self.point = match self.direction {
            Direction::Left => *regex_match.start(),
            Direction::Right => *regex_match.end(),
        };

        if self.point == self.end {
            // Stop when the match terminates right on the end limit.
            self.done = true;
        } else {
            // Move the new search origin past the match.
            self.skip();
        }

        Some(regex_match)
    }
}

impl Dimensions for GridHandler {
    #[inline]
    fn total_rows(&self) -> usize {
        self.visible_rows() + self.history_size()
    }

    #[inline]
    fn visible_rows(&self) -> usize {
        self.grid.visible_rows()
    }

    #[inline]
    fn history_size(&self) -> usize {
        self.flat_storage.total_rows()
    }

    #[inline]
    fn columns(&self) -> usize {
        self.grid.columns()
    }
}

#[cfg(test)]
#[path = "grid_handler_tests.rs"]
mod tests;
