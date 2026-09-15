use warpui_core::units::Lines;

use crate::runtime::SizeInfo;

/// The reason that the terminal size is being updated
#[derive(Debug, Copy, Clone)]
pub enum SizeUpdateReason {
    /// Updated because of some general refresh (e.g. a font-size change)
    Refresh,

    /// Updated after the temrinal has been laid out, so some of the element
    /// sizes that drive terminal size may have changed.
    AfterLayout,

    /// The shared session sharer's size changed.
    /// This is only applicable for shared session viewers.
    ///
    /// The resultant [`SizeUpdate`] will use the larger of the
    /// sharer's and viewer's size.
    SharerSizeChanged { num_rows: usize, num_cols: usize },

    /// A viewer reported its terminal size to the sharer.
    /// This is only applicable for shared session sharers.
    ///
    /// The resultant [`SizeUpdate`] will use the viewer's reported
    /// size directly (floored at 1 row and 1 column).
    ViewerSizeReported { num_rows: usize, num_cols: usize },
}

/// Encapsulates info for updating the size of the terminal.
#[derive(Debug, Copy, Clone)]
pub struct SizeUpdate {
    /// The reason for the update.
    pub update_reason: SizeUpdateReason,

    /// The last size info.
    pub last_size: SizeInfo,

    /// The new size info.
    pub new_size: SizeInfo,

    /// The new gap height, if there is one.
    pub new_gap_height: Option<Lines>,

    /// The pane-computed rows before any shared session size adjustments.
    pub natural_rows: usize,

    /// The pane-computed columns before any shared session size adjustments.
    pub natural_cols: usize,
}

impl SizeUpdate {
    /// Creates a size update for a layout measured directly in terminal cells.
    pub fn from_cell_dimensions(last_size: SizeInfo, rows: usize, columns: usize) -> Self {
        let new_size = SizeInfo::new_without_font_metrics(rows, columns);
        Self {
            update_reason: SizeUpdateReason::AfterLayout,
            last_size,
            new_size,
            new_gap_height: None,
            natural_rows: new_size.rows(),
            natural_cols: new_size.columns(),
        }
    }

    /// Returns the resulting terminal size.
    pub fn new_size(&self) -> SizeInfo {
        self.new_size
    }

    /// Whether the reason for the update is a refresh.
    pub fn is_refresh(&self) -> bool {
        matches!(self.update_reason, SizeUpdateReason::Refresh)
    }

    /// Returns whether there was any change with this update.
    pub fn anything_changed(&self) -> bool {
        self.pane_size_changed() || self.gap_height_changed() || self.rows_or_columns_changed()
    }

    pub fn rows_or_columns_changed(&self) -> bool {
        self.last_size.columns() != self.new_size.columns()
            || self.last_size.rows() != self.new_size.rows()
    }

    /// Returns whether the pane size changed with this update
    pub fn pane_size_changed(&self) -> bool {
        // It's fine for this to be a near-exact comparison because pane size
        // is not determined by summing floats like content element size is.
        (self.last_size.pane_size_px().x() - self.new_size.pane_size_px().x()).abs() > f32::EPSILON
            || (self.last_size.pane_size_px().y() - self.new_size.pane_size_px().y()).abs()
                > f32::EPSILON
    }

    /// Returns any new gap height to set with this update
    pub fn new_gap_height(&self) -> Option<Lines> {
        self.new_gap_height
    }

    /// Returns whether the gap height changed with this update
    pub fn gap_height_changed(&self) -> bool {
        self.new_gap_height.is_some()
    }

    /// The pane-computed natural rows before shared session adjustments.
    pub fn natural_rows(&self) -> usize {
        self.natural_rows
    }

    /// The pane-computed natural columns before shared session adjustments.
    pub fn natural_cols(&self) -> usize {
        self.natural_cols
    }

    /// Returns true if this resize was caused by a sharer size change.
    pub fn is_sharer_size_change(&self) -> bool {
        matches!(
            self.update_reason,
            SizeUpdateReason::SharerSizeChanged { .. }
        )
    }
}

#[cfg(test)]
#[path = "size_update_tests.rs"]
mod tests;
