use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSView, NSWindow};
use objc2_metal::MTLDevice;
use warpui_core::rendering::{
    GPUBackend, GPUDeviceInfo, GPUDeviceType, GPUPowerPreference, OnGPUDeviceSelected,
};
use warpui_core::{Scene, fonts};

use crate::platform::mac::rendering::is_integrated_gpu;
use crate::platform::mac::window::WindowState;

/// An owned handle to a Metal device, used to render with the Metal backend.
///
/// This is the objc2-metal equivalent of the legacy `metal::Device`, and is the
/// type the window layer creates (via `MTLCreateSystemDefaultDevice` /
/// `MTLCopyAllDevices`) and hands to [`Device::new`].
pub type MetalDevice = Retained<ProtocolObject<dyn MTLDevice>>;

/// Trait to render the [`Scene`] onto the screen using the provided [`WindowState`].
pub trait Renderer {
    fn render(&mut self, scene: &Scene, window: &WindowState, font_cache: &fonts::Cache);

    fn resize(&mut self, window: &WindowState);
}

/// Set of available physical graphics devices that can be used to render.
#[allow(clippy::upper_case_acronyms)]
pub enum Device {
    #[allow(dead_code)]
    Metal(MetalDevice),
    #[cfg(wgpu)]
    WGPU(Box<crate::rendering::wgpu::Resources>),
}
impl Device {
    pub fn new(
        _metal_device: MetalDevice,
        _native_view: &NSView,
        _native_window: &NSWindow,
        _gpu_power_preference: GPUPowerPreference,
        on_gpu_device_info: Box<OnGPUDeviceSelected>,
    ) -> Self {
        #[cfg(not(wgpu))]
        {
            let gpu_device_info = get_gpu_device_info(&_metal_device);
            on_gpu_device_info(gpu_device_info);
            Device::Metal(_metal_device)
        }

        #[cfg(wgpu)]
        {
            Device::new_wgpu(_native_view, _gpu_power_preference, on_gpu_device_info)
                .expect("unable to create wgpu device")
        }
    }
}

#[cfg_attr(wgpu, allow(dead_code))]
fn get_gpu_device_info(device: &ProtocolObject<dyn MTLDevice>) -> GPUDeviceInfo {
    let device_type = if is_integrated_gpu(device) {
        GPUDeviceType::IntegratedGpu
    } else {
        GPUDeviceType::DiscreteGpu
    };
    GPUDeviceInfo {
        device_type,
        device_name: device.name().to_string(),
        // Mimic wgpu by setting the driver name and info to empty strings when
        // rendering on Metal. See https://github.com/gfx-rs/wgpu/blob/8129897ccbff869ef48a3b53a4cdd8a8a21840f9/wgpu-hal/src/metal/mod.rs#L135.
        driver_name: String::new(),
        driver_info: String::new(),
        backend: GPUBackend::Metal,
    }
}
