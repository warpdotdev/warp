//! Integer cell-grid geometry for the TUI rendering layers.
//!
//! Sizes and rectangles are ratatui's own `Size`/`Rect` (re-exported as
//! [`TuiSize`]/[`TuiRect`]) so the element tree measures and paints in the same
//! coordinate types the ratatui `Buffer` uses, with no conversions.
//!
//! [`TuiConstraint`] is a local min/max measure box. It is deliberately *not*
//! ratatui's `Constraint`, which is the input to ratatui's layout *solver*
//! (Length/Min/Max/Fill/…), not a bound an element clamps its natural size
//! against. [`TuiRectExt`] adds the few slicing helpers the hand-rolled
//! column/container layout needs that ratatui's `Rect` does not provide.

pub use ratatui::layout::{Rect as TuiRect, Size as TuiSize};

/// A layout constraint: an element handed a `TuiConstraint` must return a
/// [`TuiSize`] with `min.width <= width <= max.width` and
/// `min.height <= height <= max.height`. Containers shrink their children by
/// handing down tighter constraints; leaves clamp their natural size with
/// [`clamp`](TuiConstraint::clamp).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiConstraint {
    pub min: TuiSize,
    pub max: TuiSize,
}

impl TuiConstraint {
    pub const fn new(min: TuiSize, max: TuiSize) -> Self {
        Self { min, max }
    }

    /// A constraint forcing exactly `size` (min == max).
    pub const fn tight(size: TuiSize) -> Self {
        Self {
            min: size,
            max: size,
        }
    }

    /// A constraint allowing anything from zero up to `max`.
    pub const fn loose(max: TuiSize) -> Self {
        Self {
            min: TuiSize::ZERO,
            max,
        }
    }

    /// Clamps a desired size into the `[min, max]` box on each axis.
    ///
    /// ```
    /// # use warpui_core::elements::tui::{TuiConstraint, TuiSize};
    /// let constraint = TuiConstraint::new(TuiSize::new(2, 1), TuiSize::new(10, 4));
    /// assert_eq!(constraint.clamp(TuiSize::new(7, 2)), TuiSize::new(7, 2));
    /// assert_eq!(constraint.clamp(TuiSize::new(99, 0)), TuiSize::new(10, 1));
    /// assert_eq!(constraint.clamp(TuiSize::new(0, 99)), TuiSize::new(2, 4));
    /// ```
    pub fn clamp(self, size: TuiSize) -> TuiSize {
        TuiSize::new(
            size.width.clamp(self.min.width, self.max.width),
            size.height.clamp(self.min.height, self.max.height),
        )
    }

    /// Clamps a width into `[min.width, max.width]`.
    pub fn constrain_width(self, width: u16) -> u16 {
        width.clamp(self.min.width, self.max.width)
    }

    /// Clamps a height into `[min.height, max.height]`.
    pub fn constrain_height(self, height: u16) -> u16 {
        height.clamp(self.min.height, self.max.height)
    }
}

/// Slicing helpers for [`TuiRect`] used by the hand-rolled stacking layout.
///
/// ratatui's `Rect` already provides `is_empty`/`right`/`bottom`/`area`; this
/// trait adds the symmetric inset and the top/left splits the column and
/// container rely on. Each split clamps to the rect's extent, so the two halves
/// always tile the original rect exactly.
pub trait TuiRectExt: Sized {
    /// Shrinks the rect by `inset` cells on every side, saturating at zero. A
    /// rect too small to inset advances its origin but collapses to zero
    /// width/height (it never wraps).
    fn inset(self, inset: u16) -> Self;

    /// Splits off the top `height` rows, returning `(top, remainder)`.
    fn split_top(self, height: u16) -> (Self, Self);

    /// Splits off the left `width` columns, returning `(left, remainder)`.
    fn split_left(self, width: u16) -> (Self, Self);
}

impl TuiRectExt for TuiRect {
    fn inset(self, inset: u16) -> Self {
        let shrink = inset.saturating_mul(2);
        Self {
            x: self.x.saturating_add(inset),
            y: self.y.saturating_add(inset),
            width: self.width.saturating_sub(shrink),
            height: self.height.saturating_sub(shrink),
        }
    }

    fn split_top(self, height: u16) -> (Self, Self) {
        let top_height = height.min(self.height);
        let top = Self::new(self.x, self.y, self.width, top_height);
        let remainder = Self::new(
            self.x,
            self.y.saturating_add(top_height),
            self.width,
            self.height - top_height,
        );
        (top, remainder)
    }

    fn split_left(self, width: u16) -> (Self, Self) {
        let left_width = width.min(self.width);
        let left = Self::new(self.x, self.y, left_width, self.height);
        let remainder = Self::new(
            self.x.saturating_add(left_width),
            self.y,
            self.width - left_width,
            self.height,
        );
        (left, remainder)
    }
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
