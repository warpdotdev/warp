use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::mem;
use std::ptr::NonNull;
use std::sync::Once;

use dispatch2::DispatchData;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLDrawable, MTLFunction, MTLIndexType,
    MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion,
    MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLResourceOptions, MTLScissorRect, MTLSize, MTLStoreAction,
    MTLTexture, MTLTextureDescriptor, MTLViewport,
};
use objc2_quartz_core::CAMetalDrawable;
use pathfinder_color::{ColorF, ColorU};
use pathfinder_geometry::rect::{RectF, RectI};
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_errors::report_error;
use warpui_core::fonts::{self, RasterizedGlyph, SubpixelAlignment, canvas};
use warpui_core::platform::CapturedFrame;
use warpui_core::rendering::texture_cache::TextureCache;
use warpui_core::rendering::{self};
use warpui_core::scene::{CornerRadius, GlyphFade, GlyphKey, Icon, Image, Layer, Scene};

use super::frame_capture::capture_frame;
use crate::platform::mac::rendering::renderer::Device;
use crate::platform::mac::window::WindowState;
use crate::rendering::atlas::{AllocatedRegion, TextureId};
use crate::rendering::{GlyphCache, GlyphRasterBoundsFn, RasterizeGlyphFn, get_best_dash_gap};

const METAL_LIB_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shaders.metallib"));
static WRITE_LIB_TO_FILE: Once = Once::new();

/// A structure to help manage a single rendering pass.
struct RenderPass<'a> {
    drawable: &'a ProtocolObject<dyn CAMetalDrawable>,
    buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    encoder: Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>,
    encoding_finished: bool,
}

impl<'a> RenderPass<'a> {
    fn new(
        command_queue: &ProtocolObject<dyn MTLCommandQueue>,
        drawable: &'a ProtocolObject<dyn CAMetalDrawable>,
    ) -> Self {
        let buffer = command_queue
            .commandBuffer()
            .expect("command queue should always vend a command buffer");
        let encoder = buffer
            .renderCommandEncoderWithDescriptor(&Self::create_descriptor(drawable))
            .expect("command buffer should always vend a render command encoder");
        Self {
            drawable,
            buffer,
            encoder,
            encoding_finished: false,
        }
    }

    /// Finishes a render pass with optional frame capture.
    ///
    /// If this is not called, the encoded commands will not be executed and the
    /// drawable will not be updated.
    ///
    /// Returns the captured frame data if capture was requested.
    fn finish_with_capture(
        mut self,
        drawable_size: pathfinder_geometry::vector::Vector2F,
        should_capture: bool,
        presents_with_transaction: bool,
    ) -> Option<CapturedFrame> {
        self.encoder.endEncoding();
        self.encoding_finished = true;

        // If we're able to do asynchronous presentation, do so - it allows us to avoid
        // blocking on the GPU for the duration of the frame.
        if !should_capture && !presents_with_transaction {
            self.buffer
                .presentDrawable(ProtocolObject::from_ref(self.drawable));
            self.buffer.commit();
            return None;
        }

        // Otherwise, commit the buffer and wait for it to complete before continuing.
        self.buffer.commit();
        self.buffer.waitUntilCompleted();

        let capture = if should_capture {
            let texture = self.drawable.texture();
            capture_frame(&texture, drawable_size)
        } else {
            None
        };

        self.drawable.present();
        capture
    }

    /// Creates a descriptor for a pass that renders into the provided drawable.
    fn create_descriptor(
        drawable: &ProtocolObject<dyn CAMetalDrawable>,
    ) -> Retained<MTLRenderPassDescriptor> {
        let descriptor = MTLRenderPassDescriptor::new();

        // SAFETY: index 0 is always a valid color attachment slot for a CAMetalLayer's drawable.
        let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        color_attachment.setTexture(Some(&drawable.texture()));
        color_attachment.setLoadAction(MTLLoadAction::Clear);
        color_attachment.setStoreAction(MTLStoreAction::Store);
        color_attachment.setClearColor(MTLClearColor {
            red: 0.,
            green: 0.,
            blue: 0.,
            alpha: 0.,
        });

        descriptor
    }
}

impl Drop for RenderPass<'_> {
    fn drop(&mut self) {
        // Make sure that `end_encoding()` is called, even if a panic occurs
        // during rendering.
        if !self.encoding_finished {
            self.encoder.endEncoding();
        }
    }
}

/// A set of resources necessary for rendering that retain state across frames.
struct Resources {
    draw_rects_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    draw_images_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    draw_glyphs_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    quad_vertices: Retained<ProtocolObject<dyn MTLBuffer>>,
    quad_indices: Retained<ProtocolObject<dyn MTLBuffer>>,
    glyph_cache: GlyphCache<Retained<ProtocolObject<dyn MTLTexture>>>,
    texture_cache: TextureCache<Retained<ProtocolObject<dyn MTLTexture>>>,
}

