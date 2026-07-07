//! [`TuiEditorElement`] — the core char-cell editor element, the TUI analogue
//! of the GUI's `RichTextElement`.
//!
//! The element *paints and interacts*; it does not compute row structure.
//! Rows come from the render state's single display-row implementation
//! (`CharCellState::display_lattice`), which interleaves ghost rows and
//! elides hidden line ranges; the element slices its text snapshot by each
//! row's char range, applies consumer-supplied styles, prefixes gutter cells,
//! and windows by scroll. Interaction geometry (cursor placement, mouse
//! hit-testing) queries the same lattice, so what is painted and what a click
//! resolves to can never disagree.
//!
//! Consumers configure the element; they never assemble rows:
//! - the prompt input renders it `.editable()` with scroll and an action
//!   handler,
//! - the diff wrapper renders it read-only with a line-number gutter and diff
//!   styles.
//!
//! The element knows nothing about diffs, tool calls, keybindings, or
//! kill/yank — those are consumer policy.

use std::ops::Range;
use std::rc::Rc;

use string_offset::CharOffset;
use warp::editor::CodeEditorModel;
use warp_editor::model::CoreEditorModel;
use warp_editor::render::model::{
    char_cell_display_width, CharCellTemporaryBlock, DisplayPoint, DisplayRow, DisplayRowKind,
};
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiFlex,
    TuiLayoutContext, TuiPaintContext, TuiParentElement, TuiPoint, TuiRect, TuiRectExt, TuiSize,
    TuiStyle, TuiText,
};
use warpui_core::{AppContext, ModelHandle};

/// Display columns between the line-number column and the row content.
const GUTTER_GAP: u16 = 2;

/// Logical rows scrolled per mouse-wheel notch (matches `TuiScrollable`).
const WHEEL_STEP: isize = 2;

/// Editor-generic actions the element emits from its event handling. The
/// owning view translates them into its own typed actions and applies them to
/// the editor model (mirroring how the GUI's element dispatches into its view).
#[derive(Debug, Clone)]
pub(crate) enum TuiEditorAction {
    /// Insert a printable character (only emitted when the element is
    /// [`editable`](TuiEditorElement::editable)).
    InsertChar(char),
    /// Place the cursor / begin a character selection at `offset` (single click).
    SelectionStartAt { offset: CharOffset },
    /// Extend the active selection's head to `offset` (shift-click).
    SelectionExtendTo { offset: CharOffset },
    /// Select the word at `offset` (double click).
    SelectWordAt { offset: CharOffset },
    /// Select the line at `offset` (triple click).
    SelectLineAt { offset: CharOffset },
    /// Update the in-progress drag selection to `offset` (mouse drag).
    SelectionUpdateTo { offset: CharOffset },
    /// Finish the in-progress drag selection (mouse up).
    SelectionEnd,
    /// Scroll the viewport by `rows` display rows without moving the cursor
    /// (negative scrolls toward the top). Driven by the mouse wheel.
    Scroll { rows: isize },
}

/// Handler receiving the element's [`TuiEditorAction`]s during event dispatch.
type TuiEditorActionHandler = Rc<dyn Fn(TuiEditorAction, &mut TuiEventContext)>;

/// Whole-row styles by row kind, plus per-line overrides — all consumer
/// policy. Gutter cells take their row's style.
#[derive(Debug, Clone, Default)]
pub(crate) struct TuiEditorStyles {
    /// Buffer rows not covered by `line_overrides`.
    pub text: TuiStyle,
    /// Ghost rows.
    pub ghost: TuiStyle,
    /// Gap separator rows (`… N lines`).
    pub gap: TuiStyle,
    /// Whole-line overrides by 0-based logical line index; first match wins.
    pub line_overrides: Vec<(Range<usize>, TuiStyle)>,
}

/// The core char-cell editor element. Construct per render via
/// [`TuiEditorElement::new`], configure with the builder methods, and finish
/// with [`TuiElement::finish`].
pub(crate) struct TuiEditorElement {
    /// Backing model; used at layout to push the width and at event time for
    /// fresh hit-testing geometry.
    model: ModelHandle<CodeEditorModel>,
    /// Plain-text buffer content captured at construction.
    text: String,
    /// Cursor gap offset (1-based) captured at construction.
    cursor_offset: CharOffset,
    /// Selection as a 0-based character-offset range, if any.
    sel_char_range: Option<Range<CharOffset>>,
    /// Model-derived hidden line ranges captured at construction. Structural
    /// extras are folded in via [`Self::effective_hidden_ranges`], which is
    /// also what the event path uses over fresh model state.
    hidden_line_ranges: Vec<Range<usize>>,

