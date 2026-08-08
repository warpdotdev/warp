use std::time::{Duration, SystemTime};

use ai::agent::action::{UploadArtifactRequest, UseComputerRequest};
use ai::skills::{ParsedSkill, SkillProvider, SkillReference, SkillScope};
use computer_use::{Action, ScreenshotParams, Target, TargetedAction};
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{DirectoryWatcher, RepoMetadataModel};
use warp_core::features::FeatureFlag;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;
use watcher::HomeDirectoryWatcher;

use super::{
    RecordingCardText, format_failed_read_paths, format_upload_artifact_text,
    formatted_text_for_file_glob, formatted_text_for_grep, parsed_skill_for_common_locations,
    read_files_request_display_paths, read_files_success_display_paths, read_skill_display_text,
    should_decorate_recorded_use_computer, start_recording_card_text, stop_recording_card_text,
};
use crate::ai::agent::{
    AnyFileContent, FileContext, FileLocations, ReadFilesFailedFile, RecordingStarted,
    RecordingStopped, StartRecordingResult, StopRecordingResult, UploadArtifactResult,
};
use crate::ai::skills::SkillManager;
use crate::settings::AISettings;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

fn native_path(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

#[test]
fn format_upload_artifact_text_includes_request_details() {
    let request = UploadArtifactRequest {
        file_path: "reports/daily.txt".to_string(),
        description: Some("Daily summary".to_string()),
    };

    let text = format_upload_artifact_text(&request, None, None, None);

    assert_eq!(
        text,
        "Upload artifact: reports/daily.txt\nDescription: Daily summary"
    );
}

#[test]
fn grep_renderer_preserves_copy_and_formats_explicit_and_current_paths() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo/worktree/deep".to_string();
    let queries = vec!["needle".to_string()];

    assert_eq!(
        formatted_text_for_grep(
            &queries,
            "/repo/worktree/deep/src",
            false,
            false,
            None,
            Some(&cwd),
        )
        .raw_text(),
        "Grepping for needle in src\n"
    );
    assert_eq!(
        formatted_text_for_grep(&queries, ".", true, false, None, Some(&cwd)).raw_text(),
        "Grep for needle in the current directory cancelled\n"
    );
}

#[test]
fn file_glob_and_v2_renderer_format_explicit_and_omitted_directories() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo/worktree/deep".to_string();
    let patterns = vec!["**/*.rs".to_string()];

    assert_eq!(
        formatted_text_for_file_glob(
            &patterns,
            Some("/repo/archive"),
            false,
            false,
            None,
            Some(&cwd),
        )
        .raw_text(),
        format!(
            "Finding files that match **/*.rs in {}\n",
            native_path("../../archive")
        )
    );
    assert_eq!(
        formatted_text_for_file_glob(&patterns, None, false, true, None, Some(&cwd)).raw_text(),
        "Search for files that match **/*.rs in the current directory\n"
    );
}

fn file_context(path: &str, line_range: Option<std::ops::Range<usize>>) -> FileContext {
    FileContext::new(
        path.to_string(),
        AnyFileContent::StringContent("one\ntwo\nthree".to_string()),
        line_range,
        None,
    )
}

#[test]
fn read_files_request_renderer_formats_paths_once_against_invocation_cwd() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo/worktree/deep".to_string();
    let files = vec![
        FileLocations {
            name: "/repo/worktree/deep/src/lib.rs".to_string(),
            lines: std::iter::once(10..20).collect(),
        },
        FileLocations {
            name: "/repo/archive.rs".to_string(),
            lines: vec![],
        },
        FileLocations {
            name: "/outside.rs".to_string(),
            lines: vec![],
        },
    ];
    assert_eq!(
        read_files_request_display_paths(&files, None, Some(&cwd)),
        vec![
            format!("{} (10-20)", native_path("src/lib.rs")),
            native_path("../../archive.rs"),
            native_path("/outside.rs"),
        ]
    );
}

#[test]
fn read_files_grouped_success_renderer_formats_and_groups_actual_results() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo/worktree/deep".to_string();
    let files = vec![
        file_context("/repo/worktree/deep/src/lib.rs", Some(40..45)),
        file_context("/repo/archive.rs", None),
        file_context("/repo/worktree/deep/src/lib.rs", Some(10..20)),
    ];

    assert_eq!(
        read_files_success_display_paths(&files, None, Some(&cwd)),
        vec![
            format!("{} (10-20, 40-45)", native_path("src/lib.rs")),
            native_path("../../archive.rs"),
        ]
    );
}