/// A structure that manages rendering scenes using a particular hardware
/// device.
pub struct Renderer {
    resources: Resources,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl Renderer {
    pub fn new(
        device: &ProtocolObject<dyn MTLDevice>,
        color_pixel_format: MTLPixelFormat,
        glyph_config: rendering::GlyphConfig,
    ) -> Self {
        let library = if cfg!(feature = "enable-metal-frame-capture") {
            let temp_lib_path = std::env::temp_dir().join("shaders.metallib");
            WRITE_LIB_TO_FILE.call_once(|| {
                let mut file = File::create(&temp_lib_path).unwrap();
                file.write_all(METAL_LIB_BYTES).unwrap();
            });
            let path = NSString::from_str(temp_lib_path.to_str().unwrap());
            // `newLibraryWithURL:` is the non-deprecated replacement, but we
            // load the shader library from a file path here.
            #[allow(deprecated)]
            let library = device.newLibraryWithFile_error(&path).unwrap();
            library
        } else {
            let data = DispatchData::from_static_bytes(METAL_LIB_BYTES);
            device.newLibraryWithData_error(&data).unwrap()
        };

        let rect_vertex_shader = library
            .newFunctionWithName(&NSString::from_str("rect_vertex_shader"))
            .unwrap();
        let rect_fragment_shader = library
            .newFunctionWithName(&NSString::from_str("rect_fragment_shader"))
            .unwrap();
        let rect_pipeline = Self::create_pipeline(
            "Rects",
            color_pixel_format,
            &rect_vertex_shader,
            &rect_fragment_shader,
        );
        let draw_rects_pipeline_state = device
            .newRenderPipelineStateWithDescriptor_error(&rect_pipeline)
            .unwrap();

        let image_fragment_shader = library
            .newFunctionWithName(&NSString::from_str("image_fragment_shader"))
            .unwrap();
        let image_pipeline = Self::create_pipeline(
            "Images",
            color_pixel_format,
            &rect_vertex_shader,
            &image_fragment_shader,
        );
        let draw_images_pipeline_state = device
            .newRenderPipelineStateWithDescriptor_error(&image_pipeline)
            .unwrap();

        let glyph_vertex_shader = library
            .newFunctionWithName(&NSString::from_str("glyph_vertex_shader"))
            .unwrap();
        let glyph_fragment_shader = library
            .newFunctionWithName(&NSString::from_str("glyph_fragment_shader"))
            .unwrap();
        let glyph_pipeline = Self::create_pipeline(
            "Glyphs",
            color_pixel_format,
            &glyph_vertex_shader,
            &glyph_fragment_shader,
        );
        let draw_glyphs_pipeline_state = device
            .newRenderPipelineStateWithDescriptor_error(&glyph_pipeline)
            .unwrap();

        let quad_vertices = new_metal_buffer(
            device,
            &[
                shader::Vector2F::new(0., 0.),
                shader::Vector2F::new(1., 0.),
                shader::Vector2F::new(0., 1.),
                shader::Vector2F::new(1., 1.),
            ],
            MTLResourceOptions::StorageModeManaged,
        );

        let quad_indices = new_metal_buffer(
            device,
            &[0_u16, 1, 2, 2, 3, 1],
            MTLResourceOptions::StorageModeManaged,
        );

        let glyph_cache = GlyphCache::new(glyph_config);

        Self {
            resources: Resources {
                draw_rects_pipeline_state,
                draw_images_pipeline_state,
                draw_glyphs_pipeline_state,
                quad_vertices,
                quad_indices,
                glyph_cache,
                texture_cache: TextureCache::new(),
            },
            command_queue: device
                .newCommandQueue()
                .expect("device should always vend a command queue"),
        }
    }

    fn create_pipeline(
        label: &str,
        color_pixel_format: MTLPixelFormat,
        vertex_shader: &ProtocolObject<dyn MTLFunction>,
        fragment_shader: &ProtocolObject<dyn MTLFunction>,
    ) -> Retained<MTLRenderPipelineDescriptor> {
        let pipeline = MTLRenderPipelineDescriptor::new();
        pipeline.setLabel(Some(&NSString::from_str(label)));
        pipeline.setVertexFunction(Some(vertex_shader));
        pipeline.setFragmentFunction(Some(fragment_shader));

        // SAFETY: index 0 is always a valid color attachment slot for a render pipeline.
        let attachment = unsafe { pipeline.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setPixelFormat(color_pixel_format);
        attachment.setBlendingEnabled(true);
        attachment.setRgbBlendOperation(MTLBlendOperation::Add);
        attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
        attachment.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
        attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);

        pipeline
    }

    fn render(
        &mut self,
        scene: &Scene,
        ctx: &MetalDrawContext,
        should_capture: bool,
        presents_with_transaction: bool,
    ) -> Option<CapturedFrame> {
        self.resources
            .glyph_cache
            .update_config(&scene.rendering_config().glyphs);

        let render_pass = RenderPass::new(&self.command_queue, ctx.drawable);

        Frame::new(scene, &render_pass.encoder, &mut self.resources, ctx).draw();

        render_pass.finish_with_capture(
            ctx.drawable_size,
            should_capture,
            presents_with_transaction,
        )
    }
}

/// A struct that manages rendering a single frame: the encoding of a scene into
/// a set of GPU draw calls to rasterize the scene description into a bitmap
/// image.
pub struct Frame<'a> {
    scene: &'a Scene,
    command_encoder: &'a ProtocolObject<dyn MTLRenderCommandEncoder>,
    resources: &'a mut Resources,
    ctx: &'a MetalDrawContext<'a>,
}

