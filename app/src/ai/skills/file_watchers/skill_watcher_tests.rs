use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use repo_metadata::entry::{DirectoryEntry, Entry, FileMetadata};
use repo_metadata::file_tree_store::FileTreeState;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{
    DirectoryWatcher, RepoMetadataModel, RepositoryIdentifier, RepositoryUpdate,
    StandingQueryContent, StandingQueryResults, StandingQueryResultsDelta, TargetFile,
};
use tempfile::TempDir;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::super::subscribers::SkillRepositoryMessage;
use super::{SkillWatcher, parse_project_skill_contents};
use crate::ai::skills::skill_manager::SkillWatcherEvent;

/// Helper function for creating a single skill file
fn create_skill_file(dir: &TempDir, name: &str, description: &str, content: &str) -> ParsedSkill {
    create_skill_file_in_directory(dir.path(), name, description, content)
}

fn create_skill_file_in_directory(
    parent_dir: &std::path::Path,
    name: &str,
    description: &str,
    content: &str,
) -> ParsedSkill {
    let skill_content = format!(
        r#"---
name: {}
description: {}
---
{}
"#,
        name, description, content
    );
    let skills_path = parent_dir.join(".agents").join("skills");
    let skill_dir_path = skills_path.join(name);
    let skill_file_path = skill_dir_path.join("SKILL.md");

    fs::create_dir_all(&skill_dir_path).unwrap();
    fs::write(&skill_file_path, skill_content.clone()).unwrap();
    let line_range_start = skill_content.clone().lines().count() - content.lines().count() + 1;
    let line_range_end = skill_content.clone().lines().count() + 1;
    ParsedSkill {
        path: LocalOrRemotePath::Local(skill_file_path),
        name: name.to_string(),
        description: description.to_string(),
        content: skill_content.clone(),
        line_range: Some(line_range_start..line_range_end),
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    }
}

fn skill_local_path(skill: &ParsedSkill) -> PathBuf {
    skill.path.to_local_path().unwrap().to_path_buf()
}
fn remote_skill_path(host_id: &HostId, name: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Remote(RemotePath::new(
        host_id.clone(),
        StandardizedPath::try_new(format!("/repo/.agents/skills/{name}/SKILL.md").as_str())
            .unwrap(),
    ))
}

fn remote_skill_content(name: &str, description: &str, body: &str) -> String {
    format!(
        r#"---
name: {name}
description: {description}
---
{body}
"#
    )
}

#[test]
fn parse_project_skill_contents_preserves_remote_paths() {
    let host = HostId::new("test-host".to_string());
    let first_path = remote_skill_path(&host, "first");
    let second_path = remote_skill_path(&host, "second");
    let first_content = remote_skill_content("first", "First skill", "First body");
    let second_content = remote_skill_content("second", "Second skill", "Second body");

    let skills = parse_project_skill_contents(vec![
        (first_path.clone(), first_content.clone()),
        (second_path.clone(), second_content.clone()),
    ]);

    assert_eq!(skills.len(), 2);
    assert_eq!(skills[0].path, first_path);
    assert_eq!(skills[0].name, "first");
    assert_eq!(skills[0].content, first_content);
    assert_eq!(skills[0].provider, SkillProvider::Agents);
    assert_eq!(skills[1].path, second_path);
    assert_eq!(skills[1].name, "second");
    assert_eq!(skills[1].content, second_content);
}

#[test]
fn parse_project_skill_contents_classifies_foreign_encoded_provider_path() {
    let path = LocalOrRemotePath::Remote(RemotePath::new(
        HostId::new("test-host".to_string()),
        StandardizedPath::try_new(r"C:\repo\.codex\skills\windows-skill\SKILL.md").unwrap(),
    ));
    let content = remote_skill_content("windows-skill", "Windows skill", "Windows body");

    let skills = parse_project_skill_contents(vec![(path.clone(), content)]);

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].path, path);
    assert_eq!(skills[0].provider, SkillProvider::Codex);
}

// ============================================================================
// Tests for handle_repository_update
// ============================================================================

#[test]
fn test_handle_repository_update_single_skill_added() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");

        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(skill_local_path(&skill), false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
    });
}

