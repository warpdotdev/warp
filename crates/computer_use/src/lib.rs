#[cfg_attr(macos, path = "mac/mod.rs")]
#[cfg_attr(linux, path = "linux/mod.rs")]
#[cfg_attr(windows, path = "windows/mod.rs")]
#[cfg(not(noop))]
mod imp;
// Env-var-gated mock recorder for exercising the recording UI on macOS,
// where real capture is unsupported.
#[cfg(macos)]
mod mock;
mod noop;
mod overlay;
#[cfg(any(macos, linux, windows))]
mod screenshot_utils;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
// Clippy doesn't like us pulling in a file as two different modules,
// so we add this alias instead of using another cfg_attr on the imp
// module definition.
#[cfg(noop)]
use noop as imp;
pub use overlay::{ActionLogEntry, overlay_labels_for};
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
/// Why a capture process exited before an explicit stop.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RecordingExitKind {
    LimitReached,
    Crashed,
}

pub type RecordingExitState = Arc<Mutex<Option<RecordingExitKind>>>;

#[derive(Debug, Error)]
pub enum RecordingError {
    /// Recording can't run in this environment: unsupported platform, missing or
    /// unreachable X11 display, unusable display dimensions, or ffmpeg not launchable.
    #[error("Recording environment error: {reason}")]
    Environment { reason: String },
    /// ffmpeg was launched but capture never went live.
    #[error("Recording failed to start: {reason}")]
    Start { reason: String },
    /// A live recording couldn't be finalized into a usable file.
    #[error("Recording failed to finalize: {reason}")]
    Finalize { reason: String },
}

/// Returns an actor that can perform actions on the computer.
pub fn create_actor() -> Box<dyn Actor> {
    if cfg!(feature = "test-util") {
        Box::new(noop::Actor::new())
    } else {
        Box::new(imp::Actor::new())
    }
}

/// Returns whether background, per-window control (driving a specific window without raising it
/// or moving the cursor) is available on this client and OS. When false, callers should target
/// the whole screen / frontmost application.
///
/// How faithfully "background" holds varies by platform:
///
/// - macOS: events are posted directly to the owning process (`CGEventPostToPid`), so a window
///   can be driven even while fully covered, and nothing user-visible changes.
/// - Linux X11: events come from a dedicated second input seat (an XInput2/MPX master pair), so
///   the user's cursor, keyboard focus, and modifier state are untouched and applications see
///   real (non-synthetic) input. The trade-offs, inherent to X11's position-routed event
///   delivery, are:
///   - Pointer actions land on the topmost window at the target point. If the target window is
///     covered there, the actor first raises it *without* taking the user's focus; the action
///     fails if the raise does not take effect. Keyboard input needs no raise: it follows the
///     agent seat's own focus even while the window is covered.
///   - A second visible cursor appears on screen while window-targeted actions run.
///   - Under a click-to-focus window manager, the WM itself may react to an agent click by
///     focusing/raising the target for the user too. WM-less servers (e.g. Xvfb in cloud
///     environments) have no such side effect.
/// - Linux Wayland and Windows: unsupported (this returns false); only whole-screen control is
///   available.
pub fn background_supported() -> bool {
    if cfg!(feature = "test-util") {
        noop::background_supported()
    } else {
        imp::background_supported()
    }
}

/// Enumerates the on-screen windows, returning their metadata so a caller can pick one to
/// target. Returns an empty list on platforms where window enumeration is unsupported.
pub fn enumerate_windows() -> Vec<WindowInfo> {
    #[cfg(any(macos, linux))]
    {
        imp::enumerate_windows()
    }
    #[cfg(not(any(macos, linux)))]
    {
        Vec::new()
    }
}

/// Experimental: lists on-screen windows as a formatted diagnostic string. macOS and Linux
/// (X11) only.
///
/// Unlike [`enumerate_windows`], which returns slim [`WindowInfo`] records for window selection
/// and wire serialization, this function returns richer data including window bounds, formatted
/// as a human-readable table for CLI debugging. The two use separate types intentionally:
/// [`WindowInfo`] is kept wire-safe and bounds-free; the diagnostic output carries bounds that
/// are not part of the API representation.
#[cfg(macos)]
pub fn experimental_list_windows() -> Result<String, String> {
    Ok(imp::list_windows())
}

/// Experimental: lists on-screen windows as a formatted diagnostic string. macOS and Linux
/// (X11) only.
#[cfg(linux)]
pub fn experimental_list_windows() -> Result<String, String> {
    imp::list_windows()
}

/// Experimental: lists on-screen windows. Unsupported on this platform.
#[cfg(not(any(macos, linux)))]
pub fn experimental_list_windows() -> Result<String, String> {
    Err("Window listing is only supported on macOS and Linux (X11).".to_string())
}

