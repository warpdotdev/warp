use std::ffi::OsString;
use std::path::Path;

use serde_json::json;

use super::*;

fn far_deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

fn strings(args: Vec<OsString>) -> Vec<String> {
    args.into_iter()
        .map(|arg| arg.into_string().unwrap())
        .collect()
}

#[test]
fn upload_args_request_json_output_for_the_run() {
    assert_eq!(
        strings(upload_args(
            Path::new("/tmp/video.mp4"),
            "run-1",
            Some("Sign-up flow"),
            Some("Creates an account")
        )),
        [
            "artifact",
            "upload",
            "/tmp/video.mp4",
            "--run-id",
            "run-1",
            "--output-format",
            "json",
            "--title",
            "Sign-up flow",
            "--description",
            "Creates an account",
        ]
    );
    assert_eq!(
        strings(upload_args(
            Path::new("/tmp/thumb.png"),
            "run-1",
            None,
            None
        ))
        .len(),
        7
    );
}

#[test]
fn artifact_uid_is_read_from_the_json_line() {
    let stdout = b"Uploading...\n{\"artifact_uid\":\"artifact-123\",\"filepath\":\"video.mp4\"}\n";
    assert_eq!(parse_artifact_uid(stdout).unwrap(), "artifact-123");
    assert!(parse_artifact_uid(b"Artifact uploaded\n").is_err());
}

#[tokio::test]
async fn saving_to_a_directory_moves_the_video() {
    let temp = tempfile::tempdir().unwrap();
    let video = temp.path().join("recording.mp4");
    std::fs::write(&video, b"video").unwrap();
    let directory = temp.path().join("recordings");

    let published = publish(
        &video,
        &RecordingDestination::Directory(directory.clone()),
        None,
        None,
        far_deadline(),
    )
    .await
    .unwrap();

    let saved = directory.join("recording.mp4");
    assert_eq!(published, json!({ "status": "saved", "path": saved }));
    assert!(!video.exists());
    assert_eq!(std::fs::read(saved).unwrap(), b"video");
}

/// Writes an executable that stands in for the Warp CLI: it records its arguments next to itself
/// and then runs `body`.
#[cfg(unix)]
fn fake_cli(directory: &Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let cli = directory.join("oz");
    std::fs::write(
        &cli,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$(dirname \"$0\")/args.txt\"\n{body}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    cli
}

#[cfg(unix)]
#[tokio::test]
async fn uploads_go_through_the_cli_for_the_run() {
    let temp = tempfile::tempdir().unwrap();
    let cli = fake_cli(
        temp.path(),
        r#"echo '{"artifact_uid":"artifact-123","filepath":"recording.mp4"}'"#,
    );
    let video = temp.path().join("recording.mp4");
    std::fs::write(&video, b"not a real video").unwrap();

    let published = publish(
        &video,
        &RecordingDestination::OzRun {
            cli,
            run_id: "run-1".to_owned(),
        },
        Some("Demo"),
        None,
        far_deadline(),
    )
    .await
    .unwrap();

    // The thumbnail can't be generated from a fake video, so only the video is uploaded.
    assert_eq!(
        published,
        json!({ "status": "uploaded", "artifact_uid": "artifact-123" })
    );
    assert!(!video.exists());
    let args = std::fs::read_to_string(temp.path().join("args.txt")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "artifact",
            "upload",
            video.to_str().unwrap(),
            "--run-id",
            "run-1",
            "--output-format",
            "json",
            "--title",
            "Demo",
        ]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn failed_uploads_keep_the_video() {
    let temp = tempfile::tempdir().unwrap();
    let cli = fake_cli(temp.path(), "echo 'not logged in' >&2\nexit 1");
    let video = temp.path().join("recording.mp4");
    std::fs::write(&video, b"video").unwrap();

    let error = publish(
        &video,
        &RecordingDestination::OzRun {
            cli,
            run_id: "run-1".to_owned(),
        },
        None,
        None,
        far_deadline(),
    )
    .await
    .unwrap_err();

    assert!(error.contains("not logged in"), "{error}");
    assert!(
        error.contains(&format!("kept at {}", video.display())),
        "{error}"
    );
    assert!(video.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn uploads_that_miss_the_deadline_keep_the_video() {
    let temp = tempfile::tempdir().unwrap();
    let cli = fake_cli(temp.path(), "sleep 30");
    let video = temp.path().join("recording.mp4");
    std::fs::write(&video, b"video").unwrap();

    let error = publish(
        &video,
        &RecordingDestination::OzRun {
            cli,
            run_id: "run-1".to_owned(),
        },
        None,
        None,
        Instant::now() + Duration::from_millis(200),
    )
    .await
    .unwrap_err();

    assert!(error.contains("Timed out uploading"), "{error}");
    assert!(
        error.contains(&format!("kept at {}", video.display())),
        "{error}"
    );
    assert!(video.exists());
}