impl<'a> Frame<'a> {
    fn new(
        scene: &'a Scene,
        command_encoder: &'a ProtocolObject<dyn MTLRenderCommandEncoder>,
        resources: &'a mut Resources,
        ctx: &'a MetalDrawContext<'a>,
    ) -> Self {
        Self {
            scene,
            resources,
            command_encoder,
            ctx,
        }
    }

    fn draw(&mut self) {
        self.command_encoder.setViewport(MTLViewport {
            originX: 0.0,
            originY: 0.0,
            width: self.ctx.drawable_size.x() as f64,
            height: self.ctx.drawable_size.y() as f64,
            znear: 0.0,
            zfar: 1.0,
        });

        for layer in self.scene.layers() {
            if let Some(bounds) = layer.clip_bounds {
                // Make sure the scissor rect doesn't extend beyond the boundaries
                // of the window, as required by the Metal API.
                // API docs: https://developer.apple.com/documentation/metal/mtlrendercommandencoder/1515583-setscissorrect?language=objc
                // Scissor test background reading: https://developer.mozilla.org/en-US/docs/Web/API/WebGL_API/By_example/Basic_scissoring
                let device_bounds = RectF::new(Vector2F::zero(), self.ctx.drawable_size);
                let bounds = (bounds * self.scene.scale_factor()).intersection(device_bounds);
                if let Some(intersection) = bounds {
                    self.command_encoder.setScissorRect(MTLScissorRect {
                        x: intersection.origin_x().round() as usize,
                        y: intersection.origin_y().round() as usize,
                        width: intersection.width().round() as usize,
                        height: intersection.height().round() as usize,
                    });
                } else {
                    // The layer's clip bounds don't intersect the window bounds
                    // at all; we can skip drawing anything in this layer.
                    continue;
                }
            } else {
                self.command_encoder.setScissorRect(MTLScissorRect {
                    x: 0_usize,
                    y: 0_usize,
                    width: self.ctx.drawable_size.x() as usize,
                    height: self.ctx.drawable_size.y() as usize,
                });
            }
            self.draw_rects(layer);
            self.draw_images(layer);
            self.draw_glyphs(layer);
        }
    }

