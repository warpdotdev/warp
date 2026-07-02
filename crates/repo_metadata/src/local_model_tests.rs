//! Tests for the LocalRepoMetadataModel.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::task::Poll;
use std::time::Duration;

use futures::channel::oneshot;
use futures::executor::block_on;
use ignore::gitignore::Gitignore;
use virtual_fs::{Stub, VirtualFS};
use warp_util::standardized_path::StandardizedPath;
use warpui_core::r#async::FutureExt as _;
use warpui_core::App;
#[cfg(feature = "local_fs")]
use watcher::BulkFilesystemWatcherEvent;

use crate::entry::{
    BudgetExceededBehavior, BuildTreeOptions, DirectoryEntry, Entry, FileMetadata,
    IgnoredPathStrategy,
};
use crate::file_tree_store::{FileTreeEntry, FileTreeEntryState, FileTreeState};
use crate::local_model::{
    FileTreeMutation, GetContentsArgs, IndexedRepoState, LocalRepoMetadataModel, RepoUpdate,
    RepositoryMetadataEvent, RootWatchMode,
};
use crate::repositories::DetectedRepositories;
use crate::watcher::DirectoryWatcher;
#[cfg(all(unix, feature = "local_fs"))]
use crate::StandingQueryResults;
use crate::{RepoMetadataError, StandingQueryContent, StandingQueryDefinitions};

impl LocalRepoMetadataModel {
    fn new_for_test() -> Self {
        Self {
            repositories: HashMap::new(),
            standing_results: HashMap::new(),
            lazy_loaded_paths: Default::default(),
            #[cfg(feature = "local_fs")]
            watcher: Default::default(),
            emit_incremental_updates: false,
            force_included_paths: Default::default(),
            standing_query_definitions: Default::default(),
            #[cfg(feature = "local_fs")]
            symlink_targets: Default::default(),
            #[cfg(feature = "local_fs")]
            repo_watches: Default::default(),
        }
    }
}

#[test]
#[should_panic(expected = "force-included paths must be repository-relative")]
fn force_included_paths_must_be_relative() {
    let mut model = LocalRepoMetadataModel::new_for_test();
    model.register_force_included_paths([std::env::temp_dir().join("absolute/path")]);
}
fn empty_repo_state(repo_path: &StandardizedPath) -> FileTreeState {
    let root = Entry::Directory(DirectoryEntry {
        path: repo_path.clone(),
        children: Vec::new(),
        ignored: false,
        loaded: true,
    });
    FileTreeState::new(root, Vec::new(), None)
}

#[test]
fn repository_indexed_resolves_immediately_for_indexed_repo() {
    VirtualFS::test("repository_indexed_ready", |dirs, mut vfs| {
        vfs.mkdir("repo");
        let repo_path =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("repo")).unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            let wait = model_handle.update(&mut app, |model, _ctx| {
                model.repositories.insert(
                    repo_path.clone(),
                    IndexedRepoState::Indexed(empty_repo_state(&repo_path)),
                );
                model.repository_indexed(&repo_path)
            });

            wait.await;
            let is_indexed = model_handle.read(&app, |model, _ctx| {
                matches!(
                    model.repository_state(&repo_path),
                    Some(IndexedRepoState::Indexed(_))
                )
            });
            assert!(is_indexed);
        });
    });
}

#[test]
fn repository_indexed_waits_for_pending_repo() {
    VirtualFS::test("repository_indexed_pending", |dirs, mut vfs| {
        vfs.mkdir("repo");
        let repo_path =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("repo")).unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            let wait = model_handle.update(&mut app, |model, _ctx| {
                model
                    .repositories
                    .insert(repo_path.clone(), IndexedRepoState::pending());
                model.repository_indexed(&repo_path)
            });

            futures::pin_mut!(wait);
            assert!(matches!(futures::poll!(&mut wait), Poll::Pending));

            model_handle.update(&mut app, |model, ctx| {
                model
                    .add_repository_internal(
                        repo_path.clone(),
                        empty_repo_state(&repo_path),
                        RootWatchMode::Recursive,
                        ctx,
                    )
                    .expect("repository should index");
            });

            wait.await;
            let is_indexed = model_handle.read(&app, |model, _ctx| {
                matches!(
                    model.repository_state(&repo_path),
                    Some(IndexedRepoState::Indexed(_))
                )
            });
            assert!(is_indexed);
        });
    });
}

#[test]
fn repository_state_returns_failed_state() {
    let repo_path = StandardizedPath::try_new("/failed_repo").unwrap();
    let error = RepoMetadataError::RepoNotFound(repo_path.to_string());
    let mut model = LocalRepoMetadataModel::new_for_test();
    model
        .repositories
        .insert(repo_path.clone(), IndexedRepoState::Failed(error));
    let result = model.repository_state(&repo_path);
    assert!(matches!(
        result,
        Some(IndexedRepoState::Failed(RepoMetadataError::RepoNotFound(path)))
            if path == &repo_path.to_string()
    ));
}

#[test]
fn repository_indexed_waits_for_pending_repo_failure() {
    let repo_path = StandardizedPath::try_new("/pending_failed_repo").unwrap();

    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
        let wait = model_handle.update(&mut app, |model, _ctx| {
            model
                .repositories
                .insert(repo_path.clone(), IndexedRepoState::pending());
            model.repository_indexed(&repo_path)
        });

        futures::pin_mut!(wait);
        assert!(matches!(futures::poll!(&mut wait), Poll::Pending));

        model_handle.update(&mut app, |model, ctx| {
            model.mark_repository_failed(
                repo_path.clone(),
                RepoMetadataError::RepoNotFound(repo_path.to_string()),
                ctx,
            );
        });

        wait.await;
        let is_failed = model_handle.read(&app, |model, _ctx| {
            matches!(
                model.repository_state(&repo_path),
                Some(IndexedRepoState::Failed(RepoMetadataError::RepoNotFound(path)))
                    if path == &repo_path.to_string()
            )
        });
        assert!(is_failed);
    });
}

#[test]
fn repository_indexed_waits_for_pending_repo_removal() {
    let repo_path = StandardizedPath::try_new("/pending_removed_repo").unwrap();

    App::test((), |mut app| async move {
        let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
        let wait = model_handle.update(&mut app, |model, _ctx| {
            model
                .repositories
                .insert(repo_path.clone(), IndexedRepoState::pending());
            model.repository_indexed(&repo_path)
        });

        futures::pin_mut!(wait);
        assert!(matches!(futures::poll!(&mut wait), Poll::Pending));

        model_handle.update(&mut app, |model, ctx| {
            model
                .remove_repository(&repo_path, ctx)
                .expect("repository should be removed");
        });

        wait.await;
        let result = model_handle.read(&app, |model, _ctx| {
            model.repository_state(&repo_path).is_none()
        });
        assert!(result);
    });
}

#[test]
fn test_get_repo_contents() {
    VirtualFS::test("repo_contents_test", |dirs, mut vfs| {
        let test_repo = dirs.tests().join("test_repo");

        // Create a test repository structure using VirtualFS with .git directory
        vfs.mkdir("test_repo/.git/objects")
            .mkdir("test_repo/subdir")
            .with_files(vec![
                Stub::FileWithContent("test_repo/.git/HEAD", "ref: refs/heads/main"),
                Stub::FileWithContent(
                    "test_repo/.git/config",
                    "[core]\n\trepositoryformatversion = 0",
                ),
                Stub::FileWithContent("test_repo/file1.txt", "content1"),
                Stub::FileWithContent("test_repo/subdir/file2.rs", "content2"),
                Stub::FileWithContent("test_repo/subdir/file3.py", "content3"),
                Stub::FileWithContent("test_repo/file4.md", "content4"),
                Stub::FileWithContent("test_repo/.gitignore", ""),
            ]);

        // Create a mock file tree structure
        let file1 = Entry::File(FileMetadata::new(test_repo.join("file1.txt"), false));
        let file2 = Entry::File(FileMetadata::new(test_repo.join("subdir/file2.rs"), false));
        let file3 = Entry::File(FileMetadata::new(test_repo.join("subdir/file3.py"), false));
        let file4 = Entry::File(FileMetadata::new(test_repo.join("file4.md"), false));

        let subdir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&test_repo.join("subdir")).unwrap(),
            children: vec![file2, file3],
            ignored: false,
            loaded: true,
        });

        let root = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&test_repo).unwrap(),
            children: vec![file1, subdir, file4],
            ignored: false,
            loaded: true,
        });

        let (gitignore, _) = Gitignore::new(test_repo.join(".gitignore"));

        App::test((), |mut app| async move {
            // Create RepoWatcher and get Repository handle through it
            let repo_watcher = app.add_singleton_model(DirectoryWatcher::new);
            let repo_handle = repo_watcher.update(&mut app, |repo_watcher, ctx| {
                repo_watcher
                    .add_directory(
                        StandardizedPath::from_local_canonicalized(&test_repo).unwrap(),
                        ctx,
                    )
                    .unwrap()
            });
            let state = FileTreeState::new(root, vec![gitignore], Some(repo_handle));

            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());

            model_handle.update(&mut app, |model, _ctx| {
                // Use the CanonicalizedPath as the key
                let canonical_key = StandardizedPath::from_local_canonicalized(&test_repo).unwrap();
                model
                    .repositories
                    .insert(canonical_key, IndexedRepoState::Indexed(state));
            });

            // Test getting all files
            model_handle.read(&app, |model, _ctx| {
                let args = GetContentsArgs::default().exclude_folders();
                let result = model
                    .get_repo_contents(
                        &StandardizedPath::from_local_canonicalized(&test_repo).unwrap(),
                        args,
                    )
                    .unwrap();

                // Should have 4 files total (file1.txt, file2.rs, file3.py, file4.md)
                assert_eq!(result.contents.len(), 4);
                assert!(!result.truncated);

                // Test with non-existent repository
                let non_existent = StandardizedPath::try_new("/non_existent_repo").unwrap();
                let args = GetContentsArgs::default().exclude_folders();
                let non_existent_result = model.get_repo_contents(&non_existent, args);
                assert!(matches!(
                    non_existent_result,
                    Err(RepoMetadataError::RepositoryNotIndexed)
                ));
            });
        });
    });
}

#[test]
fn test_get_repo_contents_truncates_to_max_results() {
    // Use an absolute base path so file metadata is valid on all platforms.
    let base = std::env::temp_dir().join("trunc_repo");
    let repo_path = StandardizedPath::try_from_local(&base).unwrap();

    // Build a flat repo with more files than the result cap so traversal stops early.
    let file_count = crate::local_model::MAX_REPO_CONTENTS_RESULTS + 50;
    let children: Vec<Entry> = (0..file_count)
        .map(|i| Entry::File(FileMetadata::new(base.join(format!("file{i}.txt")), false)))
        .collect();
    let root = Entry::Directory(DirectoryEntry {
        path: repo_path.clone(),
        children,
        ignored: false,
        loaded: true,
    });
    let state = FileTreeState::new(root, Vec::new(), None);

    let mut model = LocalRepoMetadataModel::new_for_test();
    model
        .repositories
        .insert(repo_path.clone(), IndexedRepoState::Indexed(state));

    let result = model
        .get_repo_contents(&repo_path, GetContentsArgs::default().exclude_folders())
        .unwrap();

    // The result is capped and flagged as truncated rather than erroring.
    assert_eq!(
        result.contents.len(),
        crate::local_model::MAX_REPO_CONTENTS_RESULTS
    );
    assert!(result.truncated);
}