    // ── Config ──────────────────────────────────────────────────────────────
    editable: bool,
    /// Maximum visible rows for a scroll-windowed consumer; the first visible
    /// row comes from the render state's char-cell scroll offset. `None`
    /// renders full height.
    viewport_rows: Option<u32>,
    /// Whether to render a line-number gutter sized to the largest number.
    line_number_gutter: bool,
    /// Whether to elide the buffer's final empty line (see
    /// [`Self::hide_trailing_empty_line`]).
    hide_trailing_empty_line: bool,
    styles: TuiEditorStyles,
    on_action: Option<TuiEditorActionHandler>,

    // ── Built during layout ─────────────────────────────────────────────────
    column: TuiFlex,
    /// Total gutter columns (number column + gap); 0 when no gutter.
    gutter_cols: u16,
    /// Selected spans `(row_in_view, start_col, exclusive_end_col)`, in
    /// element-relative columns (gutter included).
    selected_spans: Vec<(u16, u16, u16)>,
    cursor_col: u16,
    cursor_row_in_view: u16,
    cursor_visible: bool,
}

impl TuiEditorElement {
    /// Snapshots `model`'s current content, cursor, selection, and
    /// model-derived hidden line ranges. Width-dependent work happens later in
    /// [`TuiElement::layout`], the first point that knows the terminal width.
    pub(crate) fn new(model: &ModelHandle<CodeEditorModel>, app: &AppContext) -> Self {
        let inner = model.as_ref(app);
        let buffer = inner.content().as_ref(app);
        let text = if buffer.is_empty() {
            String::new()
        } else {
            buffer.text().into_string()
        };
        let cursor_offset = inner
            .selection_model()
            .as_ref(app)
            .cursors(app)
            .into_iter()
            .next()
            .unwrap_or_default();
        let sel = inner.buffer_selection_model().as_ref(app);
        let (head, tail) = (sel.first_selection_head(), sel.first_selection_tail());
        let sel_char_range = (head != tail).then(|| {
            let start = CharOffset::from(head.min(tail).as_usize().saturating_sub(1));
            let end = CharOffset::from(head.max(tail).as_usize().saturating_sub(1));
            start..end
        });
        let hidden_line_ranges = inner
            .render_state()
            .as_ref(app)
            .char_cell()
            .map(|char_cell| char_cell.hidden_line_ranges(app))
            .unwrap_or_default();

        Self {
            model: model.clone(),
            text,
            cursor_offset,
            sel_char_range,
            hidden_line_ranges,
            editable: false,
            viewport_rows: None,
            line_number_gutter: false,
            hide_trailing_empty_line: false,
            styles: TuiEditorStyles::default(),
            on_action: None,
            column: TuiFlex::column(),
            gutter_cols: 0,
            selected_spans: Vec::new(),
            cursor_col: 0,
            cursor_row_in_view: 0,
            cursor_visible: false,
        }
    }

    /// Draw the terminal cursor and dispatch printable-character insertion.
    /// Omitted = read-only (the GUI convention: read-only is not wiring
    /// editing input, not a mode).
    pub(crate) fn editable(mut self) -> Self {
        self.editable = true;
        self
    }

    /// Window the rows to `max_visible_rows`, starting at the render state's
    /// char-cell scroll offset (owned model-side; consumers drive it via
    /// `CharCellState::follow_cursor` / `scroll_by`). Omitted = render all
    /// rows (e.g. the diff body, which scrolls with the transcript).
    pub(crate) fn with_viewport_rows(mut self, max_visible_rows: u32) -> Self {
        self.viewport_rows = Some(max_visible_rows);
        self
    }

    /// Render a line-number gutter sized to the buffer's largest line number:
    /// right-aligned numbers on a buffer line's first row, blanks on
    /// continuation/ghost/gap rows, plus a [`GUTTER_GAP`]-cell gap before the
    /// content.
    pub(crate) fn with_line_number_gutter(mut self) -> Self {
        self.line_number_gutter = true;
        self
    }

