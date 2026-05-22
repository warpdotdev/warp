use std::collections::HashMap;
use std::sync::Arc;

use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::super::diff_state_tracker::RemoteDiffStateManager;
use super::super::proto::{
    Authenticate, BeginRemoteUpload, Initialize, RemoteUploadManifestEntry, UploadRemoteFileChunk,
};
use super::super::server_buffer_tracker::ServerBufferTracker;
use super::{
    append_remote_upload_chunk, build_remote_upload_session, complete_remote_upload_session,
    remote_upload_preflight, PendingFileOps, ServerModel,
};
use crate::auth::auth_state::AuthState;
use crate::code_review::diff_state::DiffMode;
use crate::remote_server::diff_state_tracker::DiffModelKey;

fn test_model(app: &mut App) -> ServerModel {
    ServerModel {
        connection_senders: HashMap::new(),
        snapshot_sent_roots_by_connection: HashMap::new(),
        grace_timer_cancel: None,
        in_progress: HashMap::new(),
        host_id: "test-host-id".to_string(),
        executors: HashMap::new(),
        pending_file_ops: PendingFileOps::new(),
        remote_upload_sessions: HashMap::new(),
        auth_state: Arc::new(AuthState::new_logged_out_for_test()),
        buffers: ServerBufferTracker::new(),
        diff_states: app.add_model(|_| RemoteDiffStateManager::new()),
    }
}

/// Uses `try_new` instead of `try_from_local` so that Unix-style paths
/// like `/repo` are recognised as absolute on all platforms (including Windows).
fn test_key(repo: &str, mode: DiffMode) -> DiffModelKey {
    DiffModelKey {
        repo_path: StandardizedPath::try_new(repo).unwrap(),
        mode,
    }
}

#[test]
fn fresh_model_starts_without_auth_token() {
    App::test((), |mut app| async move {
        let model = test_model(&mut app);

        assert_eq!(model.auth_token().as_deref(), None);
        assert_eq!(model.auth_state.user_id(), None);
        assert_eq!(model.auth_state.user_email(), None);
    });
}

#[test]
fn initialize_with_auth_token_stores_token() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);

        model.apply_initialize_auth(&Initialize {
            auth_token: "initial-token".to_string(),
            user_id: "test-user-id".to_string(),
            user_email: "test@example.com".to_string(),
            crash_reporting_enabled: true,
            codebase_index_limits: None,
        });

        assert_eq!(model.auth_token().as_deref(), Some("initial-token"));
        assert_eq!(
            model.auth_state.user_id().unwrap().as_string(),
            "test-user-id"
        );
        assert_eq!(
            model.auth_state.user_email().as_deref(),
            Some("test@example.com")
        );
    });
}

#[test]
fn empty_initialize_clears_auth_context() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        model.apply_initialize_auth(&Initialize {
            auth_token: "initial-token".to_string(),
            user_id: "test-user-id".to_string(),
            user_email: "test@example.com".to_string(),
            crash_reporting_enabled: true,
            codebase_index_limits: None,
        });

        model.apply_initialize_auth(&Initialize {
            auth_token: String::new(),
            user_id: String::new(),
            user_email: String::new(),
            crash_reporting_enabled: true,
            codebase_index_limits: None,
        });

        assert_eq!(model.auth_token().as_deref(), None);
        assert_eq!(model.auth_state.user_id(), None);
        assert_eq!(model.auth_state.user_email(), None);
    });
}

#[test]
fn authenticate_with_auth_token_replaces_auth_token() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        model.apply_initialize_auth(&Initialize {
            auth_token: "initial-token".to_string(),
            user_id: String::new(),
            user_email: String::new(),
            crash_reporting_enabled: true,
            codebase_index_limits: None,
        });

        model.handle_authenticate(Authenticate {
            auth_token: "rotated-token".to_string(),
        });

        assert_eq!(model.auth_token().as_deref(), Some("rotated-token"));
    });
}

#[test]
fn empty_authenticate_clears_auth_token() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        model.apply_initialize_auth(&Initialize {
            auth_token: "initial-token".to_string(),
            user_id: String::new(),
            user_email: String::new(),
            crash_reporting_enabled: true,
            codebase_index_limits: None,
        });

        model.handle_authenticate(Authenticate {
            auth_token: String::new(),
        });

        assert_eq!(model.auth_token().as_deref(), None);
    });
}

// ── Diff state: connection cleanup ──────────────────────────────────

#[test]
fn deregister_connection_cleans_up_diff_state_subscriptions() {
    App::test((), |mut app| async move {
        let mut model = test_model(&mut app);
        let conn = uuid::Uuid::new_v4();

        // Register the connection.
        let (tx, _rx) = async_channel::unbounded();
        model.connection_senders.insert(conn, tx);

        // Subscribe the connection to diff state via the manager.
        let key = test_key("/repo", DiffMode::Head);
        let key2 = key.clone();
        let key3 = key.clone();
        model.diff_states.update(&mut app, |mgr, _ctx| {
            mgr.subscribe_connection(key, conn);
        });
        let has_sub = model.diff_states.read(&app, |mgr, _ctx| {
            !mgr.subscribed_connections(&key2).is_empty()
        });
        assert!(has_sub);

        // Simulate deregister_connection's diff state cleanup.
        model.diff_states.update(&mut app, |mgr, _ctx| {
            mgr.remove_connection(conn);
        });
        let has_sub = model.diff_states.read(&app, |mgr, _ctx| {
            !mgr.subscribed_connections(&key3).is_empty()
        });
        assert!(!has_sub);
    });
}

