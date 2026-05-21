use crate::terminal::model::{blocks::BlockList, index::Point, terminal_model::WithinBlock};
use session_sharing_protocol::common::BlockPoint;

impl WithinBlock<Point> {
    /// Converts an un-transformed block point
    /// to a transformed [`WithinBlock<Point>`].
    ///
    /// We make the following transformations:
    /// 1. ensure the point fits our grid,
    /// 2. turn the grid-compatible point into a displayed point so that it respects filters
    pub fn from_session_sharing_block_point(
        point: BlockPoint,
        block_list: &BlockList,
    ) -> Option<Self> {
        let block_index = block_list.block_index_for_id(&point.block_id.to_string().into())?;
        let grid_type = point.grid_type.into();
        let grid = block_list
            .block_at(block_index)?
            .grid_of_type(grid_type)?
            .grid_handler();

        let inner = grid.compatible_point(point.point.into());
        let inner = if !grid.is_displayed_row(inner.row) {
            return None;
        } else {
            grid.maybe_translate_point_from_original_to_displayed(inner)
        };

        Some(Self {
            block_index,
            grid: grid_type,
            inner,
        })
    }

    /// Converts a transformed [`WithinBlock<Point>`]
    /// to a view-agnostic [`BlockPoint`].
    ///
    /// This should be the inverse of [`WithinBlock<Point>::from_session_sharing_block_point`].
    pub fn to_session_sharing_block_point(self, block_list: &BlockList) -> Option<BlockPoint> {
        let block_id = block_list.block_at(self.block_index)?.id();
        let grid = block_list.grid_at_location(&self).grid_handler();

        let point = grid.maybe_translate_point_from_displayed_to_original(self.inner);
        let point = grid.grid_agnostic_point(point);

        Some(BlockPoint {
            block_id: block_id.to_string().into(),
            point: point.into(),
            grid_type: self.grid.into(),
        })
    }
}

#[cfg(test)]
#[path = "selections_tests.rs"]
mod tests;
