use std::path::Path;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;

#[cfg(feature = "local_fs")]
use crate::server::server_api::ai::ArtifactDownloadResponse;

pub(crate) fn sanitized_basename(path_or_filename: &str) -> Option<String> {
    let file_name = Path::new(path_or_filename).file_name()?.to_str()?;
    if file_name.is_empty() {
        return None;
    }
    Some(file_name.to_string())
}

#[cfg(feature = "local_fs")]
pub(crate) fn extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/gif" => Some("gif"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "application/json" => Some("json"),
        "application/pdf" => Some("pdf"),
        "text/csv" => Some("csv"),
        "text/plain" => Some("txt"),
        "text/markdown" => Some("md"),
        "text/html" => Some("html"),
        _ => None,
    }
}

#[cfg(feature = "local_fs")]
pub(crate) fn default_download_filename(artifact: &ArtifactDownloadResponse) -> String {
    if let Some(filename) = artifact.filename().and_then(sanitized_basename) {
        return filename;
    }

    let extension = extension_for_content_type(artifact.content_type())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    format!("artifact-{}{}", artifact.artifact_uid(), extension)
}

#[cfg(feature = "local_fs")]
pub(crate) fn download_destination(
    artifact: &ArtifactDownloadResponse,
    explicit_path: Option<PathBuf>,
) -> PathBuf {
    explicit_path.unwrap_or_else(|| PathBuf::from(default_download_filename(artifact)))
}

#[cfg(feature = "local_fs")]
pub(crate) fn default_download_directory() -> Option<PathBuf> {
    dirs::download_dir()
}

#[cfg(feature = "local_fs")]
pub(crate) async fn download_artifact_bytes(
    http_client: &http_client::Client,
    artifact: &ArtifactDownloadResponse,
    path: &Path,
) -> anyhow::Result<()> {
    use std::time::Duration;

    use anyhow::{anyhow, Context as _};
    use futures::TryStreamExt as _;
    use tokio_util::io::StreamReader;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await.with_context(|| {
            format!("Failed to create download directory '{}'", parent.display())
        })?;
    }

    let response = http_client
        .get(artifact.download_url())
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .with_context(|| {
            format!(
                "Failed to download artifact '{}' from signed URL",
                artifact.artifact_uid()
            )
        })?;
    let response = response
        .error_for_status()
        .map_err(|err| anyhow!("Artifact download failed: {err}"))?;

    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("Failed to create download file '{}'", path.display()))?;
    let mut response_stream =
        StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
    tokio::io::copy(&mut response_stream, &mut file)
        .await
        .with_context(|| format!("Failed to write download file '{}'", path.display()))?;
    file.sync_data()
        .await
        .with_context(|| format!("Failed to sync download file '{}'", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "local_fs")]
    use crate::server::server_api::ai::{
        ArtifactDownloadCommonFields, FileArtifactResponseData, ScreenshotArtifactResponseData,
    };
    #[cfg(feature = "local_fs")]
    use chrono::{TimeZone, Utc};

    #[cfg(feature = "local_fs")]
    fn sample_file_download_response(filename: &str, filepath: &str) -> ArtifactDownloadResponse {
        ArtifactDownloadResponse::File {
            common: ArtifactDownloadCommonFields {
                artifact_uid: "artifact-123".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap(),
            },
            data: FileArtifactResponseData {
                download_url: "https://storage.example.com/report.txt".to_string(),
                expires_at: Utc.with_ymd_and_hms(2024, 1, 15, 11, 30, 0).unwrap(),
                content_type: "text/plain".to_string(),
                filepath: filepath.to_string(),
                filename: filename.to_string(),
                description: Some("daily summary".to_string()),
                size_bytes: Some(42),
            },
        }
    }

    #[test]
    fn sanitized_basename_accepts_plain_filename() {
        assert_eq!(
            sanitized_basename("report.txt"),
            Some("report.txt".to_string())
        );
    }

    #[test]
    fn sanitized_basename_extracts_from_path() {
        assert_eq!(
            sanitized_basename("outputs/report.txt"),
            Some("report.txt".to_string())
        );
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn extension_for_content_type_recognizes_image_jpg_alias() {
        assert_eq!(extension_for_content_type("image/jpg"), Some("jpg"));
        assert_eq!(extension_for_content_type("image/jpeg"), Some("jpg"));
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn default_download_filename_prefers_server_filename() {
        assert_eq!(
            default_download_filename(&sample_file_download_response(
                "report.txt",
                "outputs/ignored.txt"
            )),
            "report.txt"
        );
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn default_download_filename_uses_content_type_extension_when_filename_missing() {
        assert_eq!(
            default_download_filename(&sample_file_download_response("", "outputs/report.txt")),
            "artifact-artifact-123.txt"
        );
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn default_download_filename_omits_extension_when_content_type_unknown() {
        let artifact = ArtifactDownloadResponse::Screenshot {
            common: ArtifactDownloadCommonFields {
                artifact_uid: "screenshot-123".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap(),
            },
            data: ScreenshotArtifactResponseData {
                download_url: "https://storage.example.com/screenshot".to_string(),
                expires_at: Utc.with_ymd_and_hms(2024, 1, 15, 11, 30, 0).unwrap(),
                content_type: "application/octet-stream".to_string(),
                description: Some("dashboard screenshot".to_string()),
            },
        };

        assert_eq!(
            default_download_filename(&artifact),
            "artifact-screenshot-123"
        );
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn download_destination_uses_explicit_path() {
        assert_eq!(
            download_destination(
                &sample_file_download_response("report.txt", "outputs/report.txt"),
                Some(PathBuf::from("downloads/report.txt"))
            ),
            PathBuf::from("downloads/report.txt")
        );
    }
}
