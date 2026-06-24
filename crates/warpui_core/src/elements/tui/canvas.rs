//! [`TuiCanvas`]: a leaf element that paints a pre-produced cell grid
//! ([`TuiBuffer`]) into the frame, regenerating it only when the layout width
//! changes.
//!
//! This is the element layer's "reuse the grid" primitive: anything that can
//! turn into a styled cell grid (for example shell-command output parsed from
//! ANSI) is produced once per width and then blitted into the destination
//! buffer on every redraw, the same cell-copy pattern
//! [`TuiScrollable`](super::TuiScrollable) uses.
//!
//! # Width/content caching
//! The produced grid depends on the width it is laid out at (line wrapping is
//! width-dependent) and on the content, identified by a caller-supplied
//! `generation` counter. The producer is only re-run when the width or the
//! generation changes, so a streaming caller bumps `generation` as new output
//! arrives and otherwise reuses the cached grid. The cache lives in a
//! [`TuiCanvasCache`] handle the host view keeps across redraws (like a
//! [`TuiScrollHandle`](super::TuiScrollHandle)); constructing it inline in
//! `render` would recompute the grid every frame.

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::text::Text;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::{TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiRect, TuiSize};

/// Rasterizes a ratatui [`Text`] into a [`TuiBuffer`] of the given width,
/// word-wrapping long lines (whitespace preserved). Returns an empty buffer for
/// a zero width. The grid this produces is what [`TuiCanvas`] blits.
pub fn rasterize_text(text: Text<'_>, width: u16) -> TuiBuffer {
    if width == 0 {
        return TuiBuffer::empty(TuiRect::new(0, 0, 0, 0));
    }

    // Measure and paint through `Paragraph` so wrapping/measurement agree, the
    // same way `TuiText` does.
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    let height = u16::try_from(paragraph.line_count(width))
        .unwrap_or(u16::MAX)
        .max(1);
    let area = TuiRect::new(0, 0, width, height);
    let mut buffer = TuiBuffer::empty(area);
    Widget::render(paragraph, area, &mut buffer);
    buffer
}

/// A persistent, width-keyed cache of a [`TuiCanvas`]'s produced grid, shared
/// between a host view and the element it renders.
///
/// Create it once in the view and clone it into the element each render
/// (constructing it inline in `render` would recompute the grid every frame).
#[derive(Clone, Default)]
pub struct TuiCanvasCache(Rc<RefCell<Option<(u16, u64, TuiBuffer)>>>);

impl TuiCanvasCache {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A leaf element that paints a width-dependent grid produced on demand. See the
/// module docs for construction and behavior.
pub struct TuiCanvas {
    cache: TuiCanvasCache,
    generation: u64,
    produce: Box<dyn Fn(u16) -> TuiBuffer>,
}

impl TuiCanvas {
    /// Builds a canvas that paints the grid `produce` returns for the width it
    /// is laid out at. `cache` persists the produced grid across redraws so the
    /// producer only runs when the width or `generation` changes; bump
    /// `generation` whenever the produced content would differ at the same width
    /// (e.g. streaming output grew).
    pub fn new(
        cache: TuiCanvasCache,
        generation: u64,
        produce: impl Fn(u16) -> TuiBuffer + 'static,
    ) -> Self {
        Self {
            cache,
            generation,
            produce: Box::new(produce),
        }
    }

    /// Ensures the cache holds the grid for `(width, generation)` (regenerating
    /// it only when either changes), then calls `f` with it.
    fn with_buffer<R>(&self, width: u16, f: impl FnOnce(&TuiBuffer) -> R) -> R {
        let mut slot = self.cache.0.borrow_mut();
        let fresh = matches!(
            slot.as_ref(),
            Some((cached_width, cached_generation, _))
                if *cached_width == width && *cached_generation == self.generation
        );
        if !fresh {
            *slot = Some((width, self.generation, (self.produce)(width)));
        }
        let (_, _, buffer) = slot.as_ref().expect("cache populated above");
        f(buffer)
    }
}

impl TuiElement for TuiCanvas {
    fn layout(&mut self, constraint: TuiConstraint, _ctx: &mut TuiLayoutContext) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        let height = self.with_buffer(width, |buffer| buffer.area.height);
        constraint.clamp(TuiSize::new(width, height))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {
        if area.is_empty() {
            return;
        }
        // Copy the produced grid into `area`, clamping to whichever is smaller.
        // Cloning whole cells preserves wide / zero-width grapheme columns.
        self.with_buffer(area.width, |grid| {
            let rows = grid.area.height.min(area.height);
            let cols = grid.area.width.min(area.width);
            for row in 0..rows {
                for col in 0..cols {
                    if let Some(cell) = grid.cell((col, row)).cloned() {
                        if let Some(dst) = buffer.cell_mut((area.x + col, area.y + row)) {
                            *dst = cell;
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "canvas_tests.rs"]
mod tests;