/// The surface that a computer-use action or screenshot targets.
///
/// `Screen` reproduces the legacy behavior of acting on the whole screen / frontmost
/// application. `Window` drives a specific background window of a specific process without
/// moving the global cursor or taking the user's keyboard focus. On macOS the window is never
/// raised; on Linux X11 pointer events are routed by screen position, so a window that is
/// covered at the action point is raised (without focus) before clicks and scrolls — see
/// [`background_supported`] for the full per-platform semantics.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Target {
    /// Target the whole screen / frontmost application (legacy behavior).
    #[default]
    Screen,
    /// Target a specific background window of a specific process.
    Window {
        /// The platform window id (a `CGWindowID` on macOS, an X window id on Linux X11). Must
        /// be a concrete, non-zero id selected from the enumerated window list. `0` is the
        /// "unknown" sentinel and is rejected by the actor, since coordinate remapping and
        /// window capture both require a known window.
        window_id: u32,
        /// The pid of the process that owns the window. Used for event delivery on macOS;
        /// informational on Linux X11, where events are addressed by window id.
        pid: i32,
    },
}

/// An action paired with the surface it targets.
///
/// The target is carried per-action so a single batch can, in principle, drive more than one
/// window. An absent / `Screen` target reproduces the legacy whole-screen behavior.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetedAction {
    pub action: Action,
    #[serde(default)]
    pub target: Target,
}

impl TargetedAction {
    /// Builds a screen-targeted action (legacy behavior).
    pub fn screen(action: Action) -> Self {
        Self {
            action,
            target: Target::Screen,
        }
    }
}

/// Metadata about an on-screen window, so a caller can select a window to target.
/// Mirrors the fields of the `WindowInfo` API message.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    /// The platform window id (a `CGWindowID` on macOS, an X window id on Linux X11).
    pub window_id: u32,
    /// The pid of the process that owns the window.
    pub pid: i32,
    /// The owning application's name (e.g. "Arc", "Notes").
    pub app_name: String,
    /// The window title, if available.
    pub title: String,
    /// The window layer (0 is a normal application window).
    pub layer: i32,
}
/// Metadata describing a captured window screenshot.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct CapturedWindow {
    /// The platform window id that was captured.
    pub window_id: u32,
    /// The width of the native captured image, in pixels.
    pub width_px: i32,
    /// The height of the native captured image, in pixels.
    pub height_px: i32,
}

#[async_trait]
pub trait Actor: Send + Sync + 'static {
    /// Returns the platform that this actor is running on, if known.
    fn platform(&self) -> Option<Platform>;

    async fn perform_actions(
        &mut self,
        actions: &[TargetedAction],
        options: Options,
    ) -> Result<ActionResult, String>;
}

/// Returns a recorder that can capture a video of the computer-use display.
///
/// A real recorder is only available on Linux (X11); every other platform, and
/// any `test-util` build, gets a no-op recorder that reports recording as
/// unsupported. On macOS, setting `WARP_MOCK_RECORDER` opts into a mock
/// recorder for UI testing (see `mock`).
pub fn create_recorder() -> Box<dyn Recorder> {
    #[cfg(macos)]
    if std::env::var_os("WARP_MOCK_RECORDER").is_some() {
        return Box::new(mock::Recorder::new());
    }
    if cfg!(feature = "test-util") {
        Box::new(noop::Recorder::new())
    } else {
        Box::new(imp::Recorder::new())
    }
}

/// Burns action labels into a recorded video, returning the path to the
/// annotated file. The original file is left untouched; the caller owns cleanup
/// of both. Real compositing (ffmpeg + libass) only runs on the Linux capture
/// path; every other target returns `input` unchanged so callers can treat
/// annotation as best-effort and upload the original on any failure.
pub async fn burn_in_action_log(
    input: &Path,
    entries: &[ActionLogEntry],
    dimensions: (u32, u32),
) -> Result<PathBuf, RecordingError> {
    #[cfg(all(linux, not(noop)))]
    {
        imp::burn_in_action_log(input, entries, dimensions).await
    }
    #[cfg(not(all(linux, not(noop))))]
    {
        let _ = (entries, dimensions);
        Ok(input.to_path_buf())
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
    pub max_duration: Duration,
    /// Maximum output size in bytes before the runtime auto-stops recording.
    pub max_size_bytes: u64,
    /// How many times faster the output video should play back relative to real
    /// time. For example, 4.0 makes a 4-minute recording play in 1 minute. A
    /// value of 0.0 or 1.0 means real-time (no speedup). Applied via an ffmpeg
    /// presentation-timestamp rescale filter on the output video.
    pub playback_speed_multiplier: f32,
    /// The surface to capture. `Screen` records the whole X display (legacy behavior);
    /// `Window` records the targeted window after making it foreground-visible when supported.
    pub target: Target,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            // NOTE: 15fps keeps UI interactions readable while reducing file size and encoder load.
            frame_rate: 15,
            // NOTE: Bounds every capture so an unattended recording can't grow without bound (~10 min / 1 GiB).
            max_duration: Duration::from_secs(10 * 60),
            max_size_bytes: 1024 * 1024 * 1024,
            // NOTE: 4x playback speed keeps demo videos short and watchable. A 4-minute
            // recording plays in 1 minute. The server can override via the StartRecording
            // tool call's playback_speed_multiplier field.
            playback_speed_multiplier: 4.0,
            target: Target::Screen,
        }
    }
}

