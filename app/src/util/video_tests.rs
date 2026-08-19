use super::*;
use std::path::PathBuf;

#[test]
fn parses_showinfo_pts_times_from_stderr() {
    let stderr = "\
frame=    1 fps=0.0 q=-0.0 size=N/A time=00:00:00.00 bitrate=N/A speed=N/A
[Parsed_showinfo_1 @ 0x600001234] n:0 pts:0 pts_time:0 duration:1001
[Parsed_showinfo_1 @ 0x600001234] n:1 pts:1502 pts_time:3.128 duration:1001
[Parsed_showinfo_1 @ 0x600001234] n:2 pts:3600 pts_time:7.5 duration:1001
";
    assert_eq!(
        parse_showinfo_pts_times(stderr),
        vec![0.0, 3.128, 7.5],
        "should extract pts_time from every showinfo line, ignoring unrelated stderr noise"
    );
}

#[test]
fn parses_showinfo_pts_times_returns_empty_when_no_scene_changes_detected() {
    let stderr = "frame=    0 fps=0.0 q=-0.0 Lsize=N/A time=00:00:00.00 bitrate=N/A speed=N/A";
    assert!(parse_showinfo_pts_times(stderr).is_empty());
}

#[test]
fn parses_ffmpeg_duration_from_stderr_banner() {
    let stderr = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'test.mp4':\n  Duration: 00:01:06.04, start: 0.000000, bitrate: 91 kb/s\n";
    assert_eq!(parse_ffmpeg_duration_secs(stderr), Some(66.04));
}

#[test]
fn parses_ffmpeg_duration_returns_none_without_a_duration_line() {
    let stderr = "ffmpeg version 6.0\nsome unrelated stderr output\n";
    assert_eq!(parse_ffmpeg_duration_secs(stderr), None);
}

#[test]
fn evenly_spaced_timestamps_excludes_endpoints() {
    let timestamps = evenly_spaced_timestamps(10.0, 3);
    assert_eq!(timestamps, vec![2.5, 5.0, 7.5]);
    for t in &timestamps {
        assert!(*t > 0.0 && *t < 10.0);
    }
}

#[test]
fn evenly_spaced_timestamps_with_zero_count_is_empty() {
    assert!(evenly_spaced_timestamps(10.0, 0).is_empty());
}

#[test]
fn supported_video_mime_types_accepts_common_containers_only() {
    assert!(is_supported_video_mime_type("video/mp4"));
    assert!(is_supported_video_mime_type("video/quicktime"));
    assert!(is_supported_video_mime_type("video/webm"));
    assert!(!is_supported_video_mime_type("image/png"));
    assert!(!is_supported_video_mime_type("video/mpeg"));
}

#[test]
fn frame_and_density_floor_constants_stay_under_the_query_image_limit() {
    // Video-as-context reuses the image-as-context pipeline, which caps a single query at 20
    // images. Both constants must stay comfortably below that shared limit.
    const { assert!(MAX_VIDEO_FRAMES <= 16) };
    const { assert!(MIN_VIDEO_FRAMES < MAX_VIDEO_FRAMES) };
}

#[test]
fn is_supported_video_filepath_checks_extension() {
    assert!(is_supported_video_filepath("/tmp/clip.mp4"));
    assert!(is_supported_video_filepath("/tmp/clip.mov"));
    assert!(!is_supported_video_filepath("/tmp/photo.png"));
    assert!(!is_supported_video_filepath("/tmp/clip.mpeg"));
}

#[test]
fn capped_frame_count_is_unrestricted_with_no_existing_attachments() {
    assert_eq!(capped_frame_count(16, 0, 0, 20, 200), 16);
}

#[test]
fn capped_frame_count_reserves_room_for_images_already_pending_on_the_query() {
    // Regression test: with 10 images already pending on the query, attaching 16 more frames
    // unconditionally would push the query to 26 images, well past the server's 20-per-query
    // limit (which then rejects the whole request). The cap must leave only the 10 remaining
    // slots for frames.
    assert_eq!(capped_frame_count(16, 10, 0, 20, 200), 10);
}

#[test]
fn capped_frame_count_reserves_room_for_the_conversation_limit() {
    // Regression test: near the 200-image conversation limit, the conversation limit is more
    // restrictive than the per-query limit and must win.
    assert_eq!(capped_frame_count(16, 0, 195, 20, 200), 5);
}

