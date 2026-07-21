//! Regression tests for the wgpu 30.0.0 API migration (APP-4885).
//!
//! These exercise the migrated call sites against a real wgpu adapter when one
//! is available (for example the lavapipe software Vulkan driver). They skip
//! gracefully when no adapter can be created, so they stay green on hosts
//! without a Vulkan/GPU stack while still catching regressions where a device
//! is present.
//!
//! What this covers:
//! - All three WGSL shader modules compile under wgpu 30's naga. wgpu 30 no
//!   longer defaults integer inter-stage I/O to `@interpolate(flat)`, so the
//!   `is_emoji` (glyph) and `is_icon` (image) vertex outputs must declare it
//!   explicitly; a missing qualifier surfaces as a shader validation error.
//! - The rect render pipeline builds with `VertexState::buffers` entries
//!   wrapped in `Some(...)`, verifying the optional vertex-buffer-slot
//!   migration at runtime rather than only at compile time.
//! - `create_buffer_init` succeeds and propagates mapping results through the
//!   new `Result`-returning `get_mapped_range_mut` path (no panic/unchecked
//!   mapping result is introduced).
//! - A `MAP_READ` buffer round-trips through `map_async` and the new
//!   `Result`-returning `get_mapped_range`, exercising the readback path used
//!   by frame capture.

use std::borrow::Cow;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use wgpu::util::BufferInitDescriptor;

use super::util::{create_buffer_init, with_error_scope};
use crate::rendering::wgpu::shader_types;

/// Acquire a wgpu device suitable for a headless test, preferring the software
/// Vulkan fallback (lavapipe) so the test is deterministic and does not depend
/// on a physical GPU. Returns `None` when no adapter is available, in which
/// case the calling test skips.
fn acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();

    // Prefer the fallback (software) adapter so the test runs on CI hosts that
    // only ship lavapipe. fall back to any adapter if none is reported.
    let adapter =
        crate::r#async::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        }))
        .ok()
        .or_else(|| {
            crate::r#async::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
                .into_iter()
                .next()
        })?;

    let (device, queue) =
        crate::r#async::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("request_device should succeed for a software adapter");
    Some((device, queue))
}

/// The uniform bind group layout shared by all three renderer shaders
/// (`@group(0) @binding(0) var<uniform> uniforms: Uniforms;` where `Uniforms`
/// is a 16-byte viewport-size + padding struct).
fn uniform_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("uniform bind group layout (wgpu30 test)"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: Some(NonZero::new(16u64).unwrap()),
            },
            count: None,
        }],
    })
}

/// Instance-data vertex attributes for the glyph pipeline, matching the
/// production `GlyphInstanceData::ATTRIBS` so the pipeline validates against the
/// real shader interface.
const GLYPH_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    1 => Float32x4, // Bounds
    2 => Float32x4, // UV Bounds
    3 => Float32,   // Fade Start
    4 => Float32,   // Fade end
    5 => Float32x4, // Color
    6 => Sint32,    // Is Emoji
];

/// Instance-data vertex attributes for the image pipeline, matching the
/// production `ImageInstanceData::ATTRIBS`.
const IMAGE_INSTANCE_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    1 => Float32x4, // Bounds
    2 => Float32x4, // Color
    3 => Uint32,    // Is Icon
    4 => Float32x4, // Corner radius
];

/// The texture bind group layout used by the glyph and image shaders
/// (`@group(1) @binding(0) var<texture_2d> ...; @group(1) @binding(1) var<
/// sampler> ...;`), matching the production `texture_bind_group_layout`.
fn texture_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("texture bind group layout (wgpu30 test)"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Build a render pipeline for one of the renderer shaders, wrapping the
/// vertex buffer layouts in `Some(...)` as the wgpu 30 migration requires.
/// Returns an error captured through the device's validation error scope so
/// the caller can assert on the migration-specific failure mode.
fn build_pipeline(
    device: &wgpu::Device,
    shader_label: &str,
    source: &str,
    vertex_entry: &str,
    fragment_entry: &str,
    instance_layout: wgpu::VertexBufferLayout<'static>,
    bind_group_layouts: &[wgpu::BindGroupLayout],
) -> (wgpu::RenderPipeline, Option<super::Error>) {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{shader_label} pipeline layout (wgpu30 test)")),
        bind_group_layouts: &bind_group_layouts.iter().map(Some).collect::<Vec<_>>(),
        immediate_size: 0,
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(shader_label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    });

    let color_target = wgpu::ColorTargetState {
        format: wgpu::TextureFormat::Bgra8Unorm,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::all(),
    };

    with_error_scope(device, || {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("{shader_label} render pipeline (wgpu30 test)")),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(vertex_entry),
                buffers: &[Some(shader_types::Vertex::desc()), Some(instance_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fragment_entry),
                targets: &[Some(color_target)],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    })
}

