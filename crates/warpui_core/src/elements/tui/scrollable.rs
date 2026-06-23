//! [`TuiScrollable`]: a fixed-height vertical viewport that clips a taller child
//! and scrolls it with the mouse wheel and keyboard, keeping its scroll offset
//! in a [`TuiScrollHandle`] across redraws.
//!
//! # Construction
//! Create a [`TuiScrollHandle`] once in the host view (like a `MouseStateHandle`)
//! and clone it into [`TuiScrollable::new`] each render; constructing the handle
//! inline in `render` would reset the scroll position every frame. Give the
//! scrollable a bounded slot — typically a
//! [`flex_child`](super::TuiColumn::flex_child) of a
//! [`TuiColumn`](super::TuiColumn) under a fixed header — so its viewport is
//! shorter than its content and there is something to scroll.
//!
//! # Layout & paint
//! The scrollable fills the slot it is given. It measures its child at the
//! viewport width and full content height, then — because the element layer does
//! not clip a child to its `area` — paints the child into an off-screen,
//! content-sized buffer and copies the visible row window into the viewport,
//! clipping both above and below.
//!
//! # Input
//! [`dispatch_event`](TuiElement::dispatch_event) scrolls on a
//! [`ScrollWheel`](crate::Event::ScrollWheel) whose position is inside the
//! viewport, and on the `up`/`down`/`pageup`/`pagedown`/`home`/`end` keys. The
//! offset is clamped to the content, and the event is reported handled only when
//! the offset actually changes, so other handlers still see no-op scroll keys.

use std::cell::Cell;
use std::rc::Rc;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext, TuiPresentationContext,
    TuiRect, TuiSize,
};
use crate::geometry::vector::Vector2F;
use crate::{AppContext, Event};

/// Rows scrolled per mouse-wheel tick.
const WHEEL_STEP: u16 = 3;

/// A persistent vertical scroll offset (rows from the top of the content),
/// shared between a host view and the [`TuiScrollable`] it renders.
///
/// Create it once in the view and clone it into the element each render
/// (constructing it inline in `render` would reset the position every frame).
/// The host can also drive it directly — e.g. `set_offset(u16::MAX)` to follow
/// the bottom as new content arrives; out-of-range values clamp to the content
/// on the next layout.
#[derive(Clone, Default)]
pub struct TuiScrollHandle(Rc<Cell<u16>>);

impl TuiScrollHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current scroll offset, in rows from the top of the content.
    pub fn offset(&self) -> u16 {
        self.0.get()
    }

    /// Sets the scroll offset, in rows from the top of the content.
    pub fn set_offset(&self, offset: u16) {
        self.0.set(offset);
    }
}

/// A vertical viewport that clips and scrolls a single child. See the module
/// docs for construction and behavior.
pub struct TuiScrollable {
    handle: TuiScrollHandle,
    child: Box<dyn TuiElement>,
    /// The child's full height at the viewport width, cached during `layout`.
    content_height: u16,
    /// The width the child was measured/painted at, cached during `layout`.
    viewport_width: u16,
}

impl TuiScrollable {
    pub fn new(handle: TuiScrollHandle, child: impl TuiElement + 'static) -> Self {
        Self {
            handle,
            child: Box::new(child),
            content_height: 0,
            viewport_width: 0,
        }
    }

    /// The largest in-range offset: how far the content can scroll before its
    /// bottom reaches the bottom of a `viewport_height`-row viewport.
    fn max_offset(content_height: u16, viewport_height: u16) -> u16 {
        content_height.saturating_sub(viewport_height)
    }
}

impl TuiElement for TuiScrollable {
    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        // Measure the child at full (unconstrained) height so it reports its
        // natural content height. The viewport clips during render.
        let content_size = self
            .child
            .layout(TuiConstraint::loose(TuiSize::new(width, u16::MAX)), ctx);
        let content_height = content_size.height;
        self.content_height = content_height;
        self.viewport_width = width;

        // Clamp a now-out-of-range offset (content shrank or the viewport grew)
        // and store it back so render and the next event share a valid value.
        let max_offset = Self::max_offset(content_height, constraint.max.height);
        self.handle.set_offset(self.handle.offset().min(max_offset));

        // Fill the slot we are given (like the GUI `Expanded`).
        constraint.max
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        if area.is_empty() || self.content_height == 0 {
            return;
        }

        // Paint the whole child into an off-screen, content-sized buffer, then
        // copy the visible row window into `area`. Cloning whole cells preserves
        // wide / zero-width grapheme columns across the copy.
        let content_rect = TuiRect::new(0, 0, area.width, self.content_height);
        let mut content = TuiBuffer::empty(content_rect);
        self.child.render(content_rect, &mut content, ctx);

        let max_offset = Self::max_offset(self.content_height, area.height);
        let offset = self.handle.offset().min(max_offset);
        let visible = area.height.min(self.content_height.saturating_sub(offset));
        for row in 0..visible {
            let src_y = offset + row;
            let dst_y = area.y + row;
            for x in 0..area.width {
                if let Some(cell) = content.cell((x, src_y)).cloned() {
                    if let Some(dst) = buffer.cell_mut((area.x + x, dst_y)) {
                        *dst = cell;
                    }
                }
            }
        }
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        _event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        if area.is_empty() {
            return false;
        }
        // Use the content height cached during layout rather than re-measuring.
        let content_height = self.content_height;
        let max_offset = Self::max_offset(content_height, area.height);
        if max_offset == 0 {
            return false;
        }
        let current = self.handle.offset().min(max_offset);

        let next = match event {
            Event::ScrollWheel {
                position, delta, ..
            } => {
                if !contains(area, *position) {
                    return false;
                }
                // Wheel up has a positive delta and scrolls toward the top.
                offset_by(
                    current,
                    -i32::from(WHEEL_STEP) * (delta.y() as i32),
                    max_offset,
                )
            }
            Event::KeyDown { keystroke, .. } => {
                let page = i32::from(area.height.saturating_sub(1).max(1));
                match keystroke.key.as_str() {
                    "down" => offset_by(current, 1, max_offset),
                    "up" => offset_by(current, -1, max_offset),
                    "pagedown" => offset_by(current, page, max_offset),
                    "pageup" => offset_by(current, -page, max_offset),
                    "home" => 0,
                    "end" => max_offset,
                    _ => return false,
                }
            }
            _ => return false,
        };

        if next == current {
            return false;
        }
        self.handle.set_offset(next);
        true
    }
}

/// Applies a signed row delta to `current`, clamping into `[0, max_offset]`.
fn offset_by(current: u16, delta_rows: i32, max_offset: u16) -> u16 {
    (i32::from(current) + delta_rows).clamp(0, i32::from(max_offset)) as u16
}

/// Whether `position` (in terminal cells) lies within `area`.
fn contains(area: TuiRect, position: Vector2F) -> bool {
    let x = position.x();
    let y = position.y();
    x >= f32::from(area.x)
        && x < f32::from(area.right())
        && y >= f32::from(area.y)
        && y < f32::from(area.bottom())
}

#[cfg(test)]
#[path = "scrollable_tests.rs"]
mod tests;
