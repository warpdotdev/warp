//! Windows screen recording via a supervised ffmpeg `gdigrab` process.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::dpi::DpiAwarenessGuard;
use crate::{
    RecordingCompletionStatus, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput,
};

const START_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VirtualScreenGeometry {
    origin_x: i32,
    origin_y: i32,
    width: u32,
    height: u32,
}

pub struct Recorder {
    ffmpeg: PathBuf,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            ffmpeg: PathBuf::from("ffmpeg"),
        }
    }

    #[cfg(test)]
    fn with_ffmpeg(ffmpeg: PathBuf) -> Self {
        Self { ffmpeg }
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        verify_gdigrab_available(&self.ffmpeg).await?;
        let geometry = query_virtual_screen_geometry()?;
        let (path, log_path, log_file) = new_recording_path()?;
        let command = new_ffmpeg_capture_command(&self.ffmpeg, &config, geometry);
        launch_recording(
            command,
            path,
            log_path,
            log_file,
            geometry.width,
            geometry.height,
            START_TIMEOUT,
        )
        .await
    }

    async fn stop(&self, mut handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        let width = handle.width;
        let height = handle.height;
        let path = handle.path.clone();
        let duration = handle.started_at.elapsed();
        let mut process = handle
            .process
            .take()
            .ok_or_else(|| RecordingError::Finalize {
                reason: "recording process is unavailable".to_string(),
            })?;

        let completion_status = match finalize_capture(&mut process, &path, STOP_TIMEOUT).await {
            Ok(status) => status,
            Err(error) => {
                remove_recording_files(&path);
                return Err(error);
            }
        };
        let _ = std::fs::remove_file(path.with_extension("log"));

        let size_bytes = std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if size_bytes == 0 {
            remove_recording_files(&path);
            return Err(RecordingError::Finalize {
                reason: "recording produced an empty file".to_string(),
            });
        }
        if let Err(error) =
            crate::recording_metadata::video_duration_with_ffmpeg(&self.ffmpeg, &path).await
        {
            remove_recording_files(&path);
            return Err(error);
        }

        handle.cleanup_on_drop = false;
        Ok(RecordingOutput {
            path,
            duration,
            width,
            height,
            size_bytes,
            completion_status,
        })
    }
}

fn query_virtual_screen_geometry() -> Result<VirtualScreenGeometry, RecordingError> {
    let _dpi_guard = DpiAwarenessGuard::enter_per_monitor_v2();
    // SAFETY: `GetSystemMetrics` has no preconditions.
    let origin_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let origin_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    normalize_virtual_screen_geometry(origin_x, origin_y, width, height)
}

fn normalize_virtual_screen_geometry(
    origin_x: i32,
    origin_y: i32,
    width: i32,
    height: i32,
) -> Result<VirtualScreenGeometry, RecordingError> {
    if width <= 0 || height <= 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid virtual screen dimensions {width}x{height}"),
        });
    }
    let width = (width as u32) & !1;
    let height = (height as u32) & !1;
    if width == 0 || height == 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid even virtual screen dimensions {width}x{height}"),
        });
    }
    Ok(VirtualScreenGeometry {
        origin_x,
        origin_y,
        width,
        height,
    })
}

async fn verify_gdigrab_available(ffmpeg: &Path) -> Result<(), RecordingError> {
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-h", "demuxer=gdigrab"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| RecordingError::Environment {
            reason: format!("failed to launch ffmpeg for gdigrab probe: {error}"),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let identifies_gdigrab = stdout.lines().chain(stderr.lines()).any(|line| {
        line.trim()
            .to_ascii_lowercase()
            .starts_with("demuxer gdigrab")
    });
    if output.status.success() && identifies_gdigrab {
        return Ok(());
    }

    let diagnostic = diagnostic_tail(&format!("{stdout}\n{stderr}"));
    let reason = if output.status.success() {
        format!("ffmpeg does not expose the gdigrab input demuxer{diagnostic}")
    } else {
        format!(
            "ffmpeg gdigrab probe exited with status {}{diagnostic}",
            output.status
        )
    };
    Err(RecordingError::Environment { reason })
}

fn new_recording_path() -> Result<(PathBuf, PathBuf, File), RecordingError> {
    let path = std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
    let log_path = path.with_extension("log");
    let log_file = File::create(&log_path).map_err(|error| RecordingError::Start {
        reason: format!("failed to create the recording log file: {error}"),
    })?;
    Ok((path, log_path, log_file))
}

