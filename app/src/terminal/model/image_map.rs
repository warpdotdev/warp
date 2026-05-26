use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Bound::Included;

use itertools::Itertools;
use pathfinder_geometry::vector::Vector2F;

use super::grid::grid_handler::{AbsolutePoint, AbsoluteRectangle};
use super::iterm_image::ITermImageMetadata;
use super::kitty::KittyImageMetadata;

/// This image cache stores the absolute positions of points, relative to the number of rows
/// that haven't been truncated yet. Stored absolute points are nearly always stale as we store
/// the number of lines that have been truncated only when we insert a new image. This means that
/// upon retrieval, we need to update the key to correctly retrieve the data, and update the response
/// to no longer be stale.
#[derive(Clone, Default)]
pub(in crate::terminal::model) struct ImageMap {
    /// Map of image ids keyed by absolute position.
    image_ids_by_point: BTreeMap<AbsolutePoint, HashSet<(u32, u32)>>,
    /// Map of absolute positions of a given image id.
    point_by_image_id: BTreeMap<(u32, u32), AbsolutePoint>,
    /// Image with the largest height in the map.
    largest_height: usize,
    image_placement_data: HashMap<(u32, u32), ImagePlacementData>,
    num_lines_truncated_at_last_update: u64,
    image_type_by_image_id: HashMap<u32, ImageType>,
}

impl ImageMap {
    pub fn get_image_placement_data(
        &self,
        image_id: u32,
        placement_id: u32,
    ) -> Option<&ImagePlacementData> {
        self.image_placement_data.get(&(image_id, placement_id))
    }

    pub fn add_image_placement_data(
        &mut self,
        image_id: u32,
        placement_id: u32,
        image_data: ImagePlacementData,
    ) {
        self.largest_height = self.largest_height.max(image_data.height_cells);
        self.image_placement_data
            .insert((image_id, placement_id), image_data);
    }

    pub fn place(
        &mut self,
        image_id: u32,
        placement_id: u32,
        primary_point: AbsolutePoint,
        image_type: ImageType,
        num_lines_truncated: u64,
    ) {
        self.image_ids_by_point
            .entry(primary_point)
            .or_default()
            .insert((image_id, placement_id));
        self.point_by_image_id
            .insert((image_id, placement_id), primary_point);
        self.image_type_by_image_id.insert(image_id, image_type);
        self.evict_truncated_absolute_points(num_lines_truncated);
    }

    pub fn evict_images_at_point_with_type(
        &mut self,
        point: AbsolutePoint,
        image_types_to_evict: &[ImageType],
    ) {
        if self.image_ids_by_point.is_empty() {
            return;
        }

        let mut to_evict = Vec::new();
        if let Some(images_to_evict) = self.image_ids_by_point.get(&point) {
            for (image_id, placement_id) in images_to_evict.iter() {
                let Some(image_type) = self.image_type_by_image_id.get(image_id) else {
                    continue;
                };

                if !image_types_to_evict.contains(image_type) {
                    continue;
                }

                to_evict.push((*image_id, *placement_id));
            }
        }

        for (image_id, placement_id) in to_evict {
            self.evict_placement(image_id, placement_id);
        }
    }

    pub fn get_image_ids_by_rectangle(
        &self,
        rectangle: AbsoluteRectangle,
    ) -> Vec<AbsoluteImagePlacement> {
        // We extend our search window upwards by the height of the largest image in the map such that we can
        // check for images that start above the given search window but still overlap.
        let range_rectangle = AbsoluteRectangle {
            start_row: rectangle
                .start_row
                .saturating_sub(self.largest_height as u64),
            end_row: rectangle.end_row,
        };

        let range_start = range_rectangle.start_row_to_point(usize::MIN);
        let range_end = range_rectangle.end_row_to_point(usize::MAX);

        // 'end' should always be greater than or equal to 'start', but if is not, the range method on the BTreeMap will panic,
        // which we want to avoid.
        if range_start.cmp(&range_end) == Ordering::Greater {
            log::warn!("get_image_ids_by_rectangle: start > end");
            return vec![];
        }

        self.image_ids_by_point
            .range((Included(&range_start), Included(&range_end)))
            .map(|(&top_left, image_ids)| {
                let image_ids = image_ids
                    .iter()
                    .filter(|(image_id, placement_id)| {
                        let height =
                            match self.image_placement_data.get(&(*image_id, *placement_id)) {
                                Some(placement_data) => placement_data.height_cells,
                                None => return false,
                            };

                        if height == 0 {
                            return false;
                        }

                        let rectangle_start = rectangle.start_row_to_point(0);
                        let image_end = top_left.add_rows(height);

                        // Since we extended the search window by the height of the largest image, we need to ensure
                        // images actually overlap with our original search window
                        image_end >= rectangle_start
                    })
                    .copied()
                    .collect_vec();
                (top_left, image_ids)
            })
            .flat_map(|(top_left, image_ids)| {
                image_ids
                    .iter()
                    .map(|&(image_id, placement_id)| {
                        let z_index = match self.image_placement_data.get(&(image_id, placement_id))
                        {
                            Some(placement_data) => placement_data.z_index,
                            None => 0,
                        };
                        (z_index, (image_id, placement_id), top_left)
                    })
                    .collect_vec()
            })
            .sorted()
            .map(
                |(z_index, (image_id, placement_id), top_left)| AbsoluteImagePlacement {
                    image_id,
                    placement_id,
                    z_index,
                    top_left,
                },
            )
            .collect_vec()
    }

