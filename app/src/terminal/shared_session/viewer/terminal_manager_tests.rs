//! Regression tests for the viewer `TerminalManager`'s `on_view_detached`
//! discriminator and the OVM-teardown helper.
//!
//! Before the fix, closing a viewer pane (tab close / split-pane close) did
//! not flow through any of the network-event paths
//! (`SessionEnded` / `ViewerRemoved` / `FailedToReconnect`), so the
//! orchestration viewer model — and its viewer-mode registration on the
//! shared [`OrchestrationEventStreamer`] — leaked until the app exited.
//! `TerminalManager::on_view_detached` now tears down the OVM on
//! `DetachType::Closed`, while deliberately preserving it on
//! `HiddenForClose` (undo-close grace window) and `Moved`.

use async_broadcast::broadcast;
use warpui::App;

use super::*;
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::QueuedQueryModel;
// Bring the `TerminalManager` trait into scope (named under a different alias
// since the local `TerminalManager` struct shadows it) so the trait method
// `on_view_detached` is callable on the struct.
use crate::terminal::TerminalManager as _;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::workspace::ToastStack;

/// Stub UUID used for the orchestrator's `AmbientAgentTaskId`; opaque to
/// the manager.
const PARENT_TASK_ID: &str = "11111111-1111-1111-1111-111111111111";

fn task_id(s: &str) -> AmbientAgentTaskId {
    s.parse().expect("hardcoded task id parses")
}

/// Constructs a viewer `TerminalManager` whose `orchestration_viewer_model`
/// slot is populated with a real OVM registered against the
/// [`OrchestrationEventStreamer`]. The returned `parent_task_id` is the one
/// used to register the OVM, so callers can look it up via
/// [`OrchestrationEventStreamer::viewer_mode_consumer_count_for_test`].
///
/// Deliberately bypasses `TerminalManager::new_internal` / `new_deferred`
/// (which would create a whole ambient-agent view stack with a real
/// `TerminalView::new` instead of `TerminalView::new_for_test`); the
/// `on_view_detached` path only depends on a small subset of the manager's
/// fields, so a struct-literal construction keeps the test focused.
fn build_manager_with_registered_ovm(app: &mut App) -> (TerminalManager, AmbientAgentTaskId) {
    let parent = task_id(PARENT_TASK_ID);

    let terminal_view = add_window_with_terminal(app, None);
    let terminal_view_id = terminal_view.id();

    // Set up the orchestrator placeholder conversation in the shape the
    // viewer model requires (is_viewing_shared_session == true, no parent
    // conversation id, marked active for the view).
    let history = BlocklistAIHistoryModel::handle(app);
    history.update(app, |history, ctx| {
        let id = history.start_new_conversation(terminal_view_id, false, true, false, ctx);
        history.set_viewing_shared_session_for_conversation(id, true);
        history.set_active_conversation_id(id, terminal_view_id, ctx);
    });

    // The OVM registers with the streamer on construction (streamer flag
    // is expected to be ON in the calling test).
    let ovm_handle = app.add_model(|ctx| {
        OrchestrationViewerModel::new(parent, terminal_view_id, terminal_view.downgrade(), ctx)
    });

    // Build the minimal field values the `TerminalManager` struct needs.
    // The network-side fields are left in their `Idle` / `None` defaults
    // so `on_view_detached` short-circuits the live-session teardown
    // branches and only the OVM-teardown branch is exercised.
    let (wakeups_tx, _wakeups_rx) = async_channel::unbounded();
    let (events_tx, events_rx) = async_channel::unbounded();
    let (pty_reads_tx, pty_reads_rx) = broadcast(8);
    let inactive_pty_reads_rx = pty_reads_rx.deactivate();
    let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);

    let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let model_events =
        app.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));
    let prompt_type =
        app.add_model(|_| PromptType::new_static(vec![], false, WarpPromptSeparator::None));

    let manager = TerminalManager {
        model,
        view: terminal_view,
        _model_events: model_events,
        _inactive_pty_reads_rx: inactive_pty_reads_rx,
        network_state: NetworkState::Idle,
        network_resources: NetworkResources {
            prompt_type,
            channel_event_proxy,
        },
        current_network: Arc::new(FairMutex::new(None)),
        viewer_remote_update_guard: RemoteUpdateGuard::new(),
        outbound_handlers_registered: false,
        orchestration_viewer_model: Arc::new(FairMutex::new(Some(ovm_handle))),
        enable_orchestration_polling: true,
    };
    (manager, parent)
}