    // Utility function to render image or icon in Metal.
    fn render_image_or_icon(&mut self, image: Option<&Image>, icon: Option<&Icon>) {
        let opacity;
        let bounds;
        let asset;
        let is_icon;
        let icon_color;
        let ui_corner_radius;

        if let Some(to_render) = image {
            opacity = to_render.opacity;
            bounds = to_render.bounds;
            asset = &to_render.asset;
            is_icon = false;
            icon_color = ColorF::new(0.0, 0.0, 0.0, opacity).into();
            ui_corner_radius = to_render.corner_radius;
        } else {
            let to_render = icon.unwrap();
            opacity = to_render.opacity;
            bounds = to_render.bounds;
            asset = &to_render.asset;
            is_icon = true;
            icon_color = to_render.color.to_f32().into();
            ui_corner_radius = CornerRadius::default();
        }

        let mut per_rect_uniforms = Vec::new();
        let scale_factor = self.scene.scale_factor();
        let bounds = bounds * scale_factor;
        let min_dimension = f32::min(bounds.height(), bounds.width());
        let corner_radius = crate::rendering::CornerRadius::from_ui_corner_radius(
            ui_corner_radius,
            scale_factor,
            min_dimension,
        );
        per_rect_uniforms.push(shader::PerRectUniforms::new(
            bounds.origin().into(),
            bounds.size().into(),
            corner_radius,
            0.,
            0.,
            0.,
            0.,
            vec2f(0.0, 0.0).into(),
            vec2f(1.0, 0.0).into(),
            ColorF::new(0.0, 0.0, 0.0, opacity).into(),
            ColorF::new(0.0, 0.0, 0.0, opacity).into(),
            vec2f(0.0, 0.0).into(),
            vec2f(1.0, 0.0).into(),
            ColorU::transparent_black().to_f32().into(),
            ColorU::transparent_black().to_f32().into(),
            is_icon,
            icon_color,
            Vector2F::zero().into(),
            ColorU::transparent_black().to_f32().into(),
            0_f32,
            0_f32,
            0.,
            vec2f(0.0, 0.0).into(),
        ));
        let per_rect_uniforms_buffer = new_metal_buffer(
            self.ctx.device,
            &per_rect_uniforms,
            MTLResourceOptions::StorageModeManaged,
        );

        let uniforms = shader::Uniforms::new(self.ctx.drawable_size.into());
        let uniforms_ptr = NonNull::from(&uniforms).cast::<c_void>();
        let uniforms_len = mem::size_of::<shader::Uniforms>();

        // SAFETY: the per-rect uniform buffer and `uniforms` value outlive this encoded draw
        // call, and the bound buffer/byte sizes and indices match the shader bindings.
        unsafe {
            self.command_encoder.setVertexBuffer_offset_atIndex(
                Some(&per_rect_uniforms_buffer),
                0,
                1,
            );
            self.command_encoder
                .setVertexBytes_length_atIndex(uniforms_ptr, uniforms_len, 2);
            self.command_encoder
                .setFragmentBytes_length_atIndex(uniforms_ptr, uniforms_len, 0);
        }

        let (_, texture) = self
            .resources
            .texture_cache
            .get_or_insert_by_asset(asset, |asset| {
                let width = asset.size().x() as usize;
                let height = asset.size().y() as usize;

                let texture_descriptor = MTLTextureDescriptor::new();
                texture_descriptor.setPixelFormat(MTLPixelFormat::RGBA8Unorm);
                // SAFETY: width/height come from a decoded asset and are within Metal limits.
                unsafe {
                    texture_descriptor.setWidth(width);
                    texture_descriptor.setHeight(height);
                }
                let texture = self
                    .ctx
                    .device
                    .newTextureWithDescriptor(&texture_descriptor)
                    .expect("device should create an RGBA8 texture");
                let region = MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width,
                        height,
                        depth: 1,
                    },
                };

                let bytes_per_row: usize = 4 * width;
                // SAFETY: rgba_bytes holds width*height*4 bytes laid out to match the region
                // and row stride.
                unsafe {
                    texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region,
                        0,
                        NonNull::new(asset.rgba_bytes().as_ptr() as *mut c_void)
                            .expect("asset rgba bytes pointer is non-null"),
                        bytes_per_row,
                    );
                }

