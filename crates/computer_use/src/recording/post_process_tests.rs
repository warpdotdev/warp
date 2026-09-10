//! Tests for the shared post-stop cut and overlay pipeline.
//!
//! These exercise the platform-neutral recording core, so they run identically
//! under the macOS and Linux builds. They shell out to ffmpeg and skip (rather
//! than fail) when it is unavailable.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use super::{build_cut_only_filtergraph, post_process_recording};
use crate::overlay::{KeepSegment, PointerEvent, PointerEventKind};
use crate::{ActionLogEntry, MouseButton, RecordingError, Vector2I};

const FIXTURE_FRAME_RATE: u32 = 10;
// Two trailing frames beyond the last retained interval keep the final kept
// frame off the source boundary, where some muxer/decoder paths drop a
// frame that has no defined duration.
const FIXTURE_FRAMES: usize = 12;
const FIXTURE_W: u32 = 64;
const FIXTURE_H: u32 = 64;

/// Returns whether ffmpeg is available (no display required).
async fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Returns whether ffmpeg was built with the libass-backed `subtitles` filter
/// the overlay burn-in needs.
async fn subtitles_filter_available() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .await
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some("subtitles"))
        })
        .unwrap_or(false)
}

/// Parses the container duration (seconds) from `ffmpeg -i` stderr.
async fn probe_duration(path: &Path) -> f64 {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(path)
        .output()
        .await
        .expect("run ffmpeg probe");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for token in stderr.split([',', '\n']) {
        let token = token.trim();
        if let Some(rest) = token.strip_prefix("Duration:") {
            let dur = rest.trim();
            let parts: Vec<&str> = dur.split(':').collect();
            if parts.len() == 3 {
                let h: f64 = parts[0].parse().unwrap_or(0.0);
                let m: f64 = parts[1].parse().unwrap_or(0.0);
                let s: f64 = parts[2].parse().unwrap_or(0.0);
                return h * 3600.0 + m * 60.0 + s;
            }
        }
    }
    f64::NAN
}

/// Encodes a source frame's index in its red channel with a 24-step so a
/// decoded frame can be mapped back to its source index even after
/// rgb24 -> yuv420p -> rgb24 round-trip and libx264 ultrafast re-encoding.
fn fixture_frame_color(index: usize) -> (u8, u8, u8) {
    let r = (12 + (index as u32) * 24).min(240) as u8;
    (r, 128, 128)
}

