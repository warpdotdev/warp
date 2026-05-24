use std::mem;

use pathfinder_geometry::vector::Vector2F;
use wgpu::{BindGroup, BindGroupLayout, Buffer};

use crate::rendering::wgpu::{shader_types, Resources};

pub(super) struct Uniforms {
    bind_group_layout: BindGroupLayout,
    bind_group: BindGroup,
    buffer: Buffer,
}

impl Uniforms {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Quad Uniforms Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        mem::size_of::<shader_types::Uniforms>() as wgpu::BufferAddress,
                    ),
                },
                count: None,
            }],
        });

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniforms buffer"),
            size: mem::size_of::<shader_types::Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uniforms Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            bind_group_layout,
            bind_group,
            buffer,
        }
    }

    pub fn bind_group_layout(&self) -> &BindGroupLayout {
        &self.bind_group_layout
    }

    pub fn configure_render_pass<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        drawable_size: Vector2F,
        resources: &Resources,
    ) {
        let uniforms = shader_types::Uniforms::new(drawable_size);
        resources
            .queue
            .write_buffer(&self.buffer, 0, bytemuck::cast_slice(&[uniforms]));
        render_pass.set_bind_group(0, &self.bind_group, &[]);
    }
}