/// A query-style traversal filter must be evaluated *before* an entry counts
/// toward the result cap, so a matching file that sorts well past the cap in
/// traversal order is still returned. This is the core guarantee that keeps
/// file search from truncating matches away.
#[test]
fn test_get_repo_contents_filter_applies_before_cap() {
    let base = std::env::temp_dir().join("filter_before_cap_repo");
    let repo_path = StandardizedPath::try_from_local(&base).unwrap();

    // Many non-matching files, then a single matching "needle" file placed last
    // so it is well beyond the default result cap in traversal order.
    let noise_count = crate::local_model::MAX_REPO_CONTENTS_RESULTS + 50;
    let mut children: Vec<Entry> = (0..noise_count)
        .map(|i| Entry::File(FileMetadata::new(base.join(format!("file{i}.txt")), false)))
        .collect();
    children.push(Entry::File(FileMetadata::new(
        base.join("needle.rs"),
        false,
    )));
    let root = Entry::Directory(DirectoryEntry {
        path: repo_path.clone(),
        children,
        ignored: false,
        loaded: true,
    });
    let state = FileTreeState::new(root, Vec::new(), None);

    let mut model = LocalRepoMetadataModel::new_for_test();
    model
        .repositories
        .insert(repo_path.clone(), IndexedRepoState::Indexed(state));

    let args = GetContentsArgs::default().with_filter(|content| match content {
        crate::RepoContent::File(file) => file
            .path
            .to_local_path_lossy()
            .to_string_lossy()
            .contains("needle"),
        crate::RepoContent::Directory(_) => false,
    });
    let result = model.get_repo_contents(&repo_path, args).unwrap();

    // The single matching file is returned despite sorting past the cap, and
    // the result is not truncated because only one entry matched.
    assert_eq!(result.contents.len(), 1);
    assert!(!result.truncated);
    assert!(matches!(
        &result.contents[0],
        crate::RepoContent::File(file)
            if file.path.to_local_path_lossy() == base.join("needle.rs")
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn test_lazy_loaded_path_registrations_are_refcounted() {
    VirtualFS::test("lazy_loaded_path_refcount", |dirs, mut vfs| {
        vfs.mkdir("shared_dir")
            .with_files(vec![Stub::FileWithContent(
                "shared_dir/file.txt",
                "content",
            )]);

        let shared_dir = dirs.tests().join("shared_dir");

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());

            let shared_dir_for_index =
                StandardizedPath::from_local_canonicalized(&shared_dir).unwrap();
            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_lazy_loaded_path(&shared_dir_for_index, ctx)
                    .unwrap();
                model
                    .index_lazy_loaded_path(&shared_dir_for_index, ctx)
                    .unwrap();
            });

            model_handle.read(&app, |model, _ctx| {
                assert!(model.is_lazy_loaded_path(
                    &StandardizedPath::from_local_canonicalized(&shared_dir).unwrap()
                ));
                assert!(model.has_repository(
                    &StandardizedPath::from_local_canonicalized(&shared_dir).unwrap()
                ));
            });

            let shared_dir_std = StandardizedPath::from_local_canonicalized(&shared_dir).unwrap();

            model_handle.update(&mut app, |model, ctx| {
                model.remove_lazy_loaded_path(&shared_dir_std, ctx);
            });

            model_handle.read(&app, |model, _ctx| {
                assert!(model.is_lazy_loaded_path(&shared_dir_std));
                assert!(model.has_repository(&shared_dir_std));
            });

            model_handle.update(&mut app, |model, ctx| {
                model.remove_lazy_loaded_path(&shared_dir_std, ctx);
            });

            model_handle.read(&app, |model, _ctx| {
                assert!(!model.is_lazy_loaded_path(
                    &StandardizedPath::from_local_canonicalized(&shared_dir).unwrap()
                ));
                assert!(!model.has_repository(
                    &StandardizedPath::from_local_canonicalized(&shared_dir).unwrap()
                ));
            });
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_lazy_loaded_path_does_not_build_standing_rule_results_below_shallow_tree() {
    VirtualFS::test("lazy_loaded_path_standing_rules", |dirs, mut vfs| {
        vfs.mkdir("workspace/src/deep")
            .with_files(vec![Stub::FileWithContent(
                "workspace/src/deep/WARP.md",
                "project rules",
            )]);

        let workspace = dirs.tests().join("workspace");
        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            let workspace_path = StandardizedPath::from_local_canonicalized(&workspace).unwrap();
            let rule_path =
                StandardizedPath::try_from_local(&workspace.join("src/deep/WARP.md")).unwrap();

            model_handle.update(&mut app, |model, ctx| {
                model.index_lazy_loaded_path(&workspace_path, ctx).unwrap();
            });

            model_handle.read(&app, |model, _ctx| {
                let results = model
                    .standing_query_results(&workspace_path)
                    .expect("lazy indexed paths should retain standing results");
                assert!(!results
                    .project_rules()
                    .any(|content| content.path == rule_path));
            });
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_lazy_loaded_path_discovers_force_included_skills_and_emits_watcher_delta() {
    VirtualFS::test("lazy_loaded_path_force_included_skills", |dirs, mut vfs| {
        vfs.mkdir("workspace/.agents/skills/review")
            .mkdir("workspace/src/deep")
            .with_files(vec![
                Stub::FileWithContent("workspace/.agents/skills/review/SKILL.md", "name: review"),
                Stub::FileWithContent("workspace/src/deep/WARP.md", "project rules"),
            ]);

        let workspace = dirs.tests().join("workspace");
        let skill_path = workspace.join(".agents/skills/review/SKILL.md");
        let src_path = workspace.join("src");
        let rule_path = workspace.join("src/deep/WARP.md");
        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| {
                let mut model = LocalRepoMetadataModel::new_for_test();
                model.register_force_included_paths([PathBuf::from(".agents/skills")]);
                model.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
                model
            });
            let workspace_path = StandardizedPath::from_local_canonicalized(&workspace).unwrap();
            let skill_path = StandardizedPath::try_from_local(&skill_path).unwrap();
            let src_path = StandardizedPath::try_from_local(&src_path).unwrap();
            let rule_path = StandardizedPath::try_from_local(&rule_path).unwrap();

            model_handle.update(&mut app, |model, ctx| {
                model.index_lazy_loaded_path(&workspace_path, ctx).unwrap();
            });

            model_handle.read(&app, |model, _ctx| {
                let Some(IndexedRepoState::Indexed(state)) =
                    model.repository_state(&workspace_path)
                else {
                    panic!("expected indexed lazy-loaded path");
                };
                assert!(state.entry.contains(&skill_path));
                assert!(
                    matches!(state.entry.get(&src_path), Some(FileTreeEntryState::Directory(dir)) if !dir.loaded)
                );
                assert!(!state.entry.contains(&rule_path));

                let results = model
                    .standing_query_results(&workspace_path)
                    .expect("lazy indexed paths should retain standing results");
                assert!(results
                    .project_skills()
                    .any(|content| content.path == skill_path && !content.is_directory));
                assert!(!results
                    .project_rules()
                    .any(|content| content.path == rule_path));
            });

            let (tx, rx) = oneshot::channel();
            let received_delta = Rc::new(RefCell::new(Some(tx)));
            let received_delta_for_event = received_delta.clone();
            let workspace_path_for_event = workspace_path.clone();
            let skill_path_for_event = skill_path.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&model_handle, move |_, event, _ctx| {
                    if let RepositoryMetadataEvent::StandingQueryResultsUpdated { path, delta } =
                        event
                    {
                        if path == &workspace_path_for_event
                            && delta.upserted_project_skills.iter().any(|content| {
                                content.path == skill_path_for_event && !content.is_directory
                            })
                        {
                            if let Some(tx) = received_delta_for_event.borrow_mut().take() {
                                let _ = tx.send(());
                            }
                        }
                    }
                });
            });

            let skill_path = skill_path.to_local_path().unwrap();
            std::fs::write(&skill_path, "name: updated review").unwrap();
            model_handle.update(&mut app, |model, ctx| {
                model.handle_watcher_event(
                    &BulkFilesystemWatcherEvent {
                        modified: std::collections::HashSet::from([skill_path]),
                        ..Default::default()
                    },
                    ctx,
                );
            });
            rx.with_timeout(Duration::from_secs(5))
                .await
                .expect("timed out waiting for standing project-skill update")
                .expect("standing project-skill update sender dropped");
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_index_directory_path_upgrades_lazy_loaded_non_git_path() {
    VirtualFS::test("lazy_loaded_non_git_path_upgrade", |dirs, mut vfs| {
        vfs.mkdir("repo/src/nested")
            .with_files(vec![Stub::FileWithContent(
                "repo/src/nested/main.rs",
                "fn main() {}\n",
            )]);

        let repo_root = dirs.tests().join("repo");
        let src_dir = repo_root.join("src");
        let source_file = repo_root.join("src/nested/main.rs");

        App::test((), |mut app| async move {
            app.add_singleton_model(DirectoryWatcher::new_for_testing);
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());

            let repo_root_for_index =
                StandardizedPath::from_local_canonicalized(&repo_root).unwrap();
            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_lazy_loaded_path(&repo_root_for_index, ctx)
                    .unwrap();
            });

            model_handle.read(&app, |model, _ctx| {
                assert!(model.is_lazy_loaded_path(&repo_root_for_index));
                let Some(IndexedRepoState::Indexed(state)) =
                    model.repository_state(&repo_root_for_index)
                else {
                    panic!("expected indexed lazy-loaded path");
                };
                assert!(state
                    .entry
                    .contains(&StandardizedPath::try_from_local(&src_dir).unwrap()));
                assert!(!state
                    .entry
                    .contains(&StandardizedPath::try_from_local(&source_file).unwrap()));
            });
            let (tx, rx) = oneshot::channel();
            let repo_root_for_event = repo_root_for_index.clone();
            let upgrade_completed = Rc::new(RefCell::new(Some(tx)));
            let upgrade_completed_for_event = upgrade_completed.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&model_handle, move |_, event, _ctx| {
                    if matches!(
                        event,
                        RepositoryMetadataEvent::RepositoryUpdated { path }
                            if path == &repo_root_for_event
                    ) {
                        if let Some(tx) = upgrade_completed_for_event.borrow_mut().take() {
                            let _ = tx.send(());
                        }
                    }
                });
            });

            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_directory_path(&repo_root_for_index, ctx)
                    .unwrap();
            });
            rx.with_timeout(Duration::from_secs(5))
                .await
                .expect("timed out waiting for full directory upgrade")
                .expect("full directory upgrade completion sender dropped");

            model_handle.read(&app, |model, _ctx| {
                assert!(!model.is_lazy_loaded_path(&repo_root_for_index));
                let Some(IndexedRepoState::Indexed(state)) =
                    model.repository_state(&repo_root_for_index)
                else {
                    panic!("expected fully indexed directory after upgrade");
                };
                assert!(state
                    .entry
                    .contains(&StandardizedPath::try_from_local(&source_file).unwrap()));
            });
        });
    });
}

#[test]
fn test_get_repo_contents_include_ignored() {
    VirtualFS::test("repo_contents_include_ignored_test", |dirs, mut vfs| {
        let test_repo = dirs.tests().join("test_repo");

        // Create a test repository structure with both ignored and non-ignored files
        vfs.mkdir("test_repo/.git/objects")
            .mkdir("test_repo/src")
            .mkdir("test_repo/target/debug")
            .mkdir("test_repo/node_modules")
            .with_files(vec![
                Stub::FileWithContent("test_repo/.git/HEAD", "ref: refs/heads/main"),
                Stub::FileWithContent(
                    "test_repo/.git/config",
                    "[core]\n\trepositoryformatversion = 0",
                ),
                Stub::FileWithContent("test_repo/src/main.rs", "fn main() {}"),
                Stub::FileWithContent("test_repo/README.md", "# Project"),
                Stub::FileWithContent("test_repo/target/debug/binary", "binary"),
                Stub::FileWithContent("test_repo/node_modules/package.json", "{}"),
                Stub::FileWithContent("test_repo/debug.log", "log"),
                Stub::FileWithContent("test_repo/.gitignore", "*.log\n/target/\nnode_modules/"),
            ]);

        // Create mock file tree with ignored and non-ignored entries
        let main_rs = Entry::File(FileMetadata::new(test_repo.join("src/main.rs"), false));
        let readme = Entry::File(FileMetadata::new(test_repo.join("README.md"), false));
        let debug_log = Entry::File(FileMetadata::new(test_repo.join("debug.log"), true));
        let binary = Entry::File(FileMetadata::new(
            test_repo.join("target/debug/binary"),
            true,
        ));
        let package_json = Entry::File(FileMetadata::new(
            test_repo.join("node_modules/package.json"),
            true,
        ));

        let src_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&test_repo.join("src")).unwrap(),
            children: vec![main_rs],
            ignored: false,
            loaded: true,
        });

        let debug_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&test_repo.join("target/debug")).unwrap(),
            children: vec![binary],
            ignored: true,
            loaded: true,
        });

        let target_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&test_repo.join("target")).unwrap(),
            children: vec![debug_dir],
            ignored: true,
            loaded: true,
        });

        let node_modules_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&test_repo.join("node_modules")).unwrap(),
            children: vec![package_json],
            ignored: true,
            loaded: true,
        });

        let root = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&test_repo).unwrap(),
            children: vec![src_dir, readme, debug_log, target_dir, node_modules_dir],
            ignored: false,
            loaded: true,
        });

        let (gitignore, _) = Gitignore::new(test_repo.join(".gitignore"));

        App::test((), |mut app| async move {
            let repo_watcher = app.add_singleton_model(DirectoryWatcher::new);
            let repo_handle = repo_watcher.update(&mut app, |repo_watcher, ctx| {
                repo_watcher
                    .add_directory(
                        StandardizedPath::from_local_canonicalized(&test_repo).unwrap(),
                        ctx,
                    )
                    .unwrap()
            });
            let state = FileTreeState::new(root, vec![gitignore], Some(repo_handle));

            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());

            model_handle.update(&mut app, |model, _ctx| {
                let canonical_key = StandardizedPath::from_local_canonicalized(&test_repo).unwrap();
                model
                    .repositories
                    .insert(canonical_key, IndexedRepoState::Indexed(state));
            });

            // Test with include_ignored = false (should exclude ignored files and directories)
            model_handle.read(&app, |model, _ctx| {
                let args = GetContentsArgs::default();
                let contents = model
                    .get_repo_contents(
                        &StandardizedPath::from_local_canonicalized(&test_repo).unwrap(),
                        args,
                    )
                    .unwrap()
                    .contents;

                let paths: Vec<PathBuf> = contents
                    .iter()
                    .map(|c| match c {
                        crate::RepoContent::File(f) => f.path.to_local_path_lossy(),
                        crate::RepoContent::Directory(d) => d.path.to_local_path_lossy(),
                    })
                    .collect();

                // Should include non-ignored files and directories
                assert!(paths.contains(&test_repo.join("src")));
                assert!(paths.contains(&test_repo.join("src/main.rs")));
                assert!(paths.contains(&test_repo.join("README.md")));

                // Should NOT include ignored directories or files
                assert!(!paths.contains(&test_repo.join("target")));
                assert!(!paths.contains(&test_repo.join("node_modules")));
                assert!(!paths.contains(&test_repo.join("debug.log")));
            });

            // Test with include_ignored = true (should include everything)
            model_handle.read(&app, |model, _ctx| {
                let args = GetContentsArgs::default().include_ignored();
                let contents = model
                    .get_repo_contents(
                        &StandardizedPath::from_local_canonicalized(&test_repo).unwrap(),
                        args,
                    )
                    .unwrap()
                    .contents;

                let paths: Vec<PathBuf> = contents
                    .iter()
                    .map(|c| match c {
                        crate::RepoContent::File(f) => f.path.to_local_path_lossy(),
                        crate::RepoContent::Directory(d) => d.path.to_local_path_lossy(),
                    })
                    .collect();

                // Should include everything
                assert!(paths.contains(&test_repo.join("src")));
                assert!(paths.contains(&test_repo.join("target")));
                assert!(paths.contains(&test_repo.join("target/debug")));
                assert!(paths.contains(&test_repo.join("node_modules")));
                assert!(paths.contains(&test_repo.join("src/main.rs")));
                assert!(paths.contains(&test_repo.join("README.md")));
                assert!(paths.contains(&test_repo.join("debug.log")));
                assert!(paths.contains(&test_repo.join("target/debug/binary")));
                assert!(paths.contains(&test_repo.join("node_modules/package.json")));
            });
        });
    });
}

