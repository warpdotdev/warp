//! Module containing a macOS implementation of an audible bell.

unsafe extern "C" {
    pub fn NSBeep();
}

/// A MacOS implementation of an audible bell.
pub(super) struct AudibleBell;

impl AudibleBell {
    pub fn new() -> Self {
        Self
    }

    pub fn ring(&self) -> anyhow::Result<()> {
        unsafe {
            NSBeep();
        }
        Ok(())
    }
}
