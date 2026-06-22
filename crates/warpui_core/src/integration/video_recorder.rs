use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use image::ImageEncoder;
#[cfg(feature = "integration_tests")]
use instant::Instant;

use crate::platform::CapturedFrame;

/// Well-known key used to store the `VideoRecorder` inside `StepDataMap`.
pub const VIDEO_RECORDER_KEY: &str = "video_recorder";

/// Well-known key prefix for screenshot path requests stored in `StepDataMap`.
pub const SCREENSHOT_PATH_KEY: &str = "pending_screenshot_path";

/// Environment variable that enables automatic video recording for all
/// integration test steps. When set, the driver starts recording at the
/// beginning of the test and writes the video on completion.
pub const VIDEO_ENABLED_ENV_VAR: &str = "WARP_INTEGRATION_TEST_VIDEO";

/// Environment variable that sets the output directory for video recordings
/// and screenshots. Defaults to `$TMPDIR/warp_integration_video_captures` when
/// unset.
pub const VIDEO_DIR_ENV_VAR: &str = "WARP_INTEGRATION_TEST_VIDEO_DIR";

/// A captured frame paired with the wall-clock time it was taken.
pub(super) struct TimestampedFrame {
    pub(super) frame: CapturedFrame,
    #[cfg(feature = "integration_tests")]
    pub(super) captured_at: Instant,
}

/// Shared state passed to the capture loop task so it can push frames and
/// check whether recording/stopping is requested. All fields use
/// atomics/mutex so they are `Send`.
#[cfg(feature = "integration_tests")]
pub struct CaptureLoopState {
    pub(super) recording: Arc<AtomicBool>,
    pub(super) stopped: Arc<AtomicBool>,
    pub(super) frames: Arc<Mutex<Vec<TimestampedFrame>>>,
}

/// Records captured frames during integration tests and can produce
/// individual PNGs or an encoded video file.
pub struct VideoRecorder {
    /// Whether frames should currently be pushed (shared with the capture loop).
    recording: Arc<AtomicBool>,
    /// Set to `true` to tell the capture loop to exit.
    stopped: Arc<AtomicBool>,
    /// Accumulated frames (shared with the capture loop callback).
    frames: Arc<Mutex<Vec<TimestampedFrame>>>,
    /// Wall-clock time when `start_recording` was called.
    #[cfg(feature = "integration_tests")]
    recording_start: Option<Instant>,
}

impl Default for VideoRecorder {
    fn default() -> Self {
        Self {
            recording: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            frames: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "integration_tests")]
            recording_start: None,
        }
    }
}

impl VideoRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_recording(&mut self) {
        #[cfg(feature = "integration_tests")]
        {
            self.recording_start = Some(Instant::now());
        }
        self.recording.store(true, Ordering::Relaxed);
    }

    pub fn stop_recording(&mut self) {
        self.recording.store(false, Ordering::Relaxed);
    }

    /// Signals the capture loop to exit on its next iteration.
    pub fn stop_capture_loop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Returns the instant recording started, if recording has been started.
    #[cfg(feature = "integration_tests")]
    pub fn recording_start(&self) -> Option<Instant> {
        self.recording_start
    }

    /// Returns a `CaptureLoopState` that the capture loop task can use to
    /// push frames and read the recording/stopped flags.
    #[cfg(feature = "integration_tests")]
    pub fn capture_loop_state(&self) -> CaptureLoopState {
        CaptureLoopState {
            recording: self.recording.clone(),
            stopped: self.stopped.clone(),
            frames: self.frames.clone(),
        }
    }

    /// Returns the number of frames captured so far.
    pub fn frame_count(&self) -> usize {
        self.frames.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// Encodes all captured frames into a video at `output_path`.
    /// Falls back to saving individual PNGs if encoding fails.
    pub fn finalize(
        &mut self,
        output_path: &Path,
        overlay_log: Option<&super::overlay::OverlayLog>,
    ) -> anyhow::Result<()> {
        let frames: Vec<TimestampedFrame> = self
            .frames
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();

        if frames.is_empty() {
            log::info!("VideoRecorder: no frames captured, nothing to finalize");
            return Ok(());
        }

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        #[cfg(feature = "integration_tests")]
        match encode_to_mp4(output_path, &frames, overlay_log) {
            Ok(()) => {
                log::info!(
                    "VideoRecorder: wrote {} frames to {}",
                    frames.len(),
                    output_path.display()
                );
            }
            Err(e) => {
                log::warn!("VideoRecorder: MP4 encoding failed ({e}), falling back to PNGs");
                save_frames_as_pngs(output_path, &frames)?;
            }
        }

        #[cfg(not(feature = "integration_tests"))]
        {
            let _ = overlay_log;
            save_frames_as_pngs(output_path, &frames)?;
        }

        Ok(())
    }
}

