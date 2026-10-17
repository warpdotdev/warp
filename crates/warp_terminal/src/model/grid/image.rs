use pathfinder_geometry::vector::Vector2F;

use super::{AbsolutePoint, AbsoluteRectangle, GridHandler};
use crate::model::Point;
use crate::model::image_map::{ImagePlacementData, VirtualPlacement};
use crate::model::kitty::KittyPlacementData;

impl GridHandler {
    pub fn get_image_ids_in_range(
        &self,
        displayed_start_row: usize,
        displayed_end_row: usize,
    ) -> Vec<ImagePlacement> {
        if self.has_displayed_output() {
            return vec![];
        }

        if displayed_start_row > displayed_end_row {
            return vec![];
        }

        self.images
            .get_image_ids_by_rectangle(AbsoluteRectangle::from_range(
                displayed_start_row,
                displayed_end_row,
                self,
            ))
            .into_iter()
            .filter_map(|absolute_image_placement| {
                absolute_image_placement
                    .top_left
                    .to_point(self)
                    .map(|top_left| ImagePlacement {
                        image_id: absolute_image_placement.image_id,
                        placement_id: absolute_image_placement.placement_id,
                        z_index: absolute_image_placement.z_index,
                        top_left,
                    })
            })
            .collect()
    }

    pub fn has_image_in_row(&self, displayed_row: usize) -> bool {
        if self.has_displayed_output() {
            return false;
        }
        let absolute_row = AbsolutePoint::from_point(Point::new(displayed_row, 0), self).row;
        self.images.has_image_in_row(absolute_row)
    }

    pub fn has_visible_images(&self) -> bool {
        !self.has_displayed_output() && !self.images.is_empty()
    }

    pub fn get_image_placement_data(
        &self,
        image_id: u32,
        placement_id: u32,
    ) -> Option<&ImagePlacementData> {
        self.images.get_image_placement_data(image_id, placement_id)
    }

    /// The virtual placement (kitty `U=1`) that this grid's placeholder cells for `image_id`
    /// show, if the image has one.
    pub fn virtual_image_placement(&self, image_id: u32) -> Option<VirtualPlacement> {
        self.images.virtual_placement(image_id)
    }

    /// Records a kitty virtual placement (`U=1`), sized in cells as a direct placement would be.
    /// Placeholder cells show it, so nothing is placed at the cursor.
    pub(super) fn add_virtual_image_placement(
        &mut self,
        image_id: u32,
        placement_id: u32,
        placement_data: &KittyPlacementData,
        image_size: Vector2F,
    ) {
        let cell_width = self.ansi_handler_state.cell_width.max(1);
        let cell_height = self.ansi_handler_state.cell_height.max(1);
        let size = placement_data.get_desired_dimensions(
            image_size,
            cell_height,
            cell_width,
            usize::MAX,
            usize::MAX,
        );
        let placement = VirtualPlacement {
            placement_id,
            cols: (size.x() / cell_width as f32).ceil().max(1.) as u32,
            rows: (size.y() / cell_height as f32).ceil().max(1.) as u32,
        };
        self.images.add_virtual_placement(image_id, placement);
    }
}

pub struct ImagePlacement {
    pub image_id: u32,
    pub placement_id: u32,
    pub z_index: i32,
    pub top_left: Point,
}