#[test]
fn test_removing_remote_project_repo_deletes_shared_cached_skill_paths() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let host = HostId::new("test-host".to_string());
        let repo_id = RepositoryIdentifier::Remote(RemotePath::new(
            host.clone(),
            StandardizedPath::try_new("/repo").unwrap(),
        ));
        let first_path = remote_skill_path(&host, "first");
        let second_path = remote_skill_path(&host, "second");

        skill_watcher_handle.update(&mut app, |watcher, _| {
            watcher.project_skill_files_by_repo.insert(
                repo_id.clone(),
                HashSet::from([first_path.clone(), second_path.clone()]),
            );
            watcher.remove_project_skills_for_repo(&repo_id);
        });

        let SkillWatcherEvent::SkillsDeleted { mut paths } = rx.recv().await.unwrap() else {
            panic!("Expected SkillsDeleted event");
        };
        paths.sort_by_key(LocalOrRemotePath::display_path);
        let mut expected = vec![first_path, second_path];
        expected.sort_by_key(LocalOrRemotePath::display_path);
        assert_eq!(paths, expected);

        skill_watcher_handle.read(&app, |watcher, _| {
            assert!(!watcher.project_skill_files_by_repo.contains_key(&repo_id));
        });
    });
}

#[test]
fn test_stale_project_skill_refresh_result_is_ignored() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "stale", "Stale skill", "Old content");
        let repo_id = RepositoryIdentifier::try_local(temp_dir.path()).unwrap();

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            let stale_generation = skill_watcher.advance_project_skill_refresh_generation(&repo_id);
            skill_watcher.advance_project_skill_refresh_generation(&repo_id);
            skill_watcher.emit_project_skills_if_current(
                &repo_id,
                stale_generation,
                vec![skill],
                ctx,
            );
        });

        assert!(rx.try_recv().is_err());
    });
}

#[test]
fn test_removing_project_repo_invalidates_pending_refresh_result() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "removed", "Removed skill", "Old content");
        let repo_id = RepositoryIdentifier::try_local(temp_dir.path()).unwrap();

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            let pending_generation =
                skill_watcher.advance_project_skill_refresh_generation(&repo_id);
            skill_watcher.remove_project_skills_for_repo(&repo_id);
            skill_watcher.emit_project_skills_if_current(
                &repo_id,
                pending_generation,
                vec![skill],
                ctx,
            );
        });

        assert!(rx.try_recv().is_err());
    });
}

#[test]
#[cfg(unix)]
fn test_refresh_project_skills_for_repo_loads_indexed_and_symlinked_skill_directories() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        let repo_metadata_handle = app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let repo_dir = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();
        let indexed_skill = create_skill_file(
            &repo_dir,
            "indexed-skill",
            "Indexed skill",
            "Indexed content",
        );
        let target_skill = create_skill_file(
            &target_dir,
            "linked-skill",
            "Linked skill",
            "Linked content",
        );
        let repo = repo_dir.path().to_path_buf();
        let symlink_parent = repo.join(".agents/skills");
        fs::create_dir_all(&symlink_parent).unwrap();
        let symlink_skill_dir = symlink_parent.join("linked-skill");
        std::os::unix::fs::symlink(
            target_skill.path.to_local_path().unwrap().parent().unwrap(),
            &symlink_skill_dir,
        )
        .unwrap();

        let mut expected_skill = target_skill;
        expected_skill.path = LocalOrRemotePath::Local(symlink_skill_dir.join("SKILL.md"));

        let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
        let repo_key = StandardizedPath::try_from_local(&repo).unwrap();
        repo_metadata_handle.update(&mut app, |model, ctx| {
            model.insert_test_state(
                repo_key.clone(),
                project_state(&repo, Some(&indexed_skill)),
                ctx,
            );
            let mut standing_results = project_standing_results(&repo, Some(&indexed_skill));
            standing_results.insert_project_skill(StandingQueryContent::file(
                StandardizedPath::try_from_local(&skill_local_path(&expected_skill)).unwrap(),
            ));
            model.insert_test_standing_results(repo_key, standing_results, ctx);
        });

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.refresh_project_skills_for_repo(&repo_id, ctx);
        });
        let SkillWatcherEvent::SkillsAdded { mut skills } = rx.recv().await.unwrap() else {
            panic!("Expected SkillsAdded event");
        };
        skills.sort_by_key(|skill| skill.path.display_path());
        let mut expected = vec![indexed_skill, expected_skill];
        expected.sort_by_key(|skill| skill.path.display_path());
        assert_eq!(skills, expected);
    });
}