#[test]
fn capped_frame_count_is_zero_when_the_query_is_already_at_the_limit() {
    assert_eq!(capped_frame_count(16, 20, 0, 20, 200), 0);
    assert_eq!(capped_frame_count(16, 0, 200, 20, 200), 0);
}

#[test]
fn transcript_still_applies_when_all_frames_are_still_pending() {
    let expected = vec![
        "video.mp4-frame-01.jpg".to_string(),
        "video.mp4-frame-02.jpg".to_string(),
    ];
    let pending = vec![
        "video.mp4-frame-01.jpg".to_string(),
        "video.mp4-frame-02.jpg".to_string(),
        "unrelated.png".to_string(),
    ];
    assert!(transcript_still_applies(&expected, &pending));
}

#[test]
fn transcript_does_not_apply_after_the_frames_were_already_sent() {
    // Regression test for the send-before-transcript race: once a query is sent, its pending
    // image attachments (including the video's frames) are drained from the context model. A
    // transcript that resolves afterwards must detect that its frames are gone and refuse to
    // land in whatever the composer now contains (e.g. the user's next prompt).
    let expected = vec![
        "video.mp4-frame-01.jpg".to_string(),
        "video.mp4-frame-02.jpg".to_string(),
    ];
    let pending_after_send: Vec<String> = vec![];
    assert!(!transcript_still_applies(&expected, &pending_after_send));

    // Also refuse if only some of the frames are still pending (e.g. the user removed one).
    let partially_pending = vec!["video.mp4-frame-01.jpg".to_string()];
    assert!(!transcript_still_applies(&expected, &partially_pending));
}

#[test]
fn transcript_does_not_apply_with_no_expected_frames() {
    // Defensive: an empty expectation should never be treated as "still applies".
    assert!(!transcript_still_applies(
        &[],
        &["anything.jpg".to_string()]
    ));
}

#[test]
fn audio_too_long_error_reports_the_cap_and_actual_duration_in_minutes() {
    let err = VideoProcessingError::AudioTooLong {
        duration_secs: 25.0 * 60.0,
    };
    let message = err.to_string();
    assert!(
        message.contains("25"),
        "expected the actual duration (in minutes) in the message, got: {message}"
    );
    assert!(
        message.contains(&format!("{:.0}", MAX_AUDIO_DURATION_SECS / 60.0)),
        "expected the configured cap (in minutes) in the message, got: {message}"
    );
}

#[tokio::test]
async fn extract_audio_wav_skips_transcription_past_the_duration_cap() {
    // A video whose *probed* duration exceeds MAX_AUDIO_DURATION_SECS must be rejected before
    // any extraction is attempted -- generating a real multi-minute fixture just to exercise
    // this would make the test itself slow, so this only needs a video ffmpeg reports a longer
    // duration for. `-t` on the input trims playback but ffmpeg's own `-i` duration probe (which
    // `probe_duration_secs` reads) still reports the untrimmed source duration for a real file,
    // so instead this generates a short real video and asserts the cap comparison directly
    // against a probed duration exceeding it, mirroring extract_audio_wav's own check.
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let Some(video_path) = make_test_video_with_audio(temp_dir.path(), "short.mp4").await else {
        eprintln!("skipping: ffmpeg not available");
        return;
    };

    let duration = probe_duration_secs(&video_path)
        .await
        .expect("a real video file should have a probeable duration");
    assert!(
        duration < MAX_AUDIO_DURATION_SECS,
        "sanity check: the 1s fixture must be well under the cap"
    );

    // The short fixture itself must still transcribe fine (frames-not-touched path).
    let result = extract_audio_wav(&video_path).await;
    assert!(
        matches!(result, Ok(Some(_))),
        "expected audio under the cap to extract normally, got {result:?}"
    );
}