fn new_ffmpeg_capture_command(
    ffmpeg: &Path,
    config: &RecordingConfig,
    geometry: VirtualScreenGeometry,
) -> Command {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .args(["-f", "gdigrab"])
        .args(["-framerate", &config.frame_rate.to_string()])
        .args(["-offset_x", &geometry.origin_x.to_string()])
        .args(["-offset_y", &geometry.origin_y.to_string()])
        .args([
            "-video_size",
            &format!("{}x{}", geometry.width, geometry.height),
        ])
        .args(["-draw_mouse", "1"])
        .arg("-t")
        .arg(format!("{:.3}", config.max_duration.as_secs_f64()))
        .args(["-i", "desktop"])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-movflags", "+faststart"])
        .arg("-fs")
        .arg(config.max_size_bytes.to_string());
    command
}

async fn launch_recording(
    mut command: Command,
    path: PathBuf,
    log_path: PathBuf,
    log_file: File,
    width: u32,
    height: u32,
    timeout: Duration,
) -> Result<RecordingHandle, RecordingError> {
    command
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file))
        .kill_on_drop(true);
    let mut process = match command.spawn() {
        Ok(process) => process,
        Err(error) => {
            remove_recording_files(&path);
            return Err(RecordingError::Start {
                reason: format!("failed to start ffmpeg gdigrab capture: {error}"),
            });
        }
    };

    if let Err(error) = wait_for_first_output(&path, &mut process, timeout).await {
        kill_and_reap(&mut process).await;
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let diagnostic = diagnostic_tail(&log);
        remove_recording_files(&path);
        return Err(RecordingError::Start {
            reason: format!("{error}{diagnostic}"),
        });
    }

    Ok(RecordingHandle {
        width,
        height,
        exit_state: Arc::new(Mutex::new(None)),
        path,
        started_at: Instant::now(),
        process: Some(process),
        cleanup_on_drop: true,
    })
}

async fn wait_for_first_output(
    path: &Path,
    process: &mut Child,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = process
            .try_wait()
            .map_err(|error| format!("failed to poll ffmpeg: {error}"))?
        {
            return Err(format!("ffmpeg exited early with status {status}"));
        }
        if std::fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            > 0
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for ffmpeg capture output".to_string());
        }
        tokio::time::sleep(POLL_INTERVAL.min(timeout)).await;
    }
}

async fn finalize_capture(
    process: &mut Child,
    path: &Path,
    timeout: Duration,
) -> Result<RecordingCompletionStatus, RecordingError> {
    if process
        .try_wait()
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to poll ffmpeg: {error}"),
        })?
        .is_some()
    {
        return Ok(RecordingCompletionStatus::StoppedEarly);
    }

    let Some(mut stdin) = process.stdin.take() else {
        kill_and_reap(process).await;
        remove_recording_files(path);
        return Err(RecordingError::Finalize {
            reason: "ffmpeg stdin is unavailable for graceful finalization".to_string(),
        });
    };
    if let Err(error) = stdin.write_all(b"q\n").await {
        drop(stdin);
        kill_and_reap(process).await;
        remove_recording_files(path);
        return Err(RecordingError::Finalize {
            reason: format!("failed to request ffmpeg finalization: {error}"),
        });
    }
    drop(stdin);

    match tokio::time::timeout(timeout, process.wait()).await {
        Ok(Ok(_)) => Ok(RecordingCompletionStatus::Completed),
        Ok(Err(error)) => {
            kill_and_reap(process).await;
            remove_recording_files(path);
            Err(RecordingError::Finalize {
                reason: format!("failed to wait for ffmpeg finalization: {error}"),
            })
        }
        Err(_) => {
            kill_and_reap(process).await;
            remove_recording_files(path);
            Err(RecordingError::Finalize {
                reason: "ffmpeg did not finalize the recording in time".to_string(),
            })
        }
    }
}

async fn kill_and_reap(process: &mut Child) {
    let _ = process.start_kill();
    let _ = process.wait().await;
}

fn remove_recording_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("log"));
}

fn diagnostic_tail(text: &str) -> String {
    const MAX_CHARS: usize = 512;
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" ");
    if tail.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = tail.chars().collect();
    let tail = if chars.len() > MAX_CHARS {
        chars[chars.len() - MAX_CHARS..].iter().collect()
    } else {
        tail
    };
    format!(" ({tail})")
}

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
