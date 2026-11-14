use super::*;
use crate::RecordingCompletionStatus;

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "warp-recording-prepare-test-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn invalid_capture_is_deleted_with_its_log() {
    let dir = scratch_dir("invalid-capture");
    let raw = dir.join("raw.mp4");
    let capture_log = raw.with_extension("log");
    std::fs::write(&raw, b"not a video").unwrap();
    std::fs::write(&capture_log, b"ffmpeg output").unwrap();
    let output = RecordingOutput {
        path: raw.clone(),
        duration: Duration::from_secs(1),
        width: 16,
        height: 16,
        size_bytes: 11,
        completion_status: RecordingCompletionStatus::Completed,
    };

    assert!(prepare_recording(&output, &[], 15).await.is_err());

    assert!(!raw.exists());
    assert!(!capture_log.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(any(macos, linux, windows))]
#[tokio::test]
async fn invalid_processed_recording_falls_back_to_validated_raw() {
    if !tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .is_ok_and(|output| output.status.success())
    {
        return;
    }

    let root = scratch_dir("invalid-processed");
    let raw_path = root.join("raw.mp4");
    let processed_path = root.join("processed.mp4");
    let generated = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=10:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&raw_path)
        .output()
        .await
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    std::fs::write(&processed_path, b"not a video").unwrap();

    let (path, duration) = validated_recording(&raw_path, Some(processed_path.clone()))
        .await
        .unwrap();

    assert_eq!(path, raw_path);
    assert!(duration > Duration::ZERO);
    assert!(!processed_path.exists());
    std::fs::remove_dir_all(root).unwrap();
}
