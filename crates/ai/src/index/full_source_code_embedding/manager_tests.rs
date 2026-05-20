use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::index::full_source_code_embedding::store_client::MockStoreClient;
#[cfg(feature = "local_fs")]
use crate::index::full_source_code_embedding::SnapshotStorage;
use warpui::App;

use crate::workspace::WorkspaceMetadata;

use super::{
    BuildSource, CodebaseIndexFinishedStatus, CodebaseIndexManager, CodebaseIndexManagerConfig,
    CodebaseIndexStatus, CodebaseIndexStatusEventKey, CodebaseIndexingError, SyncProgress,
};

fn workspace_metadata(path: impl Into<PathBuf>) -> WorkspaceMetadata {
    WorkspaceMetadata {
        path: path.into(),
        navigated_ts: None,
        modified_ts: None,
        queried_ts: None,
    }
}

fn codebase_index_status(
    has_pending: bool,
    has_synced_version: bool,
    last_sync_successful: Option<CodebaseIndexFinishedStatus>,
    sync_progress: Option<SyncProgress>,
) -> CodebaseIndexStatus {
    CodebaseIndexStatus {
        has_pending,
        has_synced_version,
        last_sync_successful,
        sync_progress,
        root_hash: None,
    }
}

#[test]
fn codebase_index_status_event_key_matches_identical_statuses() {
    let first_status = codebase_index_status(
        true,
        true,
        None,
        Some(SyncProgress::Syncing {
            completed_nodes: 1,
            total_nodes: 2,
        }),
    );
    let duplicate_status = codebase_index_status(
        true,
        true,
        None,
        Some(SyncProgress::Syncing {
            completed_nodes: 1,
            total_nodes: 2,
        }),
    );

    assert_eq!(
        CodebaseIndexStatusEventKey::from(&first_status),
        CodebaseIndexStatusEventKey::from(&duplicate_status)
    );
}

#[test]
fn codebase_index_status_event_key_detects_semantic_changes() {
    let syncing_status = codebase_index_status(
        true,
        true,
        None,
        Some(SyncProgress::Syncing {
            completed_nodes: 1,
            total_nodes: 2,
        }),
    );
    let completed_status = codebase_index_status(
        false,
        true,
        Some(CodebaseIndexFinishedStatus::Completed),
        None,
    );
    let failed_status = codebase_index_status(
        false,
        true,
        Some(CodebaseIndexFinishedStatus::Failed(
            CodebaseIndexingError::BuildTreeError,
        )),
        None,
    );

    assert_ne!(
        CodebaseIndexStatusEventKey::from(&syncing_status),
        CodebaseIndexStatusEventKey::from(&completed_status)
    );
    assert_ne!(
        CodebaseIndexStatusEventKey::from(&completed_status),
        CodebaseIndexStatusEventKey::from(&failed_status)
    );
}

#[test]
fn initializes_with_indexing_enabled_when_configured() {
    App::test((), |app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                vec![workspace_metadata("repo")],
                Some(1),
                1000,
                32,
                Arc::new(MockStoreClient),
                true,
                ctx,
            )
        });

        manager.read(&app, |manager, _| {
            assert!(manager.is_indexing_enabled());
            assert_eq!(manager.num_active_indices(), 0);
            assert!(manager.can_create_new_indices());
        });
    });
}
#[test]
fn initializes_with_indexing_disabled_when_configured() {
    App::test((), |app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                vec![workspace_metadata("repo")],
                Some(1),
                1000,
                32,
                Arc::new(MockStoreClient),
                false,
                ctx,
            )
        });

        manager.read(&app, |manager, _| {
            assert!(!manager.is_indexing_enabled());
            assert_eq!(manager.num_active_indices(), 0);
            assert!(!manager.can_create_new_indices());
        });
    });
}

#[test]
#[cfg(feature = "local_fs")]
fn initializes_with_injected_snapshot_storage_when_configured() {
    App::test((), |app| async move {
        let snapshot_dir = tempfile::tempdir().unwrap();
        let storage = SnapshotStorage::from_dir(snapshot_dir.path().join("daemon")).unwrap();
        let expected_snapshot_dir = storage.path().to_path_buf();
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new_with_snapshot_storage(
                CodebaseIndexManagerConfig::new(
                    vec![workspace_metadata("repo")],
                    Some(1),
                    1000,
                    32,
                    Arc::new(MockStoreClient),
                    false,
                ),
                Some(storage),
                ctx,
            )
        });

        manager.read(&app, |manager, _| {
            let snapshot_storage = manager.snapshot_storage.as_ref().unwrap();
            assert_eq!(snapshot_storage.path(), expected_snapshot_dir);
            assert!(!snapshot_storage.is_app_default());
        });
    });
}

#[test]
fn can_create_new_indices_honors_max_limit_when_enabled() {
    App::test((), |mut app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                Vec::new(),
                Some(1),
                1000,
                32,
                Arc::new(MockStoreClient),
                true,
                ctx,
            )
        });

        manager.update(&mut app, |manager, ctx| {
            assert!(manager.can_create_new_indices());
            manager.update_max_limits(Some(0), 1000, 32, ctx);
            assert!(!manager.can_create_new_indices());
        });
    });
}
#[test]
fn index_directory_is_noop_when_indexing_disabled() {
    App::test((), |mut app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                Vec::new(),
                Some(1),
                1000,
                32,
                Arc::new(MockStoreClient),
                false,
                ctx,
            )
        });

        manager.update(&mut app, |manager, ctx| {
            assert!(!manager.index_directory(PathBuf::from("repo"), ctx));
            assert_eq!(manager.num_active_indices(), 0);
        });
    });
}

#[test]
fn index_directory_reports_when_max_index_limit_prevents_creation() {
    App::test((), |mut app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                Vec::new(),
                Some(0),
                1000,
                32,
                Arc::new(MockStoreClient),
                true,
                ctx,
            )
        });

        manager.update(&mut app, |manager, ctx| {
            assert!(!manager.index_directory(PathBuf::from("repo"), ctx));
            assert_eq!(manager.num_active_indices(), 0);
        });
    });
}

#[test]
fn build_and_sync_is_noop_when_indexing_disabled() {
    App::test((), |mut app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                Vec::new(),
                Some(1),
                1000,
                32,
                Arc::new(MockStoreClient),
                false,
                ctx,
            )
        });

        manager.update(&mut app, |manager, ctx| {
            assert!(!manager
                .build_and_sync_codebase_index(BuildSource::FromPath(Path::new("repo")), ctx));
            assert_eq!(manager.num_active_indices(), 0);
        });
    });
}

#[test]
fn trigger_incremental_sync_returns_err_when_enabled_and_index_missing() {
    App::test((), |mut app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                Vec::new(),
                Some(1),
                1000,
                32,
                Arc::new(MockStoreClient),
                true,
                ctx,
            )
        });

        manager.update(&mut app, |manager, ctx| {
            let result = manager.trigger_incremental_sync_for_path(Path::new("repo"), ctx);
            assert!(result.is_err());
        });
    });
}
#[test]
fn trigger_incremental_sync_returns_ok_when_indexing_disabled() {
    App::test((), |mut app| async move {
        let manager = app.add_singleton_model(|ctx| {
            CodebaseIndexManager::new(
                Vec::new(),
                Some(1),
                1000,
                32,
                Arc::new(MockStoreClient),
                false,
                ctx,
            )
        });

        manager.update(&mut app, |manager, ctx| {
            let result = manager.trigger_incremental_sync_for_path(Path::new("repo"), ctx);
            assert!(result.is_ok());
        });
    });
}
