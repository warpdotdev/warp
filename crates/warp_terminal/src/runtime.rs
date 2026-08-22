use std::cmp::max;

use ordered_float::Float;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use serde::{Deserialize, Serialize};
use warpui_core::units::{IntoPixels, Pixels};

use crate::model::Side;

/// Minimum number of visible lines.
const MIN_ROWS: usize = 1;

/// Minimum number of columns.
///
/// A minimum of 2 is necessary to hold fullwidth unicode characters.
const MIN_COLUMNS: usize = 2;
/// Terminal size info.
///
/// Note that this implements Serialize/Deserialize for ref tests.
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeInfo {
    /// The width of the TerminalView pane.
    ///
    /// This is basically the `x` of the the incoming max size constraint on the TerminalView. This
    /// is represented as a raw float rather than a Vector2F to satisfy the
    /// `Serialize`/`Deserialize` trait requirements.
    pub pane_width_px: f32,

    /// The height of the TerminalView pane.
    ///
    /// This is basically the `y` of the the incoming max size constraint on the TerminalView. This
    /// is represented as a raw float rather than a Vector2F to satisfy the
    /// `Serialize`/`Deserialize` trait requirements.
    pub pane_height_px: f32,

    /// This height in rows that the pty and grid model thinks the terminal is.
    ///
    /// Note that *rows* is always determined as a function of pane size, not
    /// the content element size, which is somewhat counterintuitive.  The reason
    /// is that the content element size changes frequenetly as the input size
    /// changes or the input dissapears for long running commands, but many
    /// programs do not handle size changes while they are running very well.  To
    /// get around this we make them think that rows always comes from the pane size.
    pub rows: usize,

    /// This is width in columns that the pty and grid model thinks the terminal is.
    pub columns: usize,

    /// Width of an individual cell.
    pub cell_width_px: Pixels,

    /// Height of an individual cell.
    pub cell_height_px: Pixels,

    /// Horizontal window padding.
    pub padding_x_px: Pixels,

    /// Vertical window padding.
    pub padding_y_px: Pixels,
}

/// Helper struct containing the cell size and window padding info.
pub struct CellSizeAndWindowPadding {
    /// Width of an individual cell.
    pub cell_width_px: Pixels,

    /// Height of an individual cell.
    pub cell_height_px: Pixels,

    /// Horizontal window padding.
    pub padding_x_px: Pixels,

    /// Vertical window padding.
    pub padding_y_px: Pixels,
}

impl SizeInfo {
    pub fn new(
        pane_size_px: Vector2F,
        cell_width_px: Pixels,
        cell_height_px: Pixels,
        padding_x_px: Pixels,
        padding_y_px: Pixels,
    ) -> SizeInfo {
        let rows = (pane_size_px.y() - 2. * padding_y_px.as_f32()) / cell_height_px.as_f32();
        let columns = (pane_size_px.x() - 2. * padding_x_px.as_f32()) / cell_width_px.as_f32();

        SizeInfo {
            pane_width_px: pane_size_px.x(),
            pane_height_px: pane_size_px.y(),
            columns: max(columns as usize, MIN_COLUMNS),
            rows: max(rows as usize, MIN_ROWS),
            cell_width_px,
            cell_height_px,
            padding_x_px: padding_x_px.floor(),
            padding_y_px: padding_y_px.floor(),
        }
    }

    /// Create SizeInfo for a [`TerminalModel`] instance that doesn't have font metrics,
    /// which comes from either a headless Warp instance or tests.
    pub fn new_without_font_metrics(rows: usize, cols: usize) -> Self {
        let width = cols as f32;
        let height = rows as f32;
        SizeInfo::new(
            vec2f(width, height),
            1.0.into_pixels(), /* cell_width */
            1.0.into_pixels(), /* cell_height */
            Pixels::zero(),    /* padding_x */
            Pixels::zero(),    /* padding_y */
        )
    }

    pub fn with_rows_and_columns(mut self, rows: usize, cols: usize) -> SizeInfo {
        self.columns = cols;
        self.rows = rows;
        self
    }

    /// Returns the side of the cell where the mouse is located.
    pub fn get_mouse_side(&self, position: Vector2F) -> Side {
        let x = position.x() as usize;

        let cell_x = x.saturating_sub(self.padding_x_px().as_f32() as usize)
            % self.cell_width_px().as_f32() as usize;
        let half_cell_width = (self.cell_width_px().as_f32() / 2.0) as usize;

        let additional_padding = (self.pane_width_px().as_f32()
            - self.padding_x_px().as_f32() * 2.)
            % self.cell_width_px().as_f32();
        let end_of_grid =
            self.pane_width_px() - self.padding_x_px() - additional_padding.into_pixels();

        if cell_x > half_cell_width
            // Edge case when mouse leaves the window.
            || x as f32 >= end_of_grid.as_f32()
        {
            Side::Right
        } else {
            Side::Left
        }
    }

    #[inline]
    pub fn pane_size_px(&self) -> Vector2F {
        vec2f(self.pane_width_px, self.pane_height_px)
    }

    #[inline]
    pub fn pane_width_px(&self) -> Pixels {
        self.pane_width_px.into_pixels()
    }

    #[inline]
    pub fn pane_height_px(&self) -> Pixels {
        self.pane_height_px.into_pixels()
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[inline]
    pub fn columns(&self) -> usize {
        self.columns
    }

    #[inline]
    pub fn cell_width_px(&self) -> Pixels {
        self.cell_width_px
    }

    #[inline]
    pub fn cell_height_px(&self) -> Pixels {
        self.cell_height_px
    }

    #[inline]
    pub fn padding_x_px(&self) -> Pixels {
        self.padding_x_px
    }

    #[inline]
    pub fn padding_y_px(&self) -> Pixels {
        self.padding_y_px
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ClipboardType {
    Clipboard,
    Selection,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub enum ImageProtocol {
    Kitty,
    ITerm,
}