                texture
            });

        // SAFETY: the bound texture and quad index buffer outlive this encoded draw call.
        unsafe {
            self.command_encoder
                .setFragmentTexture_atIndex(Some(&**texture), 0);

            self.command_encoder
                .drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount(
                    MTLPrimitiveType::Triangle,
                    6,
                    MTLIndexType::UInt16,
                    &self.resources.quad_indices,
                    0,
                    per_rect_uniforms.len(),
                );
        }
    }

    fn draw_images(&mut self, layer: &Layer) {
        if layer.images.is_empty() && layer.icons.is_empty() {
            // It's a mac assertion error to create an empty metal buffer, so exit early
            return;
        }

        self.command_encoder
            .setRenderPipelineState(&self.resources.draw_images_pipeline_state);
        // SAFETY: index 0 binds the shared quad vertex buffer, which outlives the draw calls.
        unsafe {
            self.command_encoder.setVertexBuffer_offset_atIndex(
                Some(&self.resources.quad_vertices),
                0,
                0,
            );
        }

        for image in &layer.images {
            self.render_image_or_icon(Some(image), None);
        }

        // Another iteration for rendering icons.
        for icon in &layer.icons {
            self.render_image_or_icon(None, Some(icon));
        }
    }

    fn draw_rects(&self, layer: &Layer) {
        if layer.rects.is_empty() {
            // It's a mac assertion error to create an empty metal buffer, so exit early
            return;
        }

        self.command_encoder
            .setRenderPipelineState(&self.resources.draw_rects_pipeline_state);
        // SAFETY: index 0 binds the shared quad vertex buffer, which outlives the draw call.
        unsafe {
            self.command_encoder.setVertexBuffer_offset_atIndex(
                Some(&self.resources.quad_vertices),
                0,
                0,
            );
        }

        let mut per_rect_uniforms = Vec::new();
        for rect in &layer.rects {
            let scale_factor = self.scene.scale_factor();
            let bounds = rect.bounds * scale_factor;

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
            let dash_length = dash.dash_length;
            let gap_lengths = Vector2F::new(horizontal_gap, vertical_gap);

            if let Some(drop_shadow) = rect.drop_shadow {
                let sigma = drop_shadow.blur_radius;
                let padding = drop_shadow.spread_radius * self.scene.scale_factor();
                let shadow_origin =
                    bounds.origin() + drop_shadow.offset * self.scene.scale_factor() - padding;
                let shadow_size = bounds.size() + vec2f(2. * padding, 2. * padding);

                let min_dimension = f32::min(shadow_size.x(), shadow_size.y());

                let corner_radius = crate::rendering::CornerRadius::from_ui_corner_radius(
                    rect.corner_radius,
                    scale_factor,
                    min_dimension,
                );

                // For the drop shadow case, we pass in a rect with the bounds
                // of the shadow and render that before rendering the actual rect.
                per_rect_uniforms.push(shader::PerRectUniforms::new(
                    shadow_origin.into(),
                    shadow_size.into(),
                    corner_radius,
                    0_f32,
                    0_f32,
                    0_f32,
                    0_f32,
                    Vector2F::zero().into(),
                    Vector2F::zero().into(),
                    ColorU::transparent_black().to_f32().into(),
                    ColorU::transparent_black().to_f32().into(),
                    Vector2F::zero().into(),
                    Vector2F::zero().into(),
                    ColorU::transparent_black().to_f32().into(),
                    ColorU::transparent_black().to_f32().into(),
                    false,
                    ColorU::transparent_black().to_f32().into(),
                    (drop_shadow.offset * self.scene.scale_factor()).into(),
                    drop_shadow.color.to_f32().into(),
                    sigma * self.scene.scale_factor(),
                    padding,
                    dash_length,
                    gap_lengths.into(),
                ));
            }

            let min_dimension = f32::min(bounds.height(), bounds.width());
            let corner_radius = crate::rendering::CornerRadius::from_ui_corner_radius(
                rect.corner_radius,
                scale_factor,
                min_dimension,
            );

            per_rect_uniforms.push(shader::PerRectUniforms::new(
                bounds.origin().into(),
                bounds.size().into(),
                corner_radius,
                rect.border.top_width() * scale_factor,
                rect.border.right_width() * scale_factor,
                rect.border.bottom_width() * scale_factor,
                rect.border.left_width() * scale_factor,
                rect.background.start().into(),
                rect.background.end().into(),
                rect.background.start_color().to_f32().into(),
                rect.background.end_color().to_f32().into(),
                rect.border.color.start().into(),
                rect.border.color.end().into(),
                rect.border.color.start_color().to_f32().into(),
                rect.border.color.end_color().to_f32().into(),
                false,
                ColorU::transparent_black().to_f32().into(),
                Vector2F::zero().into(),
                ColorU::transparent_black().to_f32().into(),
                0_f32,
                0_f32,
                dash_length,
                gap_lengths.into(),
            ));
        }
        let per_rect_uniforms_buffer = new_metal_buffer(
            self.ctx.device,
            &per_rect_uniforms,
            MTLResourceOptions::StorageModeManaged,
        );

        let uniforms = shader::Uniforms::new(self.ctx.drawable_size.into());
        let uniforms_ptr = NonNull::from(&uniforms).cast::<c_void>();
        let uniforms_len = mem::size_of::<shader::Uniforms>();

        // SAFETY: the per-rect uniform buffer and `uniforms` value outlive this encoded draw
        // call, and the bound buffer/byte sizes and indices match the shader bindings.
        unsafe {
            self.command_encoder.setVertexBuffer_offset_atIndex(
                Some(&per_rect_uniforms_buffer),
                0,
                1,
            );
            self.command_encoder
                .setVertexBytes_length_atIndex(uniforms_ptr, uniforms_len, 2);
            self.command_encoder
                .setFragmentBytes_length_atIndex(uniforms_ptr, uniforms_len, 0);

            self.command_encoder
                .drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount(
                    MTLPrimitiveType::Triangle,
                    6,
                    MTLIndexType::UInt16,
                    &self.resources.quad_indices,
                    0,
                    per_rect_uniforms.len(),
                );
        }
    }

    fn draw_glyphs(&mut self, layer: &Layer) {
        if layer.glyphs.is_empty() {
            // It's a mac assertion error to create an empty metal buffer, so exit early
            return;
        }

        self.command_encoder
            .setRenderPipelineState(&self.resources.draw_glyphs_pipeline_state);
        // SAFETY: index 0 binds the shared quad vertex buffer, which outlives the draw calls.
        unsafe {
            self.command_encoder.setVertexBuffer_offset_atIndex(
                Some(&self.resources.quad_vertices),
                0,
                0,
            );
        }

        let scale_factor = self.scene.scale_factor();

        let mut texture_to_glyph: HashMap<TextureId, Vec<shader::PerGlyphUniforms>> =
            HashMap::new();
        for glyph in &layer.glyphs {
            let glyph_position = glyph.position * scale_factor;
            let subpixel_alignment = SubpixelAlignment::new(glyph_position);

            match self.resources.glyph_cache.get(
                glyph.glyph_key,
                self.scene.scale_factor(),
                subpixel_alignment,
                &|atlas_size| create_new_texture_atlas(atlas_size, self.ctx.device),
                &insert_glyph_into_texture,
                &|glyph_key, scale, alignment| {
                    self.ctx.glyph_raster_bounds(glyph_key, scale, alignment)
                },
                &|glyph_key, scale, subpixel_alignment, glyph_config, format| {
                    self.ctx.rasterize_glyph(
                        glyph_key,
                        scale,
                        subpixel_alignment,
                        glyph_config,
                        format,
                    )
                },
            ) {
                Ok(Some(gto)) => {
                    let (fade_start, fade_end) = match &glyph.fade {
                        None => (&0.0, &-1.0),
                        Some(GlyphFade::Horizontal { start, end }) => (start, end),
                    };

                    // Adjust the horizontal position by the subpixel alignment
                    // so that we only shift the glyph over by the amount that
                    // isn't accounted for in the subpixel-rasterized glyph.
                    let glyph_position = glyph_position - subpixel_alignment.to_offset();

                    // Make sure to pass the glyph size in the atlas
                    // Not the size of the render bounds (which may be smaller)
                    // If you pass the render bounds as the size, the shader
                    // will try to sample from a smaller area than the size
                    // in the atlas, leading to artifacts.
                    let uv_region = gto.allocated_region.uv_region;
                    let uniform = shader::PerGlyphUniforms::new(
                        (glyph_position + gto.raster_bounds.origin()).into(),
                        gto.allocated_region.pixel_region.size().to_f32().into(),
                        uv_region.origin_x(),
                        uv_region.origin_y(),
                        uv_region.width(),
                        uv_region.height(),
                        fade_start * scale_factor,
                        fade_end * scale_factor,
                        glyph.color.to_f32().into(),
                        gto.is_emoji,
                    );

                    if let Some(per_glyph_uniforms) = texture_to_glyph.get_mut(&gto.texture_id) {
                        per_glyph_uniforms.push(uniform);
                    } else {
                        texture_to_glyph.insert(gto.texture_id, vec![uniform]);
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    report_error!(
                        "Unable to get glyph out of glyph cache",
                        extra: { "glyph" => ?glyph }
                    );
                    return;
                }
            }
        }

        if texture_to_glyph.is_empty() {
            // Early exit if there are no glyphs to render, as it causes a debug assert
            // failure in the metal code to create an empty metal buffer.
            return;
        }

        for (texture_id, per_glyph_uniforms) in texture_to_glyph {
            let per_glyph_uniforms_buffer = new_metal_buffer(
                self.ctx.device,
                &per_glyph_uniforms,
                MTLResourceOptions::StorageModeManaged,
            );

            let uniforms = shader::Uniforms::new(self.ctx.drawable_size.into());
            let uniforms_ptr = NonNull::from(&uniforms).cast::<c_void>();
            let uniforms_len = mem::size_of::<shader::Uniforms>();

            let texture = self
                .resources
                .glyph_cache
                .texture(&texture_id)
                .expect("texture ID should be in atlas");

            // SAFETY: the per-glyph uniform buffer, `uniforms` value, bound texture, and quad
            // index buffer outlive this encoded draw call, and the bound sizes/indices match the
            // shader bindings.
            unsafe {
                self.command_encoder.setVertexBuffer_offset_atIndex(
                    Some(&per_glyph_uniforms_buffer),
                    0,
                    1,
                );
                self.command_encoder
                    .setVertexBytes_length_atIndex(uniforms_ptr, uniforms_len, 2);
                self.command_encoder
                    .setFragmentTexture_atIndex(Some(&**texture), 0);
                self.command_encoder
                    .drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount(
                        MTLPrimitiveType::Triangle,
                        6,
                        MTLIndexType::UInt16,
                        &self.resources.quad_indices,
                        0,
                        per_glyph_uniforms.len(),
                    );
            }
        }
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self) {
        self.resources.texture_cache.end_frame();
    }
}

