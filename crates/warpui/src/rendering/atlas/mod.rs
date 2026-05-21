mod allocator;
mod manager;

pub(crate) use manager::{Manager, TextureId};
use pathfinder_geometry::rect::{RectF, RectI};
use thiserror::Error;

/// A region of an atlas that has been allocated.
#[derive(Copy, Debug, Clone)]
pub(crate) struct AllocatedRegion {
    /// The region of the atlas that was allocated in UV (texture) coordinates.
    pub uv_region: RectF,
    /// The region of the atlas that was allocated in screen coordinates.
    pub pixel_region: RectI,
}

/// Error that can happen when attempting to allocate an element into the atlas.
#[derive(Error, Debug)]
pub(crate) enum AllocationError {
    /// Texture atlas is full.
    #[error("Unable to insert; atlas is full")]
    Full,

    /// The item cannot fit within a single texture.
    #[error("Unable to insert; item is too large to fit into atlas")]
    ItemTooLarge,
}
