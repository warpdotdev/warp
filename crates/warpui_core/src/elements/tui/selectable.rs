//! Selection interaction plumbing for selectable TUI elements.
//!
//! [`TuiSelectable`] wraps a child implementing [`TuiSelectableElement`] and
//! owns the mouse-driven selection gesture and its persistent state. The child
//! resolves viewport-specific geometry and text and renders the selection state
//! supplied by the wrapper.
//!
//! Submodules provide the shared building blocks: [`cells`] for cell/glyph
//! geometry and row-text extraction, and [`state`] for the drag-state handle
//! shared across element rebuilds.

use std::ops::Range;

use string_offset::{ByteOffset, CharOffset};

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiGridPoint,
    TuiLayoutContext, TuiPaintContext, TuiPoint, TuiPresentationContext, TuiRect,
    TuiScrollableElement, TuiSize,
};
use crate::elements::SmartSelectFn;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{SelectionDirection, SelectionType, TextBuffer};
use crate::AppContext;

mod cells;
mod state;

pub(crate) use cells::{cell_span, row_glyphs, row_text};
pub use cells::{point_after_col, TuiRowGlyph, TuiSelectionSpan};
pub use state::TuiSelectionHandle;

type SelectionCallback = Box<dyn FnMut(&mut TuiEventContext, &AppContext)>;
type CopyCallback = Box<dyn FnMut(String, &mut TuiEventContext, &AppContext)>;

/// A content row range before layout and its height afterward.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiRowResize {
    /// Rows occupied before layout.
    pub old_rows: Range<usize>,
    /// Rows occupied after layout.
    pub new_height: usize,
}

/// Geometry, content, and rendering behavior implemented by a selectable child.
pub trait TuiSelectableElement: TuiScrollableElement {
    /// Resolves one screen position into a content-space point.
    fn selection_point_at(
        &mut self,
        position: TuiPoint,
        area: TuiRect,
        clamp_outside: bool,
    ) -> Option<TuiGridPoint>;