/// Encodes a slice of timestamped frames into an H.264/MP4 file.
/// Runs entirely on the calling thread at test finalization time.
#[cfg(feature = "integration_tests")]
fn encode_to_mp4(
    output_path: &Path,
    frames: &[TimestampedFrame],
    overlay_log: Option<&super::overlay::OverlayLog>,
) -> anyhow::Result<()> {
    use std::io::Cursor;

    use minimp4::Mp4Muxer;
    use openh264::encoder::Encoder;
    use openh264::formats::{RgbSliceU8, YUVBuffer};

    use super::overlay::OverlayState;

    const TARGET_FPS: u32 = 60;
    const FRAME_DURATION_MS: u128 = 1000 / TARGET_FPS as u128;

    let width = frames[0].frame.width;
    let height = frames[0].frame.height;

    let mut encoder = Encoder::new().map_err(|e| anyhow::anyhow!("openh264 init: {e}"))?;

    let mut overlay_state = OverlayState::new();
    let overlay_events = overlay_log.map(|ol| ol.events()).unwrap_or(&[]);
    let overlay_scale = overlay_log.map(|ol| ol.scale_factor()).unwrap_or(2.0);
    let has_overlays = !overlay_events.is_empty();

    let mut h264_buf = Vec::new();
    let mut total_encoded_frames = 0u32;

    for i in 0..frames.len() {
        let ts_frame = &frames[i];

        let rgb_data = if has_overlays {
            overlay_state.advance_to(ts_frame.captured_at, overlay_events);
            let mut composited = ts_frame.frame.data.clone();
            overlay_state.render_onto(
                &mut composited,
                width,
                height,
                ts_frame.captured_at,
                overlay_scale,
            );
            rgba_to_rgb(&composited)
        } else {
            rgba_to_rgb(&ts_frame.frame.data)
        };

        let rgb_source = RgbSliceU8::new(&rgb_data, (width as usize, height as usize));
        let yuv = YUVBuffer::from_rgb_source(rgb_source);

        let repeat_count = if i + 1 < frames.len() {
            let gap_ms = frames[i + 1]
                .captured_at
                .duration_since(ts_frame.captured_at)
                .as_millis();
            (gap_ms / FRAME_DURATION_MS).max(1) as u32
        } else {
            1
        };

        for _ in 0..repeat_count {
            let bitstream = encoder
                .encode(&yuv)
                .map_err(|e| anyhow::anyhow!("openh264 encode: {e}"))?;
            bitstream.write_vec(&mut h264_buf);
            total_encoded_frames += 1;
        }
    }

    log::info!(
        "VideoRecorder: encoded {total_encoded_frames} video frames from {} captured frames",
        frames.len()
    );

    let mut mp4_buf = Cursor::new(Vec::new());
    let mut muxer = Mp4Muxer::new(&mut mp4_buf);
    muxer.init_video(
        width as i32,
        height as i32,
        false,
        "integration test recording",
    );
    muxer.write_video_with_fps(&h264_buf, TARGET_FPS);
    muxer.close();

    std::fs::write(output_path, mp4_buf.into_inner())?;
    Ok(())
}

