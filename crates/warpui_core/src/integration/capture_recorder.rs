use std::path::Path;
use std::sync::{Arc, Mutex};

use image::ImageEncoder;
use instant::Instant;

use crate::platform::CapturedFrame;

/// The lifecycle state of the capture recorder / capture loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecorderState {
    Idle,
    Recording,
    Stopping,
}

/// Well-known key used to store the `CaptureRecorder` inside `StepDataMap`.
pub const CAPTURE_RECORDER_KEY: &str = "capture_recorder";

/// Well-known key prefix for screenshot path requests stored in `StepDataMap`.
pub const SCREENSHOT_PATH_KEY: &str = "pending_screenshot_path";

/// Environment variable that enables automatic video recording for all
/// integration test steps. When set, the driver starts recording at the
/// beginning of the test and writes the video on completion.
pub const CAPTURE_RECORDING_ENABLED_ENV_VAR: &str = "WARP_INTEGRATION_TEST_VIDEO";

/// A captured frame paired with the wall-clock time it was taken.
struct TimestampedFrame {
    frame: CapturedFrame,
    #[allow(dead_code)]
    captured_at: Instant,
}

/// Mutable state shared between `CaptureRecorder` and `CaptureLoopState`.
/// All access is serialised through a single `Mutex`.
#[allow(dead_code)]
struct SharedState {
    recorder_state: RecorderState,
    raw_frames: Vec<TimestampedFrame>,
    h264_buf: Vec<u8>,
    encoded_frame_count: u32,
    dimensions: Option<(u32, u32)>,
    encoding_in_progress: bool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            recorder_state: RecorderState::Idle,
            raw_frames: Vec::new(),
            h264_buf: Vec::new(),
            encoded_frame_count: 0,
            dimensions: None,
            encoding_in_progress: false,
        }
    }
}

/// Shared handle passed to the capture loop task.
#[allow(dead_code)]
pub struct CaptureLoopState(Arc<Mutex<SharedState>>);

/// Records captured frames during integration tests and can produce
/// individual PNGs or an encoded capture artifact.
pub struct CaptureRecorder {
    inner: Arc<Mutex<SharedState>>,
    recording_start: Option<Instant>,
}

impl Default for CaptureRecorder {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SharedState::default())),
            recording_start: None,
        }
    }
}

