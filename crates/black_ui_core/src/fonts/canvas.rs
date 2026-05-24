use pathfinder_geometry::vector::Vector2I;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RasterFormat {
    /// Premultiplied R8G8B8A8, little-endian.
    Rgba32,
    /// R8G8B8, little-endian.
    Rgb24,
    /// A8.
    A8,
}

impl RasterFormat {
    /// Returns the number of bytes per pixel that this image format corresponds to.
    pub fn bytes_per_pixel(&self) -> u8 {
        match self {
            RasterFormat::Rgba32 => 4,
            RasterFormat::Rgb24 => 3,
            RasterFormat::A8 => 1,
        }
    }
}

#[cfg(native)]
impl From<font_kit::canvas::Format> for RasterFormat {
    fn from(value: font_kit::canvas::Format) -> Self {
        match value {
            font_kit::canvas::Format::Rgba32 => Self::Rgba32,
            font_kit::canvas::Format::Rgb24 => Self::Rgb24,
            font_kit::canvas::Format::A8 => Self::A8,
        }
    }
}

/// An in-memory bitmap surface for glyph rasterization.
pub struct Canvas {
    /// The raw pixel data.
    pub pixels: Vec<u8>,
    /// The size of the buffer, in pixels.
    pub size: Vector2I,
    /// The number of *bytes* between successive rows.
    pub row_stride: usize,
    /// The image format of the canvas.
    pub format: RasterFormat,
}

#[cfg(native)]
impl From<font_kit::canvas::Canvas> for Canvas {
    fn from(value: font_kit::canvas::Canvas) -> Self {
        Self {
            pixels: value.pixels,
            size: value.size,
            row_stride: value.stride,
            format: value.format.into(),
        }
    }
}