#[test]
fn command_execution_request_failed_clears_queued_command_in_flight() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| ToastStack);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.id();
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });
        QueuedQueryModel::handle(&app).update(&mut app, |model, _ctx| {
            model.arm_command_in_flight(conversation_id);
        });

        terminal.update(&mut app, |view, ctx| {
            TerminalManager::handle_command_execution_request_failed(
                view,
                &CommandExecutionFailureReason::StaleBuffer,
                ctx,
            );
        });

        QueuedQueryModel::handle(&app).read(&app, |model, _ctx| {
            assert!(!model.has_command_in_flight(conversation_id));
        });
    });
}
#[test]
fn on_view_detached_closed_clears_orchestration_viewer_model_slot() {
    // Regression: closing a viewer pane must drop the OVM and release its
    // streamer registration so the ancestor SSE can be torn down.
    App::test((), |mut app| async move {
        let _streamer = FeatureFlag::OrchestrationViewerStreamer.override_enabled(true);

        initialize_app_for_terminal_view(&mut app);

        let (manager, parent) = build_manager_with_registered_ovm(&mut app);
        let slot = manager.orchestration_viewer_model.clone();

        // Sanity: OVM registered with the streamer.
        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                1,
                "pre-detach: OVM should have a viewer-mode registration on the streamer"
            );
        });
        assert!(
            slot.lock().is_some(),
            "pre-detach: OVM slot should be populated"
        );

        app.update(|ctx| manager.on_view_detached(DetachType::Closed, ctx));

        assert!(
            slot.lock().is_none(),
            "post-detach (Closed): OVM slot should be cleared"
        );
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                0,
                "post-detach (Closed): streamer's viewer-mode registration count should drop to 0"
            );
        });
    });
}

#[test]
fn on_view_detached_hidden_for_close_keeps_orchestration_viewer_model_alive() {
    // Negative case: HiddenForClose is part of the undo-close grace
    // window. OVM (and the ancestor SSE registration) must stay alive so
    // the pill bar restores seamlessly if the user undoes the close.
    App::test((), |mut app| async move {
        let _streamer = FeatureFlag::OrchestrationViewerStreamer.override_enabled(true);

        initialize_app_for_terminal_view(&mut app);

        let (manager, parent) = build_manager_with_registered_ovm(&mut app);
        let slot = manager.orchestration_viewer_model.clone();

        app.update(|ctx| manager.on_view_detached(DetachType::HiddenForClose, ctx));

        assert!(
            slot.lock().is_some(),
            "HiddenForClose must NOT clear the OVM slot (undo-close grace window)"
        );
        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                1,
                "HiddenForClose must NOT unregister from the streamer"
            );
        });
    });
}

#[test]
fn on_view_detached_moved_keeps_orchestration_viewer_model_alive() {
    // Negative case: Moved transfers the `TerminalManager` (and its OVM)
    // to a new pane group. Tearing down the OVM would orphan the pill
    // bar on the moved pane.
    App::test((), |mut app| async move {
        let _streamer = FeatureFlag::OrchestrationViewerStreamer.override_enabled(true);

        initialize_app_for_terminal_view(&mut app);

        let (manager, parent) = build_manager_with_registered_ovm(&mut app);
        let slot = manager.orchestration_viewer_model.clone();

        app.update(|ctx| manager.on_view_detached(DetachType::Moved, ctx));

        assert!(
            slot.lock().is_some(),
            "Moved must NOT clear the OVM slot (the manager is reused in the new pane group)"
        );
        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |me, _| {
            assert_eq!(
                me.viewer_mode_consumer_count_for_test(parent),
                1,
                "Moved must NOT unregister from the streamer"
            );
        });
    });
}

#[test]
fn handle_viewer_session_end_ignores_stale_ambient_end() {
    // A stale ambient end (the ended network is no longer the current one) must
    // be ignored: `handle_viewer_session_end` routes ambient panes through
    // `end_current_ambient_session`, whose current-network guard bails, so the
    // helper returns `false` and performs no teardown.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal_view = add_window_with_terminal(&mut app, None);
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

        let (wakeups_tx, _wakeups_rx) = async_channel::unbounded();
        let (events_tx, _events_rx) = async_channel::unbounded();
        let (pty_reads_tx, pty_reads_rx) = broadcast(8);
        let _inactive_pty_reads_rx = pty_reads_rx.deactivate();
        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);
        let (_write_to_pty_tx, write_to_pty_rx) = async_channel::unbounded();

        let ended_network = app.add_model(|ctx| {
            Network::new_for_test(
                channel_event_proxy,
                terminal_view.downgrade(),
                model.clone(),
                write_to_pty_rx,
                RemoteUpdateGuard::new(),
                ctx,
            )
        });

        // Empty `current_network` => the ended network is stale.
        let current_network = Arc::new(FairMutex::new(None));
        let orchestration_viewer_model = Arc::new(FairMutex::new(None));

        let mut handled = true;
        app.update(|ctx| {
            handled = TerminalManager::handle_viewer_session_end(
                &terminal_view,
                model.clone(),
                &current_network,
                &ended_network,
                &orchestration_viewer_model,
                /* is_ambient_agent */ true,
                ctx,
            );
        });

        assert!(
            !handled,
            "a stale ambient end (ended network != current) must be ignored"
        );
        assert!(
            !model.lock().shared_session_status().is_finished_viewer(),
            "an ignored stale ambient end must not finish the viewer"
        );
    });
}
