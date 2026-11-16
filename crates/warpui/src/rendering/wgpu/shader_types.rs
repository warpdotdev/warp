use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct ColorF {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl From<ColorU> for ColorF {
    fn from(coloru: ColorU) -> Self {
        coloru.to_f32().into()
    }
}

impl From<pathfinder_color::ColorF> for ColorF {
    fn from(color: pathfinder_color::ColorF) -> Self {
        Self {
            r: color.r(),
            g: color.g(),
            b: color.b(),
            a: color.a(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Vector2F {
    x: f32,
    y: f32,
}

pub(super) const fn vec2f(x: f32, y: f32) -> Vector2F {
    Vector2F { x, y }
}

impl From<crate::geometry::vector::Vector2F> for Vector2F {
    fn from(vec2f: pathfinder_geometry::vector::Vector2F) -> Self {
        Self {
            x: vec2f.x(),
            y: vec2f.y(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Vector4F {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

pub(super) const fn vec4f(x: f32, y: f32, z: f32, w: f32) -> Vector4F {
    Vector4F { x, y, z, w }
}

impl From<pathfinder_geometry::vector::Vector4F> for Vector4F {
    fn from(vec4f: pathfinder_geometry::vector::Vector4F) -> Self {
        Self {
            x: vec4f.x(),
            y: vec4f.y(),
            z: vec4f.z(),
            w: vec4f.w(),
        }
    }
}

impl From<pathfinder_geometry::rect::RectF> for Vector4F {
    fn from(rectf: pathfinder_geometry::rect::RectF) -> Self {
        Self {
            x: rectf.origin_x(),
            y: rectf.origin_y(),
            z: rectf.width(),
            w: rectf.height(),
        }
    }
}

/// Vertex position in normalized device coordinates (NDC). We don't need to manage padding of
/// this struct to ensure it is a power of two--WGPU does this for us via the call to
/// `create_buffer_init`.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Vertex {
    pub(super) position: Vector2F,
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

    pub(super) fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;

        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Color {
    /// The start location of the background in the range [0,1].
    pub(super) start: Vector2F,
    pub(super) start_color: ColorF,
    /// The end location of the background in the range [0,1].
    pub(super) end: Vector2F,
    pub(super) end_color: ColorF,
}

#[derive(Default)]
pub(super) struct BorderWidth {
    pub(super) top: f32,
    pub(super) right: f32,
    pub(super) bottom: f32,
    pub(super) left: f32,
}

/// Data for a rect that is stored per instance. We don't need to manage padding of
/// this struct to ensure it is a power of two--WGPU does this for us via the call to
/// `create_buffer_init`.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct RectData {
    bounds: Vector4F,
    background_color: Color,
    border_width: Vector4F,
    border_color: Color,
    corner_radius: Vector4F,
    /// The amount of blurring for the shadow, i.e. higher value means more spread out. "Sigma"
    /// refers to the term in the formula of the Gaussian distribution, which is used in computing
    /// the shadow's shading.
    drop_shadow_sigma: f32,
    /// The shadow usually spans a larger size than its corresponding rect. This value determines
    /// that additional distance in px along each direction.
    drop_shadow_padding_factor: f32,
    dash_length: f32,
    gap_lengths: Vector2F,
}

impl RectData {
    const ATTRIBS: [wgpu::VertexAttribute; 13] = wgpu::vertex_attr_array![
        // Start at location 1 here because the vertex location occupies location 0.
        1 => Float32x4,     // Bounds
        2 => Float32x2,     // Background Start
        3 => Float32x4,     // Background Start Color
        4 => Float32x2,     // Background End
        5 => Float32x4,     // Background End Color
        6 => Float32x4,     // Border
        7 => Float32x2,     // Border Start
        8 => Float32x4,     // Border Start Color
        9 => Float32x2,     // Border End
        10 => Float32x4,    // Border End Color
        11 => Float32x4,    // Corner radius
        12 => Float32x2,    // Drop Shadow Sigma (Blur Radius) and Padding Factor (Spread Radius)
        13 => Float32x3,    // Dashed border data: dash length and gap length for x and y dimension
    ];

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bounds: RectF,
        background_color: Color,
        border_color: Color,
        corner_radius: crate::rendering::CornerRadius,
        border_width: BorderWidth,
        drop_shadow_sigma: f32,
        drop_shadow_padding_factor: f32,
        dash_length: f32,
        gap_lengths: pathfinder_geometry::vector::Vector2F,
    ) -> Self {
        Self {
            bounds: bounds.into(),
            background_color,
            border_width: vec4f(
                border_width.top,
                border_width.right,
                border_width.bottom,
                border_width.left,
            ),
            border_color,
            corner_radius: vec4f(
                corner_radius.top_left,
                corner_radius.top_right,
                corner_radius.bottom_left,
                corner_radius.bottom_right,
            ),
            drop_shadow_sigma,
            drop_shadow_padding_factor,
            dash_length,
            gap_lengths: gap_lengths.into(),
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

// Uniform buffer objects need to be 16-byte aligned in WGSL, so enforce
// that constraint here.
//
// See: https://www.w3.org/TR/WGSL/#address-space-layout-constraints
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub(super) struct Uniforms {
    viewport_size: Vector2F,
    // The shader-side paired struct will automatically be padded as necessary,
    // so we add any necessary padding bytes here by adjusting the size of this
    // byte array.
    _struct_padding_bytes: [u8; 8],
}

impl Uniforms {
    pub(super) fn new(size: pathfinder_geometry::vector::Vector2F) -> Self {
        Self {
            viewport_size: size.into(),
            _struct_padding_bytes: Default::default(),
        }
    }
}