#[test]
fn test_should_include_path_respects_gitignore() {
    VirtualFS::test("gitignore_test", |dirs, mut fs| {
        let repo_path = dirs.tests();

        // Create directory structure and files using VirtualFS
        fs.mkdir("src")
            .mkdir("target/debug")
            .mkdir("node_modules/package")
            .mkdir("docs")
            .with_files(vec![
                Stub::FileWithContent("debug.log", "log"),
                Stub::FileWithContent("target/debug/main", "binary"),
                Stub::FileWithContent("node_modules/package/index.js", "js"),
                Stub::FileWithContent(".env", "env"),
                Stub::FileWithContent("src/main.rs", "rust"),
                Stub::FileWithContent("README.md", "readme"),
                Stub::FileWithContent("package.json", "json"),
                Stub::FileWithContent("docs/guide.md", "guide"),
                Stub::FileWithContent(".gitignore", "*.log\n/target/\nnode_modules/\n.env"),
            ]);

        let gitignore_path = repo_path.join(".gitignore");

        // Create the gitignore object
        let (gitignore, _) = Gitignore::new(&gitignore_path);
        let gitignores = vec![gitignore];

        // Test files that should be excluded
        let excluded_paths = vec![
            repo_path.join("debug.log"),
            repo_path.join("target").join("debug").join("main"),
            repo_path
                .join("node_modules")
                .join("package")
                .join("index.js"),
            repo_path.join(".env"),
        ];

        for path in excluded_paths {
            assert!(
                LocalRepoMetadataModel::path_is_ignored(&path, &gitignores),
                "Path should be excluded by gitignore: {path:?}"
            );
        }

        // Test files that should be included
        let included_paths = vec![
            repo_path.join("src").join("main.rs"),
            repo_path.join("README.md"),
            repo_path.join("package.json"),
            repo_path.join("docs").join("guide.md"),
        ];

        for path in included_paths {
            assert!(
                !LocalRepoMetadataModel::path_is_ignored(&path, &gitignores),
                "Path should be included: {path:?}"
            );
        }
    });
}

#[test]
fn test_update_file_tree_entry_respects_gitignore() {
    VirtualFS::test("tree_update_test", |dirs, mut fs| {
        let repo_path = dirs.tests();

        // Create initial directory structure and files
        fs.mkdir("src")
            .with_files(vec![
                Stub::FileWithContent("src/main.rs", "fn main() {}"),
                Stub::FileWithContent(".gitignore", "*.log\n/target/"),
                Stub::FileWithContent("debug.log", "log content"),
                Stub::FileWithContent("README.md", "# Project"),
            ])
            .mkdir("target");

        let gitignore_path = repo_path.join(".gitignore");
        let (gitignore, _) = Gitignore::new(&gitignore_path);
        let gitignores = vec![gitignore];

        // Create an initial file tree
        let root_entry = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(repo_path).unwrap(),
            children: vec![Entry::Directory(DirectoryEntry {
                path: StandardizedPath::try_from_local(&repo_path.join("src")).unwrap(),
                children: vec![Entry::File(FileMetadata::new(
                    repo_path.join("src").join("main.rs"),
                    false,
                ))],
                ignored: false,
                loaded: true,
            })],
            ignored: false,
            loaded: true,
        });
        let mut root = FileTreeEntry::from(root_entry);

        // Create files to test adding - some should be ignored
        let log_file = repo_path.join("debug.log");
        let target_dir = repo_path.join("target");
        let readme_file = repo_path.join("README.md");

        // Create update with both ignored and allowed files
        let update = RepoUpdate {
            added: vec![log_file.clone(), readme_file.clone(), target_dir.clone()],
            deleted: vec![],
            moved: HashMap::new(),
        };

        // Compute mutations on the "background thread" then apply on the "main thread".
        let standing_query_definitions = Default::default();
        let (mutations, _, _) = block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
            &update,
            &gitignores,
            &[],
            &standing_query_definitions,
            false,
        ));
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut root, mutations, false, false);

        // Verify that only the README.md was added (log file and target dir should be ignored)
        let mut all_paths = Vec::new();
        collect_all_paths(&root, &mut all_paths);

        // Should contain all files
        let readme_std = StandardizedPath::try_from_local(&readme_file).unwrap();
        let log_std = StandardizedPath::try_from_local(&log_file).unwrap();
        let target_std = StandardizedPath::try_from_local(&target_dir).unwrap();
        assert!(all_paths.contains(&readme_std));
        assert!(all_paths.contains(&log_std));
        assert!(all_paths.contains(&target_std));

        // Make sure that the ignored files and folders are marked as ignored.
        assert!(root
            .get(&StandardizedPath::try_from_local(&log_file).unwrap())
            .unwrap()
            .ignored());
        assert!(root
            .get(&StandardizedPath::try_from_local(&target_dir).unwrap())
            .unwrap()
            .ignored());

        // Make sure that the ignored folder is not eagerly loaded.
        assert!(!root
            .get(&StandardizedPath::try_from_local(&target_dir).unwrap())
            .unwrap()
            .loaded());
    });
}

#[test]
fn test_gitignore_patterns_comprehensive() {
    VirtualFS::test("comprehensive_test", |dirs, mut fs| {
        let repo_path = dirs.tests();

        // Create directory structure and files using VirtualFS
        fs.mkdir("target/debug")
            .mkdir("dist")
            .mkdir("build")
            .mkdir("logs")
            .mkdir("node_modules/react")
            .mkdir("vendor")
            .mkdir(".vscode")
            .mkdir(".idea")
            .mkdir("src")
            .mkdir("docs")
            .mkdir("tests")
            .mkdir(".github/workflows");

        // Create a comprehensive .gitignore
        let gitignore_content = r#"
# Build outputs
/target/
/dist/
build/

# Logs
*.log
logs/

# Dependencies
node_modules/
/vendor/

# IDE files
.vscode/
.idea/
*.swp

# Environment
.env
.env.local

# OS files
.DS_Store
Thumbs.db
"#;

        // Create all files
        fs.with_files(vec![
            Stub::FileWithContent("target/debug/main", "binary"),
            Stub::FileWithContent("dist/bundle.js", "js"),
            Stub::FileWithContent("logs/app.log", "log"),
            Stub::FileWithContent("debug.log", "log"),
            Stub::FileWithContent("node_modules/react/index.js", "js"),
            Stub::FileWithContent(".vscode/settings.json", "json"),
            Stub::FileWithContent(".env", "env"),
            Stub::FileWithContent(".DS_Store", "store"),
            Stub::FileWithContent("temp.swp", "swap"),
            Stub::FileWithContent("src/main.rs", "rust"),
            Stub::FileWithContent("README.md", "readme"),
            Stub::FileWithContent("package.json", "json"),
            Stub::FileWithContent("docs/guide.md", "guide"),
            Stub::FileWithContent("tests/integration.rs", "test"),
            Stub::FileWithContent(".github/workflows/ci.yml", "yml"),
            Stub::FileWithContent(".gitignore", gitignore_content),
        ]);

        let gitignore_path = repo_path.join(".gitignore");

        let (gitignore, _) = Gitignore::new(&gitignore_path);
        let gitignores = vec![gitignore];

        // Test various patterns
        let test_cases = vec![
            // Should be ignored
            (repo_path.join("target").join("debug").join("main"), false),
            (repo_path.join("dist").join("bundle.js"), false),
            (repo_path.join("logs").join("app.log"), false),
            (repo_path.join("debug.log"), false),
            (
                repo_path
                    .join("node_modules")
                    .join("react")
                    .join("index.js"),
                false,
            ),
            (repo_path.join(".vscode").join("settings.json"), false),
            (repo_path.join(".env"), false),
            (repo_path.join(".DS_Store"), false),
            (repo_path.join("temp.swp"), false),
            // Should be included
            (repo_path.join("src").join("main.rs"), true),
            (repo_path.join("README.md"), true),
            (repo_path.join("package.json"), true),
            (repo_path.join("docs").join("guide.md"), true),
            (repo_path.join("tests").join("integration.rs"), true),
            (
                repo_path.join(".github").join("workflows").join("ci.yml"),
                true,
            ),
        ];

        for (path, should_include) in test_cases {
            let actual = !LocalRepoMetadataModel::path_is_ignored(&path, &gitignores);
            assert_eq!(
                actual, should_include,
                "Path {path:?} - expected: {should_include}, actual: {actual}"
            );
        }
    });
}

#[test]
fn test_git_directory_exclusion() {
    VirtualFS::test("git_exclusion_test", |dirs, mut fs| {
        let repo_path = dirs.tests();

        // Create .git directory and files using VirtualFS
        fs.mkdir(".git/objects").mkdir("src").with_files(vec![
            Stub::FileWithContent(".git/config", "config"),
            Stub::FileWithContent(".git/objects/abc123", "object"),
            Stub::FileWithContent("src/main.rs", "rust"),
        ]);

        let gitignores = vec![]; // Empty gitignore rules

        // .git directory and its contents should be excluded
        assert!(LocalRepoMetadataModel::path_is_ignored(
            &repo_path.join(".git"),
            &gitignores
        ));
        assert!(LocalRepoMetadataModel::path_is_ignored(
            &repo_path.join(".git").join("config"),
            &gitignores
        ));
        assert!(LocalRepoMetadataModel::path_is_ignored(
            &repo_path.join(".git").join("objects").join("abc123"),
            &gitignores
        ));

        // Regular files should be included
        assert!(!LocalRepoMetadataModel::path_is_ignored(
            &repo_path.join("src").join("main.rs"),
            &gitignores
        ));
    });
}

