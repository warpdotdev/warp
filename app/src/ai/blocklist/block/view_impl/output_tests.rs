use ai::agent::action::UploadArtifactRequest;
use warpui::App;

use crate::ai::agent::UploadArtifactResult;
use crate::test_util::settings::initialize_settings_for_tests;

use super::format_upload_artifact_text;

#[test]
fn format_upload_artifact_text_includes_request_details() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let request = UploadArtifactRequest {
            file_path: "reports/daily.txt".to_string(),
            description: Some("Daily summary".to_string()),
        };

        let text = app.read(|_| format_upload_artifact_text(&request, None));

        assert_eq!(
            text,
            "Upload artifact: reports/daily.txt\nDescription: Daily summary"
        );
    });
}

#[test]
fn format_upload_artifact_text_includes_success_summary() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let request = UploadArtifactRequest {
            file_path: "reports/daily.txt".to_string(),
            description: Some("Daily summary".to_string()),
        };
        let result = UploadArtifactResult::Success {
            artifact_uid: "artifact-123".to_string(),
            filepath: Some("reports/daily.txt".to_string()),
            mime_type: "text/plain".to_string(),
            description: Some("Daily summary".to_string()),
            size_bytes: 128,
        };

        let text = app.read(|_| format_upload_artifact_text(&request, Some(&result)));

        assert_eq!(
            text,
            "Upload artifact: reports/daily.txt\nDescription: Daily summary\nStatus: uploaded artifact artifact-123\nUploaded file: reports/daily.txt"
        );
    });
}

#[test]
fn format_upload_artifact_text_includes_terminal_status() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let request = UploadArtifactRequest {
            file_path: "reports/daily.txt".to_string(),
            description: None,
        };

        let error_text = app.read(|_| {
            format_upload_artifact_text(
                &request,
                Some(&UploadArtifactResult::Error(
                    "permission denied".to_string(),
                )),
            )
        });
        assert_eq!(
            error_text,
            "Upload artifact: reports/daily.txt\nStatus: upload failed: permission denied"
        );

        let cancelled_text = app.read(|_| {
            format_upload_artifact_text(&request, Some(&UploadArtifactResult::Cancelled))
        });
        assert_eq!(cancelled_text, "Upload artifact: reports/daily.txt");
    });
}
