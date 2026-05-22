use std::collections::HashMap;

use warpui_core::rendering;

use crate::platform::mac::rendering::metal::renderer::Renderer;

pub struct RendererManager {
    /// Maps a device's registry ID to its renderer (collection of state related
    /// to rendering on a particular device).
    renderers: HashMap<u64, Renderer>,
}

impl RendererManager {
    pub fn new() -> Self {
        Self {
            renderers: Default::default(),
        }
    }

    pub fn renderer_for_device(&mut self, device: &metal::Device) -> &mut Renderer {
        use std::collections::hash_map::Entry::*;
        match self.renderers.entry(device.registry_id()) {
            Occupied(entry) => entry.into_mut(),
            Vacant(entry) => entry.insert(Renderer::new(
                device,
                metal::MTLPixelFormat::BGRA8Unorm,
                rendering::GlyphConfig::default(),
            )),
        }
    }
}