#[test]
fn diff_states_starts_empty() {
    App::test((), |mut app| async move {
        let model = test_model(&mut app);
        let key = test_key("/repo", DiffMode::Head);
        let empty = model.diff_states.read(&app, |mgr, _ctx| {
            mgr.subscribed_connections(&key).is_empty()
        });
        assert!(empty);
    });
}

#[test]
fn remote_upload_preflight_reports_file_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let target = repo.join("src");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("main.bin"), [1, 2, 3]).unwrap();

    let result = remote_upload_preflight(
        repo.to_str().unwrap(),
        target.to_str().unwrap(),
        &[RemoteUploadManifestEntry {
            relative_path: "main.bin".to_string(),
            is_directory: false,
            size: 4,
            unix_mode: None,
        }],
    )
    .unwrap();

    assert_eq!(result.conflicts, vec!["main.bin"]);
    assert_eq!(result.file_count, 1);
    assert_eq!(result.total_bytes, 4);
}

#[test]
fn remote_upload_preflight_rejects_path_traversal() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let error = remote_upload_preflight(
        repo.to_str().unwrap(),
        repo.to_str().unwrap(),
        &[RemoteUploadManifestEntry {
            relative_path: "../outside.bin".to_string(),
            is_directory: false,
            size: 1,
            unix_mode: None,
        }],
    )
    .expect_err("path traversal should be rejected");

    assert!(error.contains("cannot escape"));
}

#[tokio::test]
async fn remote_upload_writes_binary_chunks_and_cleans_staging_dir() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let target = repo.join("src");
    std::fs::create_dir_all(&target).unwrap();

    let upload_id = "upload-test".to_string();
    let (_upload_id, session) = build_remote_upload_session(BeginRemoteUpload {
        upload_id: upload_id.clone(),
        repo_path: repo.to_str().unwrap().to_string(),
        target_dir: target.to_str().unwrap().to_string(),
        entries: vec![RemoteUploadManifestEntry {
            relative_path: "nested/main.bin".to_string(),
            is_directory: false,
            size: 4,
            unix_mode: None,
        }],
        overwrite: false,
    })
    .await
    .unwrap();

    let temp_dir = session.temp_dir.clone();
    append_remote_upload_chunk(
        session.clone(),
        UploadRemoteFileChunk {
            upload_id,
            relative_path: "nested/main.bin".to_string(),
            offset: 0,
            data: vec![0, 159],
        },
    )
    .await
    .unwrap();
    append_remote_upload_chunk(
        session.clone(),
        UploadRemoteFileChunk {
            upload_id: "upload-test".to_string(),
            relative_path: "nested/main.bin".to_string(),
            offset: 2,
            data: vec![146, 150],
        },
    )
    .await
    .unwrap();

    let result = complete_remote_upload_session(session).await.unwrap();

    assert_eq!(result.file_count, 1);
    assert_eq!(
        std::fs::read(target.join("nested/main.bin")).unwrap(),
        vec![0, 159, 146, 150]
    );
    assert!(!temp_dir.exists());
}

#[tokio::test]
async fn remote_upload_replace_flow_overwrites_existing_file() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let target = repo.join("src");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("main.bin"), [9]).unwrap();

    let upload_id = "replace-test".to_string();
    let (_upload_id, session) = build_remote_upload_session(BeginRemoteUpload {
        upload_id: upload_id.clone(),
        repo_path: repo.to_str().unwrap().to_string(),
        target_dir: target.to_str().unwrap().to_string(),
        entries: vec![RemoteUploadManifestEntry {
            relative_path: "main.bin".to_string(),
            is_directory: false,
            size: 1,
            unix_mode: None,
        }],
        overwrite: true,
    })
    .await
    .unwrap();

    append_remote_upload_chunk(
        session.clone(),
        UploadRemoteFileChunk {
            upload_id,
            relative_path: "main.bin".to_string(),
            offset: 0,
            data: vec![7],
        },
    )
    .await
    .unwrap();

    let result = complete_remote_upload_session(session).await.unwrap();

    assert_eq!(result.file_count, 1);
    assert_eq!(std::fs::read(target.join("main.bin")).unwrap(), vec![7]);
}

#[tokio::test]
async fn remote_upload_creates_empty_file() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let target = repo.join("src");
    std::fs::create_dir_all(&target).unwrap();

    let (_upload_id, session) = build_remote_upload_session(BeginRemoteUpload {
        upload_id: "empty-file-test".to_string(),
        repo_path: repo.to_str().unwrap().to_string(),
        target_dir: target.to_str().unwrap().to_string(),
        entries: vec![RemoteUploadManifestEntry {
            relative_path: "empty.bin".to_string(),
            is_directory: false,
            size: 0,
            unix_mode: None,
        }],
        overwrite: false,
    })
    .await
    .unwrap();

    let result = complete_remote_upload_session(session).await.unwrap();

    assert_eq!(result.file_count, 1);
    assert_eq!(
        std::fs::read(target.join("empty.bin")).unwrap(),
        Vec::<u8>::new()
    );
}
