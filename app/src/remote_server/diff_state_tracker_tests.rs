use warp_util::standardized_path::StandardizedPath;

use crate::code_review::diff_state::DiffMode;

use super::super::protocol::RequestId;
use super::super::server_model::ConnectionId;
use super::{DiffModelKey, RemoteDiffStateManager};

/// Uses `try_new` instead of `try_from_local` so that Unix-style paths
/// like `/repo` are recognised as absolute on all platforms (including Windows).
fn test_key(repo: &str, mode: DiffMode) -> DiffModelKey {
    DiffModelKey {
        repo_path: StandardizedPath::try_new(repo).unwrap(),
        mode,
    }
}

fn new_conn() -> ConnectionId {
    uuid::Uuid::new_v4()
}

// ── Subscription tracking ───────────────────────────────────────────

#[test]
fn subscribe_registers_connection() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn = new_conn();

    model.subscribe_connection(key.clone(), conn);

    assert_eq!(model.subscribed_connections(&key), vec![conn]);
}

#[test]
fn subscribe_multiple_connections_to_same_key() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn_a = new_conn();
    let conn_b = new_conn();

    model.subscribe_connection(key.clone(), conn_a);
    model.subscribe_connection(key.clone(), conn_b);

    let subs = model.subscribed_connections(&key);
    assert_eq!(subs.len(), 2);
    assert!(subs.contains(&conn_a));
    assert!(subs.contains(&conn_b));
}

#[test]
fn subscribe_same_connection_to_different_keys() {
    let mut model = RemoteDiffStateManager::new();
    let key_head = test_key("/repo", DiffMode::Head);
    let key_main = test_key("/repo", DiffMode::MainBranch);
    let conn = new_conn();

    model.subscribe_connection(key_head.clone(), conn);
    model.subscribe_connection(key_main.clone(), conn);

    assert_eq!(model.subscribed_connections(&key_head), vec![conn]);
    assert_eq!(model.subscribed_connections(&key_main), vec![conn]);
}

#[test]
fn subscribed_connections_returns_empty_for_unknown_key() {
    let model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);

    assert!(model.subscribed_connections(&key).is_empty());
}

// ── Unsubscribe ─────────────────────────────────────────────────────

#[test]
fn unsubscribe_last_connection_removes_model() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn = new_conn();

    // Simulate model insertion + subscription (what handle_get_diff_state does).
    warpui::App::test((), |mut app| async move {
        let handle = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key.clone(), handle);
        model.subscribe_connection(key.clone(), conn);

        model.unsubscribe_connection(&key, conn);

        assert!(model.get_model(&key).is_none());
        assert!(model.subscribed_connections(&key).is_empty());
    });
}

#[test]
fn unsubscribe_one_of_two_keeps_model() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn_a = new_conn();
    let conn_b = new_conn();

    warpui::App::test((), |mut app| async move {
        let handle = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key.clone(), handle);
        model.subscribe_connection(key.clone(), conn_a);
        model.subscribe_connection(key.clone(), conn_b);

        model.unsubscribe_connection(&key, conn_a);

        assert!(model.get_model(&key).is_some());
        assert_eq!(model.subscribed_connections(&key), vec![conn_b]);
    });
}

#[test]
fn unsubscribe_clears_pending_responses_for_that_connection() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn_a = new_conn();
    let conn_b = new_conn();

    warpui::App::test((), |mut app| async move {
        let handle = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key.clone(), handle);
        model.subscribe_connection(key.clone(), conn_a);
        model.subscribe_connection(key.clone(), conn_b);
        model.add_pending_response(key.clone(), RequestId::new(), conn_a);
        model.add_pending_response(key.clone(), RequestId::new(), conn_b);

        model.unsubscribe_connection(&key, conn_a);

        // Only conn_b's pending response should remain.
        let pending = model.drain_pending_responses(&key);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].conn_id, conn_b);
    });
}

// ── remove_connection ───────────────────────────────────────────────

#[test]
fn remove_connection_unsubscribes_from_all_keys() {
    let mut model = RemoteDiffStateManager::new();
    let key_head = test_key("/repo", DiffMode::Head);
    let key_main = test_key("/repo", DiffMode::MainBranch);
    let conn = new_conn();

    warpui::App::test((), |mut app| async move {
        let h1 = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        let h2 = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key_head.clone(), h1);
        model.insert_model(key_main.clone(), h2);
        model.subscribe_connection(key_head.clone(), conn);
        model.subscribe_connection(key_main.clone(), conn);

        model.remove_connection(conn);

        // Both models dropped because conn was the sole subscriber.
        assert!(model.get_model(&key_head).is_none());
        assert!(model.get_model(&key_main).is_none());
    });
}

