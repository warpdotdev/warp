//! Post-stop processing shared by both capture substrates: cut the 1x master
//! down to the retained action segments, then burn the remapped action and
//! pointer overlays into the result.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::overlay::KeepSegment;
use crate::{ActionLogEntry, RecordingError};

/// Cuts `input` to only the retained action segments and returns the path to
/// the trimmed file (a sibling of `input` with extension `cut.mp4`). The
/// original 1x master is left untouched; the caller owns cleanup of both.
///
/// Each retained segment is extracted via ffmpeg `trim`/`setpts=PTS-STARTPTS`
/// and the strips are concatenated (`concat=n=N:v=1:a=0`, video-only). Source
/// gaps between segments are removed entirely, producing a compact 1x video
/// that contains only the real action windows. This step is deliberately free
/// of overlay logic; overlays are applied separately in `burn_overlays_into_cut`.
async fn cut_to_segments(
    input: &Path,
    segments: &[KeepSegment],
    frame_rate: u32,
) -> Result<PathBuf, RecordingError> {
    let output_path = input.with_extension("cut.mp4");
    let filter = build_cut_only_filtergraph(segments);
    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(&filter)
        .arg("-map")
        .arg("[vout]")
        // Force a constant output frame rate so every retained frame — including
        // the cut's final frame, which would otherwise have no defined duration
        // and be dropped by the muxer — is written. The source master is
        // captured at `frame_rate`, so this matches its cadence without
        // duplicating or dropping frames.
        .args(["-r", &frame_rate.to_string()])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-movflags", "+faststart"])
        .arg(&output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    match status {
        Ok(status) if status.success() => Ok(output_path),
        Ok(status) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("ffmpeg segment cut exited with status {status}"),
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("failed to run ffmpeg for segment cut: {e}"),
            })
        }
    }
}

/// Burns the remapped ASS overlay pills into an already-cut `input` video,
/// returning the path to the annotated file (a sibling with extension
/// `overlay.mp4`). The cut input is left untouched; the caller owns cleanup of
/// both. This step is deliberately free of segment-cut logic; cutting is done
/// separately in `cut_to_segments`.
async fn burn_overlays_into_cut(
    input: &Path,
    ass_path: &Path,
    frame_rate: u32,
) -> Result<PathBuf, RecordingError> {
    let output_path = input.with_extension("overlay.mp4");
    let subtitles_filter = format!("subtitles=filename='{}'", ass_path.display());
    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .args(["-vf", &subtitles_filter])
        .args(["-r", &frame_rate.to_string()])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-movflags", "+faststart"])
        .arg(&output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    match status {
        Ok(status) if status.success() => Ok(output_path),
        Ok(status) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("ffmpeg overlay burn-in exited with status {status}"),
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("failed to run ffmpeg for overlay burn-in: {e}"),
            })
        }
    }
}

/// Post-stop pipeline: cut the 1x source to retained action segments, then
/// burn remapped overlay pills into the result. Returns the path to the final
/// annotated file (a sibling of `input`). The original 1x master and the
/// intermediate cut file are left untouched; the caller owns cleanup of all
/// produced paths. ffmpeg demuxes each mp4 from disk frame-by-frame, so the
/// whole recording is never buffered in memory.
///
/// The two steps are independent: `cut_to_segments` knows nothing about
/// overlays, and `burn_overlays_into_cut` knows nothing about segment
/// boundaries. A recording whose committed actions yield no qualifying segment
/// returns an error rather than producing a video; the caller falls back to
/// uploading the untouched source for an unexpected processing failure after at
/// least one committed action.
pub(crate) async fn post_process_recording(
    input: &Path,
    entries: &[ActionLogEntry],
    dimensions: (u32, u32),
    source_duration: Duration,
    frame_rate: u32,
) -> Result<PathBuf, RecordingError> {
    let segments = crate::overlay::build_keep_segments(entries, source_duration, frame_rate);
    if segments.is_empty() {
        return Err(RecordingError::Finalize {
            reason: "recording has no qualifying action segments to keep".to_string(),
        });
    }

    // Step 1: cut the source to retained segments only.
    let cut_path = cut_to_segments(input, &segments, frame_rate).await?;

    // Step 2: write the remapped ASS and burn overlays into the cut video.
    let ass_path = input.with_extension("ass");
    let write_result = std::fs::write(
        &ass_path,
        crate::overlay::build_overlay_ass(entries, dimensions, source_duration, frame_rate),
    );
    let overlay_result = match write_result {
        Ok(()) => burn_overlays_into_cut(&cut_path, &ass_path, frame_rate).await,
        Err(e) => Err(RecordingError::Finalize {
            reason: format!("failed to write overlay subtitle file: {e}"),
        }),
    };
    // The subtitle file is an implementation detail; drop it regardless of outcome.
    let _ = std::fs::remove_file(&ass_path);
    // The intermediate cut file is no longer needed once overlays are applied
    // (or failed); the caller uploads the overlay output or falls back to the
    // original source on any error.
    let _ = std::fs::remove_file(&cut_path);

    overlay_result
}

/// Builds the ffmpeg `filter_complex` for the segment-cut step only (no
/// overlays). For each retained segment the input video is `trim`med to its
/// source `[start, end)` window and reset to a zero-based PTS
/// (`setpts=PTS-STARTPTS`); the trimmed strips are concatenated in source
/// order (`concat=n=N:v=1:a=0`, video-only). The result is mapped to the
/// `[vout]` label by the caller. This removes only the dead source frames
/// and preserves the 1x frame cadence inside each retained segment.
fn build_cut_only_filtergraph(segments: &[KeepSegment]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(segments.len() + 1);
    for (index, segment) in segments.iter().enumerate() {
        let start = segment.source_start.as_secs_f64();
        let end = segment.source_end.as_secs_f64();
        // `trim` selects the source frame range; `setpts=PTS-STARTPTS` relabels
        // the strip's first frame as time zero so the old gap timestamp is not
        // carried into the concatenated output.
        parts.push(format!(
            "[0:v]trim=start={start:.6}:end={end:.6},setpts=PTS-STARTPTS[v{index}]"
        ));
    }
    let inputs: String = (0..segments.len())
        .map(|index| format!("[v{index}]"))
        .collect::<Vec<_>>()
        .join("");
    let n = segments.len();
    parts.push(format!("{inputs}concat=n={n}:v=1:a=0[vout]"));
    parts.join(";")
}

#[cfg(test)]
#[path = "post_process_tests.rs"]
mod tests;
