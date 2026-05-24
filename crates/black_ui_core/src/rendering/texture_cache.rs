use std::sync::{Arc, Weak};

use crate::image_cache::StaticImage;

/// An opaque identifier for a texture from which we can render an image.
///
/// This *MUST* only be used within a single frame, and is not safe to use
/// across frames.
#[derive(Copy, Clone)]
pub struct TextureCacheIndex(usize);

pub struct TextureInfo<T> {
    /// The actual information about the texture.
    inner: T,

    /// The backing asset for the texture.
    asset: Weak<StaticImage>,

    /// The index of the last frame on which this texture was accessed.  This
    /// is used to know when a texture has gone "stale" and can be dropped from
    /// the cache.
    last_accessed_frame: usize,
}

/// A simple cache for textures from which we can render images.
pub struct TextureCache<T> {
    textures: Vec<TextureInfo<T>>,

    /// The index of the last frame that was rendered.
    frame_index: usize,
}

impl<T> TextureCache<T> {
    /// The maximum number of frames that a texture can go unused before it
    /// gets dropped from the cache.
    const MAX_UNUSED_FRAMES: usize = 10;

    pub fn new() -> Self {
        Self {
            textures: Default::default(),
            frame_index: 0,
        }
    }

    pub fn get(&self, texture_id: TextureCacheIndex) -> Option<&T> {
        self.textures.get(texture_id.0).map(|info| &info.inner)
    }

    pub fn get_or_insert_by_asset(
        &mut self,
        asset: &Arc<StaticImage>,
        texinfo_func: impl FnOnce(&Arc<StaticImage>) -> T,
    ) -> (TextureCacheIndex, &T) {
        let mut found = None;
        let weak_asset = Arc::downgrade(asset);
        for (index, texture) in self.textures.iter().enumerate() {
            if texture.asset.ptr_eq(&weak_asset) {
                found = Some(index);
            }
        }
        let index = match found {
            Some(index) => index,
            None => {
                self.textures.push(TextureInfo {
                    inner: texinfo_func(asset),
                    asset: weak_asset.clone(),
                    last_accessed_frame: self.frame_index,
                });
                self.textures.len() - 1
            }
        };

        // This array lookup is safe, as we either found the texture in the
        // cache or we inserted a new one and returned its index.
        self.textures[index].last_accessed_frame = self.frame_index;

        (TextureCacheIndex(index), &self.textures[index].inner)
    }

    /// Updates the texture cache at the end of a frame.
    ///
    /// This should be called at the end of every frame to ensure that stale
    /// texture resources get cleaned up.
    pub fn end_frame(&mut self) {
        // Drop any textures which are no longer referenced by the asset cache
        // or have not been rendered in the last MAX_UNUSED_FRAMES frames.
        self.textures.retain(|texture| {
            texture.asset.strong_count() > 0
                && self.frame_index - texture.last_accessed_frame < Self::MAX_UNUSED_FRAMES
        });

        self.frame_index += 1;
    }
}

impl<T> Default for TextureCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "texture_cache_tests.rs"]
mod tests;
