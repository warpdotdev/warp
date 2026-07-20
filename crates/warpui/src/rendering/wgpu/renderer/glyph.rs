use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pathfinder_geometry::rect::RectF;
use wgpu::util::BufferInitDescriptor;
use wgpu::{
    BindGroupLayout, BufferUsages, ColorTargetState, Device, FilterMode, RenderPass,
    RenderPipeline, Sampler,
};

use super::util::create_buffer_init;
use crate::Scene;
use crate::fonts::SubpixelAlignment;
use crate::rendering::atlas::TextureId;
use crate::rendering::wgpu::renderer::WGPUContext;
use crate::rendering::wgpu::texture_with_bind_group::TextureWithBindGroup;
use crate::rendering::wgpu::{resources, shader_types};
use crate::rendering::{GlyphCache, GlyphConfig};
use crate::scene::{GlyphFade, Layer};

pub(super) struct Pipeline {
    glyph_cache: GlyphCache<TextureWithBindGroup>,
    render_pipeline: RenderPipeline,
    texture_bind_group_layout: BindGroupLayout,
    sampler: Sampler,
}

#[derive(Default)]
pub(super) struct PerFrameState {
    glyph_data: Vec<shaders::GlyphInstanceData>,
    buffer: Option<wgpu::Buffer>,
}

pub(super) struct LayerState {
    textures: Vec<PerTextureState>,
}

pub(super) struct PerTextureState {
    texture_id: TextureId,
    start_offset: usize,
    len: usize,
}
impl Pipeline {
    pub(super) fn new(
        uniform_bind_group_layout: &BindGroupLayout,
        device: &Device,
        color_target: ColorTargetState,
        glyph_config: GlyphConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Glyph Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../shaders/glyph_shader.wgsl"
            ))),
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        // This should match the filterable field of the
                        // corresponding Texture entry above.
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let glyph_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Glyph pipeline layout"),
                bind_group_layouts: &[
                    Some(uniform_bind_group_layout),
                    Some(&texture_bind_group_layout),
                ],
                immediate_size: 0,
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Glyph Render pipeline"),
            layout: Some(&glyph_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    shader_types::Vertex::desc(),
                    shaders::GlyphInstanceData::desc(),
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(color_target)],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            // Don't use a pipeline cache. Most desktop GPU drivers have their own internal caches,
            // so we are unlikely to get much value out of this for the platforms Warp supports.
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });

        Self {
            glyph_cache: GlyphCache::new(glyph_config),
            render_pipeline,
            texture_bind_group_layout,
            sampler,
        }
    }

    pub(super) fn update_config(&mut self, glyph_config: &GlyphConfig) {
        self.glyph_cache.update_config(glyph_config);
    }

    pub(super) fn initialize_for_layer(
        &mut self,
        layer: &Layer,
        scene: &Scene,
        per_frame_state: &mut PerFrameState,
        ctx: &WGPUContext,
    ) -> Option<LayerState> {
        if layer.glyphs.is_empty() {
            // There are no glyphs to render, exit early.
            return None;
        }

        let scale_factor = scene.scale_factor();

        let mut texture_to_glyph: HashMap<TextureId, Vec<shaders::GlyphInstanceData>> =
            HashMap::new();
        for glyph in &layer.glyphs {
            let glyph_position = glyph.position * scale_factor;
            let subpixel_alignment = SubpixelAlignment::new(glyph_position);
            match self.glyph_cache.get(
                glyph.glyph_key,
                scene.scale_factor(),
                subpixel_alignment,
                &|size| {
                    TextureWithBindGroup::new(
                        size,
                        &ctx.resources.device,
                        &self.texture_bind_group_layout,
                        &self.sampler,
                    )
                },
                &|region, rasterized_glyph, texture| {
                    texture.insert_glyph_into_texture(
                        region,
                        rasterized_glyph,
                        &ctx.resources.queue,
                    )
                },
                ctx.glyph_raster_bounds_fn,
                ctx.rasterize_glyph_fn,
            ) {
                Ok(Some(gto)) => {
                    let (fade_start, fade_end) = match &glyph.fade {
                        None => (&0.0, &-1.0),
                        Some(GlyphFade::Horizontal { start, end }) => (start, end),
                    };

                    // Adjust the horizontal position by the subpixel alignment
                    // so that we only shift the glyph over by the amount that
                    // isn't accounted for in the subpixel-rasterized glyph.
                    let glyph_position = glyph_position - subpixel_alignment.to_offset();

                    // Make sure to pass the glyph size in the atlas
                    // Not the size of the render bounds (which may be smaller)
                    // If you pass the render bounds as the size, the shader
                    // will try to sample from a smaller area than the size
                    // in the atlas, leading to artifacts.
                    let glyph_instance_data = shaders::GlyphInstanceData::new(
                        RectF::new(
                            glyph_position + gto.raster_bounds.origin(),
                            gto.allocated_region.pixel_region.size().to_f32(),
                        ),
                        gto.allocated_region.uv_region,
                        fade_start * scale_factor,
                        fade_end * scale_factor,
                        glyph.color,
                        gto.is_emoji,
                    );

                    texture_to_glyph
                        .entry(gto.texture_id)
                        .or_default()
                        .push(glyph_instance_data);
                }
                Ok(None) => {}
                Err(err) => {
                    log::warn!("Unable to get glyph out of glyph cache: {err:?}, {glyph:?}");
                    return None;
                }
            }
        }

        if texture_to_glyph.is_empty() {
            // Early exit if there are no glyphs to render, as it causes a debug assert
            // failure in the metal code to create an empty metal buffer.
            return None;
        }

        let mut start_offset = per_frame_state.glyph_data.len();
        let per_texture_data = texture_to_glyph
            .into_iter()
            .map(|(texture_id, mut glyph_instance_data)| {
                let len = glyph_instance_data.len();
                per_frame_state.glyph_data.append(&mut glyph_instance_data);

                let state = PerTextureState {
                    texture_id,
                    start_offset,
                    len,
                };
                start_offset += len;
                state
            })
            .collect();

        Some(LayerState {
            textures: per_texture_data,
        })
    }

    pub(super) fn finalize_per_frame_state(
        per_frame_state: &mut PerFrameState,
        device: &Device,
        device_lost: &Arc<AtomicBool>,
    ) {
        per_frame_state.buffer = create_buffer_init(
            device,
            device_lost,
            &BufferInitDescriptor {
                label: Some("Glyph instance buffer"),
                contents: bytemuck::cast_slice(&per_frame_state.glyph_data),
                usage: BufferUsages::VERTEX,
            },
        )
        .ok();
    }

    pub(super) fn draw<'a>(
        &'a self,
        render_pass: &mut RenderPass<'a>,
        layer_state: &LayerState,
        per_frame_state: &'a PerFrameState,
    ) {
        let Some(buffer) = per_frame_state.buffer.as_ref() else {
            return;
        };

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_vertex_buffer(1, buffer.slice(..));

        for per_texture_state in &layer_state.textures {
            let texture_with_view = self
                .glyph_cache
                .texture(&per_texture_state.texture_id)
                .expect("texture ID should be in atlas");

            render_pass.set_bind_group(1, texture_with_view.bind_group(), &[]);
            let end_offset = per_texture_state.start_offset + per_texture_state.len;
            render_pass.draw_indexed(
                0..resources::quad::INDICES.len() as u32,
                0,
                per_texture_state.start_offset as u32..end_offset as u32,
            );
        }
    }
}

