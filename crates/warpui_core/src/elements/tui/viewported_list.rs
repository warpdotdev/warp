//! A source-driven viewport for ordered, variable-height TUI content.
//!
//! The caller owns item storage, identity, traversal, and height reconciliation;
//! this element owns the viewport window, scroll clamping, and the normal TUI
//! element lifecycle for the currently visible child elements.

use std::cell::RefCell;
use std::rc::Rc;

use super::{
    TuiBuffer, TuiClipped, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiScrollableElement, TuiSize,
};
use crate::AppContext;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiViewportPosition {
    End,
    RowsFromTop(usize),
}

/// Shared storage for a caller-owned viewport position.
#[derive(Clone)]
pub struct TuiViewportedListState(Rc<RefCell<TuiViewportPosition>>);

impl TuiViewportedListState {
    /// Creates viewport position storage initially following the content end.
    pub fn new_at_end() -> Self {
        Self(Rc::new(RefCell::new(TuiViewportPosition::End)))
    }

    /// Returns the current caller-owned viewport position.
    pub fn position(&self) -> TuiViewportPosition {
        self.0.borrow().clone()
    }

    /// Stores a new caller-owned viewport position.
    pub fn set_position(&self, position: TuiViewportPosition) {
        *self.0.borrow_mut() = position;
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
        matches!(*self.0.borrow(), TuiViewportPosition::End)
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
    /// measurement, not horizontal viewport state.
    fn visible_items(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
        app: &AppContext,
    ) -> TuiViewportContent;
}

struct VisibleElement {
    // Viewport-local render coordinates use `u16` to match TUI geometry.
    viewport_y: u16,
    height: u16,
    element: TuiClipped,
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
    size: TuiSize,
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
            size: TuiSize::ZERO,
        }
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
        app: &AppContext,
    ) -> (usize, TuiViewportContent) {
        let viewport_height_rows = usize::from(viewport_height);
        let mut content = self.content.visible_items(
            TuiViewportWindow {
                scroll_top,
                viewport_height,
            },
            available_width,
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
                app,
            );
        }

        if matches!(self.state.position(), TuiViewportPosition::RowsFromTop(_))
            && scroll_top > max_scroll_top
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
        self.visible_elements.clear();
        let viewport_height = constraint.max.height;
        let viewport_height_rows = usize::from(viewport_height);
        let available_width = constraint.max.width;
        let requested_scroll_top = self.requested_scroll_top(viewport_height_rows);
        let (scroll_top, content) =
            self.viewport_content(requested_scroll_top, viewport_height, available_width, app);

        self.content_height = content.content_height;

        let viewport_bottom = scroll_top.saturating_add(viewport_height_rows);
        for item in content.items {
            let mut element = item.element;
            let full_size = element.layout(
                TuiConstraint::loose(TuiSize::new(available_width, u16::MAX)),
                ctx,
                app,
            );
            let item_top = item.origin_y;
            let item_bottom = item_top.saturating_add(usize::from(full_size.height));
            let visible_top = item_top.max(scroll_top);
            let visible_bottom = item_bottom.min(viewport_bottom);
            if visible_top >= visible_bottom {
                continue;
            }

            let viewport_y = visible_top.saturating_sub(scroll_top);
            let viewport_origin_y = visible_top.saturating_sub(item_top);
            let height = visible_bottom.saturating_sub(visible_top);
            let viewport_y = viewport_y.min(usize::from(u16::MAX)) as u16;
            let height = height.min(usize::from(u16::MAX)) as u16;
            let element = TuiClipped::from_boxed(element).with_viewport_origin_y(viewport_origin_y);
            self.visible_elements.push(VisibleElement {
                viewport_y,
                height,
                element,
            });
        }
    }

    /// Scrolls the viewport by `rows` (negative = toward the top), clamping at
    /// both ends and restoring `End` when the viewport reaches the bottom.
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

        if next_scroll_top == max_scroll_top {
            self.set_position(TuiViewportPosition::End);
        } else {
            self.set_position(TuiViewportPosition::RowsFromTop(next_scroll_top));
        }
        true
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
        self.size = constraint.max;
        self.size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        for visible in &self.visible_elements {
            let slot_y = area.y.saturating_add(visible.viewport_y);
            if slot_y >= area.bottom() {
                continue;
            }
            let height = visible.height.min(area.bottom() - slot_y);
            let slot = TuiRect::new(area.x, slot_y, area.width, height);
            visible.element.render(slot, buffer, ctx);
        }
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        for visible in &self.visible_elements {
            let slot_y = area.y.saturating_add(visible.viewport_y);
            if slot_y >= area.bottom() {
                continue;
            }
            let height = visible.height.min(area.bottom() - slot_y);
            let slot = TuiRect::new(area.x, slot_y, area.width, height);
            let (x, y) = visible.element.cursor_position(slot, ctx)?;
            if y < height {
                return Some((x, slot_y.saturating_sub(area.y).saturating_add(y)));
            }
        }
        None
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for visible in &mut self.visible_elements {
            visible.element.present(ctx);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        for visible in &mut self.visible_elements {
            let slot_y = area.y.saturating_add(visible.viewport_y);
            if slot_y >= area.bottom() {
                continue;
            }
            let height = visible.height.min(area.bottom() - slot_y);
            let slot = TuiRect::new(area.x, slot_y, area.width, height);
            if visible
                .element
                .dispatch_event(event, slot, event_ctx, ctx, app)
            {
                return true;
            }
        }
        false
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
