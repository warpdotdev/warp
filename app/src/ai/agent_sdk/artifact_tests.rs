use std::path::PathBuf;

use warp_cli::agent::OutputFormat;

use super::*;

fn sample_completed_upload() -> CompletedFileArtifactUpload {
    CompletedFileArtifactUpload {
        artifact: sample_artifact_record(),
        size_bytes: 42,
    }
}

fn sample_artifact_record() -> FileArtifactRecord {
    FileArtifactRecord {
        artifact_uid: "artifact-123".to_string(),
        filepath: "outputs/report.txt".to_string(),
        description: Some("daily summary".to_string()),
        mime_type: "text/plain".to_string(),
        size_bytes: Some(42),
    }
}

fn sample_file_download_response() -> ArtifactDownloadResponse {
    serde_json::from_str(
        r#"{
            "artifact_uid": "artifact-123",
            "artifact_type": "FILE",
            "created_at": "2024-01-15T10:30:00Z",
            "data": {
                "download_url": "https://storage.example.com/report.txt",
                "expires_at": "2024-01-15T11:30:00Z",
                "content_type": "text/plain",
                "filepath": "outputs/report.txt",
                "filename": "report.txt",
                "description": "daily summary",
                "size_bytes": 42
            }
        }"#,
    )
    .unwrap()
}

fn sample_screenshot_download_response() -> ArtifactDownloadResponse {
    serde_json::from_str(
        r#"{
            "artifact_uid": "screenshot-123",
            "artifact_type": "SCREENSHOT",
            "created_at": "2024-01-15T10:30:00Z",
            "data": {
                "download_url": "https://storage.example.com/screenshot.png",
                "expires_at": "2024-01-15T11:30:00Z",
                "content_type": "image/png",
                "description": "dashboard screenshot"
            }
        }"#,
    )
    .unwrap()
}

#[test]
fn write_get_output_to_writes_json_output() {
    let mut output = Vec::new();

    write_get_output_to(
        &mut output,
        &sample_file_download_response(),
        OutputFormat::Json,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "{\"artifact_uid\":\"artifact-123\",\"artifact_type\":\"FILE\",\"created_at\":\"2024-01-15T10:30:00+00:00\",\"download_url\":\"https://storage.example.com/report.txt\",\"expires_at\":\"2024-01-15T11:30:00+00:00\",\"content_type\":\"text/plain\",\"filepath\":\"outputs/report.txt\",\"filename\":\"report.txt\",\"description\":\"daily summary\",\"size_bytes\":42}\n"
    );
}

#[test]
fn write_get_output_to_writes_ndjson_output() {
    let mut output = Vec::new();

    write_get_output_to(
        &mut output,
        &sample_file_download_response(),
        OutputFormat::Ndjson,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "{\"artifact_uid\":\"artifact-123\",\"artifact_type\":\"FILE\",\"created_at\":\"2024-01-15T10:30:00+00:00\",\"download_url\":\"https://storage.example.com/report.txt\",\"expires_at\":\"2024-01-15T11:30:00+00:00\",\"content_type\":\"text/plain\",\"filepath\":\"outputs/report.txt\",\"filename\":\"report.txt\",\"description\":\"daily summary\",\"size_bytes\":42}\n"
    );
}

#[test]
fn write_get_output_to_writes_pretty_output() {
    let mut output = Vec::new();

    write_get_output_to(
        &mut output,
        &sample_file_download_response(),
        OutputFormat::Pretty,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "Artifact UID: artifact-123\nArtifact type: FILE\nCreated at: 2024-01-15T10:30:00+00:00\nDownload URL: https://storage.example.com/report.txt\nExpires at: 2024-01-15T11:30:00+00:00\nContent type: text/plain\nFilepath: outputs/report.txt\nFilename: report.txt\nDescription: daily summary\nSize bytes: 42\n"
    );
}

