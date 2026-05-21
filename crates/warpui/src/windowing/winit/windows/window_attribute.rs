use std::ffi::c_void;
use std::mem::size_of;

use thiserror::Error;
use wgpu::rwh;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{self, DWMWINDOWATTRIBUTE};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window as WinitWindow;

#[derive(Debug, Error)]
pub enum WindowAttributeErr {
    #[error(transparent)]
    HandleError(#[from] rwh::HandleError),
    #[error(transparent)]
    Win32Error(#[from] windows::core::Error),
}

/// Uses the `windows` crate to fetch a specific window attribute.
/// First, we translate the Winit window object to a native Windows HWND handle.
/// Then, we invoke the Device Window Manager (DWM)'s `DwmGetWindowAttribute`
/// function for the attribute in question.
pub fn get_window_attribute<T>(
    window: &WinitWindow,
    attribute_name: DWMWINDOWATTRIBUTE,
) -> Result<T, WindowAttributeErr>
where
    T: Default,
{
    let hwnd_handle = to_hwnd(window)?;
    let mut result_destination: T = T::default();
    let window_attribute_result = unsafe {
        let result_address = core::ptr::addr_of_mut!(result_destination);
        Dwm::DwmGetWindowAttribute(
            hwnd_handle,
            attribute_name,
            result_address as *mut c_void,
            size_of::<T>().try_into().unwrap(),
        )
    };
    Ok(window_attribute_result.map(|_| result_destination)?)
}

/// Uses the `windows` crate to set a specific window attribute.
/// First, we translate the Winit window object to a native Windows HWND handle.
/// Then, we invoke the Device Window Manager (DWM)'s `DwmSetWindowAttribute`
/// function for the attribute in question.
pub fn set_window_attribute<T>(
    window: &WinitWindow,
    attribute_name: DWMWINDOWATTRIBUTE,
    value: T,
) -> Result<(), WindowAttributeErr> {
    let hwnd_handle = to_hwnd(window)?;
    let window_attribute_result = unsafe {
        Dwm::DwmSetWindowAttribute(
            hwnd_handle,
            attribute_name,
            core::ptr::addr_of!(value) as *const c_void,
            size_of::<T>().try_into().unwrap(),
        )
    };
    Ok(window_attribute_result?)
}

fn to_hwnd(window: &WinitWindow) -> Result<HWND, rwh::HandleError> {
    window
        .window_handle()
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Ok(handle),
            _ => Err(rwh::HandleError::NotSupported),
        })
        .map(|handle| HWND(handle.hwnd.get() as *mut c_void))
}
