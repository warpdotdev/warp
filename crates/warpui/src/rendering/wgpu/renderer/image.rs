use std::borrow::Cow;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use wgpu::util::BufferInitDescriptor;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayout, ColorTargetState, Device, Extent3d,
    FilterMode, RenderPass, RenderPipeline, Sampler, TextureDescriptor, TextureFormat,
    TextureUsages,
};

use self::shaders::{ColorModifier, ImageInstanceData};
use super::util::create_buffer_init;
use super::WGPUContext;
use crate::image_cache::StaticImage;
use crate::rendering::texture_cache::{TextureCache, TextureCacheIndex};
use crate::rendering::wgpu::{resources, shader_types};
use crate::scene::Layer;
use crate::Scene;

pub(super) struct Pipeline {
    render_pipeline: RenderPipeline,
    texture_cache: TextureCache<TextureInfo>,
    texture_bind_group_layout: BindGroupLayout,
    sampler: Sampler,
}

#[derive(Default)]
pub(super) struct PerFrameState {
    image_data: Vec<shaders::ImageInstanceData>,
    buffer: Option<wgpu::Buffer>,
}

pub(super) struct LayerState {
    start_offset: usize,
    image_textures: Vec<TextureCacheIndex>,
}

impl Pipeline {
    pub(super) fn new(
        uniform_bind_group_layout: &BindGroupLayout,
        device: &Device,
        color_target: ColorTargetState,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Image Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../shaders/image_shader.wgsl"
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Image pipeline layout"),
            bind_group_layouts: &[
                Some(uniform_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Image render pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[shader_types::Vertex::desc(), ImageInstanceData::desc()],
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
            render_pipeline,
            texture_cache: TextureCache::new(),
            texture_bind_group_layout,
            sampler,
        }
    }

    pub(super) fn initialize_for_layer(
        &mut self,
        layer: &Layer,
        scene: &Scene,
        per_frame_state: &mut PerFrameState,
        ctx: &WGPUContext,
    ) -> Option<LayerState> {
        if layer.images.is_empty() && layer.icons.is_empty() {
            return None;
        }

        let start_offset = per_frame_state.image_data.len();
        let mut layer_state = LayerState {
            start_offset,
            image_textures: Vec::with_capacity(layer.images.len() + layer.icons.len()),
        };
        let scale_factor = scene.scale_factor();
        for image in &layer.images {
            let bounds = image.bounds * scale_factor;
            let min_dimension = f32::min(bounds.height(), bounds.width());
            let corner_radius = crate::rendering::CornerRadius::from_ui_corner_radius(
                image.corner_radius,
                scale_factor,
                min_dimension,
            );

            per_frame_state.image_data.push(ImageInstanceData::new(
                image.bounds * scale_factor,
                ColorModifier::Image {
                    opacity: (image.opacity * 255.) as u8,
                },
                corner_radius,
            ));
            let (texture_id, _) =
                self.texture_cache
                    .get_or_insert_by_asset(&image.asset, |asset| {
                        TextureInfo::new(asset, &self.texture_bind_group_layout, &self.sampler, ctx)
                    });
            layer_state.image_textures.push(texture_id);
        }

        for icon in &layer.icons {
            per_frame_state.image_data.push(ImageInstanceData::new(
                icon.bounds * scale_factor,
                ColorModifier::Icon { color: icon.color },
                crate::rendering::CornerRadius::default(),
            ));
            let (texture_id, _) = self
                .texture_cache
                .get_or_insert_by_asset(&icon.asset, |asset| {
                    TextureInfo::new(asset, &self.texture_bind_group_layout, &self.sampler, ctx)
                });
            layer_state.image_textures.push(texture_id);
        }

        Some(layer_state)
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
                label: Some("Image instance buffer"),
                contents: bytemuck::cast_slice(&per_frame_state.image_data),
                usage: wgpu::BufferUsages::VERTEX,
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

        for (index, texture_id) in layer_state.image_textures.iter().enumerate() {
            let TextureInfo { bind_group, .. } = self
                .texture_cache
                .get(*texture_id)
                .expect("texture should not leave cache between generating layer data and drawing");
            render_pass.set_bind_group(1, bind_group, &[]);

            let start_offset = layer_state.start_offset + index;
            render_pass.draw_indexed(
                0..resources::quad::INDICES.len() as u32,
                0,
                start_offset as u32..(start_offset + 1) as u32,
            );
        }
    }

    pub(super) fn end_frame(&mut self) {
        self.texture_cache.end_frame();
    }
}

/// A structure containing info about a GPU texture from which we can render
/// a particular static image asset.
struct TextureInfo {
    /// A handle to the set of resources that are needed to bind the texture
    /// in a shader.
    bind_group: BindGroup,
}

impl TextureInfo {
    fn new(
        asset: &Arc<StaticImage>,
        bind_group_layout: &BindGroupLayout,
        sampler: &Sampler,
        ctx: &WGPUContext,
    ) -> Self {
        let texture_size = Extent3d {
            width: asset.width(),
            height: asset.height(),
            depth_or_array_layers: 1,
        };
        let desc = TextureDescriptor {
            label: Some("Image texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        };

        let texture = ctx.resources.device.create_texture(&desc);
        let bytes_per_row: u32 = 4 * asset.width();
        ctx.resources.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            asset.rgba_bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
            texture_size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = ctx
            .resources
            .device
            .create_bind_group(&BindGroupDescriptor {
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

        Self { bind_group }
    }
}

mod shaders {
    use pathfinder_color::ColorU;
    use pathfinder_geometry::rect::RectF;

    use crate::rendering::wgpu::shader_types::{vec4f, ColorF, Vector4F};
    use crate::rendering::CornerRadius;

    /// Icons support overriding the color, whereas images only allow setting the opacity.
    pub(super) enum ColorModifier {
        Icon { color: ColorU },
        Image { opacity: u8 },
    }

    impl From<ColorModifier> for ColorF {
        fn from(color_mod: ColorModifier) -> Self {
            match color_mod {
                ColorModifier::Icon { color } => color.to_f32().into(),
                ColorModifier::Image { opacity } => ColorU::new(0, 0, 0, opacity).to_f32().into(),
            }
        }
    }

    #[repr(C)]
    #[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
    pub(super) struct ImageInstanceData {
        bounds: Vector4F,
        color: ColorF,
        is_icon: u32,
        corner_radius: Vector4F,
    }

    impl ImageInstanceData {
        const ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
            1 => Float32x4,    // Bounds
            2 => Float32x4,    // Color
            3 => Uint32,       // Boolean, image or icon
            4 => Float32x4,    // Corner radius
        ];

        pub(super) fn new(
            bounds: RectF,
            color_modifier: ColorModifier,
            corner_radius: CornerRadius,
        ) -> Self {
            Self {
                bounds: bounds.into(),
                is_icon: matches!(color_modifier, ColorModifier::Icon { .. }).into(),
                color: color_modifier.into(),
                corner_radius: vec4f(
                    corner_radius.top_left,
                    corner_radius.top_right,
                    corner_radius.bottom_left,
                    corner_radius.bottom_right,
                ),
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
