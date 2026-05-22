use pathfinder_geometry::rect::{RectF, RectI};
use pathfinder_geometry::vector::{vec2f, vec2i, Vector2I};

use crate::rendering::atlas::{AllocatedRegion, AllocationError};

/// The number of pixels of padding that should be applied between elements
/// in an atlas row.
const HORIZONTAL_PADDING: i32 = 1;
/// The number of pixels of padding that should be applied between rows of
/// elements in the atlas.
const VERTICAL_PADDING: i32 = 1;

/// A naive allocator to determine where items should be inserted into an atlas. Items are packed in
/// by using the Shelf-Next Fit algorithm (as described in
/// <https://blog.roomanna.com/09-25-2015/binpacking-shelf>). Items are fit horizontally in the
/// current open row (aka shelf) until a new element does not fit in that row, at which point a new
/// row for elements are created.
/// Visually, this looks like the following:
///
/// ```text
///                           (width, height)
///   ┌─────┬─────┬─────┬─────┬─────┐
///   │ 10  │     │     │     │     │ <- Empty spaces; can be filled while
///   │     │     │     │     │     │    element_height < height - row_baseline
///   ├─────┼─────┼─────┼─────┼─────┤
///   │ 5   │ 6   │ 7   │ 8   │ 9   │
///   │     │     │     │     │     │
///   ├─────┼─────┼─────┼─────┴─────┤ <- Row height is tallest element in row; this is
///   │ 1   │ 2   │ 3   │ 4         │    used as the baseline for the following row.
///   │     │     │     │           │ <- Row considered full when next element doesn't
///   └─────┴─────┴─────┴───────────┘    fit in the row.
/// (0, 0)  x->
/// ```
#[derive(Debug)]
pub(crate) struct Allocator {
    /// Width of atlas.
    width: i32,

    /// Height of atlas.
    height: i32,

    /// Left-most free pixel in a row.
    ///
    /// This is called the extent because it is the upper bound of used pixels
    /// in a row.
    row_extent: i32,

    /// Baseline for elements in the current row.
    row_baseline: i32,

    /// Tallest element in current row.
    ///
    /// This is used as the advance when end of row is reached.
    row_tallest: i32,
}

impl Allocator {
    pub fn new(size: usize) -> Self {
        Self {
            width: size as i32,
            height: size as i32,
            row_extent: 0,
            row_baseline: 0,
            row_tallest: 0,
        }
    }

    /// Attempts to allocate space for an item of size `element_size` into the atlas. If allocated,
    /// returns an [`AllocatedRegion`] that describes the region of the texture that was allocated.
    /// Returns an [`AllocationError`] if the item was unable to be inserted into the atlas.
    pub fn insert(&mut self, element_size: Vector2I) -> Result<AllocatedRegion, AllocationError> {
        if element_size.x() > self.width || element_size.y() > self.height {
            return Err(AllocationError::ItemTooLarge);
        }

        // If there's not enough room in current row, go onto next one.
        if !self.room_in_row(element_size) {
            self.advance_row()?;
        }

        // If there's still not room, there's nothing that can be done here.
        if !self.room_in_row(element_size) {
            return Err(AllocationError::Full);
        }

        // There appears to be room; allocate space for the iten.
        Ok(self.insert_inner(element_size))
    }

    /// Allocate space for the item without checking for room.
    ///
    /// Internal function for use once atlas has been checked for space.
    fn insert_inner(&mut self, element_size: Vector2I) -> AllocatedRegion {
        let offset_y = self.row_baseline;
        let offset_x = self.row_extent;
        let height = element_size.y();
        let width = element_size.x();

        // Update Atlas state.
        self.row_extent = offset_x + width + HORIZONTAL_PADDING;
        if height > self.row_tallest {
            self.row_tallest = height;
        }

        // Generate UV coordinates.
        let uv_top = offset_y as f32 / self.height as f32;
        let uv_left = offset_x as f32 / self.width as f32;
        let uv_height = height as f32 / self.height as f32;
        let uv_width = width as f32 / self.width as f32;

        AllocatedRegion {
            uv_region: RectF::new(vec2f(uv_left, uv_top), vec2f(uv_width, uv_height)),
            pixel_region: RectI::new(vec2i(offset_x, offset_y), vec2i(width, height)),
        }
    }

    /// Check if there's room in the current row for given element..
    fn room_in_row(&self, element_size: Vector2I) -> bool {
        let next_extent = self.row_extent + element_size.x();
        let enough_width = next_extent <= self.width;
        let enough_height = element_size.y() < (self.height - self.row_baseline);

        enough_width && enough_height
    }

    /// Mark current row as finished and prepare to insert into the next row.
    fn advance_row(&mut self) -> Result<(), AllocationError> {
        let advance_to = self.row_baseline + self.row_tallest + VERTICAL_PADDING;
        if self.height - advance_to <= 0 {
            return Err(AllocationError::Full);
        }

        self.row_baseline = advance_to;
        self.row_extent = 0;
        self.row_tallest = 0;

        Ok(())
    }
}