/// An opaque handle to an in-progress recording, returned by [`Recorder::start`]
/// and consumed by [`Recorder::stop`]. It owns the live capture process and the
/// metadata needed to report the applied capture settings.
pub struct RecordingHandle {
    width: u32,
    height: u32,
    exit_state: RecordingExitState,
    // The live capture process plus the fields used to finalize it are only
    // populated by the real Linux recorder; the no-op recorders never construct
    // a handle.
    #[cfg(linux)]
    path: PathBuf,
    #[cfg(linux)]
    started_at: instant::Instant,
    #[cfg(linux)]
    process: Option<tokio::process::Child>,
    // The handle owns and deletes partial output until `Recorder::stop`
    // validates the file and transfers its path to `RecordingOutput`.
    #[cfg(linux)]
    cleanup_on_drop: bool,
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

    /// Checks whether capture exited without an explicit stop.
    pub fn poll_exit(&mut self) -> Option<RecordingExitKind> {
        if let Some(kind) = *self
            .exit_state
            .lock()
            .expect("recording exit state poisoned")
        {
            return Some(kind);
        }

        #[cfg(linux)]
        if let Some(process) = self.process.as_mut()
            && let Ok(Some(status)) = process.try_wait()
        {
            let kind = if status.success() {
                RecordingExitKind::LimitReached
            } else {
                RecordingExitKind::Crashed
            };
            *self
                .exit_state
                .lock()
                .expect("recording exit state poisoned") = Some(kind);
            return Some(kind);
        }

        None
    }

    #[cfg(feature = "test-util")]
    pub fn new_test(width: u32, height: u32) -> (Self, RecordingExitState) {
        let exit_state = Arc::new(Mutex::new(None));
        let handle = Self {
            width,
            height,
            exit_state: exit_state.clone(),
            #[cfg(linux)]
            path: PathBuf::new(),
            #[cfg(linux)]
            started_at: instant::Instant::now(),
            #[cfg(linux)]
            process: None,
            #[cfg(linux)]
            cleanup_on_drop: false,
        };
        (handle, exit_state)
    }
}

#[cfg(linux)]
impl Drop for RecordingHandle {
    fn drop(&mut self) {
        // A handle can be abandoned without reaching `Recorder::stop`, notably
        // when a start action finishes after cancellation. The child process's
        // kill-on-drop handles ffmpeg; this removes its partial output. A
        // successful stop disables cleanup and transfers file ownership.
        if self.cleanup_on_drop {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(self.path.with_extension("log"));
        }
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

#[cfg(test)]
#[path = "recording_tests.rs"]
mod recording_tests;

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

impl Action {
    /// Whether this action is a no-op placeholder rather than a real
    /// interaction. Agents that only want the post-actions screenshot emit a
    /// zero-duration wait, since `use_computer` requires at least one action.
    /// This cannot catch every semantically inert batch (e.g. a mouse move to
    /// the current position), but the zero-wait idiom is the documented
    /// screenshot pattern.
    pub fn is_no_op(&self) -> bool {
        matches!(self, Action::Wait(duration) if duration.is_zero())
    }
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
/// Coordinates are physical pixels relative to the selected screenshot target.
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
    /// Optional sub-region of `target` to capture, in target-relative physical pixels.
    /// If `None`, captures the full target.
    #[serde(default)]
    pub region: Option<ScreenshotRegion>,
    /// The surface to capture. `Screen` captures the main display (legacy); `Window` captures
    /// a specific window's image.
    #[serde(default)]
    pub target: Target,
}

pub struct Options {
    /// If set, a screenshot will be captured after the actions are executed.
    /// The parameters specify what constraints, if any, to apply to the screenshot.
    pub screenshot_params: Option<ScreenshotParams>,
    /// Whether background, per-window computer use is enabled. When false, actors must behave
    /// exactly like the legacy full-screen path: any window target is ignored, only the main
    /// display is captured, and no window list or captured-window metadata is returned.
    pub background_enabled: bool,
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
    /// The on-screen windows, refreshed after the actions run, so the caller always has a fresh
    /// list to target next. Empty on platforms without window enumeration.
    pub windows: Vec<WindowInfo>,
    /// Metadata about the captured window, populated only when a window target was
    /// screenshotted, so window-local coordinates map onto the screenshot image.
    pub captured_window: Option<CapturedWindow>,
}

impl ActionResult {
    /// Builds a result that carries no window list or captured-window metadata (used by
    /// platforms and code paths that do not support per-window targeting).
    pub fn legacy(screenshot: Option<Screenshot>, cursor_position: Option<Vector2I>) -> Self {
        Self {
            screenshot,
            cursor_position,
            windows: Vec::new(),
            captured_window: None,
        }
    }
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
