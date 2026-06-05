use std::path::PathBuf;

use repo_metadata::{DirectoryWatcher, RepositoryUpdate, TargetFile};
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, ModelHandle};

use super::*;

fn metadata(branch: &str) -> GitStatusMetadata {
    GitStatusMetadata {
        current_branch_name: branch.to_string(),
        main_branch_name: "main".to_string(),
        stats_against_head: DiffStats::default(),
    }
}

fn pr(number: u64) -> PrInfo {
    PrInfo {
        number,
        url: format!("https://github.com/warp/warp/pull/{number}"),
        state: "OPEN".to_string(),
        draft: false,
        base_branch: "main".to_string(),
    }
}

fn test_repository_handle(app: &mut App, temp_dir: &tempfile::TempDir) -> ModelHandle<Repository> {
    let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
    watcher_handle.update(app, |watcher, ctx| {
        watcher
            .add_directory(
                StandardizedPath::from_local_canonicalized(temp_dir.path()).unwrap(),
                ctx,
            )
            .unwrap()
    })
}

#[cfg(feature = "local_fs")]
#[test]
fn pr_info_tracks_current_branch_only() {
    App::test((), |mut app| async move {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repository = test_repository_handle(&mut app, &temp_dir);
        let git_status = app.add_model(move |_| {
            GitRepoStatusModel::new_for_test(repository, Some(metadata("feature-a")))
        });
        git_status.update(&mut app, |model, ctx| {
            model.set_pr_info_for_test(Some(pr(123)), ctx);
        });

        git_status.read(&app, |model, _| {
            assert_eq!(model.pr_info(), Some(&pr(123)));
        });

        git_status.update(&mut app, |model, ctx| {
            model.set_metadata_for_test(Some(metadata("feature-b")), ctx);
        });
        git_status.read(&app, |model, _| {
            assert_eq!(model.pr_info(), None);
        });

        git_status.update(&mut app, |model, ctx| {
            model.set_pr_info_for_test(Some(pr(456)), ctx);
        });
        git_status.read(&app, |model, _| {
            assert_eq!(model.pr_info(), Some(&pr(456)));
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn pr_info_clears_when_metadata_load_fails() {
    App::test((), |mut app| async move {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repository = test_repository_handle(&mut app, &temp_dir);
        let git_status = app.add_model(move |_| {
            GitRepoStatusModel::new_for_test(repository, Some(metadata("feature-a")))
        });
        git_status.update(&mut app, |model, ctx| {
            model.set_pr_info_for_test(Some(pr(123)), ctx);
        });

        git_status.update(&mut app, |model, ctx| {
            model.handle_metadata_result(Err(anyhow::anyhow!("metadata failed")), ctx);
        });

        git_status.read(&app, |model, _| {
            assert!(model.metadata().is_none());
            assert_eq!(model.pr_info(), None);
        });
    });
}
#[cfg(feature = "local_fs")]
#[test]
fn pr_info_consumers_control_refresh_gate() {
    App::test((), |mut app| async move {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repository = test_repository_handle(&mut app, &temp_dir);
        let git_status = app.add_model(move |_| GitRepoStatusModel::new_for_test(repository, None));

        let first_consumer = warpui::EntityId::new();
        let second_consumer = warpui::EntityId::new();
        let unknown_consumer = warpui::EntityId::new();

        git_status.read(&app, |model, _| {
            assert!(!model.should_refresh_pr_info());
        });

        git_status.update(&mut app, |model, ctx| {
            model.set_pr_info_consumer(first_consumer, true, ctx);
        });
        git_status.read(&app, |model, _| {
            assert!(model.should_refresh_pr_info());
        });

        git_status.update(&mut app, |model, ctx| {
            model.set_pr_info_consumer(first_consumer, true, ctx);
            model.set_pr_info_consumer(second_consumer, true, ctx);
            model.set_pr_info_consumer(first_consumer, false, ctx);
        });
        git_status.read(&app, |model, _| {
            assert!(model.should_refresh_pr_info());
        });

        git_status.update(&mut app, |model, ctx| {
            model.set_pr_info_consumer(unknown_consumer, false, ctx);
        });
        git_status.read(&app, |model, _| {
            assert!(model.should_refresh_pr_info());
        });

        git_status.update(&mut app, |model, ctx| {
            model.set_pr_info_consumer(second_consumer, false, ctx);
        });
        git_status.read(&app, |model, _| {
            assert!(!model.should_refresh_pr_info());
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn should_refresh_metadata_ignores_ignored_file_updates() {
    let mut ignored_update = RepositoryUpdate::default();
    ignored_update
        .modified
        .insert(TargetFile::new(PathBuf::from("/repo/ignored.log"), true));
    assert!(!GitRepoStatusModel::should_refresh_metadata(
        &ignored_update
    ));

    let mut tracked_update = RepositoryUpdate::default();
    tracked_update
        .modified
        .insert(TargetFile::new(PathBuf::from("/repo/src/main.rs"), false));
    assert!(GitRepoStatusModel::should_refresh_metadata(&tracked_update));

    let remote_ref_update = RepositoryUpdate {
        remote_ref_updated: true,
        ..Default::default()
    };
    assert!(GitRepoStatusModel::should_refresh_metadata(
        &remote_ref_update
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn file_statuses_roll_up_to_ancestor_directories() {
    let repo = PathBuf::from("/repo");
    let statuses = vec![
        ("src/lib.rs".to_string(), GitFileStatus::Modified),
        ("src/new.rs".to_string(), GitFileStatus::Untracked),
        ("README.md".to_string(), GitFileStatus::Deleted),
    ];
    let decorations = RepoGitFileStatuses::from_relative(&repo, statuses);

    let p = |rel: &str| StandardizedPath::try_from_local(&repo.join(rel)).unwrap();

    // Files report their own status.
    assert_eq!(
        decorations.file_status(&p("src/lib.rs")),
        Some(&GitFileStatus::Modified)
    );
    assert_eq!(
        decorations.file_status(&p("src/new.rs")),
        Some(&GitFileStatus::Untracked)
    );
    assert_eq!(
        decorations.file_status(&p("README.md")),
        Some(&GitFileStatus::Deleted)
    );

    // `src/` holds a Modified and an Untracked file → Modified wins (higher priority).
    assert_eq!(
        decorations.dir_status(&p("src")),
        Some(&GitFileStatus::Modified)
    );

    // The repo root rolls up the highest priority among all descendants:
    // Deleted (3) outranks Modified (2).
    let root = StandardizedPath::try_from_local(&repo).unwrap();
    assert_eq!(decorations.dir_status(&root), Some(&GitFileStatus::Deleted));

    // Nothing rolls up above the repo root.
    let above_root = StandardizedPath::try_from_local(&PathBuf::from("/")).unwrap();
    assert!(decorations.dir_status(&above_root).is_none());
}

#[cfg(feature = "local_fs")]
#[test]
fn directory_roll_up_prefers_conflicts() {
    let repo = PathBuf::from("/repo");
    let statuses = vec![
        ("a/x.rs".to_string(), GitFileStatus::Modified),
        ("a/y.rs".to_string(), GitFileStatus::Conflicted),
    ];
    let decorations = RepoGitFileStatuses::from_relative(&repo, statuses);

    let dir = StandardizedPath::try_from_local(&repo.join("a")).unwrap();
    assert_eq!(decorations.dir_status(&dir), Some(&GitFileStatus::Conflicted));
}
