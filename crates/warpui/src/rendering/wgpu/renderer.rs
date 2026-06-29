mod frame;
mod glyph;
mod image;
mod rect;
mod util;

use frame::Frame;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use util::with_error_scope;
use warpui_core::platform::CapturedFrame;
use wgpu::wgc::device::DeviceError;
use wgpu::wgc::present::SurfaceError;

pub use super::resources::{GetSurfaceTextureError, SurfaceConfigureError};
use crate::r#async::block_on;
use crate::rendering::wgpu::Resources;
use crate::rendering::{GlyphConfig, GlyphRasterBoundsFn, RasterizeGlyphFn};
use crate::scene::SceneDamage;
use crate::Scene;

const ENCODER_DESCRIPTOR: wgpu::CommandEncoderDescriptor = wgpu::CommandEncoderDescriptor {
    label: Some("Command encoder"),
};

pub struct Renderer {
    rect_pipeline: rect::Pipeline,
    glyph_pipeline: glyph::Pipeline,
    image_pipeline: image::Pipeline,
    /// Whether the offscreen target currently holds a valid full render at
    /// `offscreen_size`. Only then can a cursor-only frame `Load`+scissor onto it.
    offscreen_valid: bool,
    /// Surface size (device px) the offscreen target was last fully rendered at.
    offscreen_size: Option<(u32, u32)>,
    /// Device-space cursor rects from the previous frame. Unioned with the
    /// current frame's cursor rects to form the damage region for a cursor-only
    /// repaint (so the region the cursor just vacated is also repainted).
    prev_cursor_rects: Vec<RectF>,
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
            offscreen_valid: false,
            offscreen_size: None,
            prev_cursor_rects: Vec::new(),
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
        let surface_width = surface_texture.texture.width();
        let surface_height = surface_texture.texture.height();
        let surface_size = Vector2F::new(surface_width as f32, surface_height as f32);

        // Cursor rects painted this frame, converted to device space.
        let scale = scene.scale_factor();
        let cur_cursor_rects: Vec<RectF> =
            scene.cursor_rects().iter().map(|r| *r * scale).collect();

        let offscreen = resources.offscreen_target(surface_width, surface_height);
        let size_unchanged = self.offscreen_size == Some((surface_width, surface_height));

        // A cursor-only frame may repaint just the damage region (the union of the
        // previous and current cursor rects), but only if we have a valid prior
        // full render of the offscreen target at this exact size to Load onto.
        let damage_rect = if matches!(scene.damage(), SceneDamage::CursorOnly)
            && self.offscreen_valid
            && size_unchanged
            && offscreen.is_some()
        {
            union_bounds(self.prev_cursor_rects.iter().chain(cur_cursor_rects.iter()))
                .map(|rect| expand_and_clamp(rect, surface_size, 4.0))
        } else {
            None
        };
        let do_partial = damage_rect.is_some();
        let load_op = if do_partial {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
        };

        let mut encoder = device.create_command_encoder(&ENCODER_DESCRIPTOR);
        let (_, error) = with_error_scope(device, || {
            match &offscreen {
                Some((offscreen_texture, offscreen_view)) => {
                    // Render into the persistent offscreen target (full `Clear`, or
                    // `Load`+scissor for a partial cursor repaint), then copy the
                    // whole offscreen into the swapchain image. The copy is a cheap
                    // GPU blit; the win is that a partial frame only re-rasterizes
                    // the small damage region instead of the entire window.
                    frame.draw(
                        resources,
                        &mut encoder,
                        offscreen_view,
                        surface_size,
                        load_op,
                        damage_rect,
                    );
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: offscreen_texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &surface_texture.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width: surface_width,
                            height: surface_height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                None => {
                    // Surface can't be a copy destination; render straight to it.
                    let view = surface_texture
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor {
                            format: Some(surface_texture.texture.format()),
                            ..Default::default()
                        });
                    frame.draw(
                        resources,
                        &mut encoder,
                        &view,
                        surface_size,
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        None,
                    );
                }
            }

            queue.submit(Some(encoder.finish()));
        });

        // Update offscreen/cursor bookkeeping for the next frame.
        if offscreen.is_some() {
            if !do_partial {
                // A full render populated the offscreen target; it's now valid to
                // Load onto for a subsequent partial cursor repaint.
                self.offscreen_valid = true;
                self.offscreen_size = Some((surface_width, surface_height));
            }
            // A partial frame Loaded onto the existing offscreen, so it stays valid.
        } else {
            self.offscreen_valid = false;
            self.offscreen_size = None;
        }
        self.prev_cursor_rects = cur_cursor_rects;

        if let Some(callback) = capture_callback {
            if let Err(err) =
                capture_surface_texture(device, queue, resources, &surface_texture, callback)
            {
                log::warn!("Frame capture failed: {err}");
            }
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

/// Bounding box of an iterator of rects, or `None` if the iterator is empty.
fn union_bounds<'a>(rects: impl Iterator<Item = &'a RectF>) -> Option<RectF> {
    let mut rects = rects;
    let first = rects.next()?;
    let mut min_x = first.min_x();
    let mut min_y = first.min_y();
    let mut max_x = first.max_x();
    let mut max_y = first.max_y();
    for rect in rects {
        min_x = min_x.min(rect.min_x());
        min_y = min_y.min(rect.min_y());
        max_x = max_x.max(rect.max_x());
        max_y = max_y.max(rect.max_y());
    }
    Some(RectF::new(
        vec2f(min_x, min_y),
        vec2f(max_x - min_x, max_y - min_y),
    ))
}

/// Expands `rect` by `pad` device pixels on each side and clamps it to the
/// `[0, size]` window bounds. The padding absorbs anti-aliasing / rounding so a
/// partial repaint fully covers the cursor.
fn expand_and_clamp(rect: RectF, size: Vector2F, pad: f32) -> RectF {
    let x0 = (rect.min_x() - pad).max(0.0);
    let y0 = (rect.min_y() - pad).max(0.0);
    let x1 = (rect.max_x() + pad).min(size.x());
    let y1 = (rect.max_y() + pad).min(size.y());
    RectF::new(vec2f(x0, y0), vec2f((x1 - x0).max(0.0), (y1 - y0).max(0.0)))
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