#[test]
fn test_nested_gitignore_rules() {
    VirtualFS::test("nested_gitignore_test", |dirs, mut fs| {
        let repo_path = dirs.tests();

        // Create nested directory structure and files using VirtualFS
        fs.mkdir("frontend/dist")
            .mkdir("backend/target")
            .mkdir("frontend/src")
            .with_files(vec![
                Stub::FileWithContent("frontend/dist/bundle.js", "js"),
                Stub::FileWithContent("backend/target/binary", "bin"),
                Stub::FileWithContent("frontend/src/main.ts", "ts"),
                Stub::FileWithContent(".gitignore", "*/dist/\n*/target/"),
                Stub::FileWithContent("frontend/.gitignore", "!dist/important.js"),
            ]);

        // Create gitignore objects
        let root_gitignore_path = repo_path.join(".gitignore");
        let frontend_gitignore_path = repo_path.join("frontend").join(".gitignore");

        let (root_gitignore, _) = Gitignore::new(&root_gitignore_path);
        let (frontend_gitignore, _) = Gitignore::new(&frontend_gitignore_path);
        let gitignores = vec![root_gitignore, frontend_gitignore];

        // Test that nested gitignore rules are respected
        assert!(LocalRepoMetadataModel::path_is_ignored(
            &repo_path.join("frontend").join("dist").join("bundle.js"),
            &gitignores
        ));
        assert!(LocalRepoMetadataModel::path_is_ignored(
            &repo_path.join("backend").join("target").join("binary"),
            &gitignores
        ));
        assert!(!LocalRepoMetadataModel::path_is_ignored(
            &repo_path.join("frontend").join("src").join("main.ts"),
            &gitignores
        ));
    });
}

#[test]
fn test_ensure_parent_directories_exist() {
    use crate::local_model::LocalRepoMetadataModel;

    // Test case 1: Normal operation - creating nested parent directories
    let root_entry = Entry::Directory(DirectoryEntry {
        path: StandardizedPath::try_new("/test_repo").unwrap(),
        children: vec![Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_new("/test_repo/src").unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        })],
        ignored: false,
        loaded: true,
    });
    let mut root = FileTreeEntry::from(root_entry);

    // Try to ensure parent directories exist for a deeply nested path
    LocalRepoMetadataModel::ensure_parent_directories_exist(
        &mut root,
        &StandardizedPath::try_new("/test_repo/src/components/ui/forms").unwrap(),
    );

    // Verify that all intermediate directories were created
    let mut all_paths = Vec::new();
    collect_all_paths(&root, &mut all_paths);

    assert!(all_paths.contains(&StandardizedPath::try_new("/test_repo").unwrap()));
    assert!(all_paths.contains(&StandardizedPath::try_new("/test_repo/src").unwrap()));
    assert!(all_paths.contains(&StandardizedPath::try_new("/test_repo/src/components").unwrap()));
    assert!(all_paths.contains(&StandardizedPath::try_new("/test_repo/src/components/ui").unwrap()));
    assert!(all_paths
        .contains(&StandardizedPath::try_new("/test_repo/src/components/ui/forms").unwrap()));

    // Test case 2: Existing directories should not be recreated
    let initial_count = all_paths.len();
    LocalRepoMetadataModel::ensure_parent_directories_exist(
        &mut root,
        &StandardizedPath::try_new("/test_repo/src/components/ui/forms").unwrap(),
    );

    let mut updated_paths = Vec::new();
    collect_all_paths(&root, &mut updated_paths);
    assert_eq!(
        initial_count,
        updated_paths.len(),
        "No new directories should be created when they already exist"
    );

    // Test case 3: Edge case - file exists where directory is expected
    // This tests the edge case documented in the function's comment
    let root_with_file_conflict_entry = Entry::Directory(DirectoryEntry {
        path: StandardizedPath::try_new("/test_repo").unwrap(),
        children: vec![
            // Create a file at the path where we'll try to create a directory
            Entry::File(FileMetadata::from_standardized(
                StandardizedPath::try_new("/test_repo/conflicting_path").unwrap(),
                false,
            )),
        ],
        ignored: false,
        loaded: true,
    });
    let mut root_with_file_conflict = FileTreeEntry::from(root_with_file_conflict_entry);

    // Try to create parent directories where a file already exists
    LocalRepoMetadataModel::ensure_parent_directories_exist(
        &mut root_with_file_conflict,
        &StandardizedPath::try_new("/test_repo/conflicting_path/nested/deep").unwrap(),
    );

    // Verify that the function returned early and didn't corrupt the tree
    let mut conflict_paths = Vec::new();
    collect_all_paths(&root_with_file_conflict, &mut conflict_paths);

    // The function should detect the file conflict and return early without creating
    // any nested directories beyond the conflicting file.

    // Should still have the original file
    assert!(
        conflict_paths.contains(&StandardizedPath::try_new("/test_repo/conflicting_path").unwrap())
    );
    // Should NOT have created nested directories beyond the conflict
    assert!(!conflict_paths
        .contains(&StandardizedPath::try_new("/test_repo/conflicting_path/nested").unwrap()));
    assert!(!conflict_paths
        .contains(&StandardizedPath::try_new("/test_repo/conflicting_path/nested/deep").unwrap()));

    // Verify the conflicting entry is still a file, not a directory
    let conflicting_entry = root_with_file_conflict
        .get(&StandardizedPath::try_new("/test_repo/conflicting_path").unwrap())
        .expect("Conflicting entry should exist");
    assert!(
        matches!(conflicting_entry, FileTreeEntryState::File(_)),
        "Conflicting entry should remain a file"
    );

    {
        // Test case 3b: File conflict at intermediate level
        let root_with_intermediate_conflict_entry = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_new("/test_repo").unwrap(),
            children: vec![Entry::Directory(DirectoryEntry {
                path: StandardizedPath::try_new("/test_repo/src").unwrap(),
                children: vec![
                    // Create a file where we expect a directory
                    Entry::File(FileMetadata::from_standardized(
                        StandardizedPath::try_new("/test_repo/src/components").unwrap(),
                        false,
                    )),
                ],
                ignored: false,
                loaded: true,
            })],
            ignored: false,
            loaded: true,
        });
        let mut root_with_intermediate_conflict =
            FileTreeEntry::from(root_with_intermediate_conflict_entry);

        // Try to create nested directories where an intermediate path has a file conflict
        LocalRepoMetadataModel::ensure_parent_directories_exist(
            &mut root_with_intermediate_conflict,
            &StandardizedPath::try_new("/test_repo/src/components/ui/forms").unwrap(),
        );

        // Verify that the function handled the conflict properly
        let mut intermediate_conflict_paths = Vec::new();
        collect_all_paths(
            &root_with_intermediate_conflict,
            &mut intermediate_conflict_paths,
        );

        // Should still have the original file at components level
        assert!(intermediate_conflict_paths
            .contains(&StandardizedPath::try_new("/test_repo/src/components").unwrap()));

        // Should NOT have created deeper nested directories beyond the conflict
        assert!(!intermediate_conflict_paths
            .contains(&StandardizedPath::try_new("/test_repo/src/components/ui").unwrap()));
        assert!(!intermediate_conflict_paths
            .contains(&StandardizedPath::try_new("/test_repo/src/components/ui/forms").unwrap()));

        // Verify the conflicting entry is still a file, not a directory
        let conflicting_entry = root_with_intermediate_conflict
            .get(&StandardizedPath::try_new("/test_repo/src/components").unwrap())
            .expect("Conflicting entry should exist");
        assert!(
            matches!(conflicting_entry, FileTreeEntryState::File(_)),
            "Conflicting entry should remain a file"
        );

        // Test case 4: Single level directory creation
        let simple_root_entry = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_new("/simple").unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        });
        let mut simple_root = FileTreeEntry::from(simple_root_entry);

        let simple_target = StandardizedPath::try_new("/simple/new_dir").unwrap();
        LocalRepoMetadataModel::ensure_parent_directories_exist(&mut simple_root, &simple_target);

        let mut simple_paths = Vec::new();
        collect_all_paths(&simple_root, &mut simple_paths);
        assert!(simple_paths.contains(&StandardizedPath::try_new("/simple/new_dir").unwrap()));

        // Test case 5: Target parent is the root itself (edge case)
        let root_target_entry = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_new("/root").unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        });
        let mut root_target = FileTreeEntry::from(root_target_entry);

        // This should not crash or create any new directories
        LocalRepoMetadataModel::ensure_parent_directories_exist(
            &mut root_target,
            &StandardizedPath::try_new("/root").unwrap(),
        );

        let mut root_paths = Vec::new();
        collect_all_paths(&root_target, &mut root_paths);
        assert_eq!(root_paths.len(), 1); // Should only contain the root itself

        // Test case 6: Empty path handling
        let empty_root_entry = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_new("/empty").unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        });
        let mut empty_root = FileTreeEntry::from(empty_root_entry);

        // Test with a path that has no additional parents to create
        let same_level_target = StandardizedPath::try_new("/empty").unwrap();
        LocalRepoMetadataModel::ensure_parent_directories_exist(
            &mut empty_root,
            &same_level_target,
        );

        let mut empty_paths = Vec::new();
        collect_all_paths(&empty_root, &mut empty_paths);
        assert_eq!(empty_paths.len(), 1); // Should still only contain the root
    }
}

/// Helper function to collect all paths in a file tree
fn collect_all_paths(entry: &FileTreeEntry, paths: &mut Vec<StandardizedPath>) {
    let root_path = entry.root_directory().clone();
    collect_paths_recursive(entry, &root_path, paths);
}

fn collect_paths_recursive(
    entry: &FileTreeEntry,
    current_path: &StandardizedPath,
    paths: &mut Vec<StandardizedPath>,
) {
    paths.push(current_path.clone());
    if let Some(FileTreeEntryState::Directory(_)) = entry.get(current_path) {
        for child in entry.child_paths(current_path) {
            collect_paths_recursive(entry, child, paths);
        }
    }
}

#[cfg(unix)]
#[test]
fn added_symlinked_skill_directory_refreshes_provider_without_canonical_tree_mutation() {
    VirtualFS::test("added_symlinked_skill_directory", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills")
            .mkdir("linked-skill-target")
            .with_files(vec![Stub::FileWithContent(
                "linked-skill-target/SKILL.md",
                "linked skill",
            )]);
        let repo = dirs.tests().join("repo");
        let provider = repo.join(".agents/skills");
        let linked_skill = provider.join("linked-skill");
        std::os::unix::fs::symlink(dirs.tests().join("linked-skill-target"), &linked_skill)
            .unwrap();

        let mut definitions = StandingQueryDefinitions::default();
        definitions.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
        let update = RepoUpdate {
            added: vec![linked_skill.clone()],
            ..Default::default()
        };
        let (mutations, discovered, removed_roots) =
            block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
                &update,
                &[],
                &[],
                &definitions,
                false,
            ));

        assert!(mutations.is_empty());
        assert!(removed_roots.is_empty());
        assert!(discovered.project_skills().any(|content| {
            content
                == &StandingQueryContent::directory(
                    StandardizedPath::try_from_local(&provider).unwrap(),
                )
        }));
        assert!(discovered.project_skills().any(|content| {
            content
                == &StandingQueryContent::file(
                    StandardizedPath::try_from_local(&linked_skill.join("SKILL.md")).unwrap(),
                )
        }));
    });
}

#[test]
fn unrelated_skill_support_file_does_not_refresh_project_skills() {
    VirtualFS::test("unrelated_skill_support_file", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/review")
            .with_files(vec![Stub::FileWithContent(
                "repo/.agents/skills/review/README.md",
                "notes",
            )]);
        let repo = dirs.tests().join("repo");
        let support_file = repo.join(".agents/skills/review/README.md");

        let mut definitions = StandingQueryDefinitions::default();
        definitions.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
        let update = RepoUpdate {
            added: vec![support_file],
            ..Default::default()
        };
        let (_, discovered, _) = block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
            &update,
            &[],
            &[],
            &definitions,
            false,
        ));

        assert!(discovered.project_skills().next().is_none());
    });
}