fn new_metal_buffer<T>(
    device: &ProtocolObject<dyn MTLDevice>,
    data: &[T],
    options: MTLResourceOptions,
) -> Retained<ProtocolObject<dyn MTLBuffer>> {
    // SAFETY: `data` points to `size_of_val(data)` initialized bytes; Metal copies them into the
    // new buffer, so the pointer only needs to be valid for the duration of this call.
    unsafe {
        device.newBufferWithBytes_length_options(
            NonNull::new(data.as_ptr() as *mut c_void).expect("buffer data pointer is non-null"),
            std::mem::size_of_val(data),
            options,
        )
    }
    .expect("device should create a buffer")
}

mod shader {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    // Temporarily silence the warning coming from https://github.com/rust-lang/rust-bindgen/issues/1651
    #![allow(unknown_lints)]

    use pathfinder_color::ColorF;
    use pathfinder_geometry::vector::{
        Vector2F as PathfinderVector2F, Vector4F as PathfinderVector4F,
    };
    pub use shader_types::*;

    mod shader_types {
        // Bindgen deferences null pointers in generated test code, see:
        // https://github.com/rust-lang/rust-bindgen/issues/1651
        #![allow(deref_nullptr)]
        include!(concat!(env!("OUT_DIR"), "/shader_types.rs"));
    }

