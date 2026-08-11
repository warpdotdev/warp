use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use pathfinder_geometry::vector::Vector2F;
use warpui_core::platform::CapturedFrame;

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
pub fn capture_frame(
    texture: &ProtocolObject<dyn MTLTexture>,
    size: Vector2F,
) -> Option<CapturedFrame> {
    let width = size.x() as usize;
    let height = size.y() as usize;

    if width == 0 || height == 0 {
        log::warn!("Invalid texture dimensions: {width}x{height}");
        return None;
    }

    let bytes_per_row = width * 4;
    let buffer_size = bytes_per_row * height;

    let mut pixel_data: Vec<u8> = vec![0u8; buffer_size];

    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width,
            height,
            depth: 1,
        },
    };

    // SAFETY: `pixel_data` holds `bytes_per_row * height` bytes, matching the requested region and
    // row stride, so Metal copies the texture contents into a valid buffer.
    unsafe {
        texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
            NonNull::new(pixel_data.as_mut_ptr() as *mut c_void)
                .expect("pixel buffer pointer is non-null"),
            bytes_per_row,
            region,
            0,
        );
    }

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
    device: &ProtocolObject<dyn MTLDevice>,
    width: usize,
    height: usize,
    pixel_format: MTLPixelFormat,
) -> Retained<ProtocolObject<dyn MTLTexture>> {
    let texture_descriptor = MTLTextureDescriptor::new();
    texture_descriptor.setPixelFormat(pixel_format);
    // SAFETY: the dimensions are caller-provided valid texture sizes within Metal limits.
    unsafe {
        texture_descriptor.setWidth(width);
        texture_descriptor.setHeight(height);
        texture_descriptor.setDepth(1);
        texture_descriptor.setMipmapLevelCount(1);
        texture_descriptor.setSampleCount(1);
        texture_descriptor.setArrayLength(1);
    }

    // Set usage flags for rendering and reading
    texture_descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);

    // Use managed storage mode so we can read it back
    texture_descriptor.setStorageMode(MTLStorageMode::Managed);

    device
        .newTextureWithDescriptor(&texture_descriptor)
        .expect("device should create a capture texture")
}
