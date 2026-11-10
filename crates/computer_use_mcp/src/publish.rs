//! Publishing finished recordings.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use command::Stdio;
use command::r#async::Command;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::recording::RecordingDestination;

const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Publishes `video` to `destination`, returning how it was published. On failure the video is
/// left in place and its path is included in the error.
pub(crate) async fn publish(
    video: &Path,
    destination: &RecordingDestination,
    title: Option<&str>,
    description: Option<&str>,
) -> Result<Value, String> {
    let published = match destination {
        RecordingDestination::OzRun { cli, run_id } => {
            upload_to_run(video, cli, run_id, title, description).await
        }
        RecordingDestination::Directory(directory) => move_into(video, directory)
            .await
            .map(|path| json!({ "status": "saved", "path": path })),
    };
    published.map_err(|error| format!("{error} The recording was kept at {}.", video.display()))
}

async fn upload_to_run(
    video: &Path,
    cli: &Path,
    run_id: &str,
    title: Option<&str>,
    description: Option<&str>,
) -> Result<Value, String> {
    let artifact_uid = upload_artifact(cli, upload_args(video, run_id, title, description)).await?;
    let mut published = json!({ "status": "uploaded", "artifact_uid": artifact_uid });
    match upload_thumbnail(video, cli, run_id, &artifact_uid).await {
        Ok(thumbnail_uid) => published["thumbnail_artifact_uid"] = json!(thumbnail_uid),
        Err(error) => log::warn!("Failed to upload the recording's thumbnail: {error}"),
    }
    let _ = std::fs::remove_file(video);
    Ok(published)
}

/// Uploads a preview frame for the video uploaded as `video_uid`. The server pairs the two by the
/// thumbnail's `{video_uid}-thumb.png` file name.
async fn upload_thumbnail(
    video: &Path,
    cli: &Path,
    run_id: &str,
    video_uid: &str,
) -> Result<String, String> {
    let thumbnail = computer_use::generate_video_thumbnail(video, video_uid)
        .await
        .map_err(|error| error.to_string())?;
    let uploaded = upload_artifact(cli, upload_args(&thumbnail, run_id, None, None)).await;
    let _ = std::fs::remove_file(&thumbnail);
    uploaded
}

/// Arguments for `artifact upload`, which authenticates with the `WARP_API_KEY` in its
/// environment.
pub(crate) fn upload_args(
    file: &Path,
    run_id: &str,
    title: Option<&str>,
    description: Option<&str>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "artifact".into(),
        "upload".into(),
        file.into(),
        "--run-id".into(),
        run_id.into(),
        "--output-format".into(),
        "json".into(),
    ];
    for (flag, value) in [("--title", title), ("--description", description)] {
        if let Some(value) = value {
            args.extend([flag.into(), value.into()]);
        }
    }
    args
}

async fn upload_artifact(cli: &Path, args: Vec<OsString>) -> Result<String, String> {
    let mut command = Command::new(cli);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(UPLOAD_TIMEOUT, command.output())
        .await
        .map_err(|_| "Timed out uploading the recording.".to_owned())?
        .map_err(|error| format!("Failed to run the Warp CLI to upload the recording: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Uploading the recording failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_artifact_uid(&output.stdout)
}

/// Reads the uploaded artifact's ID from `artifact upload --output-format json` output.
pub(crate) fn parse_artifact_uid(stdout: &[u8]) -> Result<String, String> {
    #[derive(Deserialize)]
    struct UploadOutput {
        artifact_uid: String,
    }

    String::from_utf8_lossy(stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<UploadOutput>(line.trim()).ok())
        .map(|output| output.artifact_uid)
        .ok_or_else(|| {
            "The Warp CLI didn't report the uploaded recording's artifact ID.".to_owned()
        })
}

async fn move_into(video: &Path, directory: &Path) -> Result<PathBuf, String> {
    let saving_error = |error: std::io::Error| format!("Failed to save the recording: {error}.");
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(saving_error)?;
    let file_name = video
        .file_name()
        .ok_or_else(|| "The recording has no file name.".to_owned())?;
    let target = directory.join(file_name);
    // Renaming fails across filesystems, so fall back to copying.
    if tokio::fs::rename(video, &target).await.is_err() {
        tokio::fs::copy(video, &target)
            .await
            .map_err(saving_error)?;
        let _ = tokio::fs::remove_file(video).await;
    }
    Ok(target)
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