#[test]
fn failed_read_rows_use_invocation_cwd() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo/worktree/deep".to_string();
    let failed_files = vec![
        ReadFilesFailedFile {
            path: "/repo/worktree/deep/src/missing.rs".to_string(),
            message: "not found".to_string(),
        },
        ReadFilesFailedFile {
            path: "/repo/archive/missing.rs".to_string(),
            message: "not found".to_string(),
        },
        ReadFilesFailedFile {
            path: "/outside.rs".to_string(),
            message: "not found".to_string(),
        },
    ];

    assert_eq!(
        format_failed_read_paths(&failed_files, None, Some(&cwd)),
        [
            native_path("src/missing.rs"),
            native_path("../../archive/missing.rs"),
            native_path("/outside.rs"),
        ]
        .join("\n")
    );
}

#[test]
fn format_upload_artifact_text_includes_success_summary() {
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

    let text = format_upload_artifact_text(&request, Some(&result), None, None);

    assert_eq!(
        text,
        "Upload artifact: reports/daily.txt\nDescription: Daily summary\nStatus: uploaded artifact artifact-123\nUploaded file: reports/daily.txt"
    );
}

#[test]
fn format_upload_artifact_text_includes_terminal_status() {
    let request = UploadArtifactRequest {
        file_path: "reports/daily.txt".to_string(),
        description: None,
    };

    let error_text = format_upload_artifact_text(
        &request,
        Some(&UploadArtifactResult::Error(
            "permission denied".to_string(),
        )),
        None,
        None,
    );
    assert_eq!(
        error_text,
        "Upload artifact: reports/daily.txt\nStatus: upload failed: permission denied"
    );

    let cancelled_text =
        format_upload_artifact_text(&request, Some(&UploadArtifactResult::Cancelled), None, None);
    assert_eq!(cancelled_text, "Upload artifact: reports/daily.txt");
}

#[test]
fn format_upload_artifact_text_uses_invocation_cwd_for_local_paths() {
    let _relative_paths = FeatureFlag::RelativeBlocklistPaths.override_enabled(true);
    let cwd = "/repo/worktree".to_string();
    let request = UploadArtifactRequest {
        file_path: "/repo/worktree/reports/daily.txt".to_string(),
        description: Some("Daily summary".to_string()),
    };
    let result = UploadArtifactResult::Success {
        artifact_uid: "artifact-123".to_string(),
        filepath: Some("/repo/archive/daily.txt".to_string()),
        mime_type: "text/plain".to_string(),
        description: Some("Daily summary".to_string()),
        size_bytes: 128,
    };

    assert_eq!(
        format_upload_artifact_text(&request, Some(&result), None, Some(&cwd)),
        format!(
            "Upload artifact: {}\nDescription: Daily summary\nStatus: uploaded artifact artifact-123\nUploaded file: {}",
            native_path("reports/daily.txt"),
            native_path("../archive/daily.txt")
        )
    );
}

#[test]
fn start_recording_card_text_uses_static_title_and_description_subtext() {
    let result = StartRecordingResult::Success(RecordingStarted {
        recording_id: "rec-1".to_string(),
        started_at: SystemTime::UNIX_EPOCH,
        width_px: 1280,
        height_px: 720,
    });

    let text = start_recording_card_text("Demo checkout flow", Some(&result));

    assert_eq!(
        text,
        RecordingCardText {
            primary: "Recording started".to_string(),
            subtext: Some("Demo checkout flow".to_string()),
        }
    );
}

#[test]
fn start_recording_card_text_includes_failure_copy() {
    let result = StartRecordingResult::Error("unsupported platform".to_string());

    let text = start_recording_card_text("Demo checkout flow", Some(&result));

    assert_eq!(
        text,
        RecordingCardText {
            primary: "Recording failed to start".to_string(),
            subtext: Some("unsupported platform".to_string()),
        }
    );
}

#[test]
fn stop_recording_card_text_includes_complete_duration() {
    let result = StopRecordingResult::Success(RecordingStopped {
        artifact_uid: "artifact-1".to_string(),
        duration: Duration::from_secs(2),
        width_px: 1280,
        height_px: 720,
        size_bytes: 42,
        completion_status: computer_use::RecordingCompletionStatus::Completed,
        termination_reason: "Stopped by agent".to_string(),
    });

    let text = stop_recording_card_text(Some(&result));

    assert_eq!(
        text,
        RecordingCardText {
            primary: "Recording saved".to_string(),
            subtext: Some("0:02".to_string()),
        }
    );
}

#[test]
fn stop_recording_card_text_includes_partial_duration_without_raw_reason() {
    let result = StopRecordingResult::Success(RecordingStopped {
        artifact_uid: "artifact-1".to_string(),
        duration: Duration::from_secs(12),
        width_px: 1280,
        height_px: 720,
        size_bytes: 42,
        completion_status: computer_use::RecordingCompletionStatus::StoppedEarly,
        termination_reason: "internal raw reason".to_string(),
    });

    let text = stop_recording_card_text(Some(&result));

    assert_eq!(
        text,
        RecordingCardText {
            primary: "Recording saved".to_string(),
            subtext: Some("Partial recording • 0:12".to_string()),
        }
    );
}