    pub struct Vector2F(vector_float2);
    pub struct Vector4F(vector_float4);

    impl Vector2F {
        pub fn new(x: f32, y: f32) -> Self {
            let y = y.to_bits();
            let mut vec = (y as vector_float2) << 32;
            let x = x.to_bits();
            vec |= x as vector_float2;
            Self(vec)
        }
    }

    impl From<PathfinderVector2F> for Vector2F {
        fn from(vec: PathfinderVector2F) -> Self {
            Self::new(vec.x(), vec.y())
        }
    }

    impl Vector4F {
        pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
            let w = w.to_bits();
            let mut vec = w as vector_float4;
            vec <<= 32;
            let z = z.to_bits();
            vec |= z as vector_float4;
            vec <<= 32;
            let y = y.to_bits();
            vec |= y as vector_float4;
            vec <<= 32;
            let x = x.to_bits();
            vec |= x as vector_float4;
            Self(vec)
        }
    }

    impl From<PathfinderVector4F> for Vector4F {
        fn from(vec: PathfinderVector4F) -> Self {
            Self::new(vec.x(), vec.y(), vec.z(), vec.w())
        }
    }

    impl From<ColorF> for Vector4F {
        fn from(color: ColorF) -> Self {
            Self::new(color.r(), color.g(), color.b(), color.a())
        }
    }

    impl PerRectUniforms {
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            origin: Vector2F,
            size: Vector2F,
            corner_radius: crate::rendering::CornerRadius,
            border_top: f32,
            border_right: f32,
            border_bottom: f32,
            border_left: f32,
            background_start: Vector2F,
            background_end: Vector2F,
            background_start_color: Vector4F,
            background_end_color: Vector4F,
            border_start: Vector2F,
            border_end: Vector2F,
            border_start_color: Vector4F,
            border_end_color: Vector4F,
            is_icon: bool,
            icon_color: Vector4F,
            drop_shadow_offsets: Vector2F,
            drop_shadow_color: Vector4F,
            drop_shadow_sigma: f32,
            drop_shadow_padding_factor: f32,
            dash_length: f32,
            gap_lengths: Vector2F,
        ) -> Self {
            Self {
                origin: origin.0,
                size: size.0,
                corner_radius_top_left: corner_radius.top_left,
                corner_radius_top_right: corner_radius.top_right,
                corner_radius_bottom_left: corner_radius.bottom_left,
                corner_radius_bottom_right: corner_radius.bottom_right,
                border_top,
                border_right,
                border_bottom,
                border_left,
                background_start: background_start.0,
                background_end: background_end.0,
                background_start_color: background_start_color.0,
                background_end_color: background_end_color.0,
                border_start: border_start.0,
                border_end: border_end.0,
                border_start_color: border_start_color.0,
                border_end_color: border_end_color.0,
                is_icon: is_icon as i32,
                icon_color: icon_color.0,
                drop_shadow_offsets: drop_shadow_offsets.0,
                drop_shadow_color: drop_shadow_color.0,
                drop_shadow_sigma,
                drop_shadow_padding_factor,
                dash_length,
                gap_lengths: gap_lengths.0,
            }
        }
    }

    impl PerGlyphUniforms {
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            origin: Vector2F,
            size: Vector2F,
            uv_left: f32,
            uv_top: f32,
            uv_width: f32,
            uv_height: f32,
            fade_start: f32,
            fade_end: f32,
            color: Vector4F,
            is_emoji: bool,
        ) -> Self {
            Self {
                origin: origin.0,
                size: size.0,
                color: color.0,
                uv_left,
                uv_top,
                uv_width,
                uv_height,
                fade_start,
                fade_end,
                is_emoji: is_emoji as i32,
                __bindgen_padding_0: Default::default(),
            }
        }
    }

    impl Uniforms {
        pub fn new(viewport_size: Vector2F) -> Self {
            Self {
                viewport_size: viewport_size.0,
            }
        }
    }
}

