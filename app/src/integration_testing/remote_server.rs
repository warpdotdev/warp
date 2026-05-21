use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

use warp_core::{HostId, SessionId};
use warpui::{
    async_assert, async_assert_eq,
    integration::{
        AssertionCallback, AssertionOutcome, AssertionWithDataCallback, StepDataMap, TestStep,
    },
    App, SingletonEntity, WindowId,
};

use crate::{
    integration_testing::view_getters::single_terminal_view_for_tab,
    remote_server::manager::{
        RemoteServerErrorKind, RemoteServerManager, RemoteServerManagerEvent,
        RemoteServerOperation, RemoteSessionState,
    },
    terminal::model::session::command_executor::remote_server_executor::RemoteServerCommandExecutor,
};
pub type RemoteServerActionCallback = Box<dyn Fn(&mut App, WindowId, &mut StepDataMap) + 'static>;

type RemoteServerNavigationPaths = Rc<RefCell<HashMap<SessionId, String>>>;
type RemoteServerLazyLoadEvents = Rc<RefCell<LazyLoadEvents>>;

const REMOTE_SERVER_NAVIGATION_PATHS_KEY: &str = "remote_server_navigation_paths";
const REMOTE_SERVER_LAZY_LOAD_EVENTS_KEY: &str = "remote_server_lazy_load_events";

#[derive(Default)]
struct LazyLoadEvents {
    loaded_host_ids: HashSet<HostId>,
    failures_by_session: HashMap<SessionId, Vec<RemoteServerErrorKind>>,
}

/// Returns a `TestStep` that records `NavigatedToDirectory` events emitted by
/// `RemoteServerManager` into this integration test's step data.
pub fn record_remote_server_navigation_events() -> TestStep {
    TestStep::new("Record remote server navigation events").with_action(
        |app, _window_id, step_data| {
            let navigated_paths: RemoteServerNavigationPaths =
                Rc::new(RefCell::new(HashMap::new()));
            step_data.insert(
                REMOTE_SERVER_NAVIGATION_PATHS_KEY,
                Rc::clone(&navigated_paths),
            );

            app.update(|ctx| {
                let mgr = RemoteServerManager::handle(ctx);
                ctx.subscribe_to_model(&mgr, move |_mgr, event, _ctx| {
                    if let RemoteServerManagerEvent::NavigatedToDirectory {
                        session_id,
                        remote_path,
                        ..
                    } = event
                    {
                        navigated_paths
                            .borrow_mut()
                            .insert(*session_id, remote_path.path.as_str().to_string());
                    }
                });
            });
        },
    )
}

/// Returns a `TestStep` that records `LoadRepoMetadataDirectory` success and
/// failure events emitted by `RemoteServerManager` into this integration test's
/// step data.
pub fn record_remote_server_lazy_load_events() -> TestStep {
    TestStep::new("Record remote server lazy-load events").with_action(
        |app, _window_id, step_data| {
            let lazy_load_events: RemoteServerLazyLoadEvents =
                Rc::new(RefCell::new(LazyLoadEvents::default()));
            step_data.insert(
                REMOTE_SERVER_LAZY_LOAD_EVENTS_KEY,
                Rc::clone(&lazy_load_events),
            );

            app.update(|ctx| {
                let mgr = RemoteServerManager::handle(ctx);
                ctx.subscribe_to_model(&mgr, move |_mgr, event, _ctx| match event {
                    RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { host_id, .. } => {
                        lazy_load_events
                            .borrow_mut()
                            .loaded_host_ids
                            .insert(host_id.clone());
                    }
                    RemoteServerManagerEvent::ClientRequestFailed {
                        session_id,
                        operation: RemoteServerOperation::LoadRepoMetadataDirectory,
                        error_kind,
                    } => {
                        lazy_load_events
                            .borrow_mut()
                            .failures_by_session
                            .entry(*session_id)
                            .or_default()
                            .push(*error_kind);
                    }
                    _ => {}
                });
            });
        },
    )
}
/// Returns a `TestStep` that polls until the remote server setup state for
/// the active session reaches `Ready`. Times out after 60 seconds to allow
/// for the full check → install → connect → handshake flow.
pub fn wait_for_remote_server_ready(tab_idx: usize) -> TestStep {
    TestStep::new("Wait for remote server setup to be Ready")
        .set_timeout(Duration::from_secs(60))
        .add_assertion(assert_remote_server_setup_ready(tab_idx))
}