    pub(crate) fn with_styles(mut self, styles: TuiEditorStyles) -> Self {
        self.styles = styles;
        self
    }

    /// Elide the buffer's final empty line (buffers whose text ends with a
    /// newline have one). Diff bodies set this so a file's conventional
    /// trailing newline doesn't render as a blank numbered row; the input must
    /// not, since its cursor legitimately sits there.
    ///
    /// Structural extras like this one are folded into the hidden set that
    /// both painting and hit-testing use ([`Self::effective_hidden_ranges`]),
    /// so the two stay consistent.
    pub(crate) fn hide_trailing_empty_line(mut self) -> Self {
        self.hide_trailing_empty_line = true;
        self
    }

    /// Install the action handler. Omitted = the element handles no events at
    /// all (a read-only, click-through body).
    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(TuiEditorAction, &mut TuiEventContext) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    // ── Layout internals ─────────────────────────────────────────────────────────

    /// The hidden set painting and hit-testing share: `model_ranges` plus the
    /// structural extras this element is configured with (currently the
    /// trailing-empty-line elision, derived from `text`).
    fn effective_hidden_ranges(
        &self,
        text: &str,
        mut model_ranges: Vec<Range<usize>>,
    ) -> Vec<Range<usize>> {
        if self.hide_trailing_empty_line && text.ends_with('\n') {
            let last_line = text.split('\n').count() - 1;
            model_ranges.push(last_line..last_line + 1);
        }
        model_ranges
    }

    /// Builds the visible rows, cursor position, and selection spans at
    /// `full_width`, storing them for `render`/`cursor_position`.
    fn build(&mut self, full_width: u16, app: &AppContext) {
        let render_state = self.model.as_ref(app).render_state().clone();
        let render_state = render_state.as_ref(app);
        let Some(char_cell) = render_state.char_cell() else {
            self.column = TuiFlex::column();
            return;
        };

        let hidden = self.effective_hidden_ranges(&self.text, self.hidden_line_ranges.clone());

        // The gutter narrows the content width; push the content width into
        // the model so buffer softwrap math (navigation, event-time queries)
        // agrees with the display rows built below.
        self.gutter_cols = if self.line_number_gutter {
            digits(self.max_line_number(&hidden)) + GUTTER_GAP
        } else {
            0
        };
        let content_width = full_width.saturating_sub(self.gutter_cols);
        char_cell.set_terminal_width(content_width);

        let chars: Vec<char> = self.text.chars().collect();
        let cursor_offset = CharOffset::from(self.cursor_offset.as_usize().saturating_sub(1));
        // The first visible row is model-side scroll state; unwindowed
        // consumers always render from the top.
        let first_visible_row = if self.viewport_rows.is_some() {
            char_cell.scroll_offset()
        } else {
            0
        };

        // One projection serves rows, cursor placement, and selection spans,
        // so everything below is geometry over the same lattice.
        let lattice = char_cell.display_lattice(&hidden);
        let (column, selected_spans, cursor, visible_end) = {
            let rows = lattice.rows();
            // The cursor sits one row past the last text row when a logical
            // line exactly fills the width (deferred wrap); that phantom row
            // is part of the layout, so include it when sizing and windowing.
            let cursor = lattice.offset_to_display_point(cursor_offset);
            let total_rows = if self.editable {
                cursor.map_or(rows.len(), |cursor| rows.len().max(cursor.row as usize + 1))
            } else {
                rows.len()
            };

            let (visible_start, visible_rows) = match self.viewport_rows {
                Some(max_rows) => (
                    first_visible_row as usize,
                    (max_rows as usize).min(total_rows),
                ),
                None => (0, total_rows),
            };
            let visible_end = (visible_start + visible_rows).min(total_rows);
            let text_rows_end = visible_end.min(rows.len());
            let visible_slice = if visible_start < text_rows_end {
                &rows[visible_start..text_rows_end]
            } else {
                &[]
            };
            // Phantom rows in the window (past the last text row) carry no
            // text or selection; render them as blank rows so the cursor's
            // row still draws.
            let phantom_rows = visible_end
                .saturating_sub(visible_start)
                .saturating_sub(visible_slice.len());

            let mut selected_spans = Vec::new();
            let mut column = TuiFlex::column();
            for (vis_idx, row) in visible_slice.iter().enumerate() {
                column.add_child(self.render_row(row, &chars, lattice.ghosts()));
                if let Some((start_col, end_col)) = self.selection_span_in_row(row, &chars) {
                    selected_spans.push((
                        vis_idx as u16,
                        start_col + self.gutter_cols,
                        end_col + self.gutter_cols,
                    ));
                }
            }
            for _ in 0..phantom_rows {
                column.add_child(TuiText::new(" ").truncate().finish());
            }
            if visible_slice.is_empty() && phantom_rows == 0 {
                // Scrolled past the last row: keep one blank row so the
                // element never collapses to zero height.
                column.add_child(TuiText::new(" ").truncate().finish());
            }
            (column, selected_spans, cursor, visible_end)
        };

        self.column = column;
        self.selected_spans = selected_spans;
        if let Some(cursor) = cursor {
            self.cursor_col = cursor.col + self.gutter_cols;
            self.cursor_row_in_view = cursor.row.saturating_sub(first_visible_row) as u16;
            self.cursor_visible = self.editable
                && cursor.row >= first_visible_row
                && (cursor.row as usize) < visible_end.max(1);
        } else {
            self.cursor_col = 0;
            self.cursor_row_in_view = 0;
            self.cursor_visible = false;
        }
    }