#[test]
fn removed_direct_skill_child_refreshes_provider_for_possible_symlink_removal() {
    VirtualFS::test("removed_direct_skill_child", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills");
        let provider = dirs.tests().join("repo/.agents/skills");

        let mut definitions = StandingQueryDefinitions::default();
        definitions.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
        let update = RepoUpdate {
            deleted: vec![provider.join("removed-skill")],
            ..Default::default()
        };
        let (_, discovered, _) = block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
            &update,
            &[],
            &[],
            &definitions,
            false,
        ));

        assert!(discovered.project_skills().any(|content| {
            content
                == &StandingQueryContent::directory(
                    StandardizedPath::try_from_local(&provider).unwrap(),
                )
        }));
    });
}
#[cfg(all(unix, feature = "local_fs"))]
#[test]
fn added_external_target_skill_symlink_routes_to_lexical_repository() {
    VirtualFS::test(
        "added_external_target_skill_symlink_routing",
        |dirs, mut vfs| {
            vfs.mkdir("repo/.agents/skills")
                .mkdir("outside/linked-skill")
                .with_files(vec![Stub::FileWithContent(
                    "outside/linked-skill/SKILL.md",
                    "linked skill",
                )]);
            let repo = dirs.tests().join("repo");
            let provider = repo.join(".agents/skills");
            let linked_skill = provider.join("linked-skill");
            std::os::unix::fs::symlink(dirs.tests().join("outside/linked-skill"), &linked_skill)
                .unwrap();

            App::test((), |mut app| async move {
                let repo_path = StandardizedPath::from_local_canonicalized(&repo).unwrap();
                let provider_path = StandardizedPath::try_from_local(&provider).unwrap();
                let model_handle = app.add_model(|_| {
                    let mut model = LocalRepoMetadataModel::new_for_test();
                    model.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
                    model
                });
                model_handle.update(&mut app, |model, _ctx| {
                    model.repositories.insert(
                        repo_path.clone(),
                        IndexedRepoState::Indexed(empty_repo_state(&repo_path)),
                    );
                });

                let (tx, rx) = oneshot::channel();
                let received_delta = Rc::new(RefCell::new(Some(tx)));
                let received_delta_for_event = received_delta.clone();
                let repo_path_for_event = repo_path.clone();
                let provider_path_for_event = provider_path.clone();
                app.update(|ctx| {
                    ctx.subscribe_to_model(&model_handle, move |_, event, _ctx| {
                        if let RepositoryMetadataEvent::StandingQueryResultsUpdated {
                            path,
                            delta,
                        } = event
                        {
                            if path == &repo_path_for_event
                                && delta.upserted_project_skills.iter().any(|content| {
                                    content
                                        == &StandingQueryContent::directory(
                                            provider_path_for_event.clone(),
                                        )
                                })
                            {
                                if let Some(tx) = received_delta_for_event.borrow_mut().take() {
                                    let _ = tx.send(());
                                }
                            }
                        }
                    });
                });

                model_handle.update(&mut app, |model, ctx| {
                    model.handle_watcher_event(
                        &BulkFilesystemWatcherEvent {
                            added: std::collections::HashSet::from([linked_skill]),
                            ..Default::default()
                        },
                        ctx,
                    );
                });
                rx.with_timeout(Duration::from_secs(5))
                    .await
                    .expect("timed out waiting for standing project-skill update")
                    .expect("standing project-skill update sender dropped");

                model_handle.read(&app, |model, _ctx| {
                    assert!(model
                        .standing_query_results(&repo_path)
                        .expect("standing results should be retained for the repository")
                        .project_skills()
                        .any(|content| content
                            == &StandingQueryContent::directory(provider_path.clone())));
                });
            });
        },
    );
}
#[cfg(all(unix, feature = "local_fs"))]
#[test]
fn modified_external_symlink_target_upserts_lexical_project_skill() {
    VirtualFS::test(
        "modified_external_symlink_target_upserts_lexical_project_skill",
        |dirs, mut vfs| {
            vfs.mkdir("repo/.agents/skills")
                .mkdir("outside/linked-skill")
                .with_files(vec![Stub::FileWithContent(
                    "outside/linked-skill/SKILL.md",
                    "linked skill",
                )]);
            let repo = dirs.tests().join("repo");
            let logical_skill_dir = repo.join(".agents/skills/linked-skill");
            let logical_skill_path = logical_skill_dir.join("SKILL.md");
            let target_skill_path = dirs.tests().join("outside/linked-skill/SKILL.md");
            std::os::unix::fs::symlink(
                dirs.tests().join("outside/linked-skill"),
                &logical_skill_dir,
            )
            .unwrap();

            App::test((), |mut app| async move {
                let repo_path = StandardizedPath::from_local_canonicalized(&repo).unwrap();
                let logical_skill_path =
                    StandardizedPath::try_from_local(&logical_skill_path).unwrap();
                let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
                model_handle.update(&mut app, |model, ctx| {
                    model.set_emit_incremental_updates(true);
                    model.register_force_included_paths([PathBuf::from(".agents/skills")]);
                    model.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
                    model.repositories.insert(
                        repo_path.clone(),
                        IndexedRepoState::Indexed(empty_repo_state(&repo_path)),
                    );
                    let mut results = StandingQueryResults::default();
                    results.insert_project_skill(StandingQueryContent::file(
                        logical_skill_path.clone(),
                    ));
                    model.standing_results.insert(repo_path.clone(), results);
                    model.refresh_symlink_targets(&repo_path, ctx);
                });

                let (tx, rx) = oneshot::channel();
                let received_delta = Rc::new(RefCell::new(Some(tx)));
                let received_delta_for_event = received_delta.clone();
                let logical_skill_path_for_event = logical_skill_path.clone();
                app.update(|ctx| {
                    ctx.subscribe_to_model(&model_handle, move |_, event, _ctx| {
                        if let RepositoryMetadataEvent::IncrementalUpdateReady { update } = event {
                            if update
                                .standing_results_delta
                                .upserted_project_skills
                                .iter()
                                .any(|content| {
                                    content
                                        == &StandingQueryContent::file(
                                            logical_skill_path_for_event.clone(),
                                        )
                                })
                            {
                                if let Some(tx) = received_delta_for_event.borrow_mut().take() {
                                    let _ = tx.send(());
                                }
                            }
                        }
                    });
                });

                model_handle.update(&mut app, |model, ctx| {
                    model.handle_watcher_event(
                        &BulkFilesystemWatcherEvent {
                            modified: std::collections::HashSet::from([target_skill_path]),
                            ..Default::default()
                        },
                        ctx,
                    );
                });

                rx.with_timeout(Duration::from_secs(5))
                    .await
                    .expect("timed out waiting for symlink target upsert")
                    .expect("symlink target upsert sender dropped");
            });
        },
    );
}

#[cfg(all(unix, feature = "local_fs"))]
#[test]
fn removed_then_recreated_external_symlink_target_refreshes_lexical_project_skill() {
    VirtualFS::test(
        "removed_then_recreated_external_symlink_target_refreshes_lexical_project_skill",
        |dirs, mut vfs| {
            vfs.mkdir("repo/.agents/skills")
                .mkdir("outside/linked-skill")
                .with_files(vec![Stub::FileWithContent(
                    "outside/linked-skill/SKILL.md",
                    "linked skill",
                )]);
            let repo = dirs.tests().join("repo");
            let logical_skill_dir = repo.join(".agents/skills/linked-skill");
            let logical_skill_path = logical_skill_dir.join("SKILL.md");
            let target_skill_path = dirs.tests().join("outside/linked-skill/SKILL.md");
            std::os::unix::fs::symlink(
                dirs.tests().join("outside/linked-skill"),
                &logical_skill_dir,
            )
            .unwrap();

            App::test((), |mut app| async move {
                let repo_path = StandardizedPath::from_local_canonicalized(&repo).unwrap();
                let logical_skill_path =
                    StandardizedPath::try_from_local(&logical_skill_path).unwrap();
                let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
                model_handle.update(&mut app, |model, ctx| {
                    model.set_emit_incremental_updates(true);
                    model.register_force_included_paths([PathBuf::from(".agents/skills")]);
                    model.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
                    model.repositories.insert(
                        repo_path.clone(),
                        IndexedRepoState::Indexed(empty_repo_state(&repo_path)),
                    );
                    let mut results = StandingQueryResults::default();
                    results.insert_project_skill(StandingQueryContent::file(
                        logical_skill_path.clone(),
                    ));
                    model.standing_results.insert(repo_path.clone(), results);
                    model.refresh_symlink_targets(&repo_path, ctx);
                });

                let (tx, rx) = oneshot::channel();
                let received_delta = Rc::new(RefCell::new(Some(tx)));
                let received_delta_for_event = received_delta.clone();
                let logical_skill_path_for_event = logical_skill_path.clone();
                app.update(|ctx| {
                    ctx.subscribe_to_model(&model_handle, move |_, event, _ctx| {
                        if let RepositoryMetadataEvent::StandingQueryResultsUpdated {
                            delta, ..
                        } = event
                        {
                            if delta.removed_project_skills.iter().any(|content| {
                                content
                                    == &StandingQueryContent::file(
                                        logical_skill_path_for_event.clone(),
                                    )
                            }) {
                                if let Some(tx) = received_delta_for_event.borrow_mut().take() {
                                    let _ = tx.send(());
                                }
                            }
                        }
                    });
                });

                std::fs::remove_file(&target_skill_path).unwrap();
                model_handle.update(&mut app, |model, ctx| {
                    model.handle_watcher_event(
                        &BulkFilesystemWatcherEvent {
                            deleted: std::collections::HashSet::from([target_skill_path.clone()]),
                            ..Default::default()
                        },
                        ctx,
                    );
                });

                rx.with_timeout(Duration::from_secs(5))
                    .await
                    .expect("timed out waiting for symlink target removal")
                    .expect("symlink target removal sender dropped");
                model_handle.read(&app, |model, _ctx| {
                    assert!(model
                        .standing_query_results(&repo_path)
                        .expect("standing results should remain tracked")
                        .project_skills()
                        .all(|content| content
                            != &StandingQueryContent::file(logical_skill_path.clone())));
                });

                let (tx, rx) = oneshot::channel();
                let received_delta = Rc::new(RefCell::new(Some(tx)));
                let received_delta_for_event = received_delta.clone();
                let logical_skill_path_for_event = logical_skill_path.clone();
                app.update(|ctx| {
                    ctx.subscribe_to_model(&model_handle, move |_, event, _ctx| {
                        if let RepositoryMetadataEvent::StandingQueryResultsUpdated {
                            delta, ..
                        } = event
                        {
                            if delta.upserted_project_skills.iter().any(|content| {
                                content
                                    == &StandingQueryContent::file(
                                        logical_skill_path_for_event.clone(),
                                    )
                            }) {
                                if let Some(tx) = received_delta_for_event.borrow_mut().take() {
                                    let _ = tx.send(());
                                }
                            }
                        }
                    });
                });
                std::fs::write(&target_skill_path, "linked skill").unwrap();
                model_handle.update(&mut app, |model, ctx| {
                    model.handle_watcher_event(
                        &BulkFilesystemWatcherEvent {
                            added: std::collections::HashSet::from([target_skill_path]),
                            ..Default::default()
                        },
                        ctx,
                    );
                });
                rx.with_timeout(Duration::from_secs(5))
                    .await
                    .expect("timed out waiting for recreated symlink target upsert")
                    .expect("recreated symlink target upsert sender dropped");
            });
        },
    );
}

#[cfg(all(unix, feature = "local_fs"))]
#[test]
fn symlink_targets_retain_aliases_and_clear_for_removed_or_failed_repositories() {
    VirtualFS::test(
        "symlink_targets_retain_aliases_and_clear_for_removed_or_failed_repositories",
        |dirs, mut vfs| {
            vfs.mkdir("repo/.agents/skills")
                .mkdir("outside/shared-skill")
                .with_files(vec![Stub::FileWithContent(
                    "outside/shared-skill/SKILL.md",
                    "shared skill",
                )]);
            let repo = dirs.tests().join("repo");
            let target = dirs.tests().join("outside/shared-skill");
            std::os::unix::fs::symlink(&target, repo.join(".agents/skills/first")).unwrap();
            std::os::unix::fs::symlink(&target, repo.join(".agents/skills/second")).unwrap();

            App::test((), |mut app| async move {
                let repo_path = StandardizedPath::from_local_canonicalized(&repo).unwrap();
                let target = dunce::canonicalize(target).unwrap();
                let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
                model_handle.update(&mut app, |model, ctx| {
                    model.set_emit_incremental_updates(true);
                    model.register_force_included_paths([PathBuf::from(".agents/skills")]);
                    model.repositories.insert(
                        repo_path.clone(),
                        IndexedRepoState::Indexed(empty_repo_state(&repo_path)),
                    );
                    model.refresh_symlink_targets(&repo_path, ctx);
                });

                model_handle.read(&app, |model, _ctx| {
                    assert_eq!(
                        model
                            .symlink_targets
                            .get(&target)
                            .expect("resolved target should be tracked")
                            .len(),
                        2
                    );
                });

                model_handle.update(&mut app, |model, ctx| {
                    model
                        .remove_repository(&repo_path, ctx)
                        .expect("repository should be removed");
                });
                model_handle.read(&app, |model, _ctx| {
                    assert!(model.symlink_targets.is_empty());
                });

                model_handle.update(&mut app, |model, ctx| {
                    model.repositories.insert(
                        repo_path.clone(),
                        IndexedRepoState::Indexed(empty_repo_state(&repo_path)),
                    );
                    model.refresh_symlink_targets(&repo_path, ctx);
                    model.mark_repository_failed(
                        repo_path.clone(),
                        RepoMetadataError::RepoNotFound(repo_path.to_string()),
                        ctx,
                    );
                });
                model_handle.read(&app, |model, _ctx| {
                    assert!(model.symlink_targets.is_empty());
                });
            });
        },
    );
}

