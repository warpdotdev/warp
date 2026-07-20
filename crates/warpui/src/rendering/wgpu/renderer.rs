mod frame;
mod glyph;
mod image;
mod rect;
mod util;

use frame::Frame;
use pathfinder_geometry::vector::Vector2F;
use util::with_error_scope;
use warpui_core::platform::CapturedFrame;
use wgpu::wgc::device::DeviceError;
use wgpu::wgc::present::SurfaceError;

pub use super::resources::{GetSurfaceTextureError, SurfaceConfigureError};
use crate::Scene;
use crate::r#async::block_on;
use crate::rendering::wgpu::Resources;
use crate::rendering::{GlyphConfig, GlyphRasterBoundsFn, RasterizeGlyphFn};

const ENCODER_DESCRIPTOR: wgpu::CommandEncoderDescriptor = wgpu::CommandEncoderDescriptor {
    label: Some("Command encoder"),
};

pub struct Renderer {
    rect_pipeline: rect::Pipeline,
    glyph_pipeline: glyph::Pipeline,
    image_pipeline: image::Pipeline,
}

impl Renderer {
    pub fn new(resources: &Resources, glyph_config: GlyphConfig) -> Self {
        let Resources { device, .. } = resources;

        let format = resources.surface_config.borrow().format;
        let color_target = wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::all(),
        };

        let rect_pipeline = rect::Pipeline::new(
            resources.uniform_bind_group_layout(),
            device,
            color_target.clone(),
        );

        let glyph_pipeline = glyph::Pipeline::new(
            resources.uniform_bind_group_layout(),
            device,
            color_target.clone(),
            glyph_config,
        );

        let image_pipeline =
            image::Pipeline::new(resources.uniform_bind_group_layout(), device, color_target);

        Self {
            rect_pipeline,
            glyph_pipeline,
            image_pipeline,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render<'a>(
        &mut self,
        scene: &Scene,
        resources: &Resources,
        rasterize_glyph_fn: &RasterizeGlyphFn,
        glyph_raster_bounds_fn: &GlyphRasterBoundsFn,
        window_size: Vector2F,
        pre_present_callback: Option<Box<dyn FnOnce() + 'a>>,
        capture_callback: Option<Box<dyn FnOnce(CapturedFrame) + Send + 'static>>,
    ) -> Result<(), Error> {
        let Resources { device, queue, .. } = resources;

        // Don't initiate the render if we are trying to render into a
        // zero-sized window.
        if window_size.is_zero() {
            return Ok(());
        }

        let mut ctx = WGPUContext {
            resources,
            rasterize_glyph_fn,
            glyph_raster_bounds_fn,
        };

        let frame = match with_error_scope(device, || {
            Frame::new(
                scene,
                &mut ctx,
                &self.rect_pipeline,
                &mut self.glyph_pipeline,
                &mut self.image_pipeline,
            )
        }) {
            (_, Some(error)) => return Err(error),
            (frame, _) => frame,
        };

        let surface_texture = resources.get_surface_texture()?;

        let mut encoder = device.create_command_encoder(&ENCODER_DESCRIPTOR);
        let (_, error) = with_error_scope(device, || {
            frame.draw(resources, &mut encoder, &surface_texture);
            queue.submit(Some(encoder.finish()));
        });

        if let Some(callback) = capture_callback
            && let Err(err) =
                capture_surface_texture(device, queue, resources, &surface_texture, callback)
        {
            log::warn!("Frame capture failed: {err}");
        }

        if let Some(callback) = pre_present_callback {
            callback();
        }

        match error {
            Some(error) => Err(error),
            None => {
                // Only present the surface if there were no errors, otherwise
                // wgpu will print out an error that we attempted to present a
                // texture without submitting any work to the GPU.
                match with_error_scope(device, || {
                    surface_texture.present();
                }) {
                    (_, None) => Ok(()),
                    (_, Some(error)) => Err(error),
                }
            }
        }
    }
}

/// Errors that can occur while rendering a scene.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Device was lost")]
    DeviceLost,
    #[error("Failed to acquire surface texture: {0:#}")]
    SurfaceError(#[from] GetSurfaceTextureError),
    #[error("Failed to configure surface: {0:#}")]
    SurfaceConfigureError(#[from] SurfaceConfigureError),
    #[error("{0:#}")]
    Unknown(#[source] wgpu::Error),
}

impl From<wgpu::Error> for Error {
    fn from(value: wgpu::Error) -> Self {
        for error in anyhow::Chain::new(&value) {
            if let Some(DeviceError::Lost) = error.downcast_ref::<DeviceError>() {
                return Error::DeviceLost;
            }

            // The use of `#[transparent]` for many nested device errors breaks
            // error chaining - the call to `source()` gets forwarded to the
            // DeviceError::Lost, which returns None (it doesn't wrap an error).
            // Ideally, these wrapped errors should use `#[from]` instead, but
            // until then, we need to do this to properly catch DeviceError::Lost
            // from within a call to present().
            if let Some(SurfaceError::Device(DeviceError::Lost)) =
                error.downcast_ref::<SurfaceError>()
            {
                return Error::DeviceLost;
            }
        }
        Error::Unknown(value)
    }
}

/// Copies the current surface texture into a `CapturedFrame` and delivers it via `callback`.
///
/// **`callback` is invoked synchronously on the render thread** once the GPU readback
/// completes. It must be lightweight (e.g., move the frame into a shared buffer and return
/// immediately) to avoid stalling frame presentation.
fn capture_surface_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &Resources,
    surface_texture: &wgpu::SurfaceTexture,
    callback: Box<dyn FnOnce(CapturedFrame) + Send + 'static>,
) -> Result<(), String> {
    let texture = &surface_texture.texture;
    let width = texture.width();
    let height = texture.height();

    if width == 0 || height == 0 {
        return Err(format!("Invalid texture dimensions: {width}x{height}"));
    }

    let format = resources.surface_config.borrow().format;
    let bytes_per_pixel = 4u32;
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
    let buffer_size = (padded_bytes_per_row * height) as u64;

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Frame capture staging buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Frame capture encoder"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let buffer_slice = staging_buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });

    block_on(async {
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    });

    let map_result = receiver
        .recv()
        .map_err(|e| format!("Failed to receive map result: {e}"))?
        .map_err(|e| format!("Buffer mapping failed: {e}"));

    map_result?;

    let data = buffer_slice.get_mapped_range();
    let mut rgba_data = Vec::with_capacity((width * height * bytes_per_pixel) as usize);
    for row in 0..height {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + unpadded_bytes_per_row as usize;
        rgba_data.extend_from_slice(&data[start..end]);
    }
    drop(data);
    staging_buffer.unmap();

    if format == wgpu::TextureFormat::Bgra8Unorm || format == wgpu::TextureFormat::Bgra8UnormSrgb {
        for chunk in rgba_data.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }
    }

    callback(CapturedFrame::new(width, height, rgba_data));
    Ok(())
}

struct WGPUContext<'a> {
    resources: &'a Resources,
    rasterize_glyph_fn: &'a RasterizeGlyphFn<'a>,
    glyph_raster_bounds_fn: &'a GlyphRasterBoundsFn<'a>,
}