    /// The largest line number the gutter can display: the buffer's line
    /// count, ignoring a trailing empty line that is hidden (so a file's
    /// conventional trailing newline doesn't widen the number column).
    fn max_line_number(&self, hidden_line_ranges: &[Range<usize>]) -> usize {
        let line_count = self.text.split('\n').count();
        let last_line_hidden = hidden_line_ranges
            .iter()
            .any(|range| range.contains(&(line_count.saturating_sub(1))));
        if self.text.ends_with('\n') && last_line_hidden {
            line_count.saturating_sub(1)
        } else {
            line_count
        }
    }

    /// Renders one display row: gutter cells + content in the row's style, or
    /// the elision separator for gap rows.
    fn render_row(
        &self,
        row: &DisplayRow,
        chars: &[char],
        ghosts: &[CharCellTemporaryBlock],
    ) -> Box<dyn TuiElement> {
        let (content, style) = match &row.kind {
            DisplayRowKind::Buffer { line_index } => {
                let content = slice_chars(chars, &row.char_range);
                let style = self
                    .styles
                    .line_overrides
                    .iter()
                    .find(|(range, _)| range.contains(line_index))
                    .map(|(_, style)| *style)
                    .unwrap_or(self.styles.text);
                (content, style)
            }
            DisplayRowKind::Ghost { ghost_index } => {
                let ghost_chars: Vec<char> = ghosts[*ghost_index].content.chars().collect();
                (
                    slice_chars(&ghost_chars, &row.char_range),
                    self.styles.ghost,
                )
            }
            DisplayRowKind::Gap { line_range } => {
                (format!("… {} lines", line_range.len()), self.styles.gap)
            }
        };
        let gutter = self.gutter_cells(row);
        // An empty `TuiText` lays out to zero rows, which would collapse the
        // row and clip the cursor (or following rows) off the column; render
        // a single space so every display row keeps a height of exactly one.
        let line = if gutter.is_empty() && content.is_empty() {
            " ".to_string()
        } else {
            format!("{gutter}{content}")
        };
        TuiText::new(line).with_style(style).truncate().finish()
    }

    /// The row's gutter cells: a right-aligned line number on a buffer line's
    /// first row, blanks otherwise. Empty string when the gutter is disabled.
    fn gutter_cells(&self, row: &DisplayRow) -> String {
        if self.gutter_cols == 0 {
            return String::new();
        }
        let number_width = (self.gutter_cols - GUTTER_GAP) as usize;
        match &row.kind {
            DisplayRowKind::Buffer { line_index } if !row.is_continuation => {
                let number = line_index + 1;
                format!("{number:>number_width$}{}", " ".repeat(GUTTER_GAP as usize))
            }
            DisplayRowKind::Buffer { .. }
            | DisplayRowKind::Ghost { .. }
            | DisplayRowKind::Gap { .. } => " ".repeat(self.gutter_cols as usize),
        }
    }