#[cfg(all(unix, feature = "local_fs"))]
#[test]
fn removed_external_symlink_target_directory_queues_lexical_removal_and_clears_mapping() {
    VirtualFS::test(
        "removed_external_symlink_target_directory_queues_lexical_removal_and_clears_mapping",
        |dirs, mut vfs| {
            vfs.mkdir("repo/.agents/skills")
                .mkdir("outside/linked-skill")
                .with_files(vec![Stub::FileWithContent(
                    "outside/linked-skill/SKILL.md",
                    "linked skill",
                )]);
            let repo = dirs.tests().join("repo");
            let logical_skill_dir = repo.join(".agents/skills/linked-skill");
            let target_skill_dir = dirs.tests().join("outside/linked-skill");
            std::os::unix::fs::symlink(&target_skill_dir, &logical_skill_dir).unwrap();

            App::test((), |mut app| async move {
                let repo_path = StandardizedPath::from_local_canonicalized(&repo).unwrap();
                let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
                model_handle.update(&mut app, |model, ctx| {
                    model.set_emit_incremental_updates(true);
                    model.register_force_included_paths([PathBuf::from(".agents/skills")]);
                    model.repositories.insert(
                        repo_path.clone(),
                        IndexedRepoState::Indexed(empty_repo_state(&repo_path)),
                    );
                    model.refresh_symlink_targets(&repo_path, ctx);
                });

                std::fs::remove_dir_all(&target_skill_dir).unwrap();
                model_handle.update(&mut app, |model, ctx| {
                    let mut repo_updates = HashMap::new();
                    let event = BulkFilesystemWatcherEvent {
                        deleted: std::collections::HashSet::from([target_skill_dir.clone()]),
                        ..Default::default()
                    };
                    let matched_paths = model.add_symlink_target_updates(&event, &mut repo_updates);
                    assert!(matched_paths.contains(&target_skill_dir));
                    let update = repo_updates
                        .get(&repo_path)
                        .expect("target directory deletion should queue a lexical refresh");
                    assert_eq!(update.deleted, vec![logical_skill_dir.clone()]);
                    assert!(update.added.is_empty());

                    model.refresh_symlink_targets(&repo_path, ctx);
                    assert!(model.symlink_targets.is_empty());
                });
            });
        },
    );
}
#[test]
fn test_canonicalized_path_functionality() {
    use warp_util::standardized_path::StandardizedPath;
    VirtualFS::test("canonicalized_path_test", |dirs, mut vfs| {
        let repo_path = dirs.tests();

        // Create a directory structure with symlinks
        vfs.mkdir("real_dir/subdir")
            .mkdir("other_dir")
            .with_files(vec![
                Stub::FileWithContent("real_dir/file.txt", "content"),
                Stub::FileWithContent("real_dir/subdir/nested.rs", "rust code"),
            ]);

        let real_dir = repo_path.join("real_dir");
        let symlink_dir = repo_path.join("symlinked_dir");
        let relative_path = repo_path.join("./real_dir");

        // Create a symlink to real_dir
        #[cfg(unix)]
        let symlink_created = std::os::unix::fs::symlink(&real_dir, &symlink_dir).is_ok();
        #[cfg(windows)]
        let symlink_created = std::os::windows::fs::symlink_dir(&real_dir, &symlink_dir).is_ok();

        if symlink_created {
            // Test that different path representations canonicalize to the same path
            let canonical_real = StandardizedPath::from_local_canonicalized(&real_dir).unwrap();
            let canonical_symlink =
                StandardizedPath::from_local_canonicalized(&symlink_dir).unwrap();
            let canonical_relative =
                StandardizedPath::from_local_canonicalized(&relative_path).unwrap();

            // All should point to the same canonical path
            assert_eq!(canonical_real, canonical_symlink);
            assert_eq!(canonical_real, canonical_relative);

            // Test that the canonical path is absolute and resolved
            let local = canonical_real.to_local_path().unwrap();
            assert!(local.is_absolute());
            assert!(!local.to_string_lossy().contains("./"));
        }

        // Test with various input types
        let path_buf = real_dir.clone();
        let path_ref = real_dir.as_path();

        let canonical_from_pathbuf = StandardizedPath::from_local_canonicalized(&path_buf).unwrap();
        let canonical_from_path = StandardizedPath::from_local_canonicalized(path_ref).unwrap();

        // All should be equal
        assert_eq!(canonical_from_pathbuf, canonical_from_path);

        // Test conversion to local path
        let canonical = StandardizedPath::from_local_canonicalized(&real_dir).unwrap();
        let local_path = canonical.to_local_path().unwrap();

        // Test internal consistency - compare with dunce-canonicalized version
        let expected_canonical = dunce::canonicalize(&real_dir).unwrap();
        assert_eq!(local_path, expected_canonical);

        // Test error handling for non-existent paths
        let nonexistent = repo_path.join("nonexistent");
        let result = StandardizedPath::from_local_canonicalized(&nonexistent);
        assert!(result.is_err());
    });
}

#[test]
fn test_repository_operations_with_standardized_paths() {
    use warp_util::standardized_path::StandardizedPath;

    VirtualFS::test("repo_canonicalized_test", |dirs, mut vfs| {
        let test_root = dirs.tests();

        // Create a real repository directory
        vfs.mkdir("real_repo/src")
            .mkdir("other_location")
            .with_files(vec![
                Stub::FileWithContent("real_repo/src/main.rs", "fn main() {}"),
                Stub::FileWithContent("real_repo/.gitignore", "*.log\n/target/"),
                Stub::FileWithContent("real_repo/README.md", "# Project"),
            ]);

        let real_repo = test_root.join("real_repo");
        let symlink_repo = test_root.join("symlinked_repo");
        let relative_repo = test_root.join("./real_repo");

        // Create symlink to the repo
        #[cfg(unix)]
        let symlink_created = std::os::unix::fs::symlink(&real_repo, &symlink_repo).is_ok();
        #[cfg(windows)]
        let symlink_created = std::os::windows::fs::symlink_dir(&real_repo, &symlink_repo).is_ok();

        if symlink_created {
            App::test((), |mut app| async move {
                let repo_watcher = app.add_singleton_model(DirectoryWatcher::new);
                let _detected_repo = app.add_singleton_model(|_| DetectedRepositories::default());
                let model_handle = app.add_model(LocalRepoMetadataModel::new);

                // Create file tree state for testing
                let src_file = Entry::File(FileMetadata::new(real_repo.join("src/main.rs"), false));
                let readme_file =
                    Entry::File(FileMetadata::new(real_repo.join("README.md"), false));
                let src_dir = Entry::Directory(DirectoryEntry {
                    path: StandardizedPath::try_from_local(&real_repo.join("src")).unwrap(),
                    children: vec![src_file],
                    ignored: false,
                    loaded: true,
                });
                let root = Entry::Directory(DirectoryEntry {
                    path: StandardizedPath::try_from_local(&real_repo).unwrap(),
                    children: vec![src_dir, readme_file],
                    ignored: false,
                    loaded: true,
                });

                let (gitignore, _) = Gitignore::new(real_repo.join(".gitignore"));
                let repo_handle = repo_watcher.update(&mut app, |repo_watcher, ctx| {
                    repo_watcher
                        .add_directory(
                            StandardizedPath::from_local_canonicalized(&real_repo).unwrap(),
                            ctx,
                        )
                        .unwrap()
                });
                let state = FileTreeState::new(root, vec![gitignore], Some(repo_handle));

                // Test adding repository using different path representations
                model_handle.update(&mut app, |model, ctx| {
                    // Add using real path
                    let result1 = model.add_repository_internal(
                        StandardizedPath::from_local_canonicalized(&real_repo).unwrap(),
                        state.clone(),
                        RootWatchMode::Recursive,
                        ctx,
                    );
                    assert!(result1.is_ok());

                    // Try to add using symlink path - this should canonicalize to the same path
                    let result2 = model.add_repository_internal(
                        StandardizedPath::from_local_canonicalized(&symlink_repo).unwrap(),
                        state.clone(),
                        RootWatchMode::Recursive,
                        ctx,
                    );
                    assert!(result2.is_ok());

                    // Try to add using relative path
                    let result3 = model.add_repository_internal(
                        StandardizedPath::from_local_canonicalized(&relative_repo).unwrap(),
                        state.clone(),
                        RootWatchMode::Recursive,
                        ctx,
                    );
                    assert!(result3.is_ok());

                    // Verify that only one repository entry exists (all paths canonicalized to the same)
                    let canonical_path =
                        StandardizedPath::from_local_canonicalized(&real_repo).unwrap();
                    assert!(model.repositories.contains_key(&canonical_path));
                });

                // Test find_repository_for_path with different path formats
                model_handle.read(&app, |model, _ctx| {
                    let file_in_repo = real_repo.join("src/main.rs");
                    let symlink_file = symlink_repo.join("src/main.rs");

                    let found_real = model.find_repository_for_path(&file_in_repo);
                    let found_symlink = model.find_repository_for_path(&symlink_file);

                    // Both should find the same repository
                    assert!(found_real.is_some());
                    assert!(found_symlink.is_some());
                    assert_eq!(found_real, found_symlink);
                });
            });
        }
    });
}

#[test]
fn test_standardized_path_edge_cases() {
    use warp_util::standardized_path::StandardizedPath;

    VirtualFS::test("canonicalized_edge_cases", |dirs, mut vfs| {
        let test_root = dirs.tests();

        // Create test files and directories
        vfs.mkdir("existing_dir")
            .with_files(vec![Stub::FileWithContent("existing_file.txt", "content")]);

        let existing_dir = test_root.join("existing_dir");
        let existing_file = test_root.join("existing_file.txt");
        let nonexistent = test_root.join("nonexistent");

        // Test successful canonicalization
        assert!(StandardizedPath::from_local_canonicalized(&existing_dir).is_ok());
        assert!(StandardizedPath::from_local_canonicalized(&existing_file).is_ok());

        // Test failed canonicalization
        assert!(StandardizedPath::from_local_canonicalized(&nonexistent).is_err());

        // Test equality and hashing
        let canonical1 = StandardizedPath::from_local_canonicalized(&existing_dir).unwrap();
        let canonical2 = StandardizedPath::from_local_canonicalized(&existing_dir).unwrap();

        assert_eq!(canonical1, canonical2);

        // Test that they can be used in HashMaps
        let mut map = std::collections::HashMap::new();
        map.insert(canonical1.clone(), "value1");
        assert_eq!(map.get(&canonical2), Some(&"value1"));

        // Test Debug trait
        let debug_str = format!("{canonical1:?}");
        assert!(debug_str.contains("StandardizedPath"));
    });
}

/// On Linux, a lazy (non-git) root is watched non-recursively, so only the root
/// itself should be tracked initially. On other platforms the root is watched
/// recursively and nothing is tracked for per-directory teardown.
#[cfg(feature = "local_fs")]
#[test]
fn index_lazy_loaded_path_tracks_only_root() {
    VirtualFS::test("lazy_root_tracking", |dirs, mut vfs| {
        vfs.mkdir("workspace/sub")
            .with_files(vec![Stub::FileWithContent("workspace/file.txt", "x")]);
        let root =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace")).unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_lazy_loaded_path(&root, ctx)
                    .expect("should index lazy path");
            });

            model_handle.read(&app, |model, _ctx| {
                assert!(model.is_lazy_loaded_path(&root));
                let repo_watch = model
                    .repo_watches
                    .get(&root)
                    .expect("watch should be recorded");
                if cfg!(target_os = "linux") {
                    // Linux: the root is watched non-recursively and no subdirs
                    // are tracked yet.
                    assert_eq!(repo_watch.root_mode, RootWatchMode::NonRecursive);
                    assert!(repo_watch.extra_dirs.is_empty());
                } else {
                    // Other platforms: a single recursive watch on the root.
                    assert_eq!(repo_watch.root_mode, RootWatchMode::Recursive);
                }
            });
        });
    });
}

