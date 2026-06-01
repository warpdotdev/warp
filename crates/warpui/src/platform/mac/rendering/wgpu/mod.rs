mod renderer;
mod renderer_manager;

use std::ptr::NonNull;

use anyhow::Result;
use objc2_app_kit::NSView;
use pathfinder_geometry::vector::vec2f;
pub use renderer_manager::RendererManager;
use wgpu::rwh::{
    AppKitDisplayHandle, AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle,
    HasWindowHandle, RawDisplayHandle, RawWindowHandle, WindowHandle,
};

use crate::platform::mac::rendering::Device;
use crate::rendering::wgpu::Resources;
use crate::rendering::{GPUPowerPreference, OnGPUDeviceSelected};

impl Device {
    /// Constructs a new [`Device`] to render using WGPU.
    pub fn new_wgpu(
        native_view: &NSView,
        gpu_power_preference: GPUPowerPreference,
        on_gpu_device_info: Box<OnGPUDeviceSelected>,
    ) -> Result<Device> {
        let view_frame = native_view.frame();
        let surface_size = vec2f(view_frame.size.width as f32, view_frame.size.height as f32);

        let appkit_window_handle = AppKitWindowHandle::new(NonNull::from(native_view).cast());
        let window_handle =
            unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(appkit_window_handle)) };
        let display_handle = unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::AppKit(AppKitDisplayHandle::new()))
        };

        let trusted_window = TrustedWindow {
            window_handle,
            display_handle,
        };

        crate::rendering::wgpu::init_wgpu_instance(Box::new(trusted_window));

        let resources = Resources::new(
            trusted_window,
            gpu_power_preference,
            None,
            &on_gpu_device_info,
            surface_size,
            false, /* downrank_non_nvidia_vulkan_adapters */
        )?;
        Ok(Device::WGPU(Box::new(resources)))
    }
}

/// Wrapper struct that implements the [`HasRawWindowHandle`] and [`HasRawDisplayHandle`] traits.
/// The raw-window-handle crate purposefully does not provide a blanket implementation of this trait
/// for any implementation of [`RawWindowHandle`] or [`RawDisplayHandle`] because it's not
/// guaranteed that the underlying window won't become invalid while the `WindowHandle` is alive.
/// In the case of Warp this _should_ be safe because we ultimately deallocate the native window
/// when [`crate::platform::mac::Window`] is deallocated (once a `Window` is deallocated, there
/// are no pointers to the native window anymore, which cause it to be deallocated via the
/// `warp_dealloc_window` callback).
/// See <https://github.com/rust-windowing/raw-window-handle/pull/73> for more information on the
/// safety requirements of implementing the [`HasRawWindowHandle`] trait.
#[derive(Copy, Clone, Debug)]
struct TrustedWindow {
    window_handle: WindowHandle<'static>,
    display_handle: DisplayHandle<'static>,
}

// THIS IS INCREDIBLY UNSAFE!!!  DO NOT DO THIS!!!
//
// That said, we're not using this codepath in production, and it unblocks us
// moving to wgpu 0.19 (an important migration for the Linux target), so we're
// doing this and covering our eyes for now, with the intention of fixing it or
// removing support for `wpgu` in our macOS backend.
unsafe impl Send for TrustedWindow {}
unsafe impl Sync for TrustedWindow {}

impl HasWindowHandle for TrustedWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(self.window_handle)
    }
}

impl HasDisplayHandle for TrustedWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(self.display_handle)
    }
}