/// Asserts that `Sessions::remote_server_setup_state` for the active session
/// is `RemoteServerSetupState::Ready`.
fn assert_remote_server_setup_ready(tab_idx: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
        terminal_view.read(app, |view, ctx| {
            let session_id = view
                .model
                .lock()
                .pending_session_id()
                .or_else(|| view.active_block_session_id());
            let Some(session_id) = session_id else {
                return AssertionOutcome::failure("No pending or active session ID yet".into());
            };
            let sessions = view.sessions(ctx);
            let Some(state) = sessions.remote_server_setup_state(session_id) else {
                return AssertionOutcome::failure(format!(
                    "No remote server setup state for session {session_id:?} yet"
                ));
            };
            async_assert!(
                state.is_ready(),
                "Expected RemoteServerSetupState::Ready, got {state:?}"
            )
        })
    })
}

/// Asserts that the `LoadRepoMetadataDirectory` request emitted a successful
/// `RepoMetadataDirectoryLoaded` event and did not emit `ClientRequestFailed`
/// for the active session.
pub fn assert_remote_server_loaded_repo_metadata_directory(
    tab_idx: usize,
) -> AssertionWithDataCallback {
    Box::new(move |app, window_id, step_data| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
        terminal_view.read(app, |view, ctx| {
            let Some(session_id) = view.active_block_session_id() else {
                return AssertionOutcome::PreconditionFailed("No active session ID".into());
            };
            let mgr = RemoteServerManager::as_ref(ctx);
            let session_state = mgr.session(session_id);
            let Some(RemoteSessionState::Connected { host_id, .. }) = session_state else {
                return AssertionOutcome::failure(format!(
                    "Expected RemoteSessionState::Connected, got {session_state:?}"
                ));
            };
            let Some(lazy_load_events) =
                step_data.get::<_, RemoteServerLazyLoadEvents>(REMOTE_SERVER_LAZY_LOAD_EVENTS_KEY)
            else {
                return AssertionOutcome::failure(
                    "No remote server lazy-load event recorder installed".into(),
                );
            };
            let lazy_load_events = lazy_load_events.borrow();
            if let Some(failures) = lazy_load_events.failures_by_session.get(&session_id) {
                return AssertionOutcome::failure(format!(
                    "LoadRepoMetadataDirectory failed for session {session_id:?}: {failures:?}"
                ));
            }
            async_assert!(
                lazy_load_events.loaded_host_ids.contains(host_id),
                "No RepoMetadataDirectoryLoaded event recorded for LoadRepoMetadataDirectory"
            )
        })
    })
}

/// Asserts that `RemoteServerManager` has the active session in `Connected`
/// state (i.e., the initialize handshake succeeded and a `HostId` is present).
pub fn assert_remote_server_connected(tab_idx: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
        terminal_view.read(app, |view, ctx| {
            let Some(session_id) = view.active_block_session_id() else {
                return AssertionOutcome::PreconditionFailed("No active session ID".into());
            };
            let mgr = RemoteServerManager::as_ref(ctx);
            let Some(session_state) = mgr.session(session_id) else {
                return AssertionOutcome::failure(format!(
                    "RemoteServerManager has no session for {session_id:?}"
                ));
            };
            async_assert!(
                matches!(session_state, RemoteSessionState::Connected { .. }),
                "Expected RemoteSessionState::Connected, got {session_state:?}"
            )
        })
    })
}

