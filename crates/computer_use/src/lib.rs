#[cfg_attr(macos, path = "mac/mod.rs")]
#[cfg_attr(linux, path = "linux/mod.rs")]
#[cfg_attr(windows, path = "windows/mod.rs")]
#[cfg(not(noop))]
mod imp;
mod noop;
#[cfg(any(macos, linux, windows))]
mod screenshot_utils;

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
// Clippy doesn't like us pulling in a file as two different modules,
// so we add this alias instead of using another cfg_attr on the imp
// module definition.
#[cfg(noop)]
use noop as imp;
pub use pathfinder_geometry::vector::Vector2I;
use serde::{Deserialize, Serialize};
use serde_with::{DurationSecondsWithFrac, serde_as};
use thiserror::Error;

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

#[derive(Debug, Error)]
pub enum RecordingError {
    #[error("Video recording is not supported on this platform.")]
    UnsupportedPlatform,
    #[error("Cannot start recording: DISPLAY is not set (X11 required).")]
    MissingDisplay,
    #[error("Failed to connect to X11: {0}")]
    X11Connection(String),
    #[error("Cannot start recording: invalid display dimensions {width}x{height}.")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("Failed to create recording log file: {source}")]
    CreateLogFile {
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to spawn ffmpeg for recording: {source}")]
    SpawnFfmpeg {
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to poll ffmpeg: {source}")]
    PollFfmpeg {
        #[source]
        source: std::io::Error,
    },
    #[error("ffmpeg exited early with status {status}")]
    FfmpegExitedEarly { status: std::process::ExitStatus },
    #[error("timed out waiting for capture to begin")]
    StartTimedOut,
    #[error("ffmpeg failed to start recording: {error}{detail}")]
    StartFailed { error: String, detail: String },
    #[error("Failed to wait for ffmpeg to stop: {source}")]
    WaitFfmpeg {
        #[source]
        source: std::io::Error,
    },
    #[error("ffmpeg did not finalize the recording in time")]
    StopTimedOut,
    #[error("Recording produced an empty file.")]
    EmptyOutput,
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

/// Returns a recorder that can capture a video of the computer-use display.
///
/// A real recorder is only available on Linux (X11); every other platform, and
/// any `test-util` build, gets a no-op recorder that reports recording as
/// unsupported.
pub fn create_recorder() -> Box<dyn Recorder> {
    if cfg!(feature = "test-util") {
        Box::new(noop::Recorder::new())
    } else {
        Box::new(imp::Recorder::new())
    }
}

/// A long-lived capability that records a video of the computer-use display.
///
/// Unlike [`Actor`], a recorder spans many tool calls: `start` launches capture
/// and returns a [`RecordingHandle`] that the caller holds for the duration of
/// the flow, and `stop` consumes that handle to finalize the video.
#[async_trait]
pub trait Recorder: Send + Sync + 'static {
    /// Begins capturing the display. Resolves once capture is confirmed live
    /// (the display is open and the encoder has produced its first output).
    async fn start(&self, config: RecordingConfig) -> Result<RecordingHandle, RecordingError>;

    /// Stops an in-progress recording, finalizes the container, and returns the
    /// resulting file path and metadata. The file is streamed to disk; the
    /// caller owns publishing and cleanup.
    async fn stop(&self, handle: RecordingHandle) -> Result<RecordingOutput, RecordingError>;
}

/// Runtime-owned capture configuration for a recording.
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    /// Capture frame rate in frames per second.
    pub frame_rate: u32,
    /// Maximum duration before the runtime auto-stops recording.
    pub max_duration: Option<Duration>,
    /// Maximum output size before the runtime auto-stops recording.
    pub max_size_bytes: Option<u64>,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            // NOTE: 15fps keeps UI interactions readable while reducing file size and encoder load.
            frame_rate: 15,
            max_duration: None,
            max_size_bytes: None,
        }
    }
}

/// An opaque handle to an in-progress recording, returned by [`Recorder::start`]
/// and consumed by [`Recorder::stop`]. It owns the live capture process and the
/// metadata needed to report the applied capture settings.
pub struct RecordingHandle {
    width: u32,
    height: u32,
    frame_rate: u32,
    max_duration: Option<Duration>,
    max_size_bytes: Option<u64>,
    // The live capture process plus the fields used to finalize it are only
    // populated by the real Linux recorder; the no-op recorders never construct
    // a handle.
    #[cfg(linux)]
    path: PathBuf,
    #[cfg(linux)]
    started_at: std::time::Instant,
    #[cfg(linux)]
    process: tokio::process::Child,
}

impl RecordingHandle {
    /// The applied capture width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The applied capture height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The applied capture frame rate in frames per second.
    pub fn frame_rate(&self) -> u32 {
        self.frame_rate
    }

    /// The enforced maximum recording duration, if any.
    pub fn max_duration(&self) -> Option<Duration> {
        self.max_duration
    }

    /// The enforced maximum recording size in bytes, if any.
    pub fn max_size_bytes(&self) -> Option<u64> {
        self.max_size_bytes
    }
}

/// The finalized output of a stopped recording. Carries the local file path and
/// metadata only; callers are responsible for publishing and deleting the file.
#[derive(Debug, Clone)]
pub struct RecordingOutput {
    pub path: PathBuf,
    pub duration: Duration,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
    pub completion_status: RecordingCompletionStatus,
}

/// Whether capture completed normally or stopped before an explicit stop.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RecordingCompletionStatus {
    Completed,
    StoppedEarly,
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