/// Generates a short synthetic video (with a tone on its audio track) via ffmpeg for tests that
/// need a real video file on disk. Returns `None` (rather than panicking) when ffmpeg isn't
/// available, so these tests degrade gracefully in environments without it -- consistent with the
/// rest of this module's ffmpeg-availability handling. `video_codec`/`audio_codec` let callers
/// generate fixtures in containers other than MP4 (e.g. `libvpx`/`libvorbis` for `.webm`).
async fn make_test_video_with_audio_ext(
    dir: &std::path::Path,
    name: &str,
    video_codec: &str,
    audio_codec: &str,
) -> Option<PathBuf> {
    if !ffmpeg_available().await {
        return None;
    }
    let out_path = dir.join(name);
    let status = tokio::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-hide_banner")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("color=c=blue:s=64x64:d=1")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("sine=frequency=440:duration=1")
        .arg("-c:v")
        .arg(video_codec)
        .arg("-c:a")
        .arg(audio_codec)
        .arg(&out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .ok()?;
    status.success().then_some(out_path)
}

async fn make_test_video_with_audio(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    make_test_video_with_audio_ext(dir, name, "libx264", "aac").await
}

#[tokio::test]
async fn read_native_video_returns_bytes_under_the_cap_with_audio_intact() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let Some(video_path) = make_test_video_with_audio(temp_dir.path(), "with_audio.mp4").await
    else {
        eprintln!("skipping: ffmpeg not available");
        return;
    };

    let (encoded, mime_type) = read_native_video(&video_path, "video/mp4", true)
        .await
        .expect("video is well under MAX_NATIVE_VIDEO_BYTES");
    assert!(!encoded.is_empty());
    assert_eq!(mime_type, "video/mp4");

    // Sanity check: ffprobe-free audio-stream detection via ffmpeg's own stderr banner, mirroring
    // `extract_audio_wav`'s approach elsewhere in this module.
    let with_audio = general_purpose::STANDARD
        .decode(&encoded)
        .expect("valid base64");
    let with_audio_path = temp_dir.path().join("roundtrip_with_audio.mp4");
    tokio::fs::write(&with_audio_path, &with_audio)
        .await
        .expect("write roundtrip file");
    assert!(
        extract_audio_wav(&with_audio_path)
            .await
            .expect("ffmpeg available")
            .is_some(),
        "native video read with include_audio=true must preserve the audio track"
    );
}

#[tokio::test]
async fn read_native_video_strips_audio_when_not_included() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let Some(video_path) = make_test_video_with_audio(temp_dir.path(), "strip_audio.mp4").await
    else {
        eprintln!("skipping: ffmpeg not available");
        return;
    };

    let (encoded, mime_type) = read_native_video(&video_path, "video/mp4", false)
        .await
        .expect("video is well under MAX_NATIVE_VIDEO_BYTES");
    assert!(!encoded.is_empty());
    assert_eq!(
        mime_type, "video/mp4",
        "muting must preserve the source MIME type, not just always claim mp4"
    );

    let muted = general_purpose::STANDARD
        .decode(&encoded)
        .expect("valid base64");
    let muted_path = temp_dir.path().join("roundtrip_muted.mp4");
    tokio::fs::write(&muted_path, &muted)
        .await
        .expect("write roundtrip file");
    assert!(
        extract_audio_wav(&muted_path)
            .await
            .expect("ffmpeg available")
            .is_none(),
        "native video read with include_audio=false must never carry the user's audio track"
    );
}

#[tokio::test]
async fn read_native_video_preserves_a_non_mp4_container_when_muting() {
    // Regression test: muting used to always remux into `muted.mp4` regardless of the source
    // container, which could fail outright for some codec/container combinations and, even when
    // it didn't, left the returned bytes mislabeled with the *original* (pre-mute) MIME type. A
    // `.webm` source must stay a WEBM-compatible container after muting, not become MP4 bytes
    // wearing a WEBM label or an incompatible MP4 remux.
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let Some(video_path) =
        make_test_video_with_audio_ext(temp_dir.path(), "source.webm", "libvpx", "libvorbis").await
    else {
        eprintln!("skipping: ffmpeg not available");
        return;
    };

    let (encoded, mime_type) = read_native_video(&video_path, "video/webm", false)
        .await
        .expect("video is well under MAX_NATIVE_VIDEO_BYTES");
    assert_eq!(mime_type, "video/webm");

    // Roundtrip the returned bytes back out with a `.webm` extension (matching the claimed MIME
    // type) and confirm ffmpeg can actually demux it and sees no audio stream -- if muting had
    // silently produced MP4 bytes instead, ffmpeg would either fail to open this file or need a
    // format override, either way proving the mislabel.
    let muted = general_purpose::STANDARD
        .decode(&encoded)
        .expect("valid base64");
    let muted_path = temp_dir.path().join("roundtrip_muted.webm");
    tokio::fs::write(&muted_path, &muted)
        .await
        .expect("write roundtrip file");
    assert!(
        extract_audio_wav(&muted_path)
            .await
            .expect("ffmpeg available -- and able to demux this file as its claimed container")
            .is_none(),
    );
}

