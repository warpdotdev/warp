use std::sync::Arc;

use repo_metadata::entry::{DirectoryEntry, Entry, FileMetadata};
use repo_metadata::file_tree_store::FileTreeState;
use repo_metadata::local_model::IndexedRepoState;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use repo_metadata::RepoMetadataModel;
use virtual_fs::{Stub, VirtualFS};
use warp_core::ui::appearance::Appearance;
use warpui::{platform::WindowStyle, App, ModelHandle};

use crate::auth::AuthStateProvider;
use crate::server::server_api::{team::MockTeamClient, workspace::MockWorkspaceClient};
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;

use super::FileTreeView;

fn std_path(path: &std::path::Path) -> warp_util::standardized_path::StandardizedPath {
    warp_util::standardized_path::StandardizedPath::try_from_local(path).unwrap()
}

fn initialize_app(
    app: &mut App,
) -> (
    ModelHandle<DetectedRepositories>,
    ModelHandle<RepoMetadataModel>,
) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ToastStack);
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());

    let team_client = Arc::new(MockTeamClient::new());
    let workspace_client = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(team_client.clone(), workspace_client.clone(), vec![], ctx)
    });

    let detected_repositories = app.add_singleton_model(|_| DetectedRepositories::default());
    let repository_metadata_model = app.add_singleton_model(RepoMetadataModel::new);

    (detected_repositories, repository_metadata_model)
}

fn build_repo_state(repo_root: &std::path::Path) -> FileTreeState {
    let source_file = Entry::File(FileMetadata::new(
        repo_root.join("packages/app/src/main.rs"),
        false,
    ));
    let src_dir = Entry::Directory(DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(
            &repo_root.join("packages/app/src"),
        )
        .unwrap(),
        children: vec![source_file],
        ignored: false,
        loaded: true,
    });
    let app_dir = Entry::Directory(DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(
            &repo_root.join("packages/app"),
        )
        .unwrap(),
        children: vec![src_dir],
        ignored: false,
        loaded: true,
    });
    let packages_dir = Entry::Directory(DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(
            &repo_root.join("packages"),
        )
        .unwrap(),
        children: vec![app_dir],
        ignored: false,
        loaded: true,
    });
    let root = Entry::Directory(DirectoryEntry {
        path: std_path(repo_root),
        children: vec![packages_dir],
        ignored: false,
        loaded: true,
    });
    FileTreeState::new(root, vec![], None)
}

fn build_repo_state_with_unloaded_directory(repo_root: &std::path::Path) -> FileTreeState {
    let unloaded_src_dir = Entry::Directory(DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(
            &repo_root.join("src"),
        )
        .unwrap(),
        children: vec![],
        ignored: false,
        loaded: false,
    });
    let root = Entry::Directory(DirectoryEntry {
        path: std_path(repo_root),
        children: vec![unloaded_src_dir],
        ignored: false,
        loaded: true,
    });
    FileTreeState::new(root, vec![], None)
}

