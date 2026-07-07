use pathfinder_geometry::vector::Vector2F;

use super::metal;
use super::renderer::{Device, Renderer};

pub struct RendererManager {
    metal_renderer_manager: metal::RendererManager,
    #[cfg(wgpu)]
    wgpu_renderer_manager: super::wgpu::RendererManager,
}

impl Default for RendererManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RendererManager {
    pub fn new() -> Self {
        Self {
            metal_renderer_manager: metal::RendererManager::new(),
            #[cfg(wgpu)]
            wgpu_renderer_manager: super::wgpu::RendererManager::new(),
        }
    }

    /// Returns a [`Renderer`] that can be used to render on the given [`Device`].
    #[allow(unused_variables)]
    pub fn renderer_for_device(
        &mut self,
        device: &Device,
        window_size: Vector2F,
    ) -> &mut dyn Renderer {
        match device {
            Device::Metal(device) => self.metal_renderer_manager.renderer_for_device(device),
            #[cfg(wgpu)]
            Device::WGPU(resources) => self
                .wgpu_renderer_manager
                .renderer_for_resources(resources, window_size),
        }
    }
}