pub(super) struct MetalDrawContext<'a> {
    pub(super) device: &'a ProtocolObject<dyn MTLDevice>,
    pub(super) drawable: &'a ProtocolObject<dyn CAMetalDrawable>,
    pub(super) drawable_size: Vector2F,
    rasterize_glyph_fn: &'a RasterizeGlyphFn<'a>,
    glyph_raster_bounds_fn: &'a GlyphRasterBoundsFn<'a>,
}

impl MetalDrawContext<'_> {
    pub(super) fn rasterize_glyph(
        &self,
        glyph_key: GlyphKey,
        scale: Vector2F,
        subpixel_alignment: SubpixelAlignment,
        glyph_config: &rendering::GlyphConfig,
        format: canvas::RasterFormat,
    ) -> anyhow::Result<RasterizedGlyph> {
        (self.rasterize_glyph_fn)(glyph_key, scale, subpixel_alignment, glyph_config, format)
    }

    pub(super) fn glyph_raster_bounds(
        &self,
        glyph_key: GlyphKey,
        scale: Vector2F,
        glyph_config: &rendering::GlyphConfig,
    ) -> anyhow::Result<RectI> {
        (self.glyph_raster_bounds_fn)(glyph_key, scale, glyph_config)
    }
}

impl super::super::Renderer for Renderer {
    fn render(&mut self, scene: &Scene, window: &WindowState, font_cache: &fonts::Cache) {
        // SAFETY: `render` is called via `warp_update_layer`, which is only be invoked for
        // windows created via Window::open() and always sets a non-`None` device.
        #[allow(irrefutable_let_patterns)]
        let Device::Metal(metal_device) = window
            .device()
            .expect("render is only called for a window that has a real display")
        else {
            report_error!("Metal renderer called with non-metal device");
            return;
        };
        let metal_device: &ProtocolObject<dyn MTLDevice> = metal_device;

        let metal_layer = window.metal_layer();
        let presents_with_transaction = metal_layer.presentsWithTransaction();
        let drawable = metal_layer
            .nextDrawable()
            .expect("CAMetalLayer with allowsNextDrawableTimeout disabled always vends a drawable");

        let ctx = &MetalDrawContext {
            device: metal_device,
            drawable: &drawable,
            drawable_size: window.physical_size(),
            rasterize_glyph_fn: &|glyph_key, scale, subpixel_alignment, glyph_config, format| {
                font_cache.rasterized_glyph(
                    glyph_key,
                    scale,
                    subpixel_alignment,
                    glyph_config,
                    format,
                )
            },
            glyph_raster_bounds_fn: &|glyph_key, scale, alignment| {
                font_cache.glyph_raster_bounds(glyph_key, scale, alignment)
            },
        };

        let capture_callback = window.capture_callback.borrow_mut().take();
        let should_capture = capture_callback.is_some();
        let captured = Self::render(self, scene, ctx, should_capture, presents_with_transaction);
        if let (Some(frame), Some(callback)) = (captured, capture_callback) {
            callback(frame);
        }
    }

    fn resize(&mut self, _window: &WindowState) {
        // TODO(alokedesai): Backport the optimization to only set the size of surface when a
        // window is resized to the Metal renderer.
    }
}

/// Writes the bytes of the `glyph` into a region of the current texture identified by `region`.
fn insert_glyph_into_texture(
    region: AllocatedRegion,
    glyph: &RasterizedGlyph,
    texture: &mut Retained<ProtocolObject<dyn MTLTexture>>,
) {
    let region = MTLRegion {
        origin: MTLOrigin {
            x: region.pixel_region.origin_x() as usize,
            y: region.pixel_region.origin_y() as usize,
            z: 0,
        },
        size: MTLSize {
            width: region.pixel_region.width() as usize,
            height: region.pixel_region.height() as usize,
            depth: 1,
        },
    };

    let bytes_per_row: usize = 4 * (glyph.canvas.size.x() as usize);
    // SAFETY: the glyph canvas holds at least `bytes_per_row * region.height` bytes laid out to
    // match the destination region.
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
            region,
            0,
            NonNull::new(glyph.canvas.pixels.as_slice().as_ptr() as *mut c_void)
                .expect("glyph canvas pixel pointer is non-null"),
            bytes_per_row,
        );
    }
}

/// Creates a new texture atlas for use in the cache.
fn create_new_texture_atlas(
    atlas_size: usize,
    device: &ProtocolObject<dyn MTLDevice>,
) -> Retained<ProtocolObject<dyn MTLTexture>> {
    let texture_descriptor = MTLTextureDescriptor::new();
    texture_descriptor.setPixelFormat(MTLPixelFormat::RGBA8Unorm);
    // SAFETY: `atlas_size` is a fixed, valid texture dimension within Metal limits.
    unsafe {
        texture_descriptor.setWidth(atlas_size);
        texture_descriptor.setHeight(atlas_size);
    }
    device
        .newTextureWithDescriptor(&texture_descriptor)
        .expect("device should create an atlas texture")
}
