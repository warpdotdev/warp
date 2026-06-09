//! Integer cell-grid geometry shared across the TUI rendering layers.
//!
//! All dimensions are in terminal cells (`u16`). The geometry deliberately
//! mirrors a small slice of the GUI geometry vocabulary but stays integral,
//! since a terminal grid has no sub-cell positions.

use warpui_core::geometry::vector::Vector2F;

/// A width/height pair in terminal cells.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiSize {
    pub width: u16,
    pub height: u16,
}

impl TuiSize {
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
    };

    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }

    /// The number of cells covered by a region of this size.
    pub fn area(self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }
}

/// An axis-aligned rectangle of terminal cells. The covered columns are
/// `x .. x + width` and rows `y .. y + height` (half-open).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl TuiRect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Builds a rect at the origin with the given size.
    pub const fn from_size(size: TuiSize) -> Self {
        Self::new(0, 0, size.width, size.height)
    }

    pub const fn size(self) -> TuiSize {
        TuiSize::new(self.width, self.height)
    }

    /// The first column past the right edge (saturating).
    pub fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// The first row past the bottom edge (saturating).
    pub fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Shrinks the rect by `inset` cells on every side, saturating at zero. A
    /// rect too small to inset collapses to zero width/height (never wraps).
    ///
    /// ```
    /// # use warpui_tui::TuiRect;
    /// assert_eq!(
    ///     TuiRect::new(2, 2, 10, 6).inset(1),
    ///     TuiRect::new(3, 3, 8, 4),
    /// );
    /// assert_eq!(TuiRect::new(0, 0, 1, 1).inset(2), TuiRect::new(2, 2, 0, 0));
    /// ```
    pub fn inset(self, inset: u16) -> Self {
        let shrink = inset.saturating_mul(2);
        Self {
            x: self.x.saturating_add(inset),
            y: self.y.saturating_add(inset),
            width: self.width.saturating_sub(shrink),
            height: self.height.saturating_sub(shrink),
        }
    }

    /// Splits off the top `height` rows, returning `(top, remainder)`. `height`
    /// is clamped to this rect's height, so the two halves always tile the
    /// original rect exactly.
    ///
    /// ```
    /// # use warpui_tui::TuiRect;
    /// let (top, rest) = TuiRect::new(0, 0, 8, 5).split_top(2);
    /// assert_eq!(top, TuiRect::new(0, 0, 8, 2));
    /// assert_eq!(rest, TuiRect::new(0, 2, 8, 3));
    /// ```
    pub fn split_top(self, height: u16) -> (Self, Self) {
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

    /// Splits off the left `width` columns, returning `(left, remainder)`.
    /// `width` is clamped to this rect's width, so the two halves always tile
    /// the original rect exactly.
    ///
    /// ```
    /// # use warpui_tui::TuiRect;
    /// let (left, rest) = TuiRect::new(0, 0, 8, 5).split_left(3);
    /// assert_eq!(left, TuiRect::new(0, 0, 3, 5));
    /// assert_eq!(rest, TuiRect::new(3, 0, 5, 5));
    /// ```
    pub fn split_left(self, width: u16) -> (Self, Self) {
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

    /// Whether a (sub-cell) point falls inside this rect, used for mouse
    /// hit-testing against crossterm's pixel-free cell coordinates.
    pub fn contains_position(self, position: Vector2F) -> bool {
        position.x() >= f32::from(self.x)
            && position.x() < f32::from(self.right())
            && position.y() >= f32::from(self.y)
            && position.y() < f32::from(self.bottom())
    }
}

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
    /// # use warpui_tui::{TuiConstraint, TuiSize};
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

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