#[test]
fn test_refresh_project_skills_for_repo_uses_repo_metadata_without_fallback_watcher() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        let repo_metadata_handle = app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "metadata-skill", "Metadata skill", "Content");
        let repo = temp_dir.path().to_path_buf();
        let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
        let repo_key = StandardizedPath::try_from_local(&repo).unwrap();

        repo_metadata_handle.update(&mut app, |model, ctx| {
            model.insert_test_state(repo_key.clone(), project_state(&repo, Some(&skill)), ctx);
            model.insert_test_standing_results(
                repo_key,
                project_standing_results(&repo, Some(&skill)),
                ctx,
            );
        });
        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.refresh_project_skills_for_repo(&repo_id, ctx);
            assert!(skill_watcher.failed_local_project_watchers.is_empty());
        });

        assert_eq!(
            rx.recv().await.unwrap(),
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
    });
}

#[test]
fn test_local_project_fallback_scans_filesystem_when_repo_metadata_fails() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let repo = dunce::canonicalize(temp_dir.path()).unwrap();
        let root_skill =
            create_skill_file_in_directory(&repo, "root-skill", "Root skill", "Root content");
        let subdir = repo.join("packages/frontend");
        let subdir_skill =
            create_skill_file_in_directory(&subdir, "frontend-skill", "Frontend skill", "Content");

        let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.fallback_to_local_project_watcher(&repo_id, ctx);
            assert!(skill_watcher.failed_local_project_watchers.is_empty());
        });

        let SkillWatcherEvent::SkillsAdded { mut skills } = rx.recv().await.unwrap() else {
            panic!("Expected SkillsAdded event");
        };
        skills.sort_by_key(|skill| skill.path.display_path());
        let mut expected = vec![root_skill, subdir_skill];
        expected.sort_by_key(|skill| skill.path.display_path());
        assert_eq!(skills, expected);
    });
}

#[test]
#[cfg(unix)]
fn test_local_project_fallback_initial_scan_loads_symlinked_skill_directory() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let repo_dir = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();
        let target_skill = create_skill_file(
            &target_dir,
            "fallback-linked-skill",
            "Fallback linked skill",
            "Linked content",
        );
        let repo = dunce::canonicalize(repo_dir.path()).unwrap();
        let symlink_parent = repo.join(".agents/skills");
        fs::create_dir_all(&symlink_parent).unwrap();
        let symlink_skill_dir = symlink_parent.join("fallback-linked-skill");
        std::os::unix::fs::symlink(
            skill_local_path(&target_skill).parent().unwrap(),
            &symlink_skill_dir,
        )
        .unwrap();

        let mut expected_skill = target_skill;
        expected_skill.path = LocalOrRemotePath::Local(symlink_skill_dir.join("SKILL.md"));

        let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.fallback_to_local_project_watcher(&repo_id, ctx);
        });

        assert_eq!(
            rx.recv().await.unwrap(),
            SkillWatcherEvent::SkillsAdded {
                skills: vec![expected_skill]
            }
        );
    });
}
#[test]
fn test_local_project_fallback_update_reuses_repository_update_handler() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "fallback-update", "Fallback update", "Content");
        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::from([TargetFile::new(skill_local_path(&skill), false)]),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_message(
                SkillRepositoryMessage::ProjectRepositoryUpdate { update },
                ctx,
            );
        });

        assert_eq!(
            rx.recv().await.unwrap(),
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
    });
}

#[test]
fn test_local_project_fallback_directory_addition_scans_filesystem() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let new_dir = temp_dir.path().join("packages/frontend");
        let skill =
            create_skill_file_in_directory(&new_dir, "fallback-dir", "Fallback dir", "Content");
        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(new_dir, false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_message(
                SkillRepositoryMessage::ProjectRepositoryUpdate { update },
                ctx,
            );
        });

        assert_eq!(
            rx.recv().await.unwrap(),
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
    });
}
#[test]
fn test_handle_repository_update_skill_modified() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::from([TargetFile::new(skill_local_path(&skill), false)]),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
    });
}

#[test]
fn test_handle_repository_update_skill_deleted() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::from([TargetFile::new(skill_local_path(&skill), false)]),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            SkillWatcherEvent::SkillsDeleted {
                paths: vec![skill.path]
            }
        );
    });
}

#[test]
fn test_handle_repository_update_multiple_skills_deleted() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill_a = create_skill_file(&temp_dir, "skill-a", "Skill A", "Content A");
        let skill_b = create_skill_file(&temp_dir, "skill-b", "Skill B", "Content B");

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::from([
                TargetFile::new(skill_local_path(&skill_a), false),
                TargetFile::new(skill_local_path(&skill_b), false),
            ]),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        let SkillWatcherEvent::SkillsDeleted { mut paths } = event else {
            panic!("Expected SkillsDeleted event");
        };
        paths.sort_by_key(LocalOrRemotePath::display_path);
        let mut expected = vec![skill_a.path, skill_b.path];
        expected.sort_by_key(LocalOrRemotePath::display_path);
        assert_eq!(paths, expected);
    });
}