#[test]
fn repo_transition_unregisters_lazy_loaded_path() {
    VirtualFS::test("file_tree_repo_transition", |dirs, mut vfs| {
        vfs.mkdir("repo/.git/objects")
            .mkdir("repo/packages/app/src")
            .with_files(vec![
                Stub::FileWithContent("repo/.git/HEAD", "ref: refs/heads/main"),
                Stub::FileWithContent("repo/.git/config", "[core]\n\trepositoryformatversion = 0"),
                Stub::FileWithContent("repo/packages/app/src/main.rs", "fn main() {}\n"),
            ]);

        let repo_root = dirs.tests().join("repo");
        let displayed_root = repo_root.join("packages/app");
        let canonical_repo_root =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo_root)
                .unwrap();

        App::test((), |mut app| async move {
            let (detected_repositories, repository_metadata_model) = initialize_app(&mut app);

            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            detected_repositories.update(&mut app, |repositories, _ctx| {
                repositories.insert_test_repo_root(canonical_repo_root.clone());
            });

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![displayed_root.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view.registered_lazy_loaded_paths.contains(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(
                        &displayed_root
                    )
                    .unwrap()
                ));
                let displayed_std =
                    warp_util::standardized_path::StandardizedPath::try_from_local(&displayed_root)
                        .unwrap();
                assert_eq!(
                    view.root_directories
                        .get(&displayed_std)
                        .map(|root_dir| root_dir.entry.root_directory().to_local_path_lossy()),
                    Some(displayed_root.clone())
                );
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(model.is_lazy_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(
                        &displayed_root
                    )
                    .unwrap(),
                    ctx
                ));
            });

            repository_metadata_model.update(&mut app, |model, ctx| {
                model.insert_test_state(canonical_repo_root, build_repo_state(&repo_root), ctx);
            });

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_root_directories(vec![displayed_root.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                let displayed_std =
                    warp_util::standardized_path::StandardizedPath::try_from_local(&displayed_root)
                        .unwrap();
                let repo_std =
                    warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap();
                assert!(!view.registered_lazy_loaded_paths.contains(&displayed_std));
                assert_eq!(view.root_for_path(&displayed_std), Some(repo_std.clone()));
                assert_eq!(
                    view.root_directories
                        .get(&displayed_std)
                        .map(|root_dir| (**root_dir.entry.root_directory()).clone()),
                    Some(repo_std)
                );
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(!model.is_lazy_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(
                        &displayed_root
                    )
                    .unwrap(),
                    ctx
                ));
            });
        });
    });
}

#[test]
fn repo_backed_unloaded_directory_loads_through_model() {
    VirtualFS::test("file_tree_repo_backed_load", |dirs, mut vfs| {
        vfs.mkdir("repo/.git/objects")
            .mkdir("repo/src/nested")
            .with_files(vec![
                Stub::FileWithContent("repo/.git/HEAD", "ref: refs/heads/main"),
                Stub::FileWithContent(
                    "repo/.git/config",
                    "[core]
\trepositoryformatversion = 0",
                ),
                Stub::FileWithContent(
                    "repo/src/nested/main.rs",
                    "fn main() {}
",
                ),
            ]);

        let repo_root = dirs.tests().join("repo");
        let src_dir = repo_root.join("src");
        let nested_dir = repo_root.join("src/nested");
        let source_file = repo_root.join("src/nested/main.rs");
        let canonical_repo_root =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo_root)
                .unwrap();

        App::test((), |mut app| async move {
            let (detected_repositories, repository_metadata_model) = initialize_app(&mut app);

            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            detected_repositories.update(&mut app, |repositories, _ctx| {
                repositories.insert_test_repo_root(canonical_repo_root.clone());
            });
            repository_metadata_model.update(&mut app, |model, ctx| {
                model.insert_test_state(
                    canonical_repo_root,
                    build_repo_state_with_unloaded_directory(&repo_root),
                    ctx,
                );
            });

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![repo_root.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(!view
                    .root_directories
                    .get(
                        &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                            .unwrap()
                    )
                    .is_some_and(|root_dir| root_dir.entry.contains(
                        &warp_util::standardized_path::StandardizedPath::try_from_local(
                            &source_file
                        )
                        .unwrap()
                    )));
            });

            file_tree_view.update(&mut app, |view, ctx| {
                view.ensure_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap(),
                    &warp_util::standardized_path::StandardizedPath::try_from_local(&src_dir)
                        .unwrap(),
                    ctx,
                );
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view
                    .root_directories
                    .get(
                        &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                            .unwrap()
                    )
                    .is_some_and(|root_dir| root_dir.entry.contains(
                        &warp_util::standardized_path::StandardizedPath::try_from_local(
                            &nested_dir
                        )
                        .unwrap()
                    )));
            });

            file_tree_view.update(&mut app, |view, ctx| {
                view.ensure_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap(),
                    &warp_util::standardized_path::StandardizedPath::try_from_local(&nested_dir)
                        .unwrap(),
                    ctx,
                );
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view
                    .root_directories
                    .get(
                        &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                            .unwrap()
                    )
                    .is_some_and(|root_dir| root_dir.entry.contains(
                        &warp_util::standardized_path::StandardizedPath::try_from_local(
                            &source_file
                        )
                        .unwrap()
                    )));
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(!model.is_lazy_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap(),
                    ctx
                ));
                let id = repo_metadata::RepositoryIdentifier::local(
                    warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap(),
                );
                assert!(model.get_repository(&id, ctx).is_some_and(|state| {
                    state.entry.contains(
                        &warp_util::standardized_path::StandardizedPath::try_from_local(
                            &source_file,
                        )
                        .unwrap(),
                    )
                }));
            });
        });
    });
}

