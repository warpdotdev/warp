//! Module containing the definition of the current windowing [`System`] the application is
//! rendering to.

use std::fmt::{Display, Formatter};

use raw_window_handle::RawDisplayHandle;

/// The windowing system that is being used.
#[derive(Copy, Clone, Debug)]
pub enum System {
    X11 { is_x_wayland: bool },
    Wayland,
    AppKit,
    Windows,
}

impl System {
    /// Whether this window server protocol allows windows to programmatically "activate" or
    /// show/focus themselves.
    pub fn allows_programmatic_window_activation(&self) -> bool {
        match self {
            Self::AppKit | Self::X11 { .. } | Self::Windows => true,
            Self::Wayland => false,
        }
    }
}

impl Display for System {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            System::X11 { is_x_wayland } => {
                if *is_x_wayland {
                    write!(f, "Xwayland")
                } else {
                    write!(f, "X11")
                }
            }
            System::Wayland => write!(f, "Wayland"),
            System::AppKit => write!(f, "AppKit"),
            System::Windows => write!(f, "Windows"),
        }
    }
}

impl TryFrom<RawDisplayHandle> for System {
    type Error = CreateWindowingSystemError;

    fn try_from(raw_display_handle: RawDisplayHandle) -> Result<Self, Self::Error> {
        let display = match raw_display_handle {
            RawDisplayHandle::AppKit(_) => System::AppKit,
            RawDisplayHandle::Wayland(_) => System::Wayland,
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => System::X11 {
                is_x_wayland: std::env::var("WAYLAND_DISPLAY")
                    .ok()
                    .filter(|val| !val.is_empty())
                    .is_some(),
            },
            RawDisplayHandle::Windows(_) => System::Windows,
            _ => {
                return Err(Self::Error::UnrecognizedDisplayHandle);
            }
        };

        Ok(display)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum CreateWindowingSystemError {
    #[error("Unrecognized DisplayHandle")]
    UnrecognizedDisplayHandle,
}