mod shaders {
    use pathfinder_color::ColorU;
    use pathfinder_geometry::rect::RectF;

    use crate::rendering::wgpu::shader_types::{ColorF, Vector4F};

    #[repr(C)]
    #[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct GlyphInstanceData {
        bounds: Vector4F,
        uv_bounds: Vector4F,
        fade_start: f32,
        fade_end: f32,
        color: ColorF,
        is_emoji: i32,
    }

    impl GlyphInstanceData {
        const ATTRIBS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
            1 => Float32x4,    // Bounds
            2 => Float32x4,     // UV Bounds
            3 => Float32,       // Fade Start
            4 => Float32,       // Fade end
            5 => Float32x4,     // Color
            6 => Sint32,        // Is Emoji
        ];

        pub(super) fn new(
            bounds: RectF,
            uv_left: RectF,
            fade_start: f32,
            fade_end: f32,
            color: ColorU,
            is_emoji: bool,
        ) -> Self {
            Self {
                bounds: bounds.into(),
                uv_bounds: uv_left.into(),
                fade_start,
                fade_end,
                color: color.into(),
                is_emoji: is_emoji as i32,
            }
        }

        pub(super) fn desc() -> wgpu::VertexBufferLayout<'static> {
            use std::mem;

            wgpu::VertexBufferLayout {
                array_stride: mem::size_of::<Self>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &Self::ATTRIBS,
            }
        }
    }
}
