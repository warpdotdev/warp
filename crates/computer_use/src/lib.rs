#[cfg_attr(macos, path = "mac/mod.rs")]
#[cfg_attr(linux, path = "linux/mod.rs")]
#[cfg_attr(windows, path = "windows/mod.rs")]
#[cfg(not(noop))]
mod imp;
mod noop;
#[cfg(any(macos, linux, windows))]
mod screenshot_utils;

use std::borrow::Cow;

use async_trait::async_trait;
// Clippy doesn't like us pulling in a file as two different modules,
// so we add this alias instead of using another cfg_attr on the imp
// module definition.
#[cfg(noop)]
use noop as imp;
pub use pathfinder_geometry::vector::Vector2I;
use serde::{Deserialize, Serialize};
use serde_with::{DurationSecondsWithFrac, serde_as};

/// The platform that computer use is running on.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Platform {
    Mac,
    Windows,
    LinuxX11,
    LinuxWayland,
}

pub fn is_supported_on_current_platform() -> bool {
    if cfg!(feature = "test-util") {
        noop::is_supported_on_current_platform()
    } else {
        imp::is_supported_on_current_platform()
    }
}

/// Returns an actor that can perform actions on the computer.
pub fn create_actor() -> Box<dyn Actor> {
    if cfg!(feature = "test-util") {
        Box::new(noop::Actor::new())
    } else {
        Box::new(imp::Actor::new())
    }
}

#[async_trait]
pub trait Actor: Send + Sync + 'static {
    /// Returns the platform that this actor is running on, if known.
    fn platform(&self) -> Option<Platform>;

    async fn perform_actions(
        &mut self,
        actions: &[Action],
        options: Options,
    ) -> Result<ActionResult, String>;
}

/// A key that can be pressed or released.
#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Key {
    /// A platform-specific keycode. On macOS and Windows, this is a virtual keycode.
    /// On Linux, this is an X11 keysym.
    Keycode(i32),
    /// A character key (e.g., 'a', '+'). On Windows, `Key::Char` only supports characters in
    /// the Basic Multilingual Plane (BMP, `U+0000`–`U+FFFF`). Supplementary-plane characters
    /// (emoji, some CJK extension blocks, etc.) will return an error; use `TypeText` instead for
    /// those.
    Char(char),
}

/// The actions that an actor can perform on the computer.
#[serde_as]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Wait(#[serde_as(as = "DurationSecondsWithFrac<f64>")] std::time::Duration),
    MouseDown {
        button: MouseButton,
        #[serde(with = "Vector2IDef")]
        at: Vector2I,
    },
    MouseUp {
        button: MouseButton,
    },
    MouseMove {
        #[serde(with = "Vector2IDef")]
        to: Vector2I,
    },
    MouseWheel {
        #[serde(with = "Vector2IDef")]
        at: Vector2I,
        direction: ScrollDirection,
        distance: ScrollDistance,
    },
    TypeText {
        text: String,
    },
    KeyDown {
        key: Key,
    },
    KeyUp {
        key: Key,
    },
}

/// The direction of a scroll action.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// The distance of a scroll action.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScrollDistance {
    /// Scroll by a number of pixels.
    Pixels(i32),
    /// Scroll by a number of discrete "clicks" (wheel notches).
    Clicks(i32),
}

/// A rectangular region defined by top-left and bottom-right corners.
/// Coordinates are in physical screen pixels (same coordinate space as mouse actions).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotRegion {
    #[serde(with = "Vector2IDef")]
    pub top_left: Vector2I,
    #[serde(with = "Vector2IDef")]
    pub bottom_right: Vector2I,
}

impl ScreenshotRegion {
    /// Validates that the region has valid coordinates for screenshot capture.
    ///
    /// Returns an error if:
    /// - `top_left` has negative coordinates
    /// - `bottom_right` is not strictly greater than `top_left` in both dimensions
    pub fn validate(&self) -> Result<(), String> {
        if self.top_left.x() < 0 || self.top_left.y() < 0 {
            return Err(format!(
                "Screenshot region top_left must be non-negative, got ({}, {})",
                self.top_left.x(),
                self.top_left.y()
            ));
        }
        if self.bottom_right.x() <= self.top_left.x() {
            return Err(format!(
                "Screenshot region must have positive width (bottom_right.x {} must be > top_left.x {})",
                self.bottom_right.x(),
                self.top_left.x()
            ));
        }
        if self.bottom_right.y() <= self.top_left.y() {
            return Err(format!(
                "Screenshot region must have positive height (bottom_right.y {} must be > top_left.y {})",
                self.bottom_right.y(),
                self.top_left.y()
            ));
        }
        Ok(())
    }
}

/// Parameters for taking a screenshot after actions.
/// If provided, a screenshot will be taken; if `None`, no screenshot is taken.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotParams {
    /// The maximum length of the long edge of the screenshot in pixels.
    pub max_long_edge_px: Option<usize>,
    /// The maximum total number of pixels in the screenshot.
    pub max_total_px: Option<usize>,
    /// Optional region to capture. If `None`, captures the full display.
    #[serde(default)]
    pub region: Option<ScreenshotRegion>,
}

pub struct Options {
    /// If set, a screenshot will be captured after the actions are executed.
    /// The parameters specify what constraints, if any, to apply to the screenshot.
    pub screenshot_params: Option<ScreenshotParams>,
}

/// The buttons of a mouse.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// Mouse button 3 (Back).
    Back,
    /// Mouse button 4 (Forward).
    Forward,
}

/// The result of performing an action.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ActionResult {
    pub screenshot: Option<Screenshot>,
    pub cursor_position: Option<Vector2I>,
}

/// A simple representation of a screenshot.
#[derive(Clone, Eq, PartialEq)]
pub struct Screenshot {
    /// The width of the screenshot image data in pixels.
    pub width: usize,
    /// The height of the screenshot image data in pixels.
    pub height: usize,
    /// The original width of the screenshot before any downscaling was applied.
    pub original_width: usize,
    /// The original height of the screenshot before any downscaling was applied.
    pub original_height: usize,
    // TODO(AGENT-2283): consider making this a type that is cheap to clone
    // (e.g.: `Arc<[u8]>`)
    pub data: Vec<u8>,
    pub mime_type: Cow<'static, str>,
}

impl std::fmt::Debug for Screenshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Screenshot")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("original_width", &self.original_width)
            .field("original_height", &self.original_height)
            .field("num_data_bytes", &self.data.len())
            .finish()
    }
}

/// Remote derive helper for `Vector2I` from `pathfinder_geometry`.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Vector2I")]
struct Vector2IDef {
    #[serde(getter = "get_vector2i_x")]
    x: i32,
    #[serde(getter = "get_vector2i_y")]
    y: i32,
}

fn get_vector2i_x(v: &Vector2I) -> i32 {
    v.x()
}

fn get_vector2i_y(v: &Vector2I) -> i32 {
    v.y()
}

impl From<Vector2IDef> for Vector2I {
    fn from(def: Vector2IDef) -> Self {
        Vector2I::new(def.x, def.y)
    }
}
