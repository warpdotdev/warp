use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{Buffer, RenderPass};

use crate::rendering::wgpu::shader_types;

/// The vertex buffer slot used for quad vertex data.
const VERTEX_BUFFER_SLOT: u32 = 0;

/// Ordered list of indices in the [`VERTICES`] array to be used as part of an index buffer.
pub(in crate::rendering::wgpu) const INDICES: &[u16] = &[0, 1, 2, 2, 3, 1];

/// List of vertex positions in normalized device coordinates (NDC) that are used when rendering.
/// Similar to our metal renderer, we hardcode a list of vertices for each rect we render, and then
/// determine the actual position of the rect in NDC within the vertex shader.
const VERTICES: &[shader_types::Vertex] = &[
    shader_types::Vertex {
        position: shader_types::vec2f(0.0, 0.0),
    },
    shader_types::Vertex {
        position: shader_types::vec2f(1.0, 0.0),
    },
    shader_types::Vertex {
        position: shader_types::vec2f(0.0, 1.0),
    },
    shader_types::Vertex {
        position: shader_types::vec2f(1.0, 1.0),
    },
];

pub(super) struct Resources {
    index_buffer: Buffer,
    vertex_buffer: Buffer,
}

impl Resources {
    pub fn new(device: &wgpu::Device) -> Self {
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Quad Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            index_buffer,
            vertex_buffer,
        }
    }

    pub fn configure_render_pass<'a>(&'a self, render_pass: &mut RenderPass<'a>) {
        render_pass.set_vertex_buffer(VERTEX_BUFFER_SLOT, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
    }
}
