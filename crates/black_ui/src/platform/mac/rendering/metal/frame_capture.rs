use metal::{MTLPixelFormat, MTLStorageMode};
use pathfinder_geometry::vector::Vector2F;
use black_ui_core::platform::CapturedFrame;

#[cfg(test)]
#[path = "frame_capture_tests.rs"]
mod tests;

/// Captures a rendered frame from a Metal texture and returns the raw BGRA pixel data.
///
/// The data is returned in Metal's native BGRA format to avoid an expensive
/// pixel-format conversion on the render thread. Consumers that need RGBA
/// should call `CapturedFrame::ensure_rgba()`.
///
/// # Arguments
/// * `texture` - The Metal texture containing the rendered frame
/// * `size` - The dimensions of the texture (width, height)
///
/// # Returns
/// * `Some(CapturedFrame)` containing the RGBA pixel data if successful
/// * `None` if the texture dimensions are invalid
pub fn capture_frame(texture: &metal::TextureRef, size: Vector2F) -> Option<CapturedFrame> {
    let width = size.x() as usize;
    let height = size.y() as usize;

    if width == 0 || height == 0 {
        log::warn!("Invalid texture dimensions: {}x{}", width, height);
        return None;
    }

    let bytes_per_row = width * 4;
    let buffer_size = bytes_per_row * height;

    let mut pixel_data: Vec<u8> = vec![0u8; buffer_size];

    let region = metal::MTLRegion {
        origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
        size: metal::MTLSize {
            width: width as u64,
            height: height as u64,
            depth: 1,
        },
    };

    texture.get_bytes(
        pixel_data.as_mut_ptr() as *mut std::ffi::c_void,
        bytes_per_row as u64,
        region,
        0,
    );

    Some(CapturedFrame::new_bgra(
        width as u32,
        height as u32,
        pixel_data,
    ))
}

#[cfg(test)]
pub(crate) fn convert_bgra_to_rgba(data: &mut [u8]) {
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
}

/// Creates an off-screen Metal texture
///
/// This is a utility function for headless/off-screen rendering scenarios where
/// you need to render to a texture rather than a window drawable. Currently unused
/// but kept for future headless capture or visual regression testing support.
///
/// # Arguments
/// * `device` - The Metal device to create the texture on
/// * `width` - The width of the texture in pixels
/// * `height` - The height of the texture in pixels
/// * `pixel_format` - The pixel format (should match the drawable format)
///
/// # Returns
/// * A new Metal texture that can be rendered to and read back from
#[allow(dead_code)]
pub fn create_capture_texture(
    device: &metal::Device,
    width: u64,
    height: u64,
    pixel_format: MTLPixelFormat,
) -> metal::Texture {
    let texture_descriptor = metal::TextureDescriptor::new();
    texture_descriptor.set_pixel_format(pixel_format);
    texture_descriptor.set_width(width);
    texture_descriptor.set_height(height);
    texture_descriptor.set_depth(1);
    texture_descriptor.set_mipmap_level_count(1);
    texture_descriptor.set_sample_count(1);
    texture_descriptor.set_array_length(1);

    // Set usage flags for rendering and reading
    texture_descriptor
        .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);

    // Use managed storage mode so we can read it back
    texture_descriptor.set_storage_mode(MTLStorageMode::Managed);

    device.new_texture(&texture_descriptor)
}