/// Saves each frame as a PNG into a subdirectory next to `output_path`.
/// Heavy PNG encoding is offloaded to a Tokio blocking thread.
fn save_frames_as_pngs(output_path: &Path, frames: &[TimestampedFrame]) -> anyhow::Result<()> {
    let stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("frame");
    let dir = output_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}_frames"));
    std::fs::create_dir_all(&dir)?;

    for (i, ts_frame) in frames.iter().enumerate() {
        let path = dir.join(format!("{stem}_{i:04}.png"));
        save_captured_frame_as_png(&ts_frame.frame, &path)?;
    }

    log::info!(
        "VideoRecorder: saved {} PNGs to {}",
        frames.len(),
        dir.display()
    );
    Ok(())
}

/// Background task that drives continuous frame capture at ~60 FPS.
///
/// This future is `!Send` (it holds a clone of `App` which is `Rc<RefCell<...>>`) and
/// must be spawned on the foreground (main-thread) executor. It interleaves with the
/// step execution loop at every `Timer` yield point.
///
/// When `state.recording` is `false` the loop sleeps without requesting any captures,
/// so there is zero rendering overhead when recording is not active. When
/// `state.stopped` is set the loop exits cleanly.
#[cfg(feature = "integration_tests")]
pub async fn run_capture_loop(app: crate::App, state: CaptureLoopState) {
    use std::time::Duration;

    use crate::r#async::Timer;

    loop {
        Timer::after(Duration::from_millis(16)).await;

        if state.stopped.load(Ordering::Relaxed) {
            break;
        }

        if !state.recording.load(Ordering::Relaxed) {
            continue;
        }

        let window = app.read(|ctx| {
            let windowing_state = ctx.windows();
            windowing_state
                .active_window()
                .and_then(|id| windowing_state.platform_window(id))
        });

        let Some(window) = window else {
            continue;
        };

        let frames = state.frames.clone();
        window
            .as_ctx()
            .request_frame_capture(Box::new(move |frame| {
                let captured_at = Instant::now();
                if let Ok(mut guard) = frames.lock() {
                    guard.push(TimestampedFrame { frame, captured_at });
                }
            }));
        window.as_ctx().request_redraw();
    }
}

#[cfg(feature = "integration_tests")]
fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    let pixel_count = rgba.len() / 4;
    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for chunk in rgba.chunks_exact(4) {
        rgb.push(chunk[0]);
        rgb.push(chunk[1]);
        rgb.push(chunk[2]);
    }
    rgb
}

/// Saves a single `CapturedFrame` to a PNG file at the given path.
pub fn save_captured_frame_as_png(frame: &CapturedFrame, path: &Path) -> anyhow::Result<()> {
    let mut frame = frame.clone();
    frame.ensure_rgba();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);

    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut writer,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );

    encoder.write_image(
        &frame.data,
        frame.width,
        frame.height,
        image::ColorType::Rgba8.into(),
    )?;

    Ok(())
}

/// Returns the directory where video recordings and screenshots should be
/// written, creating it if necessary.
pub fn output_dir() -> PathBuf {
    let dir = std::env::var(VIDEO_DIR_ENV_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("warp_integration_video_captures"));
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Helper to retrieve a mutable reference to the recorder from a `StepDataMap`.
pub fn get_recorder_mut(
    step_data_map: &mut super::step::StepDataMap,
) -> Option<&mut VideoRecorder> {
    step_data_map.get_mut::<_, VideoRecorder>(VIDEO_RECORDER_KEY)
}

/// Helper to retrieve a shared reference to the recorder from a `StepDataMap`.
pub fn get_recorder(step_data_map: &super::step::StepDataMap) -> Option<&VideoRecorder> {
    step_data_map.get::<_, VideoRecorder>(VIDEO_RECORDER_KEY)
}