#[test]
fn pending_repository_root_does_not_register_lazy_loaded_path() {
    VirtualFS::test("file_tree_pending_repo_root", |dirs, mut vfs| {
        vfs.mkdir("repo/.git/objects").with_files(vec![
            Stub::FileWithContent("repo/.git/HEAD", "ref: refs/heads/main"),
            Stub::FileWithContent("repo/.git/config", "[core]\n\trepositoryformatversion = 0"),
        ]);

        let repo_root = dirs.tests().join("repo");
        let canonical_repo_root =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo_root)
                .unwrap();

        App::test((), |mut app| async move {
            let (detected_repositories, repository_metadata_model) = initialize_app(&mut app);
            let directory_watcher = app.add_singleton_model(DirectoryWatcher::new);

            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);
            let repository_handle = directory_watcher.update(&mut app, |watcher, ctx| {
                watcher
                    .add_directory(canonical_repo_root.clone(), ctx)
                    .unwrap()
            });

            detected_repositories.update(&mut app, |repositories, _ctx| {
                repositories.insert_test_repo_root(canonical_repo_root.clone());
            });
            repository_metadata_model.update(&mut app, |model, ctx| {
                model.index_directory(repository_handle, ctx).unwrap();
            });
            repository_metadata_model.read(&app, |model, ctx| {
                let id = repo_metadata::RepositoryIdentifier::local(
                    warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap(),
                );
                assert!(matches!(
                    model.repository_state(&id, ctx),
                    Some(IndexedRepoState::Pending(_))
                ));
            });

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![repo_root.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(!view.registered_lazy_loaded_paths.contains(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap()
                ));
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(!model.is_lazy_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap(),
                    ctx
                ));
                let id = repo_metadata::RepositoryIdentifier::local(
                    warp_util::standardized_path::StandardizedPath::try_from_local(&repo_root)
                        .unwrap(),
                );
                assert!(matches!(
                    model.repository_state(&id, ctx),
                    Some(IndexedRepoState::Pending(_))
                ));
            });
        });
    });
}

#[test]
fn failed_lazy_loaded_path_registration_is_retried() {
    VirtualFS::test("file_tree_lazy_loaded_path_retry", |dirs, mut vfs| {
        let displayed_root = dirs.tests().join("late_dir");

        App::test((), |mut app| async move {
            let (_detected_repositories, repository_metadata_model) = initialize_app(&mut app);

            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![displayed_root.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(!view.registered_lazy_loaded_paths.contains(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(
                        &displayed_root
                    )
                    .unwrap()
                ));
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(!model.is_lazy_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(
                        &displayed_root
                    )
                    .unwrap(),
                    ctx
                ));
            });

            vfs.mkdir("late_dir")
                .with_files(vec![Stub::FileWithContent("late_dir/file.txt", "content")]);

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_root_directories(vec![displayed_root.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view.registered_lazy_loaded_paths.contains(&std_path(&displayed_root)));
                assert!(matches!(
                    view.root_directories.get(&std_path(&displayed_root)).map(|root_dir| &root_dir.entry),
                    Some(entry)
                        if entry.contains(&std_path(&displayed_root.join("file.txt")))
                ));
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(model.is_lazy_loaded_path(
                    &warp_util::standardized_path::StandardizedPath::try_from_local(
                        &displayed_root
                    )
                    .unwrap(),
                    ctx
                ));
            });
        });
    });
}

// ── Ancestor grouping (APP-4106) ────────────────────────────────────

#[test]
fn sibling_roots_are_preserved() {
    VirtualFS::test("file_tree_sibling_roots", |dirs, mut vfs| {
        vfs.mkdir("tree/a").mkdir("tree/b").with_files(vec![
            Stub::FileWithContent("tree/a/x.txt", "x"),
            Stub::FileWithContent("tree/b/y.txt", "y"),
        ]);
        let a = dirs.tests().join("tree/a");
        let b = dirs.tests().join("tree/b");

        App::test((), |mut app| async move {
            let _ = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![a.clone(), b.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert_eq!(view.displayed_directories, vec![std_path(&a), std_path(&b)]);
            });
        });
    });
}

