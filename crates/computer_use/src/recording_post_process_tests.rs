use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

use super::{burn_overlays_into_cut, post_process_recording, subtitles_filter_context};
use crate::RecordingError;

#[test]
fn keeps_subtitle_directory_characters_out_of_filtergraph() {
    let path = Path::new("runner's temp").join("warp-recording-overlay-123.ass");
    let (directory, filter) = subtitles_filter_context(&path).unwrap();

    assert_eq!(directory, Path::new("runner's temp"));
    assert_eq!(
        filter,
        "subtitles=filename='warp-recording-overlay-123.ass'"
    );
}

#[tokio::test]
async fn burns_subtitles_from_directory_with_filter_special_characters() {
    if Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .is_err()
    {
        return;
    }

    let root = std::env::temp_dir()
        .join(format!("warp-overlay-test-{}", uuid::Uuid::new_v4()))
        .join("runner's temp");
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("cut.mp4");
    let ass = root.join("warp-recording-overlay-123.ass");
    let source = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:rate=1:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&input)
        .output()
        .await
        .unwrap();
    assert!(
        source.status.success(),
        "{}",
        String::from_utf8_lossy(&source.stderr)
    );
    std::fs::write(
        &ass,
        "[Script Info]\n\
         ScriptType: v4.00+\n\
         PlayResX: 64\n\
         PlayResY: 64\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, \
         BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, \
         BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,Arial,12,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,\
         100,0,0,1,1,0,2,10,10,10,1\n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
         Dialogue: 0,0:00:00.00,0:00:00.50,Default,,0,0,0,,ok\n",
    )
    .unwrap();

    let output = burn_overlays_into_cut(&input, &ass, 1).await.unwrap();

    assert!(output.exists());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}
#[tokio::test]
async fn rejects_recordings_without_qualifying_segments_before_running_ffmpeg() {
    let error = post_process_recording(
        Path::new("unused.mp4"),
        &[],
        (1280, 720),
        Duration::from_secs(10),
        15,
    )
    .await
    .unwrap_err();

    assert!(matches!(error, RecordingError::Finalize { .. }));
    assert!(error.to_string().contains("no qualifying action segments"));
}
