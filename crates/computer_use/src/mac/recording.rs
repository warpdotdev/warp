//! macOS screen recording: the `avfoundation` input adapter over the shared recording core.
//!
//! There are two capture paths, selected by [`RecordingConfig::target`]:
//!
//! - `Target::Screen`: avfoundation captures the whole main display.
//! - `Target::Window`: avfoundation still captures the main display, and a fixed `crop` filter
//!   narrows the encoded frame to the target window's rectangle. The window must be
//!   foreground-visible at start, the crop is resolved once and never follows a move, resize,
//!   or a later occluder. Capturing window content independently of the composited display
//!   needs a different substrate (ScreenCaptureKit) rather than another filter.
//!
//! The temp-file lifecycle, encode settings, launch/finalize supervision, and post-stop
//! processing all live in [`crate::recording`].

use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use tokio::process::Command;

use super::util::{main_display_bounds, main_display_dimensions, main_display_scale_factor};
use super::window;
use crate::recording::capture;
use crate::recording::window_crop::{CaptureCrop, PointRect, window_crop_in_capture_space};
use crate::{RecordingConfig, RecordingError, RecordingHandle, RecordingOutput, Target};

/// How often to re-check whether a target window has come to the front.
const RAISE_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long to wait for an activated window to become foreground-visible.
const RAISE_TIMEOUT: Duration = Duration::from_millis(500);
/// Layer of an ordinary application window; menus, panels, and the Dock live elsewhere.
const NORMAL_WINDOW_LAYER: i64 = 0;

/// The avfoundation input spec for the main display, with no audio device.
///
/// The screen is selected by NAME rather than integer index: ffmpeg parses
/// `Capture screen %d` directly, and the name is stable/English where the index
/// shifts when the camera count changes (cameras precede screens in
/// avfoundation's combined index space). `none` disables audio capture. This
/// matches the macOS screenshot path's main-display-only behavior
/// (`screencapture -m`); multi-display support is out of scope.
const AVFOUNDATION_INPUT: &str = "Capture screen 0:none";

pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        let plan = match config.target {
            Target::Screen => screen_capture_plan()?,
            Target::Window { window_id, pid } => prepare_window_capture(window_id, pid).await?,
        };
        let (width, height) = plan.encoded_dimensions();

        let files = capture::new_capture_files()?;
        let command = new_ffmpeg_capture_command(&config, &plan);
        capture::launch_capture(command, files, width, height, Some(diagnose_start_failure)).await
    }

    async fn stop(&self, handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        capture::finalize_capture(handle).await
    }
}

/// What avfoundation is asked to capture, and what is actually encoded from it.
struct CapturePlan {
    /// Dimensions of the avfoundation input, in physical pixels.
    input: (u32, u32),
    /// The fixed rectangle of the input frame that is encoded, for a window recording.
    crop: Option<CaptureCrop>,
}

impl CapturePlan {
    /// The encoded frame dimensions, which window-local pointer coordinates and the overlay
    /// `PlayRes` map onto.
    fn encoded_dimensions(&self) -> (u32, u32) {
        self.crop
            .map_or(self.input, |crop| (crop.width, crop.height))
    }

    fn video_filter(&self) -> Option<String> {
        self.crop.map(|crop| crop.filter_arg())
    }
}

/// Plans a whole-main-display capture.
fn screen_capture_plan() -> Result<CapturePlan, RecordingError> {
    // libx264 with yuv420p requires even dimensions.
    let (width, height) = main_display_dimensions();
    let width = width & !1;
    let height = height & !1;
    if width == 0 || height == 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid display dimensions {width}x{height}"),
        });
    }
    Ok(CapturePlan {
        input: (width, height),
        crop: None,
    })
}