#[test]
fn auto_expand_overrides_selection_when_most_recent_root_changes() {
    VirtualFS::test(
        "file_tree_auto_expand_overrides_on_new_root",
        |dirs, mut vfs| {
            vfs.mkdir("code/foo").mkdir("other").with_files(vec![
                Stub::FileWithContent("code/foo/file.txt", "x"),
                Stub::FileWithContent("other/file.txt", "y"),
            ]);
            let code = dirs.tests().join("code");
            let other = dirs.tests().join("other");

            App::test((), |mut app| async move {
                let _ = initialize_app(&mut app);
                let (_, file_tree_view) =
                    app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

                // Start with `code` as the only root and select its header.
                file_tree_view.update(&mut app, |view, ctx| {
                    view.set_is_active(true, ctx);
                    view.set_root_directories(vec![code.clone()], ctx);
                    view.auto_expand_to_most_recent_directory(ctx);
                });
                file_tree_view.read(&app, |view, _ctx| {
                    let selected = view.selected_item.as_ref().unwrap();
                    assert_eq!(selected.root, std_path(&code));
                });

                // Now cd to a brand-new root. `other` becomes most-recent.
                // Selection must move to `other`, not stay on `code`.
                file_tree_view.update(&mut app, |view, ctx| {
                    view.set_root_directories(vec![other.clone(), code.clone()], ctx);
                    view.auto_expand_to_most_recent_directory(ctx);
                });

                file_tree_view.read(&app, |view, _ctx| {
                    let selected = view.selected_item.as_ref().expect("selection set");
                    assert_eq!(selected.root, std_path(&other));
                });
            });
        },
    );
}

#[test]
fn auto_expand_preserves_existing_selection() {
    VirtualFS::test(
        "file_tree_auto_expand_preserves_selection",
        |dirs, mut vfs| {
            vfs.mkdir("tree/sub")
                .with_files(vec![Stub::FileWithContent("tree/sub/file.txt", "content")]);
            let tree = dirs.tests().join("tree");
            let sub = tree.join("sub");

            App::test((), |mut app| async move {
                let _ = initialize_app(&mut app);
                let (_, file_tree_view) =
                    app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

                file_tree_view.update(&mut app, |view, ctx| {
                    view.set_is_active(true, ctx);
                    view.set_root_directories(vec![tree.clone()], ctx);
                });

                // Simulate a prior explicit selection (e.g. user focused a
                // file in the code editor and `scroll_to_file` selected it).
                file_tree_view.update(&mut app, |view, ctx| {
                    view.toggle_folder_expansion(&std_path(&tree), &std_path(&sub), ctx);
                    let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                    let (index, _) = root_dir
                        .items
                        .iter()
                        .enumerate()
                        .find(|(_, item)| item.path() == &std_path(&sub))
                        .expect("sub directory is flattened");
                    let id = super::FileTreeIdentifier {
                        root: std_path(&tree),
                        index,
                    };
                    view.select_id(&id, ctx);
                });

                // Auto-expand must not override that selection with the root header.
                file_tree_view.update(&mut app, |view, ctx| {
                    view.auto_expand_to_most_recent_directory(ctx);
                });

                file_tree_view.read(&app, |view, _ctx| {
                    let selected = view.selected_item.clone().expect("selection set");
                    let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                    let selected_path = root_dir.items.get(selected.index).unwrap().path();
                    assert_eq!(selected_path, &std_path(&sub));
                });
            });
        },
    );
}

