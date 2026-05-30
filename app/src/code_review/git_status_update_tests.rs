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