#[test]
fn write_get_output_to_writes_text_output() {
    let mut output = Vec::new();

    write_get_output_to(
        &mut output,
        &sample_file_download_response(),
        OutputFormat::Text,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "Artifact UID\tArtifact type\tCreated at\tDownload URL\tExpires at\tContent type\tFilepath\tFilename\tDescription\tSize bytes\nartifact-123\tFILE\t2024-01-15T10:30:00+00:00\thttps://storage.example.com/report.txt\t2024-01-15T11:30:00+00:00\ttext/plain\toutputs/report.txt\treport.txt\tdaily summary\t42\n"
    );
}

#[test]
fn write_download_output_to_writes_pretty_output() {
    let artifact = sample_file_download_response();
    let path = std::path::absolute("report.txt").unwrap();
    let output_record = DownloadArtifactOutput::new(&artifact, path.clone());
    let mut output = Vec::new();

    write_download_output_to(&mut output, &output_record, OutputFormat::Pretty).unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        format!(
            "Artifact downloaded\nArtifact UID: artifact-123\nArtifact type: FILE\nPath: {}\n",
            path.display()
        )
    );
}

#[test]
fn download_destination_uses_explicit_path() {
    assert_eq!(
        download_destination(
            &sample_file_download_response(),
            Some(PathBuf::from("downloads/report.txt"))
        ),
        PathBuf::from("downloads/report.txt")
    );
}

#[test]
fn download_destination_defaults_to_file_artifact_filename() {
    assert_eq!(
        download_destination(&sample_file_download_response(), None),
        PathBuf::from("report.txt")
    );
}

#[test]
fn download_destination_defaults_screenshot_to_artifact_uid_with_extension() {
    assert_eq!(
        download_destination(&sample_screenshot_download_response(), None),
        PathBuf::from("artifact-screenshot-123.png")
    );
}

#[test]
fn download_destination_defaults_pdf_to_artifact_uid_with_extension() {
    let artifact: ArtifactDownloadResponse = serde_json::from_str(
        r#"{
            "artifact_uid": "artifact-pdf-123",
            "artifact_type": "FILE",
            "created_at": "2024-01-15T10:30:00Z",
            "data": {
                "download_url": "https://storage.example.com/report.pdf",
                "expires_at": "2024-01-15T11:30:00Z",
                "content_type": "application/pdf",
                "filepath": "outputs/report.pdf",
                "filename": "",
                "description": "pdf report",
                "size_bytes": 42
            }
        }"#,
    )
    .unwrap();

    assert_eq!(
        download_destination(&artifact, None),
        PathBuf::from("artifact-artifact-pdf-123.pdf")
    );
}

#[test]
fn write_upload_output_to_writes_json_output() {
    let mut output = Vec::new();

    write_upload_output_to(&mut output, &sample_completed_upload(), OutputFormat::Json).unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "{\"artifact_uid\":\"artifact-123\",\"filepath\":\"outputs/report.txt\",\"description\":\"daily summary\",\"mime_type\":\"text/plain\",\"size_bytes\":42}\n"
    );
}

#[test]
fn write_upload_output_to_writes_ndjson_output() {
    let mut output = Vec::new();

    write_upload_output_to(
        &mut output,
        &sample_completed_upload(),
        OutputFormat::Ndjson,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "{\"artifact_uid\":\"artifact-123\",\"filepath\":\"outputs/report.txt\",\"description\":\"daily summary\",\"mime_type\":\"text/plain\",\"size_bytes\":42}\n"
    );
}

#[test]
fn write_upload_output_to_writes_pretty_output() {
    let mut output = Vec::new();

    write_upload_output_to(
        &mut output,
        &sample_completed_upload(),
        OutputFormat::Pretty,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "Artifact uploaded\nArtifact UID: artifact-123\nFilepath: outputs/report.txt\nDescription: daily summary\nMIME type: text/plain\nSize bytes: 42\n"
    );
}

#[test]
fn write_upload_output_to_writes_text_output() {
    let mut output = Vec::new();

    write_upload_output_to(&mut output, &sample_completed_upload(), OutputFormat::Text).unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "Artifact UID\tFilepath\tDescription\tMIME type\tSize bytes\nartifact-123\toutputs/report.txt\tdaily summary\ttext/plain\t42\n"
    );
}
