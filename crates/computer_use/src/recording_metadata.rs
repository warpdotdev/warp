use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::RecordingError;

pub(super) async fn video_duration(input: &Path) -> Result<Duration, RecordingError> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to inspect finalized video: {error}"),
        })?;

    parse_duration(&String::from_utf8_lossy(&output.stderr)).ok_or_else(|| {
        RecordingError::Finalize {
            reason: "ffmpeg did not report a valid finalized video duration".to_string(),
        }
    })
}

fn parse_duration(stderr: &str) -> Option<Duration> {
    let timestamp = stderr.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Duration:")
            .and_then(|value| value.split(',').next())
            .map(str::trim)
    })?;
    let mut components = timestamp.split(':');
    let hours = components.next()?.parse::<u64>().ok()?;
    let minutes = components.next()?.parse::<u64>().ok()?;
    let seconds = components.next()?;
    if components.next().is_some() || minutes >= 60 {
        return None;
    }

    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let seconds = seconds.parse::<u64>().ok()?;
    if seconds >= 60 || fraction.len() > 9 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let nanos = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>().ok()? * 10_u32.pow(9 - fraction.len() as u32)
    };
    let seconds = hours
        .checked_mul(60 * 60)?
        .checked_add(minutes * 60)?
        .checked_add(seconds)?;
    Some(Duration::new(seconds, nanos))
}

#[cfg(test)]
#[path = "recording_metadata_tests.rs"]
mod tests;
