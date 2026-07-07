pub(crate) mod atlas;
pub(crate) mod glyph_cache;
#[cfg(wgpu)]
pub mod wgpu;

pub(crate) use glyph_cache::{GlyphCache, GlyphRasterBoundsFn, RasterizeGlyphFn};
pub use warpui_core::rendering::*;
use warpui_core::scene::Dash;

/// Cache for the result of calling [`is_low_power_gpu_available`], as the
/// check can be expensive.
static LOW_POWER_GPU_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Returns `true` if a low power GPU is available for rendering. Typically, this is true for
/// machines with two GPUs -- a dedicated discrete high-performance GPU and a lower power
/// integrated GPU.
pub fn is_low_power_gpu_available() -> bool {
    *LOW_POWER_GPU_AVAILABLE.get_or_init(|| {
        cfg_if::cfg_if! {
            if #[cfg(target_os = "macos")] {
                crate::platform::mac::is_low_power_gpu_available()
            } else if #[cfg(wgpu)] {
                warpui_core::r#async::block_on(wgpu::is_low_power_gpu_available())
            } else {
                false
            }
        }
    })
}

/// Returns the gap length between each dash to ensure that the stroke begins and ends with a full dash,
/// minimizing deviation from the target gap length.
// adapted from Blink dashed border rendering code:
// https://source.chromium.org/chromium/chromium/src/+/refs/heads/main:third_party/blink/renderer/platform/graphics/stroke_data.cc;l=130-147;drc=51e1b713f6da38219910bf8fb93a81262340bf97
pub(crate) fn get_best_dash_gap(
    stroke_length: f32,
    Dash {
        dash_length,
        gap_length,
        force_consistent_gap_length,
    }: Dash,
) -> f32 {
    if force_consistent_gap_length {
        return gap_length;
    }

    // If no space for two dashes and a gap between, return gap length 0 (solid border)
    if stroke_length < 2. * dash_length + gap_length {
        return 0.;
    }

    let min_num_dashes = (stroke_length / (dash_length + gap_length)).floor();
    let max_num_dashes = min_num_dashes + 1.;
    let min_num_gaps = min_num_dashes - 1.;
    let max_num_gaps = max_num_dashes - 1.;
    let min_gap = (stroke_length - min_num_dashes * dash_length) / min_num_gaps;
    let max_gap = (stroke_length - max_num_dashes * dash_length) / max_num_gaps;
    if max_gap <= 0. || ((min_gap - gap_length).abs() < (max_gap - gap_length).abs()) {
        min_gap
    } else {
        max_gap
    }
}
