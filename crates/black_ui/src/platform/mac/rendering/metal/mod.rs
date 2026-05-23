pub mod frame_capture;
mod renderer;
mod renderer_manager;

pub use renderer_manager::RendererManager;

/// Returns `true` if the given metal Device corresponds to the low power/integrated GPU.
///
/// In dual GPU Macs, this is `false` for the discrete high-performance GPU.
#[cfg_attr(wgpu, allow(dead_code))]
pub fn is_integrated_gpu(device: &metal::Device) -> bool {
    device.is_low_power() && !device.is_removable()
}
