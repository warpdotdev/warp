//! Module containing a Windows implementation of an audible bell.

use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::UI::WindowsAndMessaging::MB_OK;
pub(super) struct AudibleBell;

impl AudibleBell {
    pub fn new() -> Self {
        Self
    }

    pub fn ring(&self) -> anyhow::Result<()> {
        Ok(unsafe { MessageBeep(MB_OK) }?)
    }
}
