use warp_terminal::model::Point;

use super::{AbsolutePoint, AbsoluteRectangle, GridHandler};
use crate::terminal::model::image_map::ImagePlacementData;

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

    pub(in crate::terminal::model) fn has_image_in_row(&self, displayed_row: usize) -> bool {
        if self.has_displayed_output() {
            return false;
        }
        let absolute_row = AbsolutePoint::from_point(Point::new(displayed_row, 0), self).row;
        self.images.has_image_in_row(absolute_row)
    }

    pub fn get_image_placement_data(
        &self,
        image_id: u32,
        placement_id: u32,
    ) -> Option<&ImagePlacementData> {
        self.images.get_image_placement_data(image_id, placement_id)
    }
}

pub struct ImagePlacement {
    pub image_id: u32,
    pub placement_id: u32,
    pub z_index: i32,
    pub top_left: Point,
}