#[test]
fn click_on_file_under_absorbed_descendant_keeps_file_selected() {
    // Simulates: user clicks a file in the tree. The code view opens it,
    // which causes `DirectoriesChanged` to fire with the file's
    // parent/repo added. The resulting `set_root_directories` must NOT
    // override the user's file selection with the cwd-follow parent.
    VirtualFS::test(
        "file_tree_click_file_preserves_selection",
        |dirs, mut vfs| {
            vfs.mkdir("code/warp-server")
                .with_files(vec![Stub::FileWithContent(
                    "code/warp-server/main.rs",
                    "fn main() {}\n",
                )]);
            let code = dirs.tests().join("code");
            let warp_server = code.join("warp-server");
            let main_rs = warp_server.join("main.rs");

            App::test((), |mut app| async move {
                let _ = initialize_app(&mut app);
                let (_, file_tree_view) =
                    app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

                // Seed with `code` as the only root and expand warp-server so
                // main.rs is materialized in the flattened items.
                file_tree_view.update(&mut app, |view, ctx| {
                    view.set_is_active(true, ctx);
                    view.set_root_directories(vec![code.clone()], ctx);
                    view.toggle_folder_expansion(&std_path(&code), &std_path(&warp_server), ctx);
                });

                // Simulate a click on main.rs (select_id is what the click
                // action and the active-file scroll both go through).
                file_tree_view.update(&mut app, |view, ctx| {
                    let root_dir = view.root_directories.get(&std_path(&code)).unwrap();
                    let (index, _) = root_dir
                        .items
                        .iter()
                        .enumerate()
                        .find(|(_, item)| item.path() == &std_path(&main_rs))
                        .expect("main.rs materialized");
                    let id = super::FileTreeIdentifier {
                        root: std_path(&code),
                        index,
                    };
                    view.select_id(&id, ctx);
                });

                // Now `DirectoriesChanged` fires as a side effect of the file
                // opening in a code view — the working-directories-model adds
                // the file's repo/parent (warp-server) to the active set.
                file_tree_view.update(&mut app, |view, ctx| {
                    view.set_root_directories(vec![warp_server.clone(), code.clone()], ctx);
                });

                file_tree_view.read(&app, |view, _ctx| {
                    // Selection is still on main.rs, not on warp-server.
                    let selected = view.selected_item.clone().expect("selection");
                    let root_dir = view.root_directories.get(&std_path(&code)).unwrap();
                    let path = root_dir.items.get(selected.index).unwrap().path();
                    assert_eq!(path, &std_path(&main_rs));
                    // And we didn't set a pending focus target that could
                    // later steal focus back to the parent directory.
                    assert!(view.pending_focus_target.is_none());
                });
            });
        },
    );
}

#[test]
fn pending_focus_target_does_not_re_scroll_after_first_apply() {
    // After the initial focus-follow scrolls to the cwd, subsequent
    // rebuilds (e.g. from repo-metadata updates) must keep the
    // selection but NOT re-scroll, so user scrolling is respected.
    VirtualFS::test("file_tree_pending_respects_user_scroll", |dirs, mut vfs| {
        vfs.mkdir("tree/warp-server")
            .with_files(vec![Stub::FileWithContent(
                "tree/warp-server/main.rs",
                "fn main() {}\n",
            )]);
        let tree = dirs.tests().join("tree");
        let warp_server = tree.join("warp-server");

        App::test((), |mut app| async move {
            let _ = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![warp_server.clone(), tree.clone()], ctx);
            });

            // Initial apply should have scrolled once.
            file_tree_view.read(&app, |view, _ctx| {
                let pending = view.pending_focus_target.as_ref().expect("pending");
                assert!(pending.scrolled);
            });

            // Simulate a later rebuild (e.g. metadata update). Selection
            // should still land on warp-server, but `scrolled` must stay
            // true (no re-scroll).
            file_tree_view.update(&mut app, |view, _ctx| {
                view.rebuild_flattened_items();
                view.apply_pending_focus_target();
            });

            file_tree_view.read(&app, |view, _ctx| {
                let selected = view.selected_item.clone().expect("selection");
                let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                let path = root_dir.items.get(selected.index).unwrap().path();
                assert_eq!(path, &std_path(&warp_server));
                let pending = view.pending_focus_target.as_ref().expect("pending");
                assert!(pending.scrolled, "scrolled flag stays set after re-apply");
            });
        });
    });
}