/// Expanding a subdirectory of a lazy root should add a per-directory watch for
/// it on Linux (so its children stay fresh) while leaving non-Linux untouched.
#[cfg(feature = "local_fs")]
#[test]
fn load_directory_tracks_expanded_subdir_for_lazy_root() {
    VirtualFS::test("lazy_load_subdir_tracking", |dirs, mut vfs| {
        vfs.mkdir("workspace/sub/inner");
        let root =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace")).unwrap();
        let sub = StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace/sub"))
            .unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_lazy_loaded_path(&root, ctx)
                    .expect("should index lazy path");
                model
                    .load_directory(&root, &sub, ctx)
                    .expect("should load subdir");
            });

            model_handle.read(&app, |model, _ctx| {
                let repo_watch = model
                    .repo_watches
                    .get(&root)
                    .expect("watch should be recorded");
                if cfg!(target_os = "linux") {
                    // Linux: the expanded subdir now has its own non-recursive
                    // watch; the root is never stored in `extra_dirs`.
                    assert_eq!(repo_watch.root_mode, RootWatchMode::NonRecursive);
                    assert!(repo_watch.extra_dirs.contains(&sub));
                    assert!(!repo_watch.extra_dirs.contains(&root));
                } else {
                    // Other platforms: a single recursive watch on the root.
                    assert_eq!(repo_watch.root_mode, RootWatchMode::Recursive);
                }
            });
        });
    });
}

/// Indexing a git repo records a recursive watch mode (not a lazy one) and is
/// not tracked as a lazy-loaded path, on any platform.
#[cfg(feature = "local_fs")]
#[test]
fn recursive_repo_uses_recursive_watch_mode() {
    VirtualFS::test("recursive_repo_watch_mode", |dirs, mut vfs| {
        vfs.mkdir("repo/src");
        let repo_path =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("repo")).unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            model_handle.update(&mut app, |model, ctx| {
                model
                    .add_repository_internal(
                        repo_path.clone(),
                        empty_repo_state(&repo_path),
                        RootWatchMode::Recursive,
                        ctx,
                    )
                    .expect("repo should index");
            });

            model_handle.read(&app, |model, _ctx| {
                let repo_watch = model
                    .repo_watches
                    .get(&repo_path)
                    .expect("watch should be recorded");
                assert_eq!(repo_watch.root_mode, RootWatchMode::Recursive);
                assert!(repo_watch.extra_dirs.is_empty());
                assert!(!model.is_lazy_loaded_path(&repo_path));
            });
        });
    });
}

#[test]
fn incremental_force_included_dir_under_ignored_parent_matches_initial_index() {
    fn find_entry<'a>(entry: &'a Entry, target: &StandardizedPath) -> Option<&'a Entry> {
        if entry.path() == target {
            return Some(entry);
        }
        if let Entry::Directory(dir) = entry {
            for child in &dir.children {
                if let Some(found) = find_entry(child, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    VirtualFS::test(
        "incremental_force_included_under_ignored_parent",
        |dirs, mut vfs| {
            // `.agents/` is ignored by the repo-root .gitignore; `.agents/skills`
            // is force-included, so it is ignored only because of its ancestor.
            vfs.mkdir("repo/.agents/skills").with_files(vec![
                Stub::FileWithContent("repo/.gitignore", ".agents/\n"),
                Stub::FileWithContent("repo/.agents/skills/SKILL.md", "skill"),
            ]);

            let repo_local = dirs.tests().join("repo");
            let skills_local = repo_local.join(".agents").join("skills");

            let force_included = vec![PathBuf::from(".agents/skills")];
            let gitignores = crate::gitignores_for_directory(&repo_local);
            let definitions = StandingQueryDefinitions::default();

            // Ground truth: how the initial full index classifies `.agents/skills`.
            // Mirrors `index_directory`, which builds from the repo root with
            // `IncludeLazy` + force-included paths so the ignored `.agents`
            // ancestor propagates down into `.agents/skills`.
            let expected_ignored = {
                let mut files = Vec::new();
                let mut gitignores = gitignores.clone();
                let mut budget = 100_000usize;
                let mut standing_results = crate::StandingQueryResults::default();
                let root = Entry::build_tree_with_standing_queries(
                    &repo_local,
                    &mut files,
                    &mut gitignores,
                    Some(&mut budget),
                    BuildTreeOptions {
                        max_depth: 64,
                        current_depth: 0,
                        ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                        force_included_paths: &force_included,
                        budget_exceeded_behavior: BudgetExceededBehavior::StopAndLazyLoad,
                    },
                    false,
                    &mut standing_results,
                    &definitions,
                )
                .expect("initial index build should succeed");

                let skills_canonical =
                    dunce::canonicalize(&skills_local).expect("skills dir should exist");
                let skills_node_path =
                    StandardizedPath::from_local_absolute_unchecked(&skills_canonical);
                find_entry(&root, &skills_node_path)
                    .expect("`.agents/skills` should be materialized by the initial index")
                    .ignored()
            };

            assert!(
                expected_ignored,
                "fixture sanity: the initial index should mark `.agents/skills` ignored \
                 via its `.agents` ancestor"
            );

            // Incremental watcher path: `.agents/skills` is reported as added.
            let update = RepoUpdate {
                added: vec![skills_local.clone()],
                ..Default::default()
            };
            let (mutations, _standing_results, _removed) =
                block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
                    &update,
                    &gitignores,
                    &force_included,
                    &definitions,
                    false, /* lazy_load */
                ));

            let incremental_ignored = mutations
                .iter()
                .find_map(|mutation| match mutation {
                    FileTreeMutation::AddDirectorySubtree { dir_path, subtree }
                        if dir_path == &skills_local =>
                    {
                        Some(subtree.ignored())
                    }
                    FileTreeMutation::AddDirectorySubtree { .. }
                    | FileTreeMutation::Remove(_)
                    | FileTreeMutation::AddFile { .. }
                    | FileTreeMutation::AddUnloadedDirectory { .. } => None,
                })
                .expect(
                    "incremental update should materialize the force-included subtree \
                     as an AddDirectorySubtree mutation",
                );

            assert_eq!(
                incremental_ignored, expected_ignored,
                "force-included `.agents/skills` under ignored `.agents`: incremental watcher \
                 update recorded ignored={incremental_ignored}, but the initial index records \
                 ignored={expected_ignored}"
            );
        },
    );
}

/// A filesystem event deep under an UNLOADED (collapsed) gitignored directory is
/// dropped at apply time, so nothing below the unloaded placeholder is
/// materialized — matching the initial index's single-placeholder representation.
#[test]
fn incremental_deep_event_under_unloaded_ignored_dir_is_collapsed() {
    VirtualFS::test(
        "incremental_deep_event_under_unloaded_ignored_dir",
        |dirs, mut vfs| {
            vfs.mkdir("repo/target/debug/.fingerprint").with_files(vec![
                Stub::FileWithContent("repo/.gitignore", "target/\n"),
                Stub::FileWithContent("repo/target/debug/.fingerprint/x.json", "{}"),
            ]);

            let repo_local = dirs.tests().join("repo");
            let target_local = repo_local.join("target");
            let deep_dir_local = target_local.join("debug").join(".fingerprint");
            let deep_file_local = deep_dir_local.join("x.json");

            let repo_std = StandardizedPath::try_from_local(&repo_local).unwrap();
            let target_std = StandardizedPath::try_from_local(&target_local).unwrap();
            let debug_std = StandardizedPath::try_from_local(&target_local.join("debug")).unwrap();

            // Post-initial-index state: `target` is a single UNLOADED ignored placeholder.
            let root_entry = Entry::Directory(DirectoryEntry {
                path: repo_std,
                ignored: false,
                loaded: true,
                children: vec![Entry::Directory(DirectoryEntry {
                    path: target_std.clone(),
                    ignored: true,
                    loaded: false,
                    children: vec![],
                })],
            });
            let mut tree = FileTreeEntry::from(root_entry);

            let gitignores = crate::gitignores_for_directory(&repo_local);
            let definitions = StandingQueryDefinitions::default();

            let update = RepoUpdate {
                added: vec![deep_dir_local, deep_file_local],
                ..Default::default()
            };
            let (mutations, _standing_results, _removed) =
                block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
                    &update,
                    &gitignores,
                    &[], /* force_included_paths */
                    &definitions,
                    false, /* lazy_load */
                ));
            LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, false);

            // `target` stays a single unloaded placeholder; nothing below it is materialized.
            match tree
                .get(&target_std)
                .expect("`target` should remain in the tree")
            {
                FileTreeEntryState::Directory(directory) => {
                    assert!(
                        !directory.loaded,
                        "`target` should remain an unloaded placeholder"
                    );
                    assert!(directory.ignored, "`target` should stay ignored");
                }
                FileTreeEntryState::File(_) => panic!("`target` should be a directory"),
            }
            assert!(
                tree.get(&debug_std).is_none(),
                "nothing below the unloaded `target` placeholder should be materialized"
            );
        },
    );
}

/// A filesystem event under a gitignored directory the user has already
/// expanded (so it is `loaded`) must keep that directory loaded.
#[test]
fn incremental_event_under_expanded_ignored_dir_keeps_it_loaded() {
    VirtualFS::test(
        "incremental_event_under_expanded_ignored_dir",
        |dirs, mut vfs| {
            vfs.mkdir("repo/target/debug").with_files(vec![
                Stub::FileWithContent("repo/.gitignore", "target/\n"),
                Stub::FileWithContent("repo/target/debug/new.rs", "x"),
            ]);

            let repo_local = dirs.tests().join("repo");
            let target_local = repo_local.join("target");
            let new_file_local = target_local.join("debug").join("new.rs");

            let repo_std = StandardizedPath::try_from_local(&repo_local).unwrap();
            let target_std = StandardizedPath::try_from_local(&target_local).unwrap();
            let debug_std = StandardizedPath::try_from_local(&target_local.join("debug")).unwrap();
            let new_file_std = StandardizedPath::try_from_local(&new_file_local).unwrap();

            // The user has expanded the gitignored `target/`, so it is loaded.
            let root_entry = Entry::Directory(DirectoryEntry {
                path: repo_std,
                ignored: false,
                loaded: true,
                children: vec![Entry::Directory(DirectoryEntry {
                    path: target_std.clone(),
                    ignored: true,
                    loaded: true,
                    children: vec![Entry::Directory(DirectoryEntry {
                        path: debug_std,
                        ignored: true,
                        loaded: true,
                        children: vec![],
                    })],
                })],
            });
            let mut tree = FileTreeEntry::from(root_entry);

            let gitignores = crate::gitignores_for_directory(&repo_local);
            let definitions = StandingQueryDefinitions::default();

            let update = RepoUpdate {
                added: vec![new_file_local],
                ..Default::default()
            };
            let (mutations, _standing_results, _removed) =
                block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
                    &update,
                    &gitignores,
                    &[], /* force_included_paths */
                    &definitions,
                    false, /* lazy_load */
                ));
            LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, false);

            match tree
                .get(&target_std)
                .expect("expanded `target` should remain in the tree")
            {
                FileTreeEntryState::Directory(directory) => {
                    assert!(
                        directory.loaded,
                        "an event under an expanded ignored dir must not collapse it to an \
                         unloaded placeholder"
                    );
                    assert!(directory.ignored, "`target` should still be ignored");
                }
                FileTreeEntryState::File(_) => panic!("`target` should be a directory"),
            }
            assert!(
                tree.get(&new_file_std).is_some(),
                "the new file under the expanded ignored dir should be delivered"
            );
        },
    );
}

/// Expanding a gitignored directory inside a git repo registers an on-demand
/// non-recursive watch for it on Linux (where the recursive root watch prunes
/// gitignored dirs), while other platforms rely on the recursive root watch.
#[cfg(feature = "local_fs")]
#[test]
fn load_directory_watches_expanded_gitignored_dir_for_git_repo() {
    VirtualFS::test("git_repo_gitignored_expand", |dirs, mut vfs| {
        vfs.mkdir("repo/node_modules/pkg")
            .with_files(vec![Stub::FileWithContent(
                "repo/.gitignore",
                "node_modules/\n",
            )]);
        let repo_path =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("repo")).unwrap();
        let node_modules =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("repo/node_modules"))
                .unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            let (gitignore, _) = Gitignore::new(dirs.tests().join("repo/.gitignore"));
            let root = Entry::Directory(DirectoryEntry {
                path: repo_path.clone(),
                children: Vec::new(),
                ignored: false,
                loaded: true,
            });
            let state = FileTreeState::new(root, vec![gitignore], None);

            model_handle.update(&mut app, |model, ctx| {
                model
                    .add_repository_internal(
                        repo_path.clone(),
                        state,
                        RootWatchMode::Recursive,
                        ctx,
                    )
                    .expect("repo should index");
                model
                    .load_directory(&repo_path, &node_modules, ctx)
                    .expect("should load gitignored dir");
            });

            model_handle.read(&app, |model, _ctx| {
                let repo_watch = model
                    .repo_watches
                    .get(&repo_path)
                    .expect("watch should be recorded");
                assert_eq!(repo_watch.root_mode, RootWatchMode::Recursive);
                if cfg!(target_os = "linux") {
                    // Linux prunes node_modules from the recursive root watch, so
                    // expanding it registers an on-demand non-recursive watch.
                    assert!(repo_watch.extra_dirs.contains(&node_modules));
                } else {
                    // Other backends still deliver gitignored events through the
                    // recursive root watch, so no extra watch is registered.
                    assert!(repo_watch.extra_dirs.is_empty());
                }
            });
        });
    });
}

