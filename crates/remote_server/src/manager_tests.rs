use futures::channel::oneshot;
use warp_core::SessionId;
use warp_util::standardized_path::StandardizedPath;
use warpui_core::App;

use super::{
    HostRequestError, PendingHostRequest, RemoteServerManager, RemoteServerManagerEvent,
    RipgrepSearchParams,
};
use crate::HostId;
use crate::proto::{ClientMessage, RemoteAgentContextSnapshot, WriteFile, host_scoped_request};
use crate::protocol::RequestId;

#[test]
fn abort_host_request_removes_pending_request_and_resolves_caller() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let host_id = HostId::new("test-host".to_string());
        let request_id = RequestId::new();
        let (result_tx, result_rx) = oneshot::channel();
        let msg = ClientMessage::host_scoped(
            request_id.to_string(),
            host_scoped_request::Message::WriteFile(WriteFile {
                path: "/tmp/test".to_string(),
                content: String::new(),
            }),
        );

        manager.update(&mut app, |manager, _ctx| {
            manager.pending_host_requests.insert(
                request_id.clone(),
                PendingHostRequest {
                    host_id,
                    dispatched_session_id: SessionId::from(1),
                    msg,
                    result_tx,
                    timeout_abort: None,
                },
            );
            manager.abort_host_request(&request_id);
            assert!(!manager.pending_host_requests.contains_key(&request_id));
        });

        assert!(matches!(
            result_rx.await.expect("manager should resolve caller"),
            Err(HostRequestError::Aborted)
        ));
    });
}

#[test]
fn remote_agent_context_snapshot_is_a_host_scoped_manager_event() {
    let host_id = HostId::new("test-host".to_string());
    let event = RemoteServerManagerEvent::RemoteAgentContextSnapshot {
        host_id,
        snapshot: RemoteAgentContextSnapshot {
            revision: 1,
            home_dir: "/home/user".to_string(),
            skills: Vec::new(),
            global_rules: Vec::new(),
        },
    };
    assert!(event.session_id().is_none());
}

#[test]
fn remote_agent_context_snapshot_revisions_are_deduplicated_per_host() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let host_id = HostId::new("test-host".to_string());
        let other_host_id = HostId::new("other-host".to_string());

        manager.update(&mut app, |manager, ctx| {
            assert!(manager.accept_remote_agent_context_snapshot_revision(&host_id, 2));
            assert!(!manager.accept_remote_agent_context_snapshot_revision(&host_id, 2));
            assert!(!manager.accept_remote_agent_context_snapshot_revision(&host_id, 1));
            assert!(manager.accept_remote_agent_context_snapshot_revision(&host_id, 3));
            assert!(manager.accept_remote_agent_context_snapshot_revision(&other_host_id, 1));

            manager.handle_host_disconnected(&host_id, ctx);
            assert!(manager.accept_remote_agent_context_snapshot_revision(&host_id, 3));
        });
    });
}

#[test]
fn start_ripgrep_search_without_connected_host_resolves_immediately() {
    App::test((), |mut app| async move {
        let manager = app.add_model(RemoteServerManager::new);
        let host_id = HostId::new("missing-host".to_string());
        let pending = manager.update(&mut app, |manager, _ctx| {
            manager.start_ripgrep_search(
                &host_id,
                RipgrepSearchParams {
                    pattern: "needle".to_string(),
                    roots: vec![StandardizedPath::try_new("/repo").unwrap()],
                    ignore_case: false,
                    multiline: false,
                    max_matches: 100,
                },
            )
        });

        assert!(matches!(
            pending.result().await,
            Err(HostRequestError::AllSessionsDisconnected)
        ));
    });
}
