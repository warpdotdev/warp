//! Module containing the definition of a platform agnostic audible bell.

use anyhow::Result;
use warpui::{Entity, SingletonEntity};

#[cfg_attr(any(target_os = "linux", target_os = "freebsd"), path = "linux.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(target_os = "windows", path = "windows.rs")]
// TODO(WASM): Replace this with a functional implementation for the web.
#[cfg_attr(target_family = "wasm", path = "noop.rs")]
mod imp;

/// A singleton model that provides a way convenient way to make a "beep" when rung (via a call to
/// [`AudibleBell::ring`]).
pub struct AudibleBell(imp::AudibleBell);

impl AudibleBell {
    pub fn new() -> Self {
        Self(imp::AudibleBell::new())
    }

    /// Rings the audible bell. Returns an [`Err`] if the bell was unable to be rung for any reason.
    pub fn ring(&self) -> Result<()> {
        self.0.ring()
    }
}

impl Default for AudibleBell {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for AudibleBell {
    type Event = ();
}

impl SingletonEntity for AudibleBell {}
