use std::time::Duration;

use tokio::process::Command;

use super::*;

#[test]
fn parses_ffmpeg_container_duration() {
    let stderr = "  Duration: 01:02:03.456789, start: 0.000000, bitrate: 64 kb/s";

    assert_eq!(
        parse_duration(stderr),
        Some(Duration::new(60 * 60 + 2 * 60 + 3, 456_789_000))
    );
}

#[test]
fn rejects_missing_or_invalid_duration() {
    for stderr in [
        "",
        "Duration: N/A, start: 0.000000",
        "Duration: 00:60:00.00, start: 0.000000",
        "Duration: 00:00:60.00, start: 0.000000",
    ] {
        assert_eq!(parse_duration(stderr), None);
    }
}

#[tokio::test]
async fn probes_duration_after_timestamp_rescaling() {
    if !Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .is_ok_and(|output| output.status.success())
    {
        return;
    }

    let path = std::env::temp_dir().join(format!(
        "warp-duration-probe-test-{}.mp4",
        uuid::Uuid::new_v4()
    ));
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=16x16:rate=10:duration=4",
            "-vf",
            "setpts=0.25*PTS",
            "-an",
            "-r",
            "10",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .output()
        .await
        .expect("run ffmpeg");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let duration = video_duration(&path).await.expect("probe duration");
    let _ = std::fs::remove_file(path);

    assert!(
        (Duration::from_millis(800)..=Duration::from_millis(1200)).contains(&duration),
        "expected a roughly 1-second final timeline, got {duration:?}"
    );
}
