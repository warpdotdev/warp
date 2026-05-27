mod metal;
mod renderer;
mod renderer_manager;

#[cfg(wgpu)]
mod wgpu;

pub use renderer::{Device, Renderer};
pub use renderer_manager::RendererManager;

pub use self::metal::is_integrated_gpu;

/// Returns `true` if a low power GPU is available for rendering. Typically, this is true for
/// machines with two GPUs -- a dedicated discrete high-performance GPU and a lower power
/// integrated GPU.
pub fn is_low_power_gpu_available() -> bool {
    cfg_if::cfg_if! {
        if #[cfg(wgpu)] {
            crate::r#async::block_on(crate::rendering::wgpu::is_low_power_gpu_available())
        } else {
            let devices = ::metal::Device::all();
            let gpu_count = devices.len();
            gpu_count > 1
                && devices
                    .iter()
                    .any(metal::is_integrated_gpu)
        }
    }
}