#[test]
fn remove_connection_keeps_models_with_other_subscribers() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn_a = new_conn();
    let conn_b = new_conn();

    warpui::App::test((), |mut app| async move {
        let handle = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key.clone(), handle);
        model.subscribe_connection(key.clone(), conn_a);
        model.subscribe_connection(key.clone(), conn_b);

        model.remove_connection(conn_a);

        assert!(model.get_model(&key).is_some());
        assert_eq!(model.subscribed_connections(&key), vec![conn_b]);
    });
}

#[test]
fn remove_connection_clears_pending_responses() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn = new_conn();

    warpui::App::test((), |mut app| async move {
        let handle = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key.clone(), handle);
        model.subscribe_connection(key.clone(), conn);
        model.add_pending_response(key.clone(), RequestId::new(), conn);

        model.remove_connection(conn);

        // Model removed, so pending responses should be gone too.
        assert!(!model.has_pending_responses(&key));
    });
}

// ── Pending response tracking ───────────────────────────────────────

#[test]
fn has_pending_responses_false_when_empty() {
    let model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);

    assert!(!model.has_pending_responses(&key));
}

#[test]
fn add_and_drain_pending_responses() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn = new_conn();
    let rid = RequestId::new();

    model.add_pending_response(key.clone(), rid, conn);

    assert!(model.has_pending_responses(&key));

    let drained = model.drain_pending_responses(&key);
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].conn_id, conn);

    // After drain, no pending responses remain.
    assert!(!model.has_pending_responses(&key));
}

#[test]
fn drain_pending_responses_returns_empty_for_unknown_key() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);

    assert!(model.drain_pending_responses(&key).is_empty());
}

#[test]
fn multiple_pending_responses_for_same_key() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn_a = new_conn();
    let conn_b = new_conn();

    model.add_pending_response(key.clone(), RequestId::new(), conn_a);
    model.add_pending_response(key.clone(), RequestId::new(), conn_b);

    let drained = model.drain_pending_responses(&key);
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].conn_id, conn_a);
    assert_eq!(drained[1].conn_id, conn_b);
}

// ── Model CRUD ──────────────────────────────────────────────────────

#[test]
fn get_model_returns_none_when_empty() {
    let model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);

    assert!(model.get_model(&key).is_none());
}

#[test]
fn insert_and_get_model() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);

    warpui::App::test((), |mut app| async move {
        let handle = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key.clone(), handle);

        assert!(model.get_model(&key).is_some());
    });
}

#[test]
fn remove_model_clears_pending_and_subscriptions() {
    let mut model = RemoteDiffStateManager::new();
    let key = test_key("/repo", DiffMode::Head);
    let conn = new_conn();

    warpui::App::test((), |mut app| async move {
        let handle = app
            .add_model(|ctx| crate::code_review::diff_state::LocalDiffStateModel::new(None, ctx));
        model.insert_model(key.clone(), handle);
        model.subscribe_connection(key.clone(), conn);
        model.add_pending_response(key.clone(), RequestId::new(), conn);

        model.remove_model(&key);

        assert!(model.get_model(&key).is_none());
        assert!(!model.has_pending_responses(&key));
        assert!(model.subscribed_connections(&key).is_empty());
    });
}

// ── Key equality ────────────────────────────────────────────────────

#[test]
fn different_modes_are_different_keys() {
    let mut model = RemoteDiffStateManager::new();
    let key_head = test_key("/repo", DiffMode::Head);
    let key_main = test_key("/repo", DiffMode::MainBranch);
    let conn = new_conn();

    model.subscribe_connection(key_head.clone(), conn);

    assert_eq!(model.subscribed_connections(&key_head).len(), 1);
    assert!(model.subscribed_connections(&key_main).is_empty());
}

#[test]
fn different_repos_are_different_keys() {
    let mut model = RemoteDiffStateManager::new();
    let key_a = test_key("/repo-a", DiffMode::Head);
    let key_b = test_key("/repo-b", DiffMode::Head);
    let conn = new_conn();

    model.subscribe_connection(key_a.clone(), conn);

    assert_eq!(model.subscribed_connections(&key_a).len(), 1);
    assert!(model.subscribed_connections(&key_b).is_empty());
}
