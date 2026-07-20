use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use wgpu::util::BufferInitDescriptor;
use wgpu::{BindGroupLayout, ColorTargetState, Device, RenderPass, RenderPipeline};

use super::util::create_buffer_init;
use crate::Scene;
use crate::rendering::get_best_dash_gap;
use crate::rendering::wgpu::shader_types::BorderWidth;
use crate::rendering::wgpu::{resources, shader_types};
use crate::scene::Layer;

pub(super) struct Pipeline {
    render_pipeline: RenderPipeline,
}

#[derive(Default)]
pub(super) struct PerFrameState {
    rect_data: Vec<shader_types::RectData>,
    buffer: Option<wgpu::Buffer>,
}

pub(super) struct LayerState {
    start_offset: usize,
    len: usize,
}

impl Pipeline {
    pub(super) fn new(
        uniform_bind_group_layout: &BindGroupLayout,
        device: &Device,
        color_target: ColorTargetState,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Rect Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../shaders/rect_shader.wgsl"
            ))),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Rect pipeline layout"),
            bind_group_layouts: &[Some(uniform_bind_group_layout)],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Rect render pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[shader_types::Vertex::desc(), shader_types::RectData::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("rect_fs_main"),
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

        Self { render_pipeline }
    }

    pub(super) fn initialize_for_layer(
        &self,
        layer: &Layer,
        scene: &Scene,
        per_frame_state: &mut PerFrameState,
    ) -> Option<LayerState> {
        if layer.rects.is_empty() {
            // It's a mac assertion error to create an empty metal buffer, so exit early
            return None;
        }

        let scale_factor = scene.scale_factor();
        let mut rect_instance_data = Vec::with_capacity(layer.rects.len());
        for rect in &layer.rects {
            let bounds = rect.bounds * scale_factor;

            if let Some(drop_shadow) = rect.drop_shadow {
                let sigma = drop_shadow.blur_radius * scale_factor;
                let padding = drop_shadow.spread_radius * scale_factor;
                let shadow_origin = bounds.origin() + drop_shadow.offset * scale_factor - padding;
                let shadow_size = bounds.size() + vec2f(2. * padding, 2. * padding);

                let min_dimension = f32::min(shadow_size.x(), shadow_size.y());
                let corner_radius = crate::rendering::CornerRadius::from_ui_corner_radius(
                    rect.corner_radius,
                    scale_factor,
                    min_dimension,
                );
                let bounds = RectF::new(shadow_origin, shadow_size);
                let shadow_color = shader_types::Color {
                    start: vec2f(0., 0.).into(),
                    start_color: drop_shadow.color.into(),
                    end: vec2f(1., 0.).into(),
                    end_color: drop_shadow.color.into(),
                };

                let border_color = shader_types::Color {
                    start: vec2f(0., 0.).into(),
                    start_color: ColorU::transparent_black().into(),
                    end: vec2f(1., 0.).into(),
                    end_color: ColorU::transparent_black().into(),
                };

                rect_instance_data.push(shader_types::RectData::new(
                    bounds,
                    shadow_color,
                    border_color,
                    corner_radius.clone(),
                    BorderWidth::default(),
                    sigma,
                    padding,
                    0.,
                    vec2f(0., 0.),
                ));
            }

            let min_dimension = f32::min(bounds.height(), bounds.width());
            let corner_radius = crate::rendering::CornerRadius::from_ui_corner_radius(
                rect.corner_radius,
                scale_factor,
                min_dimension,
            );
            let background_color = shader_types::Color {
                start: rect.background.start().into(),
                start_color: (rect.background.start_color().into()),
                end: rect.background.end().into(),
                end_color: (rect.background.end_color().into()),
            };

            let border_color = shader_types::Color {
                start: rect.border.color.start().into(),
                start_color: (rect.border.color.start_color().into()),
                end: rect.border.color.end().into(),
                end_color: (rect.border.color.end_color().into()),
            };

            let border_width = shader_types::BorderWidth {
                top: rect.border.top_width() * scale_factor,
                right: rect.border.right_width() * scale_factor,
                bottom: rect.border.bottom_width() * scale_factor,
                left: rect.border.left_width() * scale_factor,
            };

            let dash = rect
                .border
                .dash
                .map(|mut dash| {
                    dash.dash_length *= scale_factor;
                    dash.gap_length *= scale_factor;
                    dash
                })
                .unwrap_or_default();
            let horizontal_gap = get_best_dash_gap(bounds.width(), dash);
            let vertical_gap = get_best_dash_gap(bounds.height(), dash);
            let gap_lengths = vec2f(horizontal_gap, vertical_gap);

            let rect_data = shader_types::RectData::new(
                bounds,
                background_color,
                border_color,
                corner_radius,
                border_width,
                0.,
                0.,
                dash.dash_length,
                gap_lengths,
            );
            rect_instance_data.push(rect_data);
        }

        let start_offset = per_frame_state.rect_data.len();
        let len = rect_instance_data.len();
        per_frame_state.rect_data.append(&mut rect_instance_data);

        Some(LayerState { start_offset, len })
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
                label: Some("Rect instance buffer"),
                contents: bytemuck::cast_slice(&per_frame_state.rect_data),
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

        let end_offset = layer_state.start_offset + layer_state.len;
        render_pass.draw_indexed(
            0..resources::quad::INDICES.len() as u32,
            0,
            layer_state.start_offset as u32..end_offset as u32,
        );
    }
}