#[test]
fn test_handle_repository_update_skill_moved() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let old_skill = create_skill_file(&temp_dir, "old-skill", "Old skill", "Old content");
        let new_skill = create_skill_file(&temp_dir, "new-skill", "New skill", "New content");

        // moved is HashMap<to_target, from_target>
        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::from([(
                TargetFile::new(skill_local_path(&new_skill), false),
                TargetFile::new(skill_local_path(&old_skill), false),
            )]),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // Collect both events: SkillsAdded for the new location and SkillsDeleted for the old
        let event1 = rx.recv().await.unwrap();
        let event2 = rx.recv().await.unwrap();

        let added_event = SkillWatcherEvent::SkillsAdded {
            skills: vec![new_skill],
        };
        let deleted_event = SkillWatcherEvent::SkillsDeleted {
            paths: vec![old_skill.path],
        };
        assert!(
            (event1 == added_event && event2 == deleted_event)
                || (event1 == deleted_event && event2 == added_event),
            "Expected one SkillsAdded and one SkillsDeleted event; got: {event1:?} and {event2:?}"
        );
    });
}

// ============================================================================
// Tests for project skill refreshes
// ============================================================================

#[test]
fn test_handle_repository_update_non_skill_directory_added_does_not_emit_project_event() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let new_dir = temp_dir.path().join("new-feature");
        fs::create_dir_all(&new_dir).unwrap();

        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(new_dir, false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        assert!(rx.try_recv().is_err());
    });
}

fn project_state(repo: &std::path::Path, skill: Option<&ParsedSkill>) -> FileTreeState {
    let children = if let Some(skill) = skill {
        let skill_path = skill_local_path(skill);
        let skill_file = Entry::File(FileMetadata::new(skill_path.clone(), false));
        let skill_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(skill_path.parent().unwrap()).unwrap(),
            children: vec![skill_file],
            ignored: false,
            loaded: true,
        });
        let skills_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&repo.join(".agents/skills")).unwrap(),
            children: vec![skill_dir],
            ignored: false,
            loaded: true,
        });
        let agents_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&repo.join(".agents")).unwrap(),
            children: vec![skills_dir],
            ignored: false,
            loaded: true,
        });
        vec![agents_dir]
    } else {
        Vec::new()
    };

    let root = Entry::Directory(DirectoryEntry {
        path: StandardizedPath::try_from_local(repo).unwrap(),
        children,
        ignored: false,
        loaded: true,
    });
    FileTreeState::new(root, Vec::new(), None)
}

fn project_standing_results(
    repo: &std::path::Path,
    skill: Option<&ParsedSkill>,
) -> StandingQueryResults {
    let mut delta = StandingQueryResultsDelta {
        upserted_project_skills: vec![StandingQueryContent::directory(
            StandardizedPath::try_from_local(&repo.join(".agents/skills")).unwrap(),
        )],
        ..StandingQueryResultsDelta::default()
    };
    if let Some(skill) = skill {
        delta
            .upserted_project_skills
            .push(StandingQueryContent::file(
                StandardizedPath::try_from_local(&skill_local_path(skill)).unwrap(),
            ));
    }
    let mut results = StandingQueryResults::default();
    results.apply_delta(&delta);
    results
}

#[test]
fn test_refresh_project_skills_for_repo_removes_missing_project_skill_paths() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        let repo_metadata_handle = app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");
        let repo = temp_dir.path().to_path_buf();
        let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
        let repo_key = StandardizedPath::try_from_local(&repo).unwrap();

        repo_metadata_handle.update(&mut app, |model, ctx| {
            model.insert_test_state(repo_key.clone(), project_state(&repo, Some(&skill)), ctx);
            model.insert_test_standing_results(
                repo_key.clone(),
                project_standing_results(&repo, Some(&skill)),
                ctx,
            );
        });
        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.refresh_project_skills_for_repo(&repo_id, ctx);
        });

        assert_eq!(
            rx.recv().await.unwrap(),
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill.clone()]
            }
        );

        repo_metadata_handle.update(&mut app, |model, ctx| {
            model.insert_test_state(repo_key.clone(), project_state(&repo, None), ctx);
            model.insert_test_standing_results(
                repo_key,
                project_standing_results(&repo, None),
                ctx,
            );
        });
        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.refresh_project_skills_for_repo(&repo_id, ctx);
        });

        assert_eq!(
            rx.recv().await.unwrap(),
            SkillWatcherEvent::SkillsDeleted {
                paths: vec![skill.path]
            }
        );
    });
}
