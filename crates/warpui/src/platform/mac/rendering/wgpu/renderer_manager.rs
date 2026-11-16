use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use pathfinder_geometry::vector::Vector2F;
use wgpu::Device;

use crate::rendering::GlyphConfig;
use crate::rendering::wgpu::{Renderer, Resources};

pub struct RendererManager {
    renderers: HashMap<DeviceID, Renderer>,
}

#[derive(Copy, Clone, Hash, Eq, PartialEq)]
struct DeviceID(u64);

impl From<&Device> for DeviceID {
    fn from(value: &Device) -> Self {
        let mut s = DefaultHasher::new();
        value.hash(&mut s);
        DeviceID(s.finish())
    }
}

impl RendererManager {
    pub fn new() -> Self {
        Self {
            renderers: Default::default(),
        }
    }

    /// Returns a [`Renderer`] identified by the device contained in [`Resources`].
    pub fn renderer_for_resources(
        &mut self,
        resources: &Resources,
        _window_size: Vector2F,
    ) -> &mut Renderer {
        use std::collections::hash_map::Entry::*;
        match self.renderers.entry((&resources.device).into()) {
            Occupied(entry) => entry.into_mut(),
            Vacant(entry) => entry.insert(Renderer::new(resources, GlyphConfig::default())),
        }
    }
}