#[test]
fn use_computer_decoration_skips_screenshot_only_rows() {
    // Agents that only want a screenshot emit a zero-duration wait plus
    // screenshot params; a real wait is a captured interaction.
    let mut request = UseComputerRequest {
        action_summary: "Screenshot".to_string(),
        actions: vec![TargetedAction::screen(Action::Wait(Duration::ZERO))],
        screenshot_params: Some(ScreenshotParams {
            max_long_edge_px: None,
            max_total_px: None,
            region: None,
            target: Target::Screen,
        }),
    };
    assert!(!should_decorate_recorded_use_computer(&request));

    request.actions = vec![TargetedAction::screen(Action::Wait(Duration::from_secs(1)))];
    assert!(should_decorate_recorded_use_computer(&request));
}

fn make_skill(name: &str) -> ParsedSkill {
    ParsedSkill {
        name: name.to_string(),
        description: String::new(),
        path: LocalOrRemotePath::Local(
            std::path::PathBuf::from("/home/user/.agents/skills")
                .join(name)
                .join("SKILL.md"),
        ),
        content: String::new(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Home,
    }
}

#[test]
fn read_skill_display_text_shows_slash_command_when_skill_found() {
    let skill = make_skill("hello-world");
    let reference = SkillReference::Path(skill.path.clone());
    assert_eq!(
        read_skill_display_text(Some(&skill), &reference),
        "/hello-world"
    );
}

#[test]
fn read_skill_display_text_no_double_slash_when_skill_not_found_with_path_reference() {
    // When the skill is not in the manager the fallback is skill_reference.to_string(),
    // which for a path reference is an absolute path starting with '/'.  The display
    // text must NOT prepend an extra '/' — doing so would produce '//home/…'.
    let path = LocalOrRemotePath::Local(std::path::PathBuf::from(
        "/home/devbox/.warp-local/skills/hello-world/SKILL.md",
    ));
    let reference = SkillReference::Path(path);
    let display = read_skill_display_text(None, &reference);
    assert!(
        !display.starts_with("//"),
        "display text must not start with '//': {display}"
    );
    assert!(
        display.starts_with('/'),
        "display text should start with '/': {display}"
    );
}

#[test]
fn read_skill_display_text_bundled_id_fallback_when_skill_not_found() {
    // The fallback uses the user-facing label (the bare id), not the canonical
    // `@warp-skill:<id>` reference form, so bundled-skill copy reads the same
    // way as path-based skill copy.
    let reference = SkillReference::BundledSkillId("create-pr".to_string());
    let display = read_skill_display_text(None, &reference);
    assert_eq!(display, "create-pr");
}

fn remote_location(host_id: &HostId, path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Remote(RemotePath::new(
        host_id.clone(),
        StandardizedPath::try_new(path).unwrap(),
    ))
}

#[test]
fn parsed_skill_for_common_locations_resolves_cached_remote_skill() {
    let host_id = HostId::new("remote-host".to_string());
    let skill = ParsedSkill {
        name: "deploy".to_string(),
        description: "Deploy skill".to_string(),
        path: remote_location(&host_id, "/repo/.agents/skills/deploy/SKILL.md"),
        content: "# Deploy".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };
    let locations = vec![
        remote_location(&host_id, "/repo/.agents/skills/deploy/README.md"),
        remote_location(&host_id, "/repo/.agents/skills/deploy/scripts/run.sh"),
    ];

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(AISettings::new_with_defaults);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let manager = app.add_singleton_model(SkillManager::new);
        manager.update(&mut app, |manager, _| {
            manager.add_skill_for_testing(skill.clone());
        });

        let resolved = manager.read(&app, |_, ctx| {
            parsed_skill_for_common_locations(locations, ctx).map(|skill| skill.path.clone())
        });
        assert_eq!(resolved, Some(skill.path));
    });
}

#[test]
fn parsed_skill_for_common_locations_does_not_mix_remote_hosts() {
    let first_host = HostId::new("first-host".to_string());
    let second_host = HostId::new("second-host".to_string());
    let skill = ParsedSkill {
        name: "deploy".to_string(),
        description: "Deploy skill".to_string(),
        path: remote_location(&first_host, "/repo/.agents/skills/deploy/SKILL.md"),
        content: "# Deploy".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };
    let locations = vec![
        remote_location(&first_host, "/repo/.agents/skills/deploy/README.md"),
        remote_location(&second_host, "/repo/.agents/skills/deploy/README.md"),
    ];

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(AISettings::new_with_defaults);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let manager = app.add_singleton_model(SkillManager::new);
        manager.update(&mut app, |manager, _| {
            manager.add_skill_for_testing(skill);
        });

        assert!(manager.read(&app, |_, ctx| {
            parsed_skill_for_common_locations(locations, ctx).is_none()
        }));
    });
}
