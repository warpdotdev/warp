use anyhow::anyhow;
use chrono::{TimeZone, Utc};

use super::*;
#[cfg(feature = "local_fs")]
use crate::ai::artifact_download::default_download_filename;
use crate::server::server_api::ai::{
    ArtifactDownloadCommonFields, FileArtifactResponseData, ScreenshotArtifactResponseData,
};

#[test]
fn test_parse_github_pr_url() {
    assert_eq!(
        parse_github_pr_url("https://github.com/owner/repo/pull/123"),
        Some(("repo".to_string(), 123))
    );
    assert_eq!(
        parse_github_pr_url("https://github.com/my-org/my-repo/pull/456"),
        Some(("my-repo".to_string(), 456))
    );
    assert_eq!(
        parse_github_pr_url("https://github.com/my-org/my-repo"),
        None
    );
    assert_eq!(parse_github_pr_url("not a url"), None);
}

#[test]
fn skips_lightbox_update_for_non_screenshot_artifact() {
    let image = screenshot_lightbox_image_from_download_result(
        Ok(ArtifactDownloadResponse::File {
            common: ArtifactDownloadCommonFields {
                artifact_uid: "artifact-123".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap(),
            },
            data: FileArtifactResponseData {
                download_url: "https://storage.example.com/report.txt".to_string(),
                expires_at: Utc.with_ymd_and_hms(2024, 1, 15, 11, 30, 0).unwrap(),
                content_type: "text/plain".to_string(),
                filepath: "outputs/report.txt".to_string(),
                filename: "report.txt".to_string(),
                description: Some("daily summary".to_string()),
                size_bytes: Some(42),
            },
        }),
        "artifact-123",
        0,
    );

    assert!(image.is_none());
}

#[test]
fn returns_failure_placeholder_for_screenshot_load_errors() {
    let image = screenshot_lightbox_image_from_download_result(
        Err(anyhow!("network error")),
        "artifact-123",
        0,
    )
    .expect("expected failure placeholder");

    assert!(matches!(image.source, LightboxImageSource::Loading));
    assert_eq!(image.description.as_deref(), Some("Failed to load"));
}

#[test]
fn resolves_lightbox_image_for_screenshot_artifact() {
    let image = screenshot_lightbox_image_from_download_result(
        Ok(ArtifactDownloadResponse::Screenshot {
            common: ArtifactDownloadCommonFields {
                artifact_uid: "screenshot-123".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap(),
            },
            data: ScreenshotArtifactResponseData {
                download_url: "https://storage.example.com/screenshot.png".to_string(),
                expires_at: Utc.with_ymd_and_hms(2024, 1, 15, 11, 30, 0).unwrap(),
                content_type: "image/png".to_string(),
                description: Some("dashboard screenshot".to_string()),
            },
        }),
        "screenshot-123",
        0,
    )
    .expect("expected screenshot image");

    assert!(matches!(image.source, LightboxImageSource::Resolved { .. }));
    assert_eq!(image.description.as_deref(), Some("dashboard screenshot"));
}

#[test]
fn file_button_label_prefers_filename() {
    assert_eq!(
        file_button_label("report.txt", "outputs/other.txt"),
        "report.txt"
    );
}

#[test]
fn file_button_label_falls_back_to_filepath_basename() {
    assert_eq!(file_button_label("", "outputs/report.txt"), "report.txt");
}

#[test]
fn file_button_label_falls_back_to_generic_label() {
    assert_eq!(file_button_label("", ""), "File");
}

#[test]
#[cfg(feature = "local_fs")]
fn default_download_filename_prefers_server_filename() {
    assert_eq!(
        default_download_filename(&ArtifactDownloadResponse::File {
            common: ArtifactDownloadCommonFields {
                artifact_uid: "artifact-123".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap(),
            },
            data: FileArtifactResponseData {
                download_url: "https://storage.example.com/report.txt".to_string(),
                expires_at: Utc.with_ymd_and_hms(2024, 1, 15, 11, 30, 0).unwrap(),
                content_type: "text/plain".to_string(),
                filepath: "outputs/report.txt".to_string(),
                filename: "report.txt".to_string(),
                description: Some("daily summary".to_string()),
                size_bytes: Some(42),
            },
        }),
        "report.txt"
    );
}

#[test]
#[cfg(feature = "local_fs")]
fn default_download_filename_falls_back_to_artifact_uid_with_extension() {
    assert_eq!(
        default_download_filename(&ArtifactDownloadResponse::File {
            common: ArtifactDownloadCommonFields {
                artifact_uid: "artifact-123".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap(),
            },
            data: FileArtifactResponseData {
                download_url: "https://storage.example.com/report.txt".to_string(),
                expires_at: Utc.with_ymd_and_hms(2024, 1, 15, 11, 30, 0).unwrap(),
                content_type: "text/plain".to_string(),
                filepath: "outputs/report.txt".to_string(),
                filename: "".to_string(),
                description: Some("daily summary".to_string()),
                size_bytes: Some(42),
            },
        }),
        "artifact-artifact-123.txt"
    );
}

#[test]
fn converts_graphql_file_artifact() {
    let artifact = Artifact::try_from(warp_graphql::ai::AIConversationArtifact::FileArtifact(
        warp_graphql::ai::FileArtifact {
            artifact_uid: "artifact-file-1".into(),
            filepath: "outputs/report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            description: Some("Daily summary".to_string()),
            size_bytes: Some(42),
        },
    ))
    .expect("expected file artifact conversion");

    assert_eq!(
        artifact,
        Artifact::File {
            artifact_uid: "artifact-file-1".to_string(),
            filepath: "outputs/report.txt".to_string(),
            filename: "report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            description: Some("Daily summary".to_string()),
            size_bytes: Some(42),
        }
    );
}
