use anyhow::Result;
use pathfinder_geometry::vector::Vector2I;

use crate::rendering::atlas::allocator::Allocator;
use crate::rendering::atlas::{AllocatedRegion, AllocationError};

/// Manager that is responsible for allocating areas into a series of textures atlases.
pub(crate) struct Manager {
    current_allocator: Allocator,
    current_texture_id: TextureId,
    atlas_size: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TextureId(usize);

impl TextureId {
    /// Returns the initial [`TextureId`] value to use in a fresh texture atlas
    /// cache.
    pub fn initial_value() -> Self {
        Self(0)
    }

    /// Returns the next [`TextureId`] value to use after this one.
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }
}

/// An offset into a region of a given texture that has been allocated for an item.
#[derive(Copy, Debug, Clone)]
pub(crate) struct TextureOffset {
    /// The unique identifier for the texture.
    pub texture_id: TextureId,
    /// The region of the texture that was allocated.
    pub allocated_region: AllocatedRegion,
}

impl Manager {
    pub fn new(atlas_size: usize) -> Self {
        Self {
            current_allocator: Allocator::new(atlas_size),
            current_texture_id: TextureId::initial_value(),
            atlas_size,
        }
    }

    /// Allocates a region of `size` into a texture. Returns a [`TextureOffset`] denoting the region
    /// that was allocated.
    pub fn insert(&mut self, size: Vector2I) -> Result<TextureOffset> {
        match self.current_allocator.insert(size) {
            Ok(allocated_region) => Ok(TextureOffset {
                texture_id: self.current_texture_id,
                allocated_region,
            }),
            Err(AllocationError::Full) => {
                self.current_texture_id = self.current_texture_id.next();
                self.current_allocator = Allocator::new(self.atlas_size);
                self.insert(size)
            }
            Err(insert_error) => Err(insert_error.into()),
        }
    }
}
