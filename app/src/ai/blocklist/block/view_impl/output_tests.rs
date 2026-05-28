use ai::agent::action::UploadArtifactRequest;
use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{DirectoryWatcher, RepoMetadataModel};
use warp_util::{
    host_id::HostId, local_or_remote_path::LocalOrRemotePath, remote_path::RemotePath,
    standardized_path::StandardizedPath,
};
use warpui::App;
use watcher::HomeDirectoryWatcher;

use super::{format_upload_artifact_text, parsed_skill_for_common_locations};
use crate::ai::agent::UploadArtifactResult;
use crate::ai::skills::SkillManager;
use crate::settings::AISettings;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

#[test]
fn format_upload_artifact_text_includes_request_details() {
    let request = UploadArtifactRequest {
        file_path: "reports/daily.txt".to_string(),
        description: Some("Daily summary".to_string()),
    };

    let text = format_upload_artifact_text(&request, None);

    assert_eq!(
        text,
        "Upload artifact: reports/daily.txt\nDescription: Daily summary"
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

    let text = format_upload_artifact_text(&request, Some(&result));

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
    );
    assert_eq!(
        error_text,
        "Upload artifact: reports/daily.txt\nStatus: upload failed: permission denied"
    );

    let cancelled_text =
        format_upload_artifact_text(&request, Some(&UploadArtifactResult::Cancelled));
    assert_eq!(cancelled_text, "Upload artifact: reports/daily.txt");
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