    /// The selection's display-column span within `row`, if the selection
    /// overlaps it. Selection offsets are char indices; terminal highlighting
    /// works in display columns, so convert via each char's display width.
    fn selection_span_in_row(&self, row: &DisplayRow, chars: &[char]) -> Option<(u16, u16)> {
        let selection = self.sel_char_range.clone()?;
        if !matches!(row.kind, DisplayRowKind::Buffer { .. }) {
            return None;
        }
        if selection.end <= row.char_range.start || selection.start >= row.char_range.end {
            return None;
        }
        let start_offset = selection.start.max(row.char_range.start);
        let end_offset = selection.end.min(row.char_range.end);
        let row_start = row.char_range.start.as_usize();
        let display_col = |offset: CharOffset| -> u16 {
            chars[row_start..offset.as_usize()]
                .iter()
                .map(|&c| char_cell_display_width(c) as u16)
                .sum()
        };
        let start_col = display_col(start_offset);
        let end_col = display_col(end_offset);
        (end_col > start_col).then_some((start_col, end_col))
    }

    // ── Event internals ──────────────────────────────────────────────────────

    /// Maps a terminal cell `position` to the 1-based buffer [`CharOffset`]
    /// under it. The element may be cached across frames while the model
    /// changes, so everything the hit-test reads — text, hidden ranges, wrap
    /// tables — is re-derived from the model here rather than taken from the
    /// construction-time snapshots.
    ///
    /// Points outside the element's vertical bounds are intentionally *not*
    /// clamped to the viewport: a point above maps toward row 0 and a point
    /// below maps to the last display row (or the buffer's end on the
    /// deferred-wrap phantom row), so a drag that leaves the element drives
    /// auto-scroll.
    fn offset_at(&self, position: TuiPoint, area: TuiRect, app: &AppContext) -> Option<CharOffset> {
        let inner = self.model.as_ref(app);
        let render_state = inner.render_state().as_ref(app);
        let char_cell = render_state.char_cell()?;
        let buffer = inner.content().as_ref(app);
        let text = if buffer.is_empty() {
            String::new()
        } else {
            buffer.text().into_string()
        };
        let hidden = self.effective_hidden_ranges(&text, char_cell.hidden_line_ranges(app));
        let first_visible_row = if self.viewport_rows.is_some() {
            char_cell.scroll_offset()
        } else {
            0
        };

        let row_in_view = i64::from(position.y) - i64::from(area.y);
        let display_row = (i64::from(first_visible_row) + row_in_view).max(0) as u32;
        let col = position
            .x
            .saturating_sub(area.x)
            .saturating_sub(self.gutter_cols);

        let lattice = char_cell.display_lattice(&hidden);
        // The rendered layout can include a "phantom" row one past the last
        // display row when the final logical line exactly fills the width
        // (deferred wrap). Resolve it directly to the end-of-buffer gap;
        // otherwise cap at the last real display row so a drag below the
        // text resolves within it rather than past it.
        let last_row = (lattice.rows().len() as u32).saturating_sub(1);
        if display_row > last_row {
            let end_char_offset = CharOffset::from(text.chars().count());
            if lattice
                .offset_to_display_point(end_char_offset)
                .is_some_and(|point| point.row > last_row)
            {
                return Some(end_char_offset + 1);
            }
        }
        let point = DisplayPoint {
            row: display_row.min(last_row),
            col,
        };
        lattice
            .display_point_to_offset(point)
            .map(|offset| offset + 1)
    }