#[test]
fn focus_follows_absorbed_descendant_once_its_item_is_materialized() {
    VirtualFS::test("file_tree_focus_follow_deferred", |dirs, mut vfs| {
        vfs.mkdir("tree/warp-server")
            .with_files(vec![Stub::FileWithContent(
                "tree/warp-server/main.rs",
                "fn main() {}\n",
            )]);
        let tree = dirs.tests().join("tree");
        let warp_server = tree.join("warp-server");

        App::test((), |mut app| async move {
            let _ = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            // User cd's into warp-server with ~/tree as the ancestor root.
            // The warp-server entry should be materialized by indexing and
            // selected as the focus-follow target.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![warp_server.clone(), tree.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                // Single displayed root, descendant absorbed.
                assert_eq!(view.displayed_directories, vec![std_path(&tree)]);
                // Selection landed on warp-server's directory header.
                let selected = view.selected_item.clone().expect("selection set");
                assert_eq!(selected.root, std_path(&tree));
                let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                let selected_item = root_dir
                    .items
                    .get(selected.index)
                    .expect("selected index in range");
                assert_eq!(selected_item.path(), &std_path(&warp_server));
                // Pending target is preserved across rebuilds so later
                // repo-metadata updates don't override the cwd-follow
                // selection. It clears when the user interacts explicitly
                // (see pending_focus_target_cleared_on_user_select).
                let pending = view
                    .pending_focus_target
                    .as_ref()
                    .expect("pending target preserved");
                assert_eq!(pending.root, std_path(&tree));
                assert_eq!(pending.path, std_path(&warp_server));
                // The initial apply scrolled; later applies must not
                // re-scroll so user scrolling is respected.
                assert!(pending.scrolled, "initial apply scrolls the tree");
            });

            // User clicks somewhere else (simulated via select_id). Pending
            // target must clear so future rebuilds don't re-steal focus.
            file_tree_view.update(&mut app, |view, ctx| {
                let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                let id = super::FileTreeIdentifier {
                    root: std_path(&tree),
                    index: 0,
                };
                // Sanity: the first item is the root header, not warp-server.
                assert_ne!(
                    root_dir.items.first().unwrap().path(),
                    &std_path(&warp_server)
                );
                view.select_id(&id, ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view.pending_focus_target.is_none());
            });
        });
    });
}

#[test]
fn descendant_is_absorbed_into_ancestor() {
    VirtualFS::test("file_tree_absorb_descendant", |dirs, mut vfs| {
        vfs.mkdir("tree/a")
            .with_files(vec![Stub::FileWithContent("tree/a/x.txt", "x")]);
        let tree = dirs.tests().join("tree");
        let a = tree.join("a");

        App::test((), |mut app| async move {
            let _ = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                // Input in most-recent-first order: descendant first.
                view.set_root_directories(vec![a.clone(), tree.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                // Only the ancestor survives as a displayed root.
                assert_eq!(view.displayed_directories, vec![std_path(&tree)]);
                assert!(view.root_directories.contains_key(&std_path(&tree)));
                assert!(!view.root_directories.contains_key(&std_path(&a)));
                // The absorbed descendant is expanded inside the surviving root.
                let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                assert!(root_dir.expanded_folders.contains(&std_path(&a)));
            });
        });
    });
}

#[test]
fn cd_into_descendant_absorbs_into_existing_ancestor_root() {
    VirtualFS::test("file_tree_cd_into_descendant", |dirs, mut vfs| {
        vfs.mkdir("tree/a/z")
            .with_files(vec![Stub::FileWithContent("tree/a/z/file.txt", "f")]);
        let tree = dirs.tests().join("tree");
        let z = tree.join("a/z");

        App::test((), |mut app| async move {
            let _ = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            // Start with only the ancestor displayed.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![tree.clone()], ctx);
            });

            // Simulate cd-ing into ~/tree/a/z by emitting the descendant as the
            // most-recent path.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_root_directories(vec![z.clone(), tree.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                // Still a single root, no new top-level entry.
                assert_eq!(view.displayed_directories, vec![std_path(&tree)]);
                // Ancestor chain is auto-expanded down to the cwd.
                let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                assert!(root_dir
                    .expanded_folders
                    .contains(&std_path(&tree.join("a"))));
                assert!(root_dir.expanded_folders.contains(&std_path(&z)));
            });
        });
    });
}