impl CaptureRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_recording(&mut self) {
        self.recording_start = Some(Instant::now());
        if let Ok(mut s) = self.inner.lock() {
            s.recorder_state = RecorderState::Recording;
        }
    }

    pub fn stop_recording(&mut self) {
        if let Ok(mut s) = self.inner.lock() {
            if s.recorder_state == RecorderState::Recording {
                s.recorder_state = RecorderState::Idle;
            }
        }
    }

    /// Signals the capture loop to exit on its next iteration.
    pub fn stop_capture_loop(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.recorder_state = RecorderState::Stopping;
        }
    }

    pub fn is_recording(&self) -> bool {
        self.inner
            .lock()
            .map(|s| s.recorder_state == RecorderState::Recording)
            .unwrap_or(false)
    }

    pub fn recording_start(&self) -> Option<Instant> {
        self.recording_start
    }

    pub fn capture_loop_state(&self) -> CaptureLoopState {
        CaptureLoopState(self.inner.clone())
    }

    pub fn raw_frame_count(&self) -> usize {
        self.inner.lock().map(|s| s.raw_frames.len()).unwrap_or(0)
    }

    pub fn is_encoding(&self) -> bool {
        self.inner
            .lock()
            .map(|s| s.encoding_in_progress)
            .unwrap_or(false)
    }

    pub fn frame_count(&self) -> usize {
        self.inner
            .lock()
            .map(|s| s.encoded_frame_count as usize)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Feature-gated recording implementation.
//
// When `integration_tests` is enabled we have access to the `openh264` and
// `minimp4` crates and can encode captured frames to H.264 / MP4.
// Otherwise we provide a no-op capture loop and a PNG-based fallback for
// `finalize`.
// ---------------------------------------------------------------------------
cfg_if::cfg_if! {
    if #[cfg(feature = "integration_tests")] {
        impl CaptureRecorder {
            pub fn finalize(&mut self, output_path: &Path) -> anyhow::Result<()> {
                if let Some(parent) = output_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                let (h264_data, total, dims) = {
                    let mut s = self.inner.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
                    (
                        std::mem::take(&mut s.h264_buf),
                        s.encoded_frame_count,
                        s.dimensions,
                    )
                };

                if h264_data.is_empty() || dims.is_none() {
                    log::info!("CaptureRecorder: no frames encoded, nothing to finalize");
                    return Ok(());
                }

                let (width, height) = dims.expect("dimensions set when h264_data is non-empty");
                match mux_h264_to_mp4(output_path, &h264_data, width, height) {
                    Ok(()) => {
                        log::info!(
                            "CaptureRecorder: wrote {total} frames to {}",
                            output_path.display()
                        );
                    }
                    Err(e) => {
                        log::warn!("CaptureRecorder: MP4 muxing failed ({e})");
                        return Err(e);
                    }
                }

                Ok(())
            }
        }

        pub async fn run_capture_loop(app: crate::App, state: CaptureLoopState) {
            use crate::r#async::Timer;
            use std::time::Duration;

            const CAPTURE_INTERVAL_MS: u64 = 66;
            const FLUSH_THRESHOLD: usize = 60;
            const KEEP_RECENT: usize = 15;

            let inner = &state.0;

            loop {
                Timer::after(Duration::from_millis(CAPTURE_INTERVAL_MS)).await;

                let (current_state, encoding, backlog) = {
                    let s = inner.lock().unwrap_or_else(|e| e.into_inner());
                    (s.recorder_state, s.encoding_in_progress, s.raw_frames.len())
                };
                let should_stop = current_state == RecorderState::Stopping;

                if should_stop && encoding {
                    Timer::after(Duration::from_millis(50)).await;
                    continue;
                }

                let should_flush =
                    (backlog >= FLUSH_THRESHOLD || (should_stop && backlog > 0)) && !encoding;

                if should_flush {
                    let drain_count = if should_stop {
                        backlog
                    } else {
                        backlog.saturating_sub(KEEP_RECENT)
                    };

                    let to_encode: Vec<TimestampedFrame> = {
                        let mut s = inner.lock().unwrap_or_else(|e| e.into_inner());
                        s.raw_frames.drain(..drain_count).collect()
                    };

                    if !to_encode.is_empty() {
                        {
                            let mut s = inner.lock().unwrap_or_else(|e| e.into_inner());
                            s.encoding_in_progress = true;
                        }
                        let inner_clone = inner.clone();
                        std::thread::Builder::new()
                            .name("video-encoder".to_string())
                            .spawn(move || {
                                encode_frame_batch(&to_encode, &inner_clone);
                            })
                            .ok();
                    }
                }

                let still_encoding = inner
                    .lock()
                    .map(|s| s.encoding_in_progress)
                    .unwrap_or(false);
                if should_stop && !still_encoding {
                    break;
                }

                if current_state != RecorderState::Recording {
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

                let inner_clone = inner.clone();
                window
                    .as_ctx()
                    .request_frame_capture(Box::new(move |frame| {
                        let captured_at = Instant::now();
                        if let Ok(mut s) = inner_clone.lock() {
                            s.raw_frames.push(TimestampedFrame { frame, captured_at });
                        }
                    }));
            }
        }

        fn mux_h264_to_mp4(
            output_path: &Path,
            h264_data: &[u8],
            width: u32,
            height: u32,
        ) -> anyhow::Result<()> {
            use minimp4::Mp4Muxer;
            use std::io::Cursor;

            const TARGET_FPS: u32 = 15;

            let mut mp4_buf = Cursor::new(Vec::new());
            let mut muxer = Mp4Muxer::new(&mut mp4_buf);
            muxer.init_video(
                width as i32,
                height as i32,
                false,
                "integration test recording",
            );
            muxer.write_video_with_fps(h264_data, TARGET_FPS);
            muxer.close();

            std::fs::write(output_path, mp4_buf.into_inner())?;
            Ok(())
        }

        /// Encodes a batch of raw frames to H.264 on the calling (background)
        /// thread. Writes results back into `SharedState` under the lock.
        fn encode_frame_batch(
            frames: &[TimestampedFrame],
            inner: &Arc<Mutex<SharedState>>,
        ) {
            use openh264::encoder::Encoder;
            use openh264::formats::{RgbSliceU8, YUVBuffer};

            const FRAME_DURATION_MS: u128 = 1000 / 15;

            let mut encoder = match Encoder::new() {
                Ok(e) => e,
                Err(e) => {
                    log::error!("CaptureRecorder: failed to create encoder on background thread: {e}");
                    if let Ok(mut s) = inner.lock() {
                        s.encoding_in_progress = false;
                    }
                    return;
                }
            };

            if let Some(first) = frames.first() {
                let mut s = inner.lock().unwrap_or_else(|e| e.into_inner());
                if s.dimensions.is_none() {
                    s.dimensions = Some((first.frame.width, first.frame.height));
                }
            }

            let mut prev_captured_at: Option<Instant> = None;
            let mut batch_h264 = Vec::new();
            let mut batch_encoded = 0u32;

            for ts_frame in frames {
                let width = ts_frame.frame.width;
                let height = ts_frame.frame.height;
                let rgb_data = pixel_data_to_rgb(&ts_frame.frame.data, ts_frame.frame.format);
                let rgb_source = RgbSliceU8::new(&rgb_data, (width as usize, height as usize));
                let yuv = YUVBuffer::from_rgb_source(rgb_source);

                let repeat_count = if let Some(prev) = prev_captured_at {
                    let gap_ms = ts_frame.captured_at.duration_since(prev).as_millis();
                    (gap_ms / FRAME_DURATION_MS).max(1) as u32
                } else {
                    1
                };
                prev_captured_at = Some(ts_frame.captured_at);

                for _ in 0..repeat_count {
                    match encoder.encode(&yuv) {
                        Ok(bitstream) => {
                            bitstream.write_vec(&mut batch_h264);
                            batch_encoded += 1;
                        }
                        Err(e) => {
                            log::error!("CaptureRecorder: encode error: {e}");
                            break;
                        }
                    }
                }
            }

            {
                let mut s = inner.lock().unwrap_or_else(|e| e.into_inner());
                if !batch_h264.is_empty() {
                    s.h264_buf.extend_from_slice(&batch_h264);
                }
                s.encoded_frame_count += batch_encoded;
                s.encoding_in_progress = false;
            }

            log::info!(
                "CaptureRecorder: background-encoded {batch_encoded} H.264 frames from {} raw frames",
                frames.len()
            );
        }

        fn pixel_data_to_rgb(data: &[u8], format: crate::platform::CapturedFrameFormat) -> Vec<u8> {
            use crate::platform::CapturedFrameFormat;
            let pixel_count = data.len() / 4;
            let mut rgb = Vec::with_capacity(pixel_count * 3);
            for chunk in data.chunks_exact(4) {
                match format {
                    CapturedFrameFormat::Rgba => {
                        rgb.push(chunk[0]);
                        rgb.push(chunk[1]);
                        rgb.push(chunk[2]);
                    }
                    CapturedFrameFormat::Bgra => {
                        rgb.push(chunk[2]);
                        rgb.push(chunk[1]);
                        rgb.push(chunk[0]);
                    }
                }
            }
            rgb
        }
    } else {
        impl CaptureRecorder {
            pub fn finalize(&mut self, output_path: &Path) -> anyhow::Result<()> {
                if let Some(parent) = output_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                let frames: Vec<TimestampedFrame> = self
                    .inner
                    .lock()
                    .map(|mut s| std::mem::take(&mut s.raw_frames))
                    .unwrap_or_default();
                if frames.is_empty() {
                    log::info!("CaptureRecorder: no frames captured, nothing to finalize");
                    return Ok(());
                }
                save_frames_as_pngs(output_path, &frames)?;

                Ok(())
            }
        }

        pub async fn run_capture_loop(_app: crate::App, _state: CaptureLoopState) {}

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
                "CaptureRecorder: saved {} PNGs to {}",
                frames.len(),
                dir.display()
            );
            Ok(())
        }
    }
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

/// Helper to retrieve a mutable reference to the recorder from a `StepDataMap`.
pub fn get_capture_recorder_mut(
    step_data_map: &mut super::step::StepDataMap,
) -> Option<&mut CaptureRecorder> {
    step_data_map.get_mut::<_, CaptureRecorder>(CAPTURE_RECORDER_KEY)
}

/// Helper to retrieve a shared reference to the recorder from a `StepDataMap`.
pub fn get_capture_recorder(step_data_map: &super::step::StepDataMap) -> Option<&CaptureRecorder> {
    step_data_map.get::<_, CaptureRecorder>(CAPTURE_RECORDER_KEY)
}