    /// Returns rendered glyphs for one selectable content row.
    fn selection_row_glyphs(
        &self,
        row: usize,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Vec<TuiRowGlyph>;

    /// Materializes text for one resolved content-space selection.
    fn selected_text(
        &self,
        selection: TuiSelectionSpan,
        area: TuiRect,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Option<String>;

    /// Paints the supplied selection state over normal child rendering.
    fn render_selection(
        &self,
        selection: &TuiSelectionHandle,
        area: TuiRect,
        buffer: &mut TuiBuffer,
        ctx: &mut TuiPaintContext,
    );

    /// Drains content-row resizes resolved during the latest child layout.
    fn take_selection_row_resizes(&self) -> Vec<TuiRowResize> {
        Vec::new()
    }
}

/// Owns selection interaction and delegates content resolution to its child.
pub struct TuiSelectable<Child> {
    selection: TuiSelectionHandle,
    child: Child,
    word_boundaries_policy: WordBoundariesPolicy,
    smart_select_fn: Option<SmartSelectFn>,
    on_selection_start: Option<SelectionCallback>,
    on_copy: Option<CopyCallback>,
}

impl<Child> TuiSelectable<Child>
where
    Child: TuiSelectableElement,
{
    /// Wraps a selectable child with persistent selection state.
    pub fn new(selection: TuiSelectionHandle, child: Child) -> Self {
        Self {
            selection,
            child,
            word_boundaries_policy: WordBoundariesPolicy::Default,
            smart_select_fn: None,
            on_selection_start: None,
            on_copy: None,
        }
    }

    /// Uses `policy` when expanding semantic word selections.
    pub fn with_word_boundaries_policy(mut self, policy: WordBoundariesPolicy) -> Self {
        self.word_boundaries_policy = policy;
        self
    }

    /// Uses `smart_select_fn` before falling back to word-boundary expansion.
    pub fn with_smart_select_fn(mut self, smart_select_fn: Option<SmartSelectFn>) -> Self {
        self.smart_select_fn = smart_select_fn;
        self
    }

    /// Resolves one screen position into the configured selection unit.
    fn selection_span_at(
        &mut self,
        position: TuiPoint,
        selection_type: SelectionType,
        area: TuiRect,
        clamp_outside: bool,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Option<TuiSelectionSpan> {
        let point = self
            .child
            .selection_point_at(position, area, clamp_outside)?;
        Some(self.selection_unit_span(selection_type, point, area.width, ctx, app))
    }

    /// Expands one content point to a character, word, or line span.
    fn selection_unit_span(
        &self,
        selection_type: SelectionType,
        point: TuiGridPoint,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSelectionSpan {
        match selection_type {
            SelectionType::Simple | SelectionType::Rect => self
                .child
                .selection_row_glyphs(point.row, width, ctx, app)
                .into_iter()
                .find(|glyph| point.col >= glyph.start_col && point.col < glyph.end_col)
                .map(|glyph| TuiSelectionSpan {
                    start: TuiGridPoint {
                        row: point.row,
                        col: glyph.start_col,
                    },
                    end: point_after_col(point.row, glyph.end_col, width),
                })
                .unwrap_or_else(|| cell_span(point, width)),
            SelectionType::Semantic => {
                let glyphs = self.child.selection_row_glyphs(point.row, width, ctx, app);
                word_span(
                    point,
                    width,
                    &glyphs,
                    &self.word_boundaries_policy,
                    self.smart_select_fn,
                )
                .unwrap_or_else(|| cell_span(point, width))
            }
            SelectionType::Lines => TuiSelectionSpan {
                start: TuiGridPoint {
                    row: point.row,
                    col: 0,
                },
                end: TuiGridPoint {
                    row: point.row.saturating_add(1),
                    col: 0,
                },
            },
        }
    }

    /// Runs `callback` when the child starts a selection.
    pub fn on_selection_start(
        mut self,
        callback: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.on_selection_start = Some(Box::new(callback));
        self
    }

    /// Runs `callback` when the child completes a non-empty selection.
    pub fn on_copy(
        mut self,
        callback: impl FnMut(String, &mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.on_copy = Some(Box::new(callback));
        self
    }
}

impl<Child> TuiElement for TuiSelectable<Child>
where
    Child: TuiSelectableElement,
{
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        // Width changes rewrap content and invalidate grid-coordinate selections.
        // Clear first so child layout cannot rebase already-stale row positions.
        self.selection.validate_width(constraint.max.width);

        let size = self.child.layout(constraint, ctx, app);
        self.selection
            .rebase_for_row_resizes(self.child.take_selection_row_resizes());
        size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        self.child.render(area, buffer, ctx);
        self.child
            .render_selection(&self.selection, area, buffer, ctx);
    }
    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        self.child.cursor_position(area, ctx)
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        let captures_drag = self.selection.is_selecting()
            && matches!(
                event,
                TuiEvent::LeftMouseDragged { .. } | TuiEvent::LeftMouseUp { .. }
            );
        if !captures_drag && self.child.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }

        match event {
            TuiEvent::LeftMouseDown {
                position,
                click_count,
                is_first_mouse,
                ..
            } if !*is_first_mouse => {
                let selection_type = SelectionType::from_click_count(*click_count);
                let Some(anchor_span) =
                    self.selection_span_at(*position, selection_type, area, false, ctx, app)
                else {
                    return false;
                };
                let focus_span = match selection_type {
                    SelectionType::Simple | SelectionType::Rect => None,
                    SelectionType::Semantic | SelectionType::Lines => Some(anchor_span),
                };
                self.selection
                    .start(anchor_span, focus_span, selection_type, area.width);
                if let Some(callback) = self.on_selection_start.as_mut() {
                    callback(event_ctx, app);
                }
                event_ctx.notify();
                true
            }
            TuiEvent::LeftMouseDragged { position, .. } if self.selection.is_selecting() => {
                // Scroll one row per drag event at an edge. The top edge is
                // inclusive because terminal mouse coordinates cannot go negative.
                let scroll_rows = if position.y <= area.y {
                    -1
                } else if position.y >= area.bottom() {
                    1
                } else {
                    0
                };
                if scroll_rows != 0 {
                    self.child
                        .scroll_by_rows(scroll_rows, usize::from(area.height));
                }

                let Some(interaction) = self.selection.interaction() else {
                    return false;
                };
                let Some(focus_span) = self.selection_span_at(
                    *position,
                    interaction.selection_type,
                    area,
                    true,
                    ctx,
                    app,
                ) else {
                    return true;
                };
                if matches!(
                    interaction.selection_type,
                    SelectionType::Simple | SelectionType::Rect
                ) && !interaction.has_focus
                    && focus_span.start == interaction.anchor_span.start
                {
                    event_ctx.notify();
                    return true;
                }
                self.selection.update_focus(focus_span);
                event_ctx.notify();
                true
            }
            TuiEvent::LeftMouseUp { .. } if self.selection.is_selecting() => {
                self.selection.finish();
                let text = self
                    .selection
                    .range()
                    .and_then(|selection| self.child.selected_text(selection, area, ctx, app));
                if text.is_none() {
                    self.selection.clear();
                }
                if let (Some(text), Some(callback)) = (text, self.on_copy.as_mut()) {
                    callback(text, event_ctx, app);
                }
                event_ctx.notify();
                true
            }
            TuiEvent::LeftMouseDown { .. }
            | TuiEvent::LeftMouseDragged { .. }
            | TuiEvent::LeftMouseUp { .. }
            | TuiEvent::ScrollWheel { .. }
            | TuiEvent::KeyDown { .. }
            | TuiEvent::MiddleMouseDown { .. }
            | TuiEvent::RightMouseDown { .. }
            | TuiEvent::MouseMoved { .. } => false,
        }
    }
}

/// Resolves a semantic word span from rendered row glyphs.
fn word_span(
    point: TuiGridPoint,
    width: u16,
    glyphs: &[TuiRowGlyph],
    policy: &WordBoundariesPolicy,
    smart_select_fn: Option<SmartSelectFn>,
) -> Option<TuiSelectionSpan> {
    let clicked = glyphs
        .iter()
        .position(|glyph| point.col >= glyph.start_col && point.col < glyph.end_col)?;
    let line = glyphs
        .iter()
        .map(|glyph| glyph.text.as_str())
        .collect::<String>();
    let byte_range = smart_select_fn
        .and_then(|smart_select| {
            smart_select(&line, ByteOffset::from(glyphs[clicked].byte_range.start))
        })
        .map(|range| range.start.as_usize()..range.end.as_usize())
        .or_else(|| word_byte_range(&line, glyphs[clicked].byte_range.start, policy))?;
    let start_index = glyphs.partition_point(|glyph| glyph.byte_range.end <= byte_range.start);
    let end_index = glyphs.partition_point(|glyph| glyph.byte_range.start < byte_range.end);
    let start = glyphs.get(start_index)?;
    let end = glyphs.get(end_index.saturating_sub(1))?;
    Some(TuiSelectionSpan {
        start: TuiGridPoint {
            row: point.row,
            col: start.start_col,
        },
        end: point_after_col(point.row, end.end_col, width),
    })
}

/// Expands one byte position using the shared text-buffer word semantics.
fn word_byte_range(
    line: &str,
    clicked_byte: usize,
    policy: &WordBoundariesPolicy,
) -> Option<Range<usize>> {
    let clicked_char = line.get(..clicked_byte)?.chars().count();
    let start = line
        .semantic_expansion_target(
            CharOffset::from(clicked_char),
            SelectionDirection::Backward,
            policy,
        )
        .ok()?;
    let end = line
        .semantic_expansion_target(
            CharOffset::from(clicked_char),
            SelectionDirection::Forward,
            policy,
        )
        .ok()?;
    let start_char = line.to_offset(start).ok()?.as_usize();
    let end_char = line.to_offset(end).ok()?.as_usize();
    Some(byte_offset_for_char(line, start_char)..byte_offset_for_char(line, end_char))
}

/// Converts one character offset into its UTF-8 byte offset.
fn byte_offset_for_char(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .nth(char_offset)
        .map_or(text.len(), |(byte_offset, _)| byte_offset)
}

impl<Child> TuiScrollableElement for TuiSelectable<Child>
where
    Child: TuiSelectableElement,
{
    fn scroll_by_rows(&mut self, rows: isize, viewport_height: usize) -> bool {
        self.child.scroll_by_rows(rows, viewport_height)
    }
}