const SHADERS: &[(&str, &str)] = &[
    ("rect_shader", include_str!("../shaders/rect_shader.wgsl")),
    ("glyph_shader", include_str!("../shaders/glyph_shader.wgsl")),
    ("image_shader", include_str!("../shaders/image_shader.wgsl")),
];

#[test]
fn test_wgpu30_shaders_compile() {
    let Some((device, _queue)) = acquire_device() else {
        eprintln!("skipping test_wgpu30_shaders_compile: no wgpu adapter available");
        return;
    };

    for (label, source) in SHADERS {
        let (shader, error) = with_error_scope(&device, || {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
            })
        });
        assert!(
            error.is_none(),
            "wgpu 30 rejected shader {label:?}: {error:?}"
        );
        // Touch the shader so it is not optimized away; creation already
        // validated it through naga.
        drop(shader);
    }
}

#[test]
fn test_wgpu30_rect_pipeline_builds_with_optional_vertex_layouts() {
    let Some((device, _queue)) = acquire_device() else {
        eprintln!(
            "skipping test_wgpu30_rect_pipeline_builds_with_optional_vertex_layouts: no wgpu adapter available"
        );
        return;
    };

    // wgpu 30's `VertexState::buffers` takes `&[Option<VertexBufferLayout>]`;
    // the rect pipeline wraps both layouts in `Some(...)`. The rect shader's
    // instance data is `RectData`, whose `desc()` is accessible via
    // `shader_types`.
    let uniform_layout = uniform_bind_group_layout(&device);
    let (_pipeline, error) = build_pipeline(
        &device,
        "Rect Shader (wgpu30 test)",
        include_str!("../shaders/rect_shader.wgsl"),
        "vs_main",
        "rect_fs_main",
        shader_types::RectData::desc(),
        &[uniform_layout],
    );

    assert!(
        error.is_none(),
        "wgpu 30 rejected the rect render pipeline (VertexState::buffers Some() migration): {error:?}"
    );
}

#[test]
fn test_wgpu30_glyph_pipeline_builds_with_interpolate_flat() {
    let Some((device, _queue)) = acquire_device() else {
        eprintln!(
            "skipping test_wgpu30_glyph_pipeline_builds_with_interpolate_flat: no wgpu adapter available"
        );
        return;
    };

    // wgpu 30 no longer defaults integer inter-stage I/O to `@interpolate(flat)`.
    // The glyph shader's `is_emoji: i32` vertex output must declare it
    // explicitly; a missing qualifier surfaces as a pipeline validation error
    // when the vertex and fragment stages are linked here.
    let instance_layout = wgpu::VertexBufferLayout {
        array_stride: 60,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &GLYPH_INSTANCE_ATTRIBS,
    };
    let uniform_layout = uniform_bind_group_layout(&device);
    let texture_layout = texture_bind_group_layout(&device);
    let (_pipeline, error) = build_pipeline(
        &device,
        "Glyph Shader (wgpu30 test)",
        include_str!("../shaders/glyph_shader.wgsl"),
        "vs_main",
        "fs_main",
        instance_layout,
        &[uniform_layout, texture_layout],
    );

    assert!(
        error.is_none(),
        "wgpu 30 rejected the glyph render pipeline (integer @interpolate(flat) migration): {error:?}"
    );
}