    /// Maps a mouse `event` to the [`TuiEditorAction`] it should emit, or
    /// `None` when the event should be ignored. Mirrors the GUI's
    /// `left_mouse_down`/`dragged`/`up` mapping: click count 1 starts a
    /// selection (shift extends), 2 selects a word, 3 selects a line; drag
    /// updates the pending selection and up ends it.
    ///
    /// Crate-visible so consumer tests can drive the mouse path directly.
    pub(crate) fn mouse_action(
        &self,
        event: &TuiEvent,
        area: TuiRect,
        app: &AppContext,
    ) -> Option<TuiEditorAction> {
        match event {
            TuiEvent::LeftMouseDown {
                position,
                modifiers,
                click_count,
                is_first_mouse,
            } => {
                // The focus-bringing first click has no matching mouse-up, and
                // a press outside the element must not start a selection.
                if *is_first_mouse || !area.contains_point(*position) {
                    return None;
                }
                let offset = self.offset_at(*position, area, app)?;
                Some(match *click_count {
                    0 | 1 if modifiers.shift => TuiEditorAction::SelectionExtendTo { offset },
                    0 | 1 => TuiEditorAction::SelectionStartAt { offset },
                    2 => TuiEditorAction::SelectWordAt { offset },
                    _ => TuiEditorAction::SelectLineAt { offset },
                })
            }
            // Drags continue even outside the element's bounds (drag-to-scroll),
            // but only while a selection that began inside it is active.
            TuiEvent::LeftMouseDragged { position, .. } if self.drag_in_progress(app) => {
                Some(TuiEditorAction::SelectionUpdateTo {
                    offset: self.offset_at(*position, area, app)?,
                })
            }
            TuiEvent::LeftMouseUp { .. } if self.drag_in_progress(app) => {
                Some(TuiEditorAction::SelectionEnd)
            }
            // Mouse wheel scrolls the viewport (cursor unmoved); only
            // meaningful for scroll-windowed consumers.
            TuiEvent::ScrollWheel {
                position, delta, ..
            } if self.viewport_rows.is_some() && area.contains_point(*position) => {
                // crossterm reports ScrollUp as +1 row / ScrollDown as -1;
                // negate so wheel-up scrolls toward the top.
                Some(TuiEditorAction::Scroll {
                    rows: -(delta.1 * WHEEL_STEP),
                })
            }
            _ => None,
        }
    }

    /// Whether a mouse drag-selection is in progress, read fresh from the
    /// selection model's pending-selection state (the element may be cached
    /// across frames while the drag progresses).
    fn drag_in_progress(&self, app: &AppContext) -> bool {
        self.model
            .as_ref(app)
            .selection_model()
            .as_ref(app)
            .has_pending_selection()
    }
}

impl TuiElement for TuiEditorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        // The layout constraint is the first place the real terminal width is
        // known — mirroring how the GUI computes geometry during layout.
        let full_width = constraint.constrain_width(constraint.max.width);
        self.build(full_width, app);
        let content_size = self.column.layout(constraint, ctx, app);
        // The editor claims the full width it was offered (its wrap width),
        // not just the longest row's width the content-sized column reports.
        TuiSize::new(full_width, content_size.height)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        self.column.render(area, buffer, ctx);
        if !self.selected_spans.is_empty() {
            let reversed = TuiStyle::default().add_modifier(Modifier::REVERSED);
            for &(row_in_view, start_col, end_col) in &self.selected_spans {
                let y = area.y.saturating_add(row_in_view);
                let x = area.x.saturating_add(start_col);
                let width = end_col.saturating_sub(start_col);
                if y < area.y + area.height && width > 0 {
                    let sel_rect =
                        TuiRect::new(x, y, width.min(area.width.saturating_sub(start_col)), 1);
                    buffer.set_style(sel_rect, reversed);
                }
            }
        }
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        if !self.cursor_visible
            || self.cursor_col >= area.width
            || self.cursor_row_in_view >= area.height
        {
            return None;
        }
        Some((self.cursor_col, self.cursor_row_in_view))
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        if self.column.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }
        let Some(handler) = self.on_action.clone() else {
            return false;
        };

        if let Some(action) = self.mouse_action(event, area, app) {
            handler(action, event_ctx);
            return true;
        }

        if self.editable {
            if let TuiEvent::KeyDown {
                keystroke, chars, ..
            } = event
            {
                // Chorded editing commands are dispatched by the keymap pass
                // (consumer keybindings) before the element pass ever sees the
                // key. Only printable-character insertion stays element-level —
                // text insertion is not a keybinding, matching the GUI.
                if !keystroke.ctrl && !keystroke.alt && !chars.is_empty() {
                    if let Some(char) = chars.chars().next() {
                        handler(TuiEditorAction::InsertChar(char), event_ctx);
                        return true;
                    }
                }
            }
        }

        false
    }
}

/// The chars in `range`, collected into the row's paint text.
fn slice_chars(chars: &[char], range: &Range<CharOffset>) -> String {
    let start = range.start.as_usize().min(chars.len());
    let end = range.end.as_usize().min(chars.len());
    chars[start..end].iter().collect()
}

/// The number of decimal digits in `n` (minimum 1), sizing the gutter's
/// number column.
fn digits(n: usize) -> u16 {
    let mut digits = 1;
    let mut n = n / 10;
    while n > 0 {
        digits += 1;
        n /= 10;
    }
    digits
}

#[cfg(test)]
#[path = "editor_element_tests.rs"]
mod tests;