#[test]
fn explicit_collapse_blocks_auto_expand_on_absorption() {
    VirtualFS::test("file_tree_collapse_blocks_expand", |dirs, mut vfs| {
        vfs.mkdir("tree/a/z")
            .with_files(vec![Stub::FileWithContent("tree/a/z/file.txt", "f")]);
        let tree = dirs.tests().join("tree");
        let a = tree.join("a");
        let z = a.join("z");

        App::test((), |mut app| async move {
            let _ = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            // Start with the ancestor displayed and explicitly collapse `a`.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![tree.clone()], ctx);
                // First expand so the toggle records a collapse.
                view.toggle_folder_expansion(&std_path(&tree), &std_path(&a), ctx);
                view.toggle_folder_expansion(&std_path(&tree), &std_path(&a), ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view.is_explicitly_collapsed(&std_path(&tree), &std_path(&a)));
            });

            // Now cd into ~/tree/a/z. Auto-expansion must not re-open `a`.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_root_directories(vec![z.clone(), tree.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                let root_dir = view.root_directories.get(&std_path(&tree)).unwrap();
                assert!(!root_dir.expanded_folders.contains(&std_path(&a)));
                assert!(!root_dir.expanded_folders.contains(&std_path(&z)));
                assert!(view.is_explicitly_collapsed(&std_path(&tree), &std_path(&a)));
            });
        });
    });
}

#[test]
fn absorption_migrates_expanded_and_explicitly_collapsed_state() {
    VirtualFS::test("file_tree_absorb_migrates_state", |dirs, mut vfs| {
        vfs.mkdir("tree/a/z")
            .with_files(vec![Stub::FileWithContent("tree/a/z/file.txt", "f")]);
        let tree = dirs.tests().join("tree");
        let a = tree.join("a");
        let z = a.join("z");

        App::test((), |mut app| async move {
            let _ = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            // Start with `a` as a standalone top-level root and record
            // an explicit collapse on `a/z` under that standalone root.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![a.clone()], ctx);
                // Expand then collapse z so the toggle records a collapse on it.
                view.toggle_folder_expansion(&std_path(&a), &std_path(&z), ctx);
                view.toggle_folder_expansion(&std_path(&a), &std_path(&z), ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view.is_explicitly_collapsed(&std_path(&a), &std_path(&z)));
            });

            // Now absorb `a` into `tree` by adding the ancestor.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_root_directories(vec![a.clone(), tree.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                // Standalone absorbed-root entry is gone.
                assert!(!view.root_directories.contains_key(&std_path(&a)));
                // Its explicit-collapse state moved over to the ancestor.
                assert!(view.is_explicitly_collapsed(&std_path(&tree), &std_path(&z)));
            });
        });
    });
}

#[test]
fn absorbed_descendant_is_unregistered_from_lazy_loaded_paths() {
    VirtualFS::test("file_tree_absorb_unregisters_lazy", |dirs, mut vfs| {
        vfs.mkdir("tree/a")
            .with_files(vec![Stub::FileWithContent("tree/a/x.txt", "x")]);
        let tree = dirs.tests().join("tree");
        let a = tree.join("a");

        App::test((), |mut app| async move {
            let (_, repository_metadata_model) = initialize_app(&mut app);
            let (_, file_tree_view) = app.add_window(WindowStyle::NotStealFocus, FileTreeView::new);

            // Initial state: `a` alone is a standalone lazy-loaded root.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_is_active(true, ctx);
                view.set_root_directories(vec![a.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(view.registered_lazy_loaded_paths.contains(&std_path(&a)));
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(model.is_lazy_loaded_path(&std_path(&a), ctx));
            });

            // Add the ancestor. `a` should be absorbed and its lazy-loaded
            // registration should be cleaned up.
            file_tree_view.update(&mut app, |view, ctx| {
                view.set_root_directories(vec![a.clone(), tree.clone()], ctx);
            });

            file_tree_view.read(&app, |view, _ctx| {
                assert!(!view.registered_lazy_loaded_paths.contains(&std_path(&a)));
                assert!(view.registered_lazy_loaded_paths.contains(&std_path(&tree)));
            });
            repository_metadata_model.read(&app, |model, ctx| {
                assert!(!model.is_lazy_loaded_path(&std_path(&a), ctx));
            });
        });
    });
}
