use crate::platform::mac::rendering::Device;
use crate::platform::mac::window::WindowState;
use crate::rendering::wgpu::{Renderer, Resources};
use crate::{fonts, Scene};

impl super::super::Renderer for Renderer {
    fn render(&mut self, scene: &Scene, window: &WindowState, font_cache: &fonts::Cache) {
        let _ = Renderer::render(
            self,
            scene,
            window.unwrap_wgpu_resources(),
            &|glyph_key, scale, subpixel_alignment, glyph_config, format| {
                font_cache.rasterized_glyph(
                    glyph_key,
                    scale,
                    subpixel_alignment,
                    glyph_config,
                    format,
                )
            },
            &|glyph_key, scale, alignment| {
                font_cache.glyph_raster_bounds(glyph_key, scale, alignment)
            },
            window.physical_size(),
            None,
            window.capture_callback.borrow_mut().take(),
        );
    }

    fn resize(&mut self, window: &WindowState) {
        let _ = window
            .unwrap_wgpu_resources()
            .update_surface_size(window.physical_size());
    }
}

impl WindowState {
    fn unwrap_wgpu_resources(&self) -> &Resources {
        match self.device().unwrap() {
            Device::Metal(_) => {
                panic!("called the WGPU renderer with a metal device");
            }
            Device::WGPU(resources) => resources,
        }
    }
}