/// Plans a window-scoped capture: resolve the target's rectangle inside the main display's
/// frame, then make sure the window is actually the surface those pixels show.
///
/// Fails rather than recording another surface when the target cannot satisfy that contract.
async fn prepare_window_capture(window_id: u32, pid: i32) -> Result<CapturePlan, RecordingError> {
    if window_id == 0 {
        return Err(RecordingError::Environment {
            reason: "A window recording requires a non-zero window id. Select a window from the \
                     enumerated window list."
                .to_string(),
        });
    }
    let display = screen_capture_plan()?;
    let target = window::description_by_id(window_id).ok_or_else(|| RecordingError::Environment {
        reason: format!(
            "Target window {window_id} is not in the on-screen window list; it may be minimized, \
             closed, or hidden behind a missing Screen Recording permission."
        ),
    })?;
    if target.owner_pid != i64::from(pid) {
        return Err(RecordingError::Environment {
            reason: format!(
                "Target window {window_id} is owned by pid {} rather than the requested pid {pid}.",
                target.owner_pid
            ),
        });
    }
    if target.layer != NORMAL_WINDOW_LAYER {
        return Err(RecordingError::Environment {
            reason: format!(
                "Target window {window_id} is on layer {} rather than a normal application \
                 window layer.",
                target.layer
            ),
        });
    }

    let (origin_x, origin_y, display_width, display_height) = main_display_bounds();
    let contained = target.x >= origin_x
        && target.y >= origin_y
        && target.x + target.width <= origin_x + display_width
        && target.y + target.height <= origin_y + display_height;
    if !contained {
        return Err(RecordingError::Environment {
            reason: format!(
                "Target window {window_id} is not fully contained on the main display, which is \
                 the only display macOS recording captures."
            ),
        });
    }

    let crop = window_crop_in_capture_space(
        PointRect {
            x: target.x,
            y: target.y,
            width: target.width,
            height: target.height,
        },
        (origin_x, origin_y),
        main_display_scale_factor(),
        display.input,
    )
    .map_err(|reason| RecordingError::Environment {
        reason: format!("Target window {window_id} cannot be recorded: {reason}"),
    })?;

    ensure_window_visible_for_recording(&target).await?;

    Ok(CapturePlan {
        input: display.input,
        crop: Some(crop),
    })
}

/// Brings the target to the front if something covers it, since a cropped display capture
/// records whatever the compositor puts in that rectangle.
async fn ensure_window_visible_for_recording(
    target: &window::WindowDescription,
) -> Result<(), RecordingError> {
    let window_id = target.number.max(0) as u32;
    let points = visibility_sample_points(target);
    if window::topmost_at_points(window_id, &points) {
        return Ok(());
    }

    activate_owner(target.owner_pid as libc::pid_t);
    let deadline = Instant::now() + RAISE_TIMEOUT;
    loop {
        if window::topmost_at_points(window_id, &points) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(RecordingError::Start {
                reason: format!(
                    "Target window {window_id} could not be made foreground-visible for \
                     recording."
                ),
            });
        }
        tokio::time::sleep(RAISE_POLL_INTERVAL).await;
    }
}

/// Best-effort activation of the window's owning application. Unlike background computer use,
/// which posts events straight to a process, video capture reads the composited display and so
/// needs the target actually raised.
fn activate_owner(pid: libc::pid_t) {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        // The result only says whether the request was accepted; the visibility poll decides.
        let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    }
}

/// The window's center plus four inset corners, in global display points.
fn visibility_sample_points(target: &window::WindowDescription) -> Vec<(f64, f64)> {
    let mut points = vec![(
        target.x + target.width / 2.0,
        target.y + target.height / 2.0,
    )];

    if target.width > 2.0 && target.height > 2.0 {
        let inset_x = (target.width / 10.0).clamp(1.0, 8.0);
        let inset_y = (target.height / 10.0).clamp(1.0, 8.0);
        let right = target.x + target.width - 1.0;
        let bottom = target.y + target.height - 1.0;
        points.extend([
            (target.x + inset_x, target.y + inset_y),
            (right - inset_x, target.y + inset_y),
            (target.x + inset_x, bottom - inset_y),
            (right - inset_x, bottom - inset_y),
        ]);
    }

    points
}

/// macOS-specific explanation for a capture that never went live.
///
/// A denied Screen Recording grant reaches ffmpeg only as a permission error on the
/// avfoundation input, which on its own tells the user nothing actionable.
fn diagnose_start_failure(log: &str) -> Option<String> {
    log.contains("permitted").then(|| {
        "Screen Recording permission may be denied; grant it in System Settings > Privacy & \
         Security > Screen Recording."
            .to_string()
    })
}

/// Builds the ffmpeg `avfoundation` capture command for `plan`.
///
/// The output path and stdio redirection are added by the shared launcher before spawning, so
/// this builder is unit-testable without opening a display or launching ffmpeg.
fn new_ffmpeg_capture_command(config: &RecordingConfig, plan: &CapturePlan) -> Command {
    let (width, height) = plan.input;
    let mut command = Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-f", "avfoundation"])
        .args(["-framerate", &config.frame_rate.to_string()])
        // Do NOT composite avfoundation's own cursor or click flashes. The post-stop burn-in
        // synthesizes a cursor and click rings from the recorded pointer events, and
        // PID-targeted background actions never move the real cursor, so compositing would
        // add a second, unrelated pointer next to the annotations.
        .args(["-capture_cursor", "0"])
        .args(["-capture_mouse_clicks", "0"])
        .args(["-pixel_format", "uyvy422"])
        .args(["-video_size", &format!("{width}x{height}")]);
    capture::push_duration_limit(&mut command, config.max_duration);
    command.args(["-i", AVFOUNDATION_INPUT]);
    capture::push_encode_args(
        &mut command,
        config.max_size_bytes,
        plan.video_filter().as_deref(),
    );
    command
}

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