/// Asserts that the active session's `CommandExecutor` is a
/// `RemoteServerCommandExecutor` (not the fallback ControlMaster-based
/// `RemoteCommandExecutor`).
pub fn assert_command_executor_is_remote_server(tab_idx: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
        terminal_view.read(app, |view, ctx| {
            let Some(session_id) = view.active_block_session_id() else {
                return AssertionOutcome::PreconditionFailed("No active session ID".into());
            };
            let Some(session) = view.sessions(ctx).get(session_id) else {
                return AssertionOutcome::PreconditionFailed("Session not found".into());
            };
            let executor = session.command_executor();
            let is_remote_server = executor
                .as_any()
                .downcast_ref::<RemoteServerCommandExecutor>()
                .is_some();
            async_assert!(
                is_remote_server,
                "Expected RemoteServerCommandExecutor, got {:?}",
                std::any::type_name_of_val(&*executor)
            )
        })
    })
}

/// Returns a `TestStep` action that writes a file on the remote host via
/// the `RemoteServerClient::write_file` proto API. The write is dispatched
/// on a background thread using `tokio::runtime::Runtime::block_on` since
/// the action callback is synchronous.
pub fn write_file_via_remote_server(
    tab_idx: usize,
    path: String,
    content: String,
) -> RemoteServerActionCallback {
    Box::new(move |app, window_id, _| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
        let maybe_client = terminal_view.read(app, |view, ctx| {
            let session_id = view.active_block_session_id()?;
            RemoteServerManager::as_ref(ctx)
                .client_for_session(session_id)
                .cloned()
        });

        if let Some(client) = maybe_client {
            let path = path.clone();
            let content = content.clone();
            // Spawn on a background thread because the action callback is sync
            // but write_file is async.
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                let result = rt.block_on(client.write_file(path.clone(), content));
                if let Err(e) = &result {
                    log::error!("write_file_via_remote_server failed for {path}: {e}");
                }
            });
        } else {
            log::error!("write_file_via_remote_server: no connected client");
        }
    })
}

/// Returns a `TestStep` action that calls
/// `RemoteServerManager::load_remote_repo_metadata_directory` for the active
/// session. This triggers the lazy-loading proto request.
pub fn load_repo_metadata_directory_via_remote_server(
    tab_idx: usize,
    repo_path: String,
    dir_path: String,
) -> RemoteServerActionCallback {
    Box::new(move |app, window_id, _| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
        let maybe_session_id = terminal_view.read(app, |view, _ctx| view.active_block_session_id());

        let Some(session_id) = maybe_session_id else {
            log::error!("load_repo_metadata_directory_via_remote_server: no active session");
            return;
        };

        let repo_path = repo_path.clone();
        let dir_path = dir_path.clone();
        RemoteServerManager::handle(app).update(app, |mgr, ctx| {
            mgr.load_remote_repo_metadata_directory(session_id, repo_path, dir_path, ctx);
        });
    })
}

/// Asserts that `RemoteServerManager` has a successful navigation response
/// recorded for the active session and that it matches `expected_path`.
pub fn assert_remote_server_has_navigated(
    tab_idx: usize,
    expected_path: impl Into<String>,
) -> AssertionWithDataCallback {
    let expected_path = expected_path.into();
    Box::new(move |app, window_id, step_data| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, tab_idx);
        terminal_view.read(app, |view, ctx| {
            let Some(session_id) = view.active_block_session_id() else {
                return AssertionOutcome::PreconditionFailed("No active session ID".into());
            };
            let mgr = RemoteServerManager::as_ref(ctx);
            let session_state = mgr.session(session_id);
            if !matches!(session_state, Some(RemoteSessionState::Connected { .. })) {
                return AssertionOutcome::failure(format!(
                    "Expected RemoteSessionState::Connected, got {session_state:?}"
                ));
            }
            let Some(navigated_paths) =
                step_data.get::<_, RemoteServerNavigationPaths>(REMOTE_SERVER_NAVIGATION_PATHS_KEY)
            else {
                return AssertionOutcome::failure(
                    "No remote server navigation event recorder installed".into(),
                );
            };
            let Some(navigated_path) = navigated_paths.borrow().get(&session_id).cloned() else {
                return AssertionOutcome::failure(
                    "No successful navigation path recorded for session".into(),
                );
            };
            async_assert_eq!(
                navigated_path.as_str(),
                expected_path.as_str(),
                "Expected remote server to navigate session {session_id:?} to {expected_path}"
            )
        })
    })
}
