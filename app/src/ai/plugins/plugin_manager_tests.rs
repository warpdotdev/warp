use std::collections::{HashMap, HashSet};

use repo_metadata::{RepositoryUpdate, TargetFile};
use tempfile::tempdir;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::*;

fn target(path: &str) -> TargetFile {
    TargetFile::new(PathBuf::from(path), false)
}

#[test]
fn plugin_file_add_change_delete_and_move_trigger_a_rescan() {
    let search_roots = vec![
        PathBuf::from("/repo/.agents/plugins"),
        PathBuf::from("/repo/.warp/plugins"),
    ];
    for update in [
        RepositoryUpdate {
            added: HashSet::from([target("/repo/.agents/plugins/tools/plugin.json")]),
            ..Default::default()
        },
        RepositoryUpdate {
            modified: HashSet::from([target("/repo/.warp/plugins/tools/skills/deploy/SKILL.md")]),
            ..Default::default()
        },
        RepositoryUpdate {
            deleted: HashSet::from([target("/repo/.agents/plugins/tools/mcp.json")]),
            ..Default::default()
        },
        RepositoryUpdate {
            moved: HashMap::from([(
                target("/repo/.warp/plugins/renamed/plugin.json"),
                target("/repo/.warp/plugins/original/plugin.json"),
            )]),
            ..Default::default()
        },
    ] {
        assert!(update_affects_search_roots(&update, &search_roots));
    }
}

#[test]
fn creating_or_removing_a_plugin_provider_directory_triggers_a_rescan() {
    let search_roots = vec![PathBuf::from("/repo/.agents/plugins")];

    for path in ["/repo/.agents", "/repo/.agents/plugins"] {
        let update = RepositoryUpdate {
            added: HashSet::from([target(path)]),
            ..Default::default()
        };
        assert!(update_affects_search_roots(&update, &search_roots));
    }
}

#[test]
fn unrelated_repository_changes_do_not_trigger_a_rescan() {
    let update = RepositoryUpdate {
        modified: HashSet::from([target("/repo/src/main.rs")]),
        ..Default::default()
    };
    assert!(!update_affects_search_roots(
        &update,
        &[PathBuf::from("/repo/.agents/plugins")]
    ));
}

#[test]
fn repository_bootstrap_registers_already_detected_roots() {
    App::test((), |mut app| async move {
        let directory_watcher = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let detected_repositories = app.add_singleton_model(|_| DetectedRepositories::default());
        let root = tempdir().unwrap();
        let standardized_root = StandardizedPath::from_local_canonicalized(root.path()).unwrap();
        let canonical_root = standardized_root.to_local_path().unwrap();
        let repository = directory_watcher
            .update(&mut app, |watcher, ctx| {
                watcher.add_directory(standardized_root.clone(), ctx)
            })
            .unwrap();
        detected_repositories.update(&mut app, |repositories, _| {
            repositories.insert_test_repo_root(standardized_root);
        });

        let (watcher_message_tx, _watcher_message_rx) = async_channel::unbounded();
        let manager = app.add_model(|_| PluginManager {
            registry: PluginRegistry::new(true),
            policy: PluginDiscoveryPolicy::InteractivePreference,
            repository_roots: BTreeSet::new(),
            watcher_message_tx,
            watcher_subscriptions: BTreeMap::new(),
            data_locator: LocalPluginDataLocator::new("/data", PluginFrontend::Gui),
        });
        manager.update(&mut app, |manager, ctx| {
            manager.start_repository_watchers(ctx);
        });

        manager.read(&app, |manager, _| {
            assert!(manager.repository_roots.contains(&canonical_root));
            assert!(manager.watcher_subscriptions.contains_key(&canonical_root));
        });
        repository.read(&app, |repository, _| {
            assert_eq!(repository.watcher_count(), 1);
        });
    });
}

#[test]
fn stop_watchers_removes_owned_repository_subscriptions() {
    App::test((), |mut app| async move {
        let directory_watcher = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let root = tempdir().unwrap();
        let standardized_root = StandardizedPath::from_local_canonicalized(root.path()).unwrap();
        let repository = directory_watcher
            .update(&mut app, |watcher, ctx| {
                watcher.add_directory(standardized_root, ctx)
            })
            .unwrap();
        let (watcher_message_tx, _watcher_message_rx) = async_channel::unbounded();
        let manager = app.add_model(|_| PluginManager {
            registry: PluginRegistry::new(true),
            policy: PluginDiscoveryPolicy::InteractivePreference,
            repository_roots: BTreeSet::new(),
            watcher_message_tx,
            watcher_subscriptions: BTreeMap::new(),
            data_locator: LocalPluginDataLocator::new("/data", PluginFrontend::Gui),
        });

        manager.update(&mut app, |manager, ctx| {
            manager.watch_repository(root.path().to_path_buf(), repository.clone(), ctx);
        });
        repository.read(&app, |repository, _| {
            assert_eq!(repository.watcher_count(), 1);
        });

        manager.update(&mut app, |manager, ctx| {
            manager.stop_watchers(ctx);
        });
        repository.read(&app, |repository, _| {
            assert_eq!(repository.watcher_count(), 0);
        });
        manager.read(&app, |manager, _| {
            assert!(manager.watcher_subscriptions.is_empty());
        });
    });
}