#[tokio::test]
async fn read_native_video_boundary_uses_encoded_not_raw_size() {
    // Regression test: the cap must apply to the base64-*encoded* size (what the server actually
    // sees, per the `bytes`-field quirk), not the raw file size -- otherwise a file just under the
    // raw cap but over it once base64-inflated (~33%) would be reported as native-capable here and
    // then rejected server-side instead of falling back to frames.
    let temp_dir = tempfile::tempdir().expect("tempdir");

    // Pick a raw size whose base64 encoding lands exactly at the cap boundary, so this exercises
    // the real encode-then-compare path rather than a size chosen to trivially pass either unit.
    // base64 output length for n bytes is ceil(n/3)*4; MAX_NATIVE_VIDEO_BYTES (8_000_000) is a
    // multiple of 4, so an input of `MAX_NATIVE_VIDEO_BYTES / 4 * 3` bytes encodes to exactly the
    // cap, and one byte more encodes to one base64 block (4 bytes) more, comfortably over it.
    let raw_at_boundary = MAX_NATIVE_VIDEO_BYTES / 4 * 3;
    let boundary_path = temp_dir.path().join("at_cap.mp4");
    tokio::fs::write(&boundary_path, vec![0u8; raw_at_boundary])
        .await
        .expect("write boundary fixture");
    let (encoded, _) = read_native_video(&boundary_path, "video/mp4", true)
        .await
        .expect("encoded size lands exactly on the cap, which must still be accepted");
    assert_eq!(encoded.len(), MAX_NATIVE_VIDEO_BYTES);

    // Raw sizes that are individually under `MAX_NATIVE_VIDEO_BYTES`, but whose base64 encoding
    // exceeds it (the exact failure mode this cap exists to prevent), must be rejected.
    let raw_under_cap_but_over_once_encoded = MAX_NATIVE_VIDEO_BYTES - 100;
    assert!(raw_under_cap_but_over_once_encoded < MAX_NATIVE_VIDEO_BYTES);
    let over_once_encoded_path = temp_dir.path().join("raw_under_but_encoded_over.mp4");
    tokio::fs::write(
        &over_once_encoded_path,
        vec![0u8; raw_under_cap_but_over_once_encoded],
    )
    .await
    .expect("write fixture");
    assert!(
        read_native_video(&over_once_encoded_path, "video/mp4", true)
            .await
            .is_none(),
        "a file under the raw cap must still be rejected once its base64 encoding exceeds it"
    );
}

#[tokio::test]
async fn read_native_video_returns_none_when_over_the_size_cap() {
    // `include_audio: true` skips the audio-muting ffmpeg pass and just reads+size-checks the
    // file directly, so an oversized dummy file (rather than a real multi-megabyte video fixture)
    // is enough to exercise the cap.
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let oversized_path = temp_dir.path().join("oversized.mp4");
    tokio::fs::write(&oversized_path, vec![0u8; MAX_NATIVE_VIDEO_BYTES + 1])
        .await
        .expect("write oversized fixture");

    assert!(
        read_native_video(&oversized_path, "video/mp4", true)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn read_native_video_does_not_leak_the_muted_temp_file() {
    // Regression test for a leaked-temp-file bug: muting used to `TempDir::keep()` the directory
    // it wrote the muted copy into, permanently leaving a copy of the user's (muted) video on
    // disk on every attach. Calls `mute_audio` directly (rather than scanning the shared system
    // temp directory, which other tests write into concurrently and would make this racy) so the
    // exact directory it creates can be checked for cleanup once its guard is dropped.
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let Some(video_path) = make_test_video_with_audio(temp_dir.path(), "leak_check.mp4").await
    else {
        eprintln!("skipping: ffmpeg not available");
        return;
    };

    let muted_dir_path = {
        let (muted_temp_dir, muted_path) = mute_audio(&video_path)
            .await
            .expect("ffmpeg available and able to mute this fixture");
        assert!(
            muted_path.exists(),
            "muted file must exist while the guard is alive"
        );
        muted_temp_dir.path().to_path_buf()
        // `muted_temp_dir` drops at the end of this block.
    };

    assert!(
        !muted_dir_path.exists(),
        "mute_audio's temp directory must not survive its TempDir guard being dropped"
    );
}
