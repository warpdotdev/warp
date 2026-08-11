use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Dwm;
use winit::window::Window as WinitWindow;

use super::WindowAttributeErr;
use super::window_attribute::get_window_attribute;

#[derive(Debug)]
pub struct SystemCaptionButtonData {
    bounds: RECT,
}

#[derive(Debug)]
pub enum SystemCaptionButtonSide {
    Left,
    Right,
}

impl SystemCaptionButtonData {
    pub fn total_width(&self) -> i32 {
        self.bounds.right - self.bounds.left
    }

    pub fn side(&self) -> SystemCaptionButtonSide {
        if self.bounds.left == 0 {
            SystemCaptionButtonSide::Left
        } else {
            SystemCaptionButtonSide::Right
        }
    }
}

/// Retrieves the system caption button's bounds using the window's
/// CAPTION_BUTTON_BOUNDS attribute.
pub fn get_system_caption_button_bounds(
    window: &WinitWindow,
) -> Result<SystemCaptionButtonData, WindowAttributeErr> {
    let caption_button_bounds = get_window_attribute(window, Dwm::DWMWA_CAPTION_BUTTON_BOUNDS)?;

    Ok(SystemCaptionButtonData {
        bounds: caption_button_bounds,
    })
}