#[test]
fn test_wgpu30_image_pipeline_builds_with_interpolate_flat() {
    let Some((device, _queue)) = acquire_device() else {
        eprintln!(
            "skipping test_wgpu30_image_pipeline_builds_with_interpolate_flat: no wgpu adapter available"
        );
        return;
    };

    // Same `@interpolate(flat)` requirement for the image shader's `is_icon: u32`
    // vertex output.
    let instance_layout = wgpu::VertexBufferLayout {
        array_stride: 52,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &IMAGE_INSTANCE_ATTRIBS,
    };
    let uniform_layout = uniform_bind_group_layout(&device);
    let texture_layout = texture_bind_group_layout(&device);
    let (_pipeline, error) = build_pipeline(
        &device,
        "Image Shader (wgpu30 test)",
        include_str!("../shaders/image_shader.wgsl"),
        "vs_main",
        "fs_main",
        instance_layout,
        &[uniform_layout, texture_layout],
    );

    assert!(
        error.is_none(),
        "wgpu 30 rejected the image render pipeline (integer @interpolate(flat) migration): {error:?}"
    );
}

#[test]
fn test_wgpu30_create_buffer_init_handles_mapping_result() {
    let Some((device, _queue)) = acquire_device() else {
        eprintln!(
            "skipping test_wgpu30_create_buffer_init_handles_mapping_result: no wgpu adapter available"
        );
        return;
    };

    let device_lost = Arc::new(AtomicBool::new(false));
    let contents = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

    // The non-empty path in create_buffer_init maps the buffer at creation and
    // writes through `get_mapped_range_mut`, which now returns a `Result` under
    // wgpu 30. A successful init proves the mapping result is handled without
    // panicking or leaving an unchecked `Result`.
    let buffer = create_buffer_init(
        &device,
        &device_lost,
        &BufferInitDescriptor {
            label: Some("wgpu30 test buffer"),
            contents: &contents,
            usage: wgpu::BufferUsages::VERTEX,
        },
    )
    .expect("create_buffer_init should succeed on a healthy device");

    // The buffer should be usable (unmapped after init).
    assert_eq!(buffer.size(), contents.len() as u64);
}

#[test]
fn test_wgpu30_buffer_readback_handles_get_mapped_range_result() {
    let Some((device, queue)) = acquire_device() else {
        eprintln!(
            "skipping test_wgpu30_buffer_readback_handles_get_mapped_range_result: no wgpu adapter available"
        );
        return;
    };

    // Mirror the frame-capture readback path: write a buffer, copy it into a
    // MAP_READ staging buffer, map it, and read through `get_mapped_range`
    // (which now returns a `Result` under wgpu 30).
    let payload = [0xABu8; 64];

    let source = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wgpu30 readback source"),
        size: payload.len() as u64,
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&source, 0, &payload);

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wgpu30 readback staging"),
        size: payload.len() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("wgpu30 readback encoder"),
    });
    encoder.copy_buffer_to_buffer(&source, 0, &staging, 0, payload.len() as u64);
    queue.submit(Some(encoder.finish()));

    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    crate::r#async::block_on(async {
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    });

    receiver
        .recv()
        .expect("map_async result should be received")
        .expect("map_async should succeed");

    // `get_mapped_range` now returns `Result<BufferView, MapRangeError>` under
    // wgpu 30; the readback path must handle it rather than indexing into a
    // `Result`.
    let data = slice
        .get_mapped_range()
        .expect("get_mapped_range should succeed after a successful map_async");
    assert_eq!(&data[..payload.len()], &payload[..]);
    drop(data);
    staging.unmap();
}

#[test]
fn test_wgpu30_get_mapped_range_reports_unmapped_error() {
    let Some((device, _queue)) = acquire_device() else {
        eprintln!(
            "skipping test_wgpu30_get_mapped_range_reports_unmapped_error: no wgpu adapter available"
        );
        return;
    };

    // wgpu does not expose a deterministic way to make an in-flight map fail,
    // but accessing a buffer after it is unmapped exercises the same
    // `get_mapped_range` Result error path. The production readback code must
    // propagate this error instead of assuming the view is always available.
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wgpu30 unmapped range"),
        size: 64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: true,
    });
    buffer.unmap();

    let result = buffer.slice(..).get_mapped_range();
    assert!(
        result.is_err(),
        "get_mapped_range should reject access to an unmapped buffer"
    );
}