/// Writes a deterministic source mp4 of `frames` uniquely colored frames.
async fn write_fixture_source(path: &Path, frames: usize) {
    let frame_len = (FIXTURE_W as usize) * (FIXTURE_H as usize) * 3;
    let mut raw = Vec::with_capacity(frames * frame_len);
    for index in 0..frames {
        let (r, g, b) = fixture_frame_color(index);
        for _ in 0..(FIXTURE_W as usize * FIXTURE_H as usize) {
            raw.push(r);
            raw.push(g);
            raw.push(b);
        }
    }
    let raw_path = path.with_extension("raw");
    std::fs::write(&raw_path, &raw).expect("write raw source");
    let output = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-video_size", &format!("{FIXTURE_W}x{FIXTURE_H}")])
        .args(["-framerate", &FIXTURE_FRAME_RATE.to_string()])
        .arg("-i")
        .arg(&raw_path)
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .await
        .expect("run ffmpeg source encode");
    let _ = std::fs::remove_file(&raw_path);
    assert!(
        output.status.success(),
        "ffmpeg source encode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Maps a decoded frame back to its source index by nearest red-channel value.
fn identify_fixture_frame(frame: &[u8]) -> usize {
    let n = (FIXTURE_W as usize) * (FIXTURE_H as usize);
    let mut sum_r = 0u32;
    for px in 0..n {
        sum_r += frame[px * 3] as u32;
    }
    let avg_r = (sum_r / n as u32) as i32;
    let mut best = 0usize;
    let mut best_dist = u32::MAX;
    for index in 0..FIXTURE_FRAMES {
        let (r, _, _) = fixture_frame_color(index);
        let dist = (avg_r - r as i32).unsigned_abs();
        if dist < best_dist {
            best_dist = dist;
            best = index;
        }
    }
    assert!(
        best_dist <= 12,
        "decoded frame did not match any source frame (avg_r={avg_r}, best_dist={best_dist})"
    );
    best
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("warp-cut-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn entry(start_ms: u64, finish_ms: u64, label: &str) -> ActionLogEntry {
    ActionLogEntry {
        offset: Duration::from_millis(start_ms),
        finish_offset: Duration::from_millis(finish_ms),
        labels: vec![label.to_string()],
        pointer_events: Vec::new(),
    }
}

/// The cut-only filtergraph emits one `trim`+`setpts=PTS-STARTPTS` branch per
/// retained segment, concatenates them video-only, and maps the result to
/// `[vout]`. It contains no overlay/subtitles logic, which is handled in a
/// separate `burn_overlays_into_cut` pass.
#[test]
fn build_cut_only_filtergraph_constructs_trim_setpts_concat() {
    let segments = vec![
        KeepSegment {
            source_start: Duration::from_millis(500),
            source_end: Duration::from_millis(2500),
            output_start: Duration::ZERO,
        },
        KeepSegment {
            source_start: Duration::from_millis(4500),
            source_end: Duration::from_millis(6500),
            output_start: Duration::from_millis(2000),
        },
    ];
    let filter = build_cut_only_filtergraph(&segments);

    assert!(filter.contains("[0:v]trim=start=0.500000:end=2.500000,setpts=PTS-STARTPTS[v0]"));
    assert!(filter.contains("[0:v]trim=start=4.500000:end=6.500000,setpts=PTS-STARTPTS[v1]"));
    assert!(filter.contains("[v0][v1]concat=n=2:v=1:a=0[vout]"));
    // Cut-only filtergraph must not contain subtitles/overlay logic.
    assert!(
        !filter.contains("subtitles"),
        "cut-only filtergraph should not contain subtitles filter, got {filter}"
    );
    assert!(
        filter.ends_with("[vout]"),
        "filter should end with [vout], got {filter}"
    );
}

/// Cuts a deterministic source video to two retained intervals and asserts the
/// output contains exactly the selected frames in source order, with no black
/// frames and a duration equal to the sum of the retained intervals.
#[tokio::test]
async fn smart_cut_retains_only_selected_frames_in_order() {
    if !ffmpeg_available().await {
        eprintln!("skipping smart_cut_retains_only_selected_frames_in_order: no ffmpeg");
        return;
    }

    let dir = temp_dir();
    let source = dir.join("source.mp4");
    write_fixture_source(&source, FIXTURE_FRAMES).await;

    // Keep frames 1-3 (PTS 0.1-0.4 s) and 7-9 (PTS 0.7-1.0 s); frames 0, 4, 5,
    // 6 are removed. At 10 fps each frame is 100 ms.
    let segments = vec![
        KeepSegment {
            source_start: Duration::from_millis(100),
            source_end: Duration::from_millis(400),
            output_start: Duration::ZERO,
        },
        KeepSegment {
            source_start: Duration::from_millis(700),
            source_end: Duration::from_millis(1000),
            output_start: Duration::from_millis(300),
        },
    ];
    let filter = build_cut_only_filtergraph(&segments);
    let output = dir.join("cut.mp4");
    // Mirror the production cut encode, including the constant output frame
    // rate that ensures the cut's final frame is written.
    let cut = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-i"])
        .arg(&source)
        .args(["-filter_complex", &filter])
        .args(["-map", "[vout]"])
        .args(["-r", &FIXTURE_FRAME_RATE.to_string()])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&output)
        .output()
        .await
        .expect("run ffmpeg cut");
    assert!(
        cut.status.success(),
        "ffmpeg cut failed: {}",
        String::from_utf8_lossy(&cut.stderr)
    );

    // Decode the cut output back to raw rgb24 and identify each frame.
    // `-vsync 0` (passthrough) avoids CFR duplication so the decoded frame count
    // is exact.
    let raw_out = dir.join("cut.raw");
    let decode = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-i"])
        .arg(&output)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-vsync", "0"])
        .arg(&raw_out)
        .output()
        .await
        .expect("run ffmpeg decode");
    assert!(
        decode.status.success(),
        "ffmpeg decode failed: {}",
        String::from_utf8_lossy(&decode.stderr)
    );
    let data = std::fs::read(&raw_out).expect("read decoded rawvideo");
    let frame_len = (FIXTURE_W as usize) * (FIXTURE_H as usize) * 3;
    let frame_count = data.len() / frame_len;
    let indices: Vec<usize> = (0..frame_count)
        .map(|i| identify_fixture_frame(&data[i * frame_len..(i + 1) * frame_len]))
        .collect();

    // Exactly the selected frames, in source order, with no duplicates or
    // inserted gap frames.
    assert_eq!(
        indices,
        vec![1, 2, 3, 7, 8, 9],
        "cut should retain exactly frames 1,2,3,7,8,9 in order, got {indices:?}"
    );

    // No black frames: every retained frame is a solid color.
    for i in 0..frame_count {
        let frame = &data[i * frame_len..(i + 1) * frame_len];
        let sum: u32 = frame.iter().map(|b| *b as u32).sum();
        assert!(sum > 0, "retained frame {i} is black");
    }

    // Output duration equals the sum of retained intervals (6 frames * 100 ms).
    let duration = probe_duration(&output).await;
    assert!(
        (duration - 0.6).abs() < 0.08,
        "output duration should be ~0.6s (6 frames at 10fps), got {duration}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Two action groups separated by an idle gap: the finalized media keeps both
/// groups at 1x, drops the gap, and reports a duration equal to the retained
/// segments rather than the wall-clock capture length.
#[tokio::test]
async fn post_process_drops_the_gap_between_two_action_groups() {
    if !ffmpeg_available().await || !subtitles_filter_available().await {
        eprintln!(
            "skipping post_process_drops_the_gap_between_two_action_groups: no ffmpeg with \
             libass subtitles filter"
        );
        return;
    }

    // A 10-second source: an action at 0.5-1.0 s, an 8-second idle gap, then an
    // action at 9.0-9.5 s.
    let source_duration = Duration::from_secs(10);
    let frames = source_duration.as_secs() as usize * FIXTURE_FRAME_RATE as usize;
    let dir = temp_dir();
    let source = dir.join("source.mp4");
    write_fixture_source(&source, frames).await;

    let entries = [entry(500, 1000, "first"), entry(9000, 9500, "second")];
    let processed = post_process_recording(
        &source,
        &entries,
        (FIXTURE_W, FIXTURE_H),
        source_duration,
        FIXTURE_FRAME_RATE,
    )
    .await
    .expect("post-process the fixture recording");

    // The two 250 ms-lead-in / 1000 ms-trailing windows sum to 3.5 s; the
    // 8-second idle gap between them is gone.
    let segments =
        crate::overlay::build_keep_segments(&entries, source_duration, FIXTURE_FRAME_RATE);
    assert_eq!(segments.len(), 2, "expected two retained segments");
    let retained: Duration = segments
        .iter()
        .map(|segment| segment.source_end - segment.source_start)
        .sum();
    let finalized = crate::finalized_video_duration(&processed)
        .await
        .expect("probe the finalized duration");
    assert!(
        finalized < source_duration,
        "finalized duration {finalized:?} should be shorter than the {source_duration:?} capture"
    );
    assert!(
        finalized.abs_diff(retained) < Duration::from_millis(250),
        "finalized duration {finalized:?} should match the retained segments {retained:?}"
    );

    // The intermediate cut and subtitle files are cleaned up; the 1x master is
    // left for the caller to remove.
    assert!(!source.with_extension("cut.mp4").exists());
    assert!(!source.with_extension("ass").exists());
    assert!(source.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Burns a real overlay into a real video with the host's ffmpeg and libass, and
/// asserts both an action pill and a click ring actually rasterized. Missing
/// fonts render nothing while still exiting zero, so only inspecting pixels
/// catches an unresolvable overlay font family.
#[tokio::test]
async fn libass_burn_in_rasterizes_a_pill_and_a_click_ring() {
    if !ffmpeg_available().await || !subtitles_filter_available().await {
        eprintln!(
            "skipping libass_burn_in_rasterizes_a_pill_and_a_click_ring: no ffmpeg with libass \
             subtitles filter"
        );
        return;
    }

    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 240;
    let source_duration = Duration::from_secs(2);
    let click = Vector2I::new(60, 40);

    let dir = temp_dir();
    let source = dir.join("source.mp4");
    write_black_source(&source, WIDTH, HEIGHT, 20).await;

    let entries = [ActionLogEntry {
        offset: Duration::from_millis(500),
        finish_offset: Duration::from_millis(700),
        labels: vec!["cmd+a".to_string()],
        pointer_events: vec![
            PointerEvent {
                offset: Duration::from_millis(500),
                kind: PointerEventKind::Down,
                button: Some(MouseButton::Left),
                point: click,
            },
            PointerEvent {
                offset: Duration::from_millis(560),
                kind: PointerEventKind::Up,
                button: Some(MouseButton::Left),
                point: click,
            },
        ],
    }];
    let processed = post_process_recording(
        &source,
        &entries,
        (WIDTH, HEIGHT),
        source_duration,
        FIXTURE_FRAME_RATE,
    )
    .await
    .expect("post-process the annotated fixture");

    // The pointer annotations sit around the click; the pills sit bottom-center.
    // The two regions are disjoint, so a frame lighting up both proves each pass
    // drew something rather than one bleeding into the other.
    let pointer_region = (20, 0, 100, 80);
    let pill_region = (110, 100, 210, 150);
    let frames = decode_rgb_frames(&processed, WIDTH, HEIGHT).await;
    let annotated = frames
        .iter()
        .filter(|frame| {
            region_has_ink(frame, WIDTH, pointer_region)
                && region_has_ink(frame, WIDTH, pill_region)
        })
        .count();
    assert!(
        annotated > 0,
        "no frame of {} carried both a pill and a click ring; libass drew nothing",
        frames.len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes a solid black source mp4 so any non-black pixel in the output came
/// from the burn-in.
async fn write_black_source(path: &Path, width: u32, height: u32, frames: usize) {
    let raw_path = path.with_extension("raw");
    let frame_len = (width as usize) * (height as usize) * 3;
    std::fs::write(&raw_path, vec![0u8; frame_len * frames]).expect("write raw source");
    let output = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-video_size", &format!("{width}x{height}")])
        .args(["-framerate", &FIXTURE_FRAME_RATE.to_string()])
        .arg("-i")
        .arg(&raw_path)
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .await
        .expect("run ffmpeg source encode");
    let _ = std::fs::remove_file(&raw_path);
    assert!(
        output.status.success(),
        "ffmpeg source encode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Decodes every frame of `path` to raw rgb24.
async fn decode_rgb_frames(path: &Path, width: u32, height: u32) -> Vec<Vec<u8>> {
    let raw_path = path.with_extension("decoded.raw");
    let output = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-vsync", "0"])
        .arg(&raw_path)
        .output()
        .await
        .expect("run ffmpeg decode");
    assert!(
        output.status.success(),
        "ffmpeg decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = std::fs::read(&raw_path).expect("read decoded rawvideo");
    let _ = std::fs::remove_file(&raw_path);
    let frame_len = (width as usize) * (height as usize) * 3;
    data.chunks_exact(frame_len).map(<[u8]>::to_vec).collect()
}

/// Whether any pixel in the `(left, top, right, bottom)` box is materially
/// brighter than the black source.
fn region_has_ink(frame: &[u8], width: u32, region: (u32, u32, u32, u32)) -> bool {
    const INK_THRESHOLD: u8 = 60;
    let (left, top, right, bottom) = region;
    (top..bottom).any(|y| {
        (left..right).any(|x| {
            let offset = ((y * width + x) * 3) as usize;
            frame[offset..offset + 3]
                .iter()
                .any(|channel| *channel > INK_THRESHOLD)
        })
    })
}

/// A cut failure leaves the 1x master untouched for the fallback upload and
/// removes the partial `.cut.mp4` and `.ass` intermediates.
#[tokio::test]
async fn failed_cut_leaves_the_master_and_removes_intermediates() {
    if !ffmpeg_available().await {
        eprintln!("skipping failed_cut_leaves_the_master_and_removes_intermediates: no ffmpeg");
        return;
    }

    // A file that is not decodable video makes the cut pass fail.
    let dir = temp_dir();
    let source = dir.join("source.mp4");
    std::fs::write(&source, b"not an mp4").expect("write corrupt source");

    let entries = [entry(0, 500, "first")];
    let error = post_process_recording(
        &source,
        &entries,
        (FIXTURE_W, FIXTURE_H),
        Duration::from_secs(2),
        FIXTURE_FRAME_RATE,
    )
    .await
    .expect_err("a corrupt source must fail the cut");
    assert!(matches!(error, RecordingError::Finalize { .. }));

    assert!(source.exists(), "the 1x master must survive a cut failure");
    assert!(!source.with_extension("cut.mp4").exists());
    assert!(!source.with_extension("ass").exists());

    let _ = std::fs::remove_dir_all(&dir);
}
