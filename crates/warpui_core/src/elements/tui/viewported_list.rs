//! A source-driven viewport for ordered, variable-height TUI content.
//!
//! The caller owns item storage, identity, traversal, and height reconciliation;
//! this element owns the viewport window, scroll clamping, and the normal TUI
//! element lifecycle for the currently visible child elements.

use std::cell::RefCell;
use std::cmp::{max, min};
use std::ops::Range;
use std::rc::Rc;

use super::selectable::{TuiSelectionHandle, row_glyphs, row_text};
use super::{
    TuiBuffer, TuiClipped, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiGridPoint,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPresentationContext, TuiRect,
    TuiRowResize, TuiScreenPoint, TuiScreenPosition, TuiScrollableElement, TuiSelectableElement,
    TuiSelectionSpan, TuiSize,
};
use crate::AppContext;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiViewportPosition {
    End,
    RowsFromTop(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiViewportVerticalAlignment {
    /// Render content from the top of the viewport.
    Top,
    /// Dock short content to the bottom while following the end, so transcripts grow upward.
    GrowFromBottom,
}
/// The content-space geometry resolved by the most recent viewport layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TuiResolvedViewport {
    /// The clamped content-space window rendered into the viewport.
    pub window: TuiViewportWindow,
    /// The full content height used to resolve and clamp `window`.
    pub content_height: usize,
    /// Blank screen rows before content when it is docked to the bottom.
    pub screen_offset: u16,
}

/// Mutable viewport state shared across element rebuilds.
struct TuiViewportedListStateInner {
    position: TuiViewportPosition,
    resolved: Option<TuiResolvedViewport>,
}

/// Shared storage for caller-owned viewport position and geometry.
#[derive(Clone)]
pub struct TuiViewportedListState(Rc<RefCell<TuiViewportedListStateInner>>);

impl TuiViewportedListState {
    /// Creates viewport state initially following the content end.
    pub fn new_at_end() -> Self {
        Self(Rc::new(RefCell::new(TuiViewportedListStateInner {
            position: TuiViewportPosition::End,
            resolved: None,
        })))
    }

    /// Returns the current caller-owned viewport position.
    pub fn position(&self) -> TuiViewportPosition {
        self.0.borrow().position.clone()
    }

    /// Stores a new caller-owned viewport position.
    pub fn set_position(&self, position: TuiViewportPosition) {
        self.0.borrow_mut().position = position;
    }

    /// Requests rendering from the end of the content.
    pub fn scroll_to_end(&self) {
        self.set_position(TuiViewportPosition::End);
    }

    /// Requests rendering from an absolute content-space row.
    pub fn scroll_to_rows_from_top(&self, scroll_top: usize) {
        self.set_position(TuiViewportPosition::RowsFromTop(scroll_top));
    }

    /// Returns whether the requested viewport position follows the content end.
    pub fn is_at_end(&self) -> bool {
        matches!(self.0.borrow().position, TuiViewportPosition::End)
    }

    /// Returns the geometry produced by the most recent viewport layout.
    pub(crate) fn resolved_viewport(&self) -> Option<TuiResolvedViewport> {
        self.0.borrow().resolved
    }

    /// Records geometry resolved by the viewport's layout pass.
    fn set_resolved_viewport(&self, resolved: TuiResolvedViewport) {
        self.0.borrow_mut().resolved = Some(resolved);
    }
}

impl<Content> TuiSelectableElement for TuiViewportedList<Content>
where
    Content: TuiViewportedElement,
{
    fn selection_point_at(
        &mut self,
        position: super::TuiLocalPoint,
        size: TuiSize,
        clamp_outside: bool,
    ) -> Option<TuiGridPoint> {
        self.resolve_selection_point(position, size, clamp_outside)
    }

    fn selection_row_glyphs(
        &self,
        row: usize,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Vec<super::TuiRowGlyph> {
        self.resolve_selection_row_glyphs(row, width, ctx, app)
    }

    fn selected_text(
        &self,
        selection: TuiSelectionSpan,
        size: TuiSize,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Option<String> {
        self.selection_text(selection, size, ctx, app)
    }

    fn render_selection(
        &self,
        selection: &TuiSelectionHandle,
        origin: TuiScreenPosition,
        size: TuiSize,
        surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
        let Some(resolved) = self.state.resolved_viewport() else {
            return;
        };
        let visible_height = size.height.saturating_sub(resolved.screen_offset).min(
            resolved
                .content_height
                .saturating_sub(resolved.window.scroll_top)
                .min(usize::from(u16::MAX)) as u16,
        );
        let mut snapshot = TuiBuffer::empty(TuiRect::new(0, 0, size.width, visible_height));
        for row in 0..visible_height {
            for col in 0..size.width {
                let position = origin.offset(
                    i32::from(col),
                    i32::from(resolved.screen_offset.saturating_add(row)),
                );
                if let Some(cell) = surface.cell(position) {
                    snapshot[(col, row)] = cell.clone();
                }
            }
        }
        *self.selection_snapshot.borrow_mut() = Some((resolved, snapshot));
        if !selection.validate_width(size.width) {
            return;
        }
        let Some(range) = selection.range() else {
            return;
        };
        let viewport_bottom = resolved.window.scroll_top.saturating_add(usize::from(
            size.height.saturating_sub(resolved.screen_offset),
        ));
        let first_row = max(range.start.row, resolved.window.scroll_top);
        let end_row_exclusive = if range.end.col == 0 {
            range.end.row
        } else {
            range.end.row.saturating_add(1)
        };
        let last_row = min(end_row_exclusive, viewport_bottom);
        let mut selection_rects = Vec::new();
        for row in first_row..last_row {
            let y = resolved
                .screen_offset
                .saturating_add(row.saturating_sub(resolved.window.scroll_top) as u16);
            let start_col = if row == range.start.row {
                range.start.col
            } else {
                0
            };
            let end_col = if row == range.end.row {
                range.end.col
            } else {
                size.width
            };
            if start_col < end_col {
                selection_rects.push((
                    origin.offset(i32::from(start_col), i32::from(y)),
                    TuiSize::new(end_col.saturating_sub(start_col).min(size.width), 1),
                ));
            }
        }
        for (origin, size) in selection_rects {
            toggle_selection_reverse(surface, origin, size);
        }
    }

    fn take_selection_row_resizes(&self) -> Vec<TuiRowResize> {
        self.content.take_selection_row_resizes()
    }
}

/// A content-space viewport window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiViewportWindow {
    /// Absolute content-space row at the top of the viewport.
    pub scroll_top: usize,
    /// Visible terminal height in rows.
    pub viewport_height: u16,
}

/// A child element visible in a content-space viewport.
pub struct TuiVisibleViewportItem {
    /// Absolute content-space row where this child starts.
    pub origin_y: usize,
    pub element: Box<dyn TuiElement>,
}

/// The content returned for a viewport window.
pub struct TuiViewportContent {
    /// Total content height in content-space rows.
    pub content_height: usize,
    pub items: Vec<TuiVisibleViewportItem>,
}

/// Supplies visible TUI elements for an absolute content-space viewport window.
pub trait TuiViewportedElement {
    /// Returns the content height and visible child elements for `window`.
    ///
    /// `available_width` is the layout width for width-dependent height
    /// measurement, not horizontal viewport state. `ctx` is the live layout
    /// context, so height measurement can resolve [`TuiChildView`] elements
    /// from the presenter's `rendered_views` instead of measuring them as zero.
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn visible_items(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiViewportContent;

    /// Returns selectable rows without reconciling content state.
    fn selection_content(
        &self,
        _window: TuiViewportWindow,
        _available_width: u16,
        _app: &AppContext,
    ) -> Option<TuiViewportContent> {
        None
    }

    /// Optional *logical* text for a resolved selection span.
    ///
    /// Returning `Some` lets copy source text from the content's logical model
    /// instead of the rendered cell grid, so soft-wrapped visual rows are
    /// rejoined into their original line (no newline inserted at a wrap point,
    /// no rendered wrap/quote indentation captured) and the full selected range
    /// is returned even when it exceeds what the viewport rendered. Returning
    /// `None` (the default) makes the caller fall back to per-row grid-text
    /// extraction, which is the right behavior for content that has no clean
    /// logical form (diagrams, images, tables).
    fn selection_logical_text(
        &self,
        _selection: TuiSelectionSpan,
        _available_width: u16,
        _app: &AppContext,
    ) -> Option<String> {
        None
    }

    /// Drains row resizes produced during the latest layout.
    fn take_selection_row_resizes(&self) -> Vec<TuiRowResize> {
        Vec::new()
    }
}

struct VisibleElement {
    // Viewport-local render coordinates use `u16` to match TUI geometry.
    viewport_y: u16,
    height: u16,
    element: TuiClipped,
}
impl VisibleElement {
    /// Returns this item's rendered slot within the viewport area.
    fn slot(&self, area: TuiRect) -> Option<TuiRect> {
        let slot_y = area.y.saturating_add(self.viewport_y);
        if slot_y >= area.bottom() {
            return None;
        }
        let height = self.height.min(area.bottom() - slot_y);
        Some(TuiRect::new(area.x, slot_y, area.width, height))
    }
}
/// Lays out visible items using the canonical viewport clipping rules.
fn layout_visible_elements(
    content: TuiViewportContent,
    window: TuiViewportWindow,
    screen_offset: u16,
    available_width: u16,
    ctx: &mut TuiLayoutContext,
    app: &AppContext,
) -> Vec<VisibleElement> {
    let viewport_bottom = window
        .scroll_top
        .saturating_add(usize::from(window.viewport_height));
    content
        .items
        .into_iter()
        .filter_map(|item| {
            let mut element = item.element;
            let full_size = element.layout(
                TuiConstraint::loose(TuiSize::new(available_width, u16::MAX)),
                ctx,
                app,
            );
            let item_top = item.origin_y;
            let item_bottom = item_top.saturating_add(usize::from(full_size.height));
            let visible_top = item_top.max(window.scroll_top);
            let visible_bottom = item_bottom.min(viewport_bottom);
            if visible_top >= visible_bottom {
                return None;
            }

            let viewport_y = visible_top
                .saturating_sub(window.scroll_top)
                .saturating_add(usize::from(screen_offset))
                .min(usize::from(u16::MAX)) as u16;
            let viewport_origin_y = visible_top.saturating_sub(item_top);
            let height = visible_bottom
                .saturating_sub(visible_top)
                .min(usize::from(u16::MAX)) as u16;
            let element = TuiClipped::from_laid_out_child(
                element,
                viewport_origin_y,
                TuiSize::new(available_width, height),
            );
            Some(VisibleElement {
                viewport_y,
                height,
                element,
            })
        })
        .collect()
}

/// Renders canonical visible elements at an absolute viewport origin.
fn render_visible_elements(
    visible_elements: &mut [VisibleElement],
    origin: TuiScreenPosition,
    size: TuiSize,
    surface: &mut TuiPaintSurface<'_>,
    ctx: &mut TuiPaintContext,
) {
    let area = TuiRect::new(0, 0, size.width, size.height);
    for visible in visible_elements {
        let Some(slot) = visible.slot(area) else {
            continue;
        };
        visible.element.render(
            origin.offset(i32::from(slot.x), i32::from(slot.y)),
            surface,
            ctx,
        );
    }
}

/// Materializes one content-space viewport window with canonical clipping.
fn render_viewport_content(
    content: TuiViewportContent,
    window: TuiViewportWindow,
    available_width: u16,
    ctx: &mut TuiLayoutContext,
    app: &AppContext,
) -> TuiBuffer {
    let area = TuiRect::new(0, 0, available_width, window.viewport_height);
    let mut visible_elements =
        layout_visible_elements(content, window, 0, available_width, ctx, app);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(ctx.rendered_views);
    {
        let origin = TuiScreenPosition::new(0, 0);
        let mut surface = TuiPaintSurface::new(&mut buffer);
        render_visible_elements(
            &mut visible_elements,
            origin,
            area.as_size(),
            &mut surface,
            &mut paint_ctx,
        );
    }
    buffer
}

/// Toggles reverse video over selected absolute bounds.
fn toggle_selection_reverse(
    surface: &mut TuiPaintSurface<'_>,
    origin: TuiScreenPosition,
    size: TuiSize,
) {
    for row in 0..size.height {
        for col in 0..size.width {
            let Some(cell) = surface.cell_mut(origin.offset(i32::from(col), i32::from(row))) else {
                continue;
            };
            if cell.modifier.contains(super::Modifier::REVERSED) {
                cell.modifier.remove(super::Modifier::REVERSED);
            } else {
                cell.modifier.insert(super::Modifier::REVERSED);
            }
        }
    }
}

/// A variable-height viewport that delegates content slicing to its source.
pub struct TuiViewportedList<Content>
where
    Content: TuiViewportedElement,
{
    state: TuiViewportedListState,
    content: Content,
    visible_elements: Vec<VisibleElement>,
    content_height: usize,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
    vertical_alignment: TuiViewportVerticalAlignment,
    selection_snapshot: RefCell<Option<(TuiResolvedViewport, TuiBuffer)>>,
}

impl<Content> TuiViewportedList<Content>
where
    Content: TuiViewportedElement,
{
    /// Creates a generalized viewport over `content`.
    pub fn new(state: TuiViewportedListState, content: Content) -> Self {
        Self {
            state,
            content,
            visible_elements: Vec::new(),
            content_height: 0,
            size: None,
            origin: None,
            vertical_alignment: TuiViewportVerticalAlignment::Top,
            selection_snapshot: RefCell::new(None),
        }
    }

    pub fn with_vertical_alignment(
        mut self,
        vertical_alignment: TuiViewportVerticalAlignment,
    ) -> Self {
        self.vertical_alignment = vertical_alignment;
        self
    }

    fn set_position(&mut self, position: TuiViewportPosition) {
        if self.state.position() != position {
            self.state.set_position(position);
        }
    }

    fn requested_scroll_top(&self, viewport_height: usize) -> usize {
        match self.state.position() {
            TuiViewportPosition::End => max_scroll_top(self.content_height, viewport_height),
            TuiViewportPosition::RowsFromTop(scroll_top) => scroll_top,
        }
    }

    fn viewport_content(
        &mut self,
        scroll_top: usize,
        viewport_height: u16,
        available_width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> (usize, TuiViewportContent) {
        let viewport_height_rows = usize::from(viewport_height);
        let mut content = self.content.visible_items(
            TuiViewportWindow {
                scroll_top,
                viewport_height,
            },
            available_width,
            ctx,
            app,
        );
        let max_scroll_top = max_scroll_top(content.content_height, viewport_height_rows);
        let clamped_scroll_top = match self.state.position() {
            TuiViewportPosition::End => max_scroll_top,
            TuiViewportPosition::RowsFromTop(_) => scroll_top.min(max_scroll_top),
        };

        if clamped_scroll_top != scroll_top {
            content = self.content.visible_items(
                TuiViewportWindow {
                    scroll_top: clamped_scroll_top,
                    viewport_height,
                },
                available_width,
                ctx,
                app,
            );
        }

        if matches!(self.state.position(), TuiViewportPosition::RowsFromTop(_))
            && scroll_top > 0
            && scroll_top >= max_scroll_top
        {
            self.set_position(TuiViewportPosition::End);
        }

        (clamped_scroll_top, content)
    }

    fn layout_visible_elements(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) {
        let viewport_height = constraint.max.height;
        let viewport_height_rows = usize::from(viewport_height);
        let available_width = constraint.max.width;
        let requested_scroll_top = self.requested_scroll_top(viewport_height_rows);
        let (scroll_top, content) = self.viewport_content(
            requested_scroll_top,
            viewport_height,
            available_width,
            ctx,
            app,
        );

        self.content_height = content.content_height;
        self.layout_viewport_content(
            scroll_top,
            viewport_height_rows,
            available_width,
            content,
            ctx,
            app,
        );
    }

    fn layout_viewport_content(
        &mut self,
        scroll_top: usize,
        viewport_height_rows: usize,
        available_width: u16,
        content: TuiViewportContent,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) {
        let bottom_alignment_offset =
            if matches!(
                self.vertical_alignment,
                TuiViewportVerticalAlignment::GrowFromBottom
            ) && matches!(self.state.position(), TuiViewportPosition::End)
                && content.content_height < viewport_height_rows
            {
                viewport_height_rows.saturating_sub(content.content_height)
            } else {
                0
            };
        let window = TuiViewportWindow {
            scroll_top,
            viewport_height: viewport_height_rows.min(usize::from(u16::MAX)) as u16,
        };
        let screen_offset = bottom_alignment_offset.min(usize::from(u16::MAX)) as u16;
        self.state.set_resolved_viewport(TuiResolvedViewport {
            window,
            content_height: content.content_height,
            screen_offset,
        });
        self.visible_elements =
            layout_visible_elements(content, window, screen_offset, available_width, ctx, app);
    }

    /// Scrolls by content rows using the viewport's canonical position model.
    fn scroll_by(&mut self, rows: isize, viewport_height: usize) -> bool {
        if rows == 0 || viewport_height == 0 {
            return false;
        }
        let max_scroll_top = max_scroll_top(self.content_height, viewport_height);
        let current_scroll_top = match self.state.position() {
            TuiViewportPosition::End => max_scroll_top,
            TuiViewportPosition::RowsFromTop(scroll_top) => scroll_top.min(max_scroll_top),
        };
        let next_scroll_top = if rows < 0 {
            current_scroll_top.saturating_sub(rows.unsigned_abs())
        } else {
            current_scroll_top
                .saturating_add(rows as usize)
                .min(max_scroll_top)
        };
        if next_scroll_top == current_scroll_top {
            return false;
        }
        self.set_position(if next_scroll_top == max_scroll_top {
            TuiViewportPosition::End
        } else {
            TuiViewportPosition::RowsFromTop(next_scroll_top)
        });
        if let Some(mut resolved) = self.state.resolved_viewport() {
            resolved.window.scroll_top = next_scroll_top;
            self.state.set_resolved_viewport(resolved);
        }
        true
    }

    /// Maps a screen point into the latest resolved content window.
    fn resolve_selection_point(
        &self,
        position: super::TuiLocalPoint,
        size: TuiSize,
        clamp_outside: bool,
    ) -> Option<TuiGridPoint> {
        let resolved = self.state.resolved_viewport()?;
        if resolved.content_height == 0 || size.width == 0 || size.height == 0 {
            return None;
        }
        let content_top = i32::from(resolved.screen_offset);
        let visible_height = size.height.saturating_sub(resolved.screen_offset);
        let visible_content_height = min(
            usize::from(visible_height),
            resolved
                .content_height
                .saturating_sub(resolved.window.scroll_top),
        );
        if visible_content_height == 0 {
            return None;
        }
        let row_in_view = if clamp_outside {
            position.y.saturating_sub(content_top).clamp(
                0,
                i32::try_from(visible_content_height.saturating_sub(1)).unwrap_or(i32::MAX),
            ) as usize
        } else {
            if position.x < 0
                || position.x >= i32::from(size.width)
                || position.y < content_top
                || usize::try_from(position.y.saturating_sub(content_top)).ok()?
                    >= visible_content_height
            {
                return None;
            }
            usize::try_from(position.y - content_top).ok()?
        };
        Some(TuiGridPoint {
            row: resolved
                .window
                .scroll_top
                .saturating_add(row_in_view)
                .min(resolved.content_height.saturating_sub(1)),
            col: u16::try_from(position.x.clamp(0, i32::from(size.width.saturating_sub(1))))
                .unwrap_or_default(),
        })
    }

    /// Materializes selectable rows using the content's direct hook.
    fn selection_rows(
        &self,
        rows: Range<usize>,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Option<TuiBuffer> {
        let window = TuiViewportWindow {
            scroll_top: rows.start,
            viewport_height: rows.len().min(usize::from(u16::MAX)) as u16,
        };
        let content = self.content.selection_content(window, width, app)?;
        Some(render_viewport_content(content, window, width, ctx, app))
    }

    /// Returns rendered glyphs for one selectable content row.
    fn resolve_selection_row_glyphs(
        &self,
        row: usize,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Vec<super::TuiRowGlyph> {
        if let Some((resolved, snapshot)) = self.selection_snapshot.borrow().as_ref() {
            let row_in_snapshot = row.saturating_sub(resolved.window.scroll_top);
            if row >= resolved.window.scroll_top
                && row_in_snapshot < usize::from(snapshot.area.height)
            {
                return row_glyphs(snapshot, row_in_snapshot as u16, width);
            }
        }
        self.selection_rows(row..row.saturating_add(1), width, ctx, app)
            .map(|buffer| row_glyphs(&buffer, 0, width))
            .unwrap_or_default()
    }

    /// Extracts selected text from current read-only content rows.
    fn selection_text(
        &self,
        selection: TuiSelectionSpan,
        size: TuiSize,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> Option<String> {
        let end_row_exclusive = if selection.end.col == 0 {
            selection.end.row
        } else {
            selection.end.row.saturating_add(1)
        };
        if selection.start.row >= end_row_exclusive {
            return None;
        }
        // Prefer the content's logical text so soft-wrapped rows rejoin without
        // inserted newlines or rendered wrap indentation, and the full selected
        // range is captured. Content that has no logical form returns `None`,
        // and we fall back to per-row grid extraction below.
        if let Some(logical) = self
            .content
            .selection_logical_text(selection, size.width, app)
        {
            return Some(logical);
        }
        let mut lines = Vec::new();
        let mut chunk_start = selection.start.row;
        while chunk_start < end_row_exclusive {
            let chunk_end = min(
                end_row_exclusive,
                chunk_start.saturating_add(usize::from(u16::MAX)),
            );
            let buffer = self.selection_rows(chunk_start..chunk_end, size.width, ctx, app)?;
            for row in chunk_start..chunk_end {
                let buffer_row = row.saturating_sub(chunk_start) as u16;
                let start_col = if row == selection.start.row {
                    selection.start.col
                } else {
                    0
                };
                let end_col = if row == selection.end.row {
                    selection.end.col
                } else {
                    size.width
                };
                lines.push(row_text(&buffer, buffer_row, start_col..end_col));
            }
            chunk_start = chunk_end;
        }
        Some(lines.join("\n"))
    }
}

impl<Content> TuiElement for TuiViewportedList<Content>
where
    Content: TuiViewportedElement,
{
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.layout_visible_elements(constraint, ctx, app);
        let size = constraint.max;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        for visible in &mut self.visible_elements {
            visible.element.after_layout(ctx, app);
        }
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else {
            return;
        };
        render_visible_elements(&mut self.visible_elements, origin, size, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for visible in &mut self.visible_elements {
            visible.element.present(ctx);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.visible_elements
            .iter_mut()
            .fold(false, |handled, visible| {
                visible.element.dispatch_event(event, event_ctx, app) || handled
            })
    }
}

impl<Content> TuiScrollableElement for TuiViewportedList<Content>
where
    Content: TuiViewportedElement,
{
    fn scroll_by_rows(&mut self, rows: isize, viewport_height: usize) -> bool {
        self.scroll_by(rows, viewport_height)
    }
}

fn max_scroll_top(content_height: usize, viewport_height: usize) -> usize {
    content_height.saturating_sub(viewport_height)
}

#[cfg(test)]
#[path = "viewported_list_tests.rs"]
mod tests;