/// Removing a git repo clears its tracked watch entry (root plus any on-demand
/// per-directory watches for expanded gitignored dirs).
#[cfg(feature = "local_fs")]
#[test]
fn remove_repository_clears_extra_dir_watches() {
    VirtualFS::test("git_repo_remove_clears_extra", |dirs, mut vfs| {
        vfs.mkdir("repo/build/out")
            .with_files(vec![Stub::FileWithContent("repo/.gitignore", "build/\n")]);
        let repo_path =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("repo")).unwrap();
        let build =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("repo/build")).unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            let (gitignore, _) = Gitignore::new(dirs.tests().join("repo/.gitignore"));
            let root = Entry::Directory(DirectoryEntry {
                path: repo_path.clone(),
                children: Vec::new(),
                ignored: false,
                loaded: true,
            });
            let state = FileTreeState::new(root, vec![gitignore], None);

            model_handle.update(&mut app, |model, ctx| {
                model
                    .add_repository_internal(
                        repo_path.clone(),
                        state,
                        RootWatchMode::Recursive,
                        ctx,
                    )
                    .expect("repo should index");
                model
                    .load_directory(&repo_path, &build, ctx)
                    .expect("should load gitignored dir");
                model
                    .remove_repository(&repo_path, ctx)
                    .expect("repo should be removed");
            });

            model_handle.read(&app, |model, _ctx| {
                assert!(!model.repo_watches.contains_key(&repo_path));
                assert!(model.repository_state(&repo_path).is_none());
            });
        });
    });
}

/// Tearing down a lazy root clears all of its tracked per-directory watches and
/// removes the repository state.
#[cfg(feature = "local_fs")]
#[test]
fn remove_lazy_loaded_path_clears_tracked_watches() {
    VirtualFS::test("lazy_remove_clears_tracking", |dirs, mut vfs| {
        vfs.mkdir("workspace/sub");
        let root =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace")).unwrap();
        let sub = StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace/sub"))
            .unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_lazy_loaded_path(&root, ctx)
                    .expect("should index lazy path");
                model
                    .load_directory(&root, &sub, ctx)
                    .expect("should load subdir");
                model.remove_lazy_loaded_path(&root, ctx);
            });

            model_handle.read(&app, |model, _ctx| {
                assert!(!model.repo_watches.contains_key(&root));
                assert!(!model.is_lazy_loaded_path(&root));
                assert!(model.repository_state(&root).is_none());
            });
        });
    });
}

/// Deleting an expanded subdirectory of a lazy non-recursive root drops its
/// per-directory watch (and any tracked descendants), so the entry no longer
/// lingers in `extra_dirs`. Otherwise a directory recreated at the same path
/// would be skipped by `watch_subdir` and never re-watched.
#[cfg(all(unix, feature = "local_fs"))]
#[test]
fn deleted_subdir_drops_its_tracked_watch() {
    VirtualFS::test("lazy_delete_subdir_drops_watch", |dirs, mut vfs| {
        vfs.mkdir("workspace/sub/inner");
        let root =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace")).unwrap();
        let sub = StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace/sub"))
            .unwrap();
        let inner =
            StandardizedPath::from_local_canonicalized(&dirs.tests().join("workspace/sub/inner"))
                .unwrap();
        let sub_local = sub.to_local_path().unwrap();

        App::test((), |mut app| async move {
            let model_handle = app.add_model(|_| LocalRepoMetadataModel::new_for_test());
            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_lazy_loaded_path(&root, ctx)
                    .expect("should index lazy path");
                model
                    .load_directory(&root, &sub, ctx)
                    .expect("should load subdir");
                model
                    .load_directory(&root, &inner, ctx)
                    .expect("should load nested subdir");
            });

            // Only a non-recursive (Linux) root tracks per-directory watches.
            if !cfg!(target_os = "linux") {
                return;
            }

            model_handle.read(&app, |model, _ctx| {
                let repo_watch = model.repo_watches.get(&root).expect("watch recorded");
                assert!(repo_watch.extra_dirs.contains(&sub));
                assert!(repo_watch.extra_dirs.contains(&inner));
            });

            // Wait for the spawned watcher-event handling to finish by
            // listening for the tree update it emits.
            let (tx, rx) = oneshot::channel();
            let sender = Rc::new(RefCell::new(Some(tx)));
            let root_for_event = root.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&model_handle, move |_, event, _ctx| {
                    if let RepositoryMetadataEvent::FileTreeEntryUpdated { path, .. } = event {
                        if path == &root_for_event {
                            if let Some(tx) = sender.borrow_mut().take() {
                                let _ = tx.send(());
                            }
                        }
                    }
                });
            });

            model_handle.update(&mut app, |model, ctx| {
                model.handle_watcher_event(
                    &BulkFilesystemWatcherEvent {
                        deleted: std::collections::HashSet::from([sub_local]),
                        ..Default::default()
                    },
                    ctx,
                );
            });
            rx.with_timeout(Duration::from_secs(5))
                .await
                .expect("timed out waiting for tree update")
                .expect("tree update sender dropped");

            model_handle.read(&app, |model, _ctx| {
                let repo_watch = model.repo_watches.get(&root).expect("watch recorded");
                // The deleted subdir and its tracked descendant are dropped.
                assert!(!repo_watch.extra_dirs.contains(&sub));
                assert!(!repo_watch.extra_dirs.contains(&inner));
            });
        });
    });
}

/// On a lazy root, a newly created directory is inserted as an unloaded
/// placeholder rather than having its whole subtree materialized eagerly; the
/// contents are loaded on demand when the user expands it. On an eager root the
/// same directory is fully materialized.
#[test]
fn lazy_root_created_directory_inserted_as_placeholder() {
    VirtualFS::test("lazy_created_dir_placeholder", |dirs, mut vfs| {
        let repo_path = dirs.tests();
        vfs.mkdir("newdir/sub")
            .with_files(vec![Stub::FileWithContent("newdir/sub/file.txt", "x")]);

        let new_dir = repo_path.join("newdir");
        let nested = repo_path.join("newdir/sub");
        let new_dir_std = StandardizedPath::try_from_local(&new_dir).unwrap();
        let nested_std = StandardizedPath::try_from_local(&nested).unwrap();

        let make_root = || {
            FileTreeEntry::from(Entry::Directory(DirectoryEntry {
                path: StandardizedPath::try_from_local(repo_path).unwrap(),
                children: Vec::new(),
                ignored: false,
                loaded: true,
            }))
        };
        let update = RepoUpdate {
            added: vec![new_dir.clone()],
            ..Default::default()
        };
        let definitions = StandingQueryDefinitions::default();

        // Lazy root: the new directory is an unloaded placeholder and its
        // subtree is not materialized.
        let mut lazy_root = make_root();
        let (lazy_mutations, _, _) = block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
            &update,
            &[],
            &[],
            &definitions,
            true,
        ));
        LocalRepoMetadataModel::apply_file_tree_mutations(
            &mut lazy_root,
            lazy_mutations,
            true,
            false,
        );

        let placeholder = lazy_root
            .get(&new_dir_std)
            .expect("new directory should be present");
        assert!(
            !placeholder.loaded(),
            "lazy root should add the directory as an unloaded placeholder"
        );
        assert!(
            lazy_root.get(&nested_std).is_none(),
            "lazy root should not materialize the subtree"
        );

        // Eager root: the same directory is fully materialized.
        let mut eager_root = make_root();
        let (eager_mutations, _, _) =
            block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
                &update,
                &[],
                &[],
                &definitions,
                false,
            ));
        LocalRepoMetadataModel::apply_file_tree_mutations(
            &mut eager_root,
            eager_mutations,
            false,
            false,
        );
        assert!(
            eager_root.get(&new_dir_std).is_some_and(|e| e.loaded()),
            "eager root should materialize the directory"
        );
        assert!(
            eager_root.get(&nested_std).is_some(),
            "eager root should materialize the subtree"
        );
    });
}

/// BUG BASH: a file that is ignored ONLY by a nested (per-directory)
/// `.gitignore` is tagged `is_ignored = false` when added incrementally via the
/// watcher, because `compute_file_tree_mutations` classifies with only the
/// repo-root + global gitignores (`state.gitignores`). The initial index reads
/// nested `.gitignore` files, so `sub/existing.log` is dimmed but a live
/// sub/new.log` is not. This test asserts the CORRECT (initial-index) behavior.
/// It is currently ignored because it documents a known bug (bug-bash finding):
/// the incremental watcher path does not consult nested `.gitignore` files, so a
/// live-created file matched only by a nested `.gitignore` renders un-dimmed.
/// Remove `#[ignore]` once `compute_file_tree_mutations` consults nested
/// `.gitignore` files (or inherits the parent dir's ignored state).
#[test]
#[ignore = "known bug: incremental adds don't consult nested .gitignore (bug-bash finding)"]
fn bugbash_incremental_add_respects_nested_gitignore() {
    VirtualFS::test("bugbash_nested_gitignore", |dirs, mut vfs| {
        vfs.mkdir("repo/sub").with_files(vec![
            Stub::FileWithContent("repo/.gitignore", "build/\n"),
            Stub::FileWithContent("repo/sub/.gitignore", "*.log\n"),
            Stub::FileWithContent("repo/sub/new.log", "y"),
        ]);

        let repo_local = dirs.tests().join("repo");
        let new_log_local = repo_local.join("sub").join("new.log");

        let gitignores = crate::gitignores_for_directory(&repo_local);
        let definitions = StandingQueryDefinitions::default();

        let update = RepoUpdate {
            added: vec![new_log_local.clone()],
            ..Default::default()
        };
        let (mutations, _standing_results, _removed) =
            block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
                &update,
                &gitignores,
                &[], /* force_included_paths */
                &definitions,
                false, /* lazy_load */
            ));

        let incremental_ignored = mutations
            .iter()
            .find_map(|mutation| match mutation {
                FileTreeMutation::AddFile {
                    path, is_ignored, ..
                } if path == &new_log_local => Some(*is_ignored),
                _ => None,
            })
            .expect("incremental update should contain an AddFile for sub/new.log");

        assert!(
            incremental_ignored,
            "sub/new.log is ignored by the nested sub/.gitignore (*.log), so the incremental \
             watcher add should tag it ignored=true to match the initial index; got ignored=false"
        );
    });
}

/// BUG BASH companion: a file created directly inside a directory ignored by the
/// repo-ROOT `.gitignore` (`node_modules/`). This isolates whether the
/// incremental classifier honors ancestor matches at all. Asserts the correct
/// behavior (ignored=true).
#[test]
fn bugbash_incremental_add_direct_child_of_root_ignored_dir() {
    VirtualFS::test("bugbash_root_ignored_dir_child", |dirs, mut vfs| {
        vfs.mkdir("repo/node_modules").with_files(vec![
            Stub::FileWithContent("repo/.gitignore", "node_modules/\n"),
            Stub::FileWithContent("repo/node_modules/zzz.js", "x"),
        ]);

        let repo_local = dirs.tests().join("repo");
        let zzz_local = repo_local.join("node_modules").join("zzz.js");

        let gitignores = crate::gitignores_for_directory(&repo_local);
        let definitions = StandingQueryDefinitions::default();

        let update = RepoUpdate {
            added: vec![zzz_local.clone()],
            ..Default::default()
        };
        let (mutations, _standing_results, _removed) =
            block_on(LocalRepoMetadataModel::compute_file_tree_mutations(
                &update,
                &gitignores,
                &[], /* force_included_paths */
                &definitions,
                false, /* lazy_load */
            ));

        let incremental_ignored = mutations
            .iter()
            .find_map(|mutation| match mutation {
                FileTreeMutation::AddFile {
                    path, is_ignored, ..
                } if path == &zzz_local => Some(*is_ignored),
                _ => None,
            })
            .expect("incremental update should contain an AddFile for node_modules/zzz.js");

        assert!(
            incremental_ignored,
            "node_modules/zzz.js sits under a root-gitignored directory, so the incremental \
             watcher add should tag it ignored=true; got ignored=false"
        );
    });
}
