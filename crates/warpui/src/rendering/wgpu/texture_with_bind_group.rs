use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayout, Extent3d, Queue, Sampler,
    TexelCopyBufferLayout, Texture, TextureDescriptor, TextureFormat, TextureUsages,
};

use crate::fonts::RasterizedGlyph;
use crate::rendering::atlas::AllocatedRegion;

/// Helper struct that includes a [`Texture`] and its corresponding [`BindGroup`] for use in the
/// `GlyphCache`.
pub(super) struct TextureWithBindGroup {
    texture: Texture,
    /// The [`BindGroup`] associated with the `texture`. We compute this whenever we need to create
    /// a new texture as a performance optimization to ensure we don't create it on every render.
    bind_group: BindGroup,
}

impl TextureWithBindGroup {
    pub(super) fn new(
        size: usize,
        device: &wgpu::Device,
        bind_group_layout: &BindGroupLayout,
        sampler: &Sampler,
    ) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("Glyph atlas texture"),
            size: Extent3d {
                width: size as u32,
                height: size as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
            label: None,
        });

        Self {
            texture,
            bind_group,
        }
    }

    pub(super) fn insert_glyph_into_texture(
        &mut self,
        region: AllocatedRegion,
        glyph: &RasterizedGlyph,
        queue: &Queue,
    ) {
        let bytes_per_row: u32 = 4 * (glyph.canvas.size.x() as u32);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.pixel_region.origin_x() as u32,
                    y: region.pixel_region.origin_y() as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            glyph.canvas.pixels.as_slice(),
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
            Extent3d {
                width: region.pixel_region.width() as u32,
                height: region.pixel_region.height() as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    pub(super) fn bind_group(&self) -> &BindGroup {
        &self.bind_group
    }
}