    pub fn has_image_in_row(&self, row: u64) -> bool {
        // We extend our search window upwards by the height of the largest image in the map such that we can
        // check for images that start above the given row but still overlap.
        let range_start = AbsolutePoint {
            row: row.saturating_sub(self.largest_height as u64),
            col: usize::MIN,
        };
        let range_end = AbsolutePoint {
            row,
            col: usize::MAX,
        };

        self.image_ids_by_point
            .range((Included(&range_start), Included(&range_end)))
            .any(|(&top_left, image_ids)| {
                image_ids.iter().any(|(image_id, placement_id)| {
                    let height = match self.image_placement_data.get(&(*image_id, *placement_id)) {
                        Some(placement_data) => placement_data.height_cells,
                        None => return false,
                    };
                    height != 0 && top_left.add_rows(height).row >= row
                })
            })
    }

    pub fn evict_image_ids_between_points_with_type(
        &mut self,
        start_point: AbsolutePoint,
        end_point: AbsolutePoint,
        image_types_to_evict: Vec<ImageType>,
    ) {
        // 'end' should always be greater than or equal to 'start', but if is not, the range method on the BTreeMap will panic,
        // which we want to avoid.
        if start_point.cmp(&end_point) == Ordering::Greater {
            log::warn!("evict_image_ids_between_points: start > end");
            return;
        }

        let images_to_evict = self
            .image_ids_by_point
            .range((Included(&start_point), Included(&end_point)))
            .flat_map(|(_, image_ids)| image_ids.iter().copied())
            .collect_vec();

        for (image_id, placement_id) in images_to_evict {
            let Some(image_type) = self.image_type_by_image_id.get(&image_id) else {
                continue;
            };

            if !image_types_to_evict.contains(image_type) {
                continue;
            }

            self.evict_placement(image_id, placement_id);
        }
    }

    pub fn evict_truncated_absolute_points(&mut self, new_num_lines_truncated: u64) {
        if new_num_lines_truncated == self.num_lines_truncated_at_last_update {
            return;
        }

        let mut images_to_evict = vec![];
        for (top_left, image_ids) in &self.image_ids_by_point {
            if top_left.is_truncated(new_num_lines_truncated) {
                images_to_evict.extend(image_ids.iter());
            } else {
                break;
            }
        }

        for (image_id, placement_id) in images_to_evict {
            self.evict_placement(image_id, placement_id);
        }

        self.num_lines_truncated_at_last_update = new_num_lines_truncated;
    }

    pub fn evict_all_images(&mut self) {
        self.image_placement_data.clear();
        self.image_ids_by_point.clear();
        self.point_by_image_id.clear();
        self.largest_height = 0;
    }

    pub fn evict_image(&mut self, image_id_to_evict: u32) {
        let mut images_to_evict = vec![];
        for &(image_id, placement_id) in self.point_by_image_id.keys() {
            if image_id == image_id_to_evict {
                images_to_evict.push((image_id, placement_id));
            }
        }

        for (image_id, placement_id) in images_to_evict {
            self.evict_placement(image_id, placement_id);
        }
    }

    pub fn evict_placement(&mut self, image_id: u32, placement_id: u32) {
        self.image_placement_data.remove(&(image_id, placement_id));
        if let Some(point) = self.point_by_image_id.get(&(image_id, placement_id)) {
            if let Some(image_ids) = self.image_ids_by_point.get_mut(point) {
                image_ids.remove(&(image_id, placement_id));
                if image_ids.is_empty() {
                    self.image_ids_by_point.remove(point);
                }
            }

            self.point_by_image_id.remove(&(image_id, placement_id));
        }
    }
}

pub struct AbsoluteImagePlacement {
    pub image_id: u32,
    pub placement_id: u32,
    pub z_index: i32,
    pub top_left: AbsolutePoint,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageType {
    Kitty,
    ITerm,
}

#[derive(Debug, Clone)]
pub enum StoredImageMetadata {
    ITerm(ITermImageMetadata),
    Kitty(KittyImageMetadata),
}

impl StoredImageMetadata {
    pub fn image_size(&self) -> Vector2F {
        match self {
            StoredImageMetadata::ITerm(metadata) => metadata.image_size,
            StoredImageMetadata::Kitty(metadata) => metadata.image_size,
        }
    }

    pub fn preserve_aspect_ratio(&self) -> bool {
        match self {
            StoredImageMetadata::ITerm(metadata) => metadata.preserve_aspect_ratio,
            StoredImageMetadata::Kitty(_) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImagePlacementData {
    pub z_index: i32,
    pub height_cells: usize,
    pub image_size: Vector2F,
}
