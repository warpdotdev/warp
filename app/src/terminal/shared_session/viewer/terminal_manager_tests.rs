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

use std::collections::HashSet;
use std::time::Duration;

use async_broadcast::broadcast;
use chrono::Local;
use session_sharing_protocol::common::{
    AgentPromptRequestId, ServerConversationToken as SessionSharingServerConversationToken,
};
use session_sharing_protocol::viewer::{DownstreamMessage, UpstreamMessage};
use warpui::App;

use super::*;
use crate::ai::agent::{AIAgentExchange, AIAgentExchangeId, AIAgentOutputStatus, ImageContext};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::{
    PendingAttachment, QueuedQuery, QueuedQueryModel, QueuedQueryOrigin, ResponseStream,
    ResponseStreamId,
};
use crate::ai::llms::LLMId;
use crate::pane_group::PaneConfigurationEvent;
// Bring the `TerminalManager` trait into scope (named under a different alias
// since the local `TerminalManager` struct shadows it) so the trait method
// `on_view_detached` is callable on the struct.
use crate::terminal::TerminalManager as _;
use crate::terminal::model::session::Sessions;
use crate::terminal::shared_session::viewer::network::Stage;
use crate::test_util::add_window_with_terminal;
use crate::test_util::shared_session_viewer::{
    AmbientTaskOwner, ViewerRole, ambient_viewer_pane, attach_network, drain_agent_prompts,
    exhaust_reconnect, flush, inject_downstream, reconnecting_stage, sent_agent_prompt,
    submit_viewer_prompt, subscribe_network_events, viewer_pane, viewer_pane_with_role,
};
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

    // The OVM registers with the streamer on construction.
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

/// Evaluates the two conditions that make
/// [`BlocklistAIStatusBar::render_warping_indicator_for_latest_exchange`] render `Warping...`
/// (`app/src/ai/blocklist/block/status_bar.rs:787-790`): an in-progress conversation, or an
/// agent-driven active block. The other terms in that gate can only suppress the indicator
/// further, so this disjunction is exactly what an undelivered prompt can wrongly leave true.
fn warping_gate_is_satisfied(
    terminal_view: &ViewHandle<TerminalView>,
    conversation_id: crate::ai::agent::conversation::AIConversationId,
    app: &App,
) -> bool {
    let conversation_in_progress = BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.status().is_in_progress())
    });
    let agent_drives_active_block = terminal_view.read(app, |view, _| {
        let model = view.model.lock();
        let active_block = model.block_list().active_block();
        active_block.is_agent_in_control() && !active_block.is_agent_blocked()
    });
    conversation_in_progress || agent_drives_active_block
}

/// Registers an in-flight response stream for `conversation_id`, standing in for a turn that is
/// genuinely still streaming when an unrelated prompt fails to send.
///
/// The stream has to be attached on both sides: registered with the controller *and* bound to a
/// streaming exchange on the conversation, because `has_active_stream_for_conversation` only
/// counts a stream the conversation reports it is processing.
fn register_active_stream(
    app: &mut App,
    terminal_view: &ViewHandle<TerminalView>,
    conversation_id: crate::ai::agent::conversation::AIConversationId,
) {
    terminal_view.update(app, |view, ctx| {
        let stream_id = ResponseStreamId::new_for_test();
        let terminal_view_id = view.view_id();
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history
                .conversation_mut(&conversation_id)
                .expect("the pane's conversation exists")
                .append_reassigned_exchange(&stream_id, streaming_exchange(), terminal_view_id, ctx)
                .expect("a streaming exchange appends");
        });
        let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
        view.ai_controller().clone().update(ctx, |controller, ctx| {
            controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
        });
    });
}

/// A minimal exchange in the streaming state, which is what marks its response stream in flight.
fn streaming_exchange() -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: Vec::new(),
        output_status: AIAgentOutputStatus::Streaming { output: None },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

#[test]
fn viewer_prompt_submitted_while_reconnecting_is_preserved_as_an_editable_queue_row() {
    // `SharedSessionStatus` reports `ActiveViewer` throughout a reconnect, so a prompt submitted
    // then is routed to a network that cannot carry it. It must survive as the user's own queue
    // row, with the editor released and no stale turn left advertised.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        let (terminal_view, conversation_id, network) = (
            pane.view.clone(),
            pane.conversation_id,
            pane.network.clone(),
        );
        let session_id = network.read(&app, |network, _| network.session_id());
        let server_conversation_token = BlocklistAIHistoryModel::handle(&app).read(&app, |h, _| {
            h.conversation(&conversation_id)
                .and_then(|c| c.server_conversation_token().cloned())
                .and_then(|t| {
                    t.as_str()
                        .parse()
                        .ok()
                        .map(SessionSharingServerConversationToken::from_uuid)
                })
        });

        submit_viewer_prompt(&mut app, &terminal_view, "finish the refactor");

        assert!(
            drain_agent_prompts(&app, &network).is_empty(),
            "a reconnecting network cannot carry the prompt"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(conversation_id);
            assert_eq!(
                queue.len(),
                1,
                "the undelivered prompt must be preserved as exactly one queue row"
            );
            let row = &queue[0];
            assert_eq!(row.text(), "finish the refactor");
            assert_eq!(row.origin(), QueuedQueryOrigin::DisconnectedViewer);
            assert!(!row.is_locked(), "the row must be editable and deletable");
            let target = row
                .shared_session_target()
                .expect("the row records where it should be retried");
            assert_eq!(
                target.session_id(),
                session_id,
                "the row must stay pinned to the session it was addressed to"
            );
            assert_eq!(
                target.server_conversation_token(),
                server_conversation_token,
                "and to the server conversation it was continuing, so a rejoin cannot redirect it"
            );
        });

        // With the prompt out of flight and nothing else running, the conversation must stop
        // advertising a turn, otherwise `Warping...` renders indefinitely.
        assert!(
            !warping_gate_is_satisfied(&terminal_view, conversation_id, &app),
            "an undelivered prompt must not leave the Warping... gate satisfied"
        );

        // Freezing renders the prompt with a trailing loading marker, so the marker's absence is
        // what shows the editor is back under the user's control rather than waiting on a reply.
        let input = terminal_view.read(&app, |view, _| view.input().clone());
        let buffer = input.read(&app, |input, ctx| input.buffer_text(ctx));
        assert!(
            !buffer.contains('\u{25cc}'),
            "the input must not still be frozen in its loading state, but was {buffer:?}"
        );
    });
}

#[test]
fn a_conversation_advertises_a_turn_before_the_prompt_is_even_submitted() {
    // A conversation reports `InProgress` from creation, so the status alone says nothing about
    // whether work is running. Pinning that here keeps the reproduction honest: without it, that
    // test could pass merely because the gate was never armed in the first place.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        let (terminal_view, conversation_id) = (pane.view.clone(), pane.conversation_id);

        assert!(
            warping_gate_is_satisfied(&terminal_view, conversation_id, &app),
            "the Warping... gate is expected to be armed before submission"
        );
    });
}

#[test]
fn an_undelivered_prompt_leaves_warping_alone_while_a_stream_is_still_running() {
    // The dangerous over-correction: a prompt that fails to send must not silence the indicator
    // for a turn that is genuinely still streaming. The fallback row is still filed either way.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        let (terminal_view, conversation_id) = (pane.view.clone(), pane.conversation_id);
        register_active_stream(&mut app, &terminal_view, conversation_id);

        submit_viewer_prompt(&mut app, &terminal_view, "another thought");

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(
                queue_model.queue(conversation_id).len(),
                1,
                "the undelivered prompt is still preserved as a queue row"
            );
        });
        assert!(
            warping_gate_is_satisfied(&terminal_view, conversation_id, &app),
            "a genuinely streaming turn must keep its Warping... indicator"
        );
    });
}

#[test]
fn viewer_prompt_delivered_to_a_joined_session_leaves_no_queue_row() {
    // The happy path must be untouched: a prompt the sharer acknowledges belongs to the sharer,
    // and no fallback row should linger in the panel.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, Stage::JoinedSuccessfully);
        let (terminal_view, conversation_id, network) = (
            pane.view.clone(),
            pane.conversation_id,
            pane.network.clone(),
        );

        submit_viewer_prompt(&mut app, &terminal_view, "ship it");

        let request = sent_agent_prompt(&app, &network);
        assert_eq!(request.prompt, "ship it");
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(
                queue_model.queue(conversation_id).is_empty(),
                "a locally accepted prompt must not produce a visible fallback row"
            );
        });

        terminal_view.update(&mut app, |view, ctx| {
            assert!(
                view.on_viewer_prompt_acknowledged(&request.id, ctx),
                "the pane's own request id must resolve"
            );
        });
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(queue_model.queue(conversation_id).is_empty());
        });
    });
}

#[test]
fn an_unrelated_or_duplicate_acknowledgement_resolves_nothing() {
    // Matching by request id is what makes a late echo for a retired revision, or a duplicate of
    // one already handled, incapable of resolving a prompt twice.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, Stage::JoinedSuccessfully);
        let (terminal_view, network) = (pane.view.clone(), pane.network.clone());

        submit_viewer_prompt(&mut app, &terminal_view, "ship it");
        let request = sent_agent_prompt(&app, &network);

        terminal_view.update(&mut app, |view, ctx| {
            assert!(
                !view.on_viewer_prompt_acknowledged(&AgentPromptRequestId::new(), ctx),
                "an acknowledgement for a request this pane never sent must be a no-op"
            );
            assert!(view.on_viewer_prompt_acknowledged(&request.id, ctx));
            assert!(
                !view.on_viewer_prompt_acknowledged(&request.id, ctx),
                "a duplicate acknowledgement must be a no-op"
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

/// A `RejoinedSuccessfully` message, which the server sends when a reconnect completes.
fn rejoined_message() -> DownstreamMessage {
    DownstreamMessage::RejoinedSuccessfully {
        participant_list: Default::default(),
    }
}

/// Waits past the acknowledgement deadline. The timeout is shortened under `cfg!(test)`
/// (see `VIEWER_PROMPT_ACK_TIMEOUT`), so this does not sleep for the production duration.
async fn wait_past_ack_timeout() {
    warpui::r#async::Timer::after(Duration::from_millis(120)).await;
}

#[test]
fn a_prompt_the_sharer_never_acknowledges_falls_back_to_the_queue() {
    // `try_send` only reaches the local proxy channel. If the sharer never acknowledges, the
    // prompt is just as lost as a rejected one, and the input would stay frozen forever.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, Stage::JoinedSuccessfully);

        submit_viewer_prompt(&mut app, &pane.view, "no reply expected");
        // Accepted locally, so nothing is queued yet: the client is still waiting for the ack.
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(queue_model.queue(pane.conversation_id).is_empty());
        });

        wait_past_ack_timeout().await;
        flush(&mut app);

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(pane.conversation_id);
            assert_eq!(
                queue.len(),
                1,
                "the unacknowledged prompt must be recovered"
            );
            assert_eq!(queue[0].text(), "no reply expected");
            assert_eq!(queue[0].origin(), QueuedQueryOrigin::DisconnectedViewer);
        });
    });
}

#[test]
fn an_acknowledgement_arriving_after_the_timeout_removes_the_row_instead_of_duplicating_it() {
    // The double-submit case: the client gave up and made the prompt visible again, then the
    // sharer's acknowledgement turns up late. The sharer *did* get it, so the row must be
    // retired rather than left queued for a second delivery.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, Stage::JoinedSuccessfully);

        submit_viewer_prompt(&mut app, &pane.view, "slow ack");
        let request = sent_agent_prompt(&app, &pane.network);

        wait_past_ack_timeout().await;
        flush(&mut app);
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(queue_model.queue(pane.conversation_id).len(), 1);
        });

        inject_downstream(
            &mut app,
            &pane.network,
            DownstreamMessage::AgentPromptRequestInFlight(request.id.clone()),
        );

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(
                queue_model.queue(pane.conversation_id).is_empty(),
                "a late acknowledgement must retire the row, not leave it to be sent twice"
            );
        });
    });
}

#[test]
fn a_rejoin_refires_only_the_head_and_only_once() {
    // Two rows are waiting when the session comes back. Exactly one prompt may go out: the head.
    // Re-delivering the rejoin must not send it again, nor promote the row behind it.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());

        submit_viewer_prompt(&mut app, &pane.view, "first");
        submit_viewer_prompt(&mut app, &pane.view, "second");
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(queue_model.queue(pane.conversation_id).len(), 2);
        });
        drain_agent_prompts(&app, &pane.network);

        inject_downstream(&mut app, &pane.network, rejoined_message());

        let sent = drain_agent_prompts(&app, &pane.network);
        assert_eq!(sent.len(), 1, "a rejoin refires exactly the FIFO head");
        assert_eq!(sent[0].prompt, "first");
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(pane.conversation_id);
            assert_eq!(queue.len(), 1, "the row behind the head stays queued");
            assert_eq!(queue[0].text(), "second");
        });

        // A duplicate rejoin must not resend the head that is already in flight.
        inject_downstream(&mut app, &pane.network, rejoined_message());
        assert!(
            drain_agent_prompts(&app, &pane.network).is_empty(),
            "a repeated rejoin must not send anything further"
        );
    });
}

#[test]
fn a_rejoin_on_a_replaced_network_sends_nothing() {
    // Ordering hazard: the old network can still emit a rejoin after the pane has swapped to a
    // replacement. Acting on it would push the prompt into a session the user never addressed.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        submit_viewer_prompt(&mut app, &pane.view, "queued for the old session");
        drain_agent_prompts(&app, &pane.network);

        // The pane moves on to a different network, as a fatal disconnect would cause.
        let replacement = attach_network(&mut app, &pane.view, Stage::JoinedSuccessfully);
        pane.set_current_network(Some(replacement.clone()));

        inject_downstream(&mut app, &pane.network, rejoined_message());

        assert!(
            drain_agent_prompts(&app, &pane.network).is_empty(),
            "the replaced network must not carry the prompt"
        );
        assert!(
            drain_agent_prompts(&app, &replacement).is_empty(),
            "a stale rejoin must not redirect the prompt into the replacement session"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(
                queue_model.queue(pane.conversation_id).len(),
                1,
                "the row stays queued rather than being dispatched to the wrong session"
            );
        });
    });
}

#[test]
fn a_rejoin_into_a_different_session_leaves_the_row_queued() {
    // The row records the session it was addressed to. A rejoin belonging to some other session
    // is not the one it was waiting for, so the head must stay put.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        submit_viewer_prompt(&mut app, &pane.view, "addressed elsewhere");
        drain_agent_prompts(&app, &pane.network);

        // Re-point the live slot at a network with a different session id, then rejoin it.
        let other_session = attach_network(&mut app, &pane.view, reconnecting_stage());
        pane.set_current_network(Some(other_session.clone()));
        subscribe_network_events(
            &mut app,
            &pane.view,
            &pane.model,
            &pane.current_network,
            &other_session,
        );

        inject_downstream(&mut app, &other_session, rejoined_message());

        assert!(
            drain_agent_prompts(&app, &other_session).is_empty(),
            "a row pinned to another session must not be sent into this one"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(queue_model.queue(pane.conversation_id).len(), 1);
        });
    });
}

#[test]
fn racing_rejoins_accept_each_prompt_at_most_once_and_in_order() {
    // Ordering across repeated drain triggers: three rows, several rejoins. Every prompt that
    // goes out must be distinct and strictly FIFO, which is what the per-row claim guarantees.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());
        for prompt in ["one", "two", "three"] {
            submit_viewer_prompt(&mut app, &pane.view, prompt);
        }
        drain_agent_prompts(&app, &pane.network);

        let mut accepted = Vec::new();
        for _ in 0..3 {
            // Put the network back into a reconnect so the next rejoin is meaningful, then
            // acknowledge whatever went out so the queue can advance.
            pane.network.update(&mut app, |network, _| {
                network.stage = reconnecting_stage();
            });
            inject_downstream(&mut app, &pane.network, rejoined_message());
            for request in drain_agent_prompts(&app, &pane.network) {
                accepted.push(request.prompt.clone());
                inject_downstream(
                    &mut app,
                    &pane.network,
                    DownstreamMessage::AgentPromptRequestInFlight(request.id),
                );
            }
        }

        assert_eq!(
            accepted,
            vec!["one", "two", "three"],
            "prompts must be accepted strictly in FIFO order, each exactly once"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(queue_model.queue(pane.conversation_id).is_empty());
        });
    });
}

#[test]
fn with_the_queue_surface_disabled_the_prompt_returns_to_the_input_rather_than_a_hidden_row() {
    // Feature-off behavior: with no queue panel to show it, filing a row would hide the prompt
    // somewhere the user cannot reach. It goes back to the editor instead.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(false);
        let pane = viewer_pane(&mut app, reconnecting_stage());

        submit_viewer_prompt(&mut app, &pane.view, "nowhere to queue this");

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(
                queue_model.queue(pane.conversation_id).is_empty(),
                "no invisible row may be created when the queue surface is off"
            );
        });
        let input = pane.view.read(&app, |view, _| view.input().clone());
        assert_eq!(
            input.read(&app, |input, ctx| input.buffer_text(ctx)),
            "nowhere to queue this",
            "the prompt must be restored to the input so it is not silently lost"
        );
    });
}

#[test]
fn a_read_only_viewer_never_reaches_the_fallback_queue() {
    // A reader cannot submit prompts at all, so an undeliverable send should never arise for one
    // and no disconnected-viewer row may appear on their behalf.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane_with_role(&mut app, reconnecting_stage(), ViewerRole::Reader);

        submit_viewer_prompt(&mut app, &pane.view, "not allowed");

        assert!(
            drain_agent_prompts(&app, &pane.network).is_empty(),
            "a reader's prompt must not be sent"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(
                queue_model.queue(pane.conversation_id).is_empty(),
                "a reader must not accumulate disconnected-viewer rows"
            );
        });
    });
}

#[test]
fn a_fatal_disconnect_hands_the_head_to_a_cloud_follow_up() {
    // Criterion 10. Once the execution is gone the old session cannot carry anything, so the head
    // has to become the follow-up that starts a replacement run. Without this the queue deadlocks:
    // the rows behind it only drain once a new execution joins, and nothing would request one.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let ambient = ambient_viewer_pane(
            &mut app,
            reconnecting_stage(),
            AmbientTaskOwner::CurrentUser,
        );
        let pane = &ambient.pane;

        submit_viewer_prompt(&mut app, &pane.view, "first");
        submit_viewer_prompt(&mut app, &pane.view, "second");
        drain_agent_prompts(&app, &pane.network);

        exhaust_reconnect(&mut app, pane);

        let pending_followup = pane.view.read(&app, |view, ctx| {
            view.ambient_agent_view_model().and_then(|model| {
                model
                    .as_ref(ctx)
                    .pending_followup_prompt()
                    .map(str::to_owned)
            })
        });
        assert_eq!(
            pending_followup.as_deref(),
            Some("first"),
            "the FIFO head must become the cloud follow-up that starts the replacement run"
        );
        let followup_task = pane.view.read(&app, |view, ctx| {
            view.ambient_agent_view_model()
                .and_then(|model| model.as_ref(ctx).task_id())
        });
        assert_eq!(
            followup_task,
            Some(ambient.task_id),
            "the follow-up must continue the task the pane was viewing, not some other run"
        );
        assert!(
            drain_agent_prompts(&app, &pane.network).is_empty(),
            "the ended session must receive nothing"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(pane.conversation_id);
            assert_eq!(
                queue.len(),
                1,
                "the remaining row must survive the teardown"
            );
            assert_eq!(queue[0].text(), "second");
        });
    });
}

#[test]
fn a_fatal_disconnect_on_someone_elses_task_keeps_the_whole_queue() {
    // Criterion 11. A viewer who does not own the task cannot start a replacement run, so the
    // ineligible path must leave every row exactly where it was rather than consuming the head.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let ambient = ambient_viewer_pane(
            &mut app,
            reconnecting_stage(),
            AmbientTaskOwner::SomeoneElse,
        );
        let pane = &ambient.pane;

        submit_viewer_prompt(&mut app, &pane.view, "first");
        submit_viewer_prompt(&mut app, &pane.view, "second");
        drain_agent_prompts(&app, &pane.network);

        exhaust_reconnect(&mut app, pane);

        let pending_followup = pane.view.read(&app, |view, ctx| {
            view.ambient_agent_view_model().and_then(|model| {
                model
                    .as_ref(ctx)
                    .pending_followup_prompt()
                    .map(str::to_owned)
            })
        });
        assert_eq!(
            pending_followup, None,
            "a non-owner must not be able to start a replacement run"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(pane.conversation_id);
            assert_eq!(queue.len(), 2, "the head must be restored, not consumed");
            assert_eq!(
                queue[0].text(),
                "first",
                "and restored at its original position"
            );
            assert_eq!(queue[1].text(), "second");
        });
    });
}

#[test]
fn a_fatal_disconnect_with_handoff_disabled_keeps_the_whole_queue() {
    // Criterion 11, second arm: the same ineligibility reached through the feature flag rather
    // than through ownership.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(false);
        let ambient = ambient_viewer_pane(
            &mut app,
            reconnecting_stage(),
            AmbientTaskOwner::CurrentUser,
        );
        let pane = &ambient.pane;

        submit_viewer_prompt(&mut app, &pane.view, "only");
        drain_agent_prompts(&app, &pane.network);

        exhaust_reconnect(&mut app, pane);

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(pane.conversation_id);
            assert_eq!(queue.len(), 1, "the row must remain queued and visible");
            assert_eq!(queue[0].text(), "only");
        });
    });
}

#[test]
fn the_replacement_session_continues_the_queue_only_once_it_has_joined() {
    // Criterion 12. `ExecutionSessionReady` fires before the replacement network connects, so a
    // send at that point would be dropped. Only the new session's join may drain the next row,
    // and re-delivering that join must not send it twice.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let ambient = ambient_viewer_pane(
            &mut app,
            reconnecting_stage(),
            AmbientTaskOwner::CurrentUser,
        );
        let pane = &ambient.pane;

        submit_viewer_prompt(&mut app, &pane.view, "first");
        submit_viewer_prompt(&mut app, &pane.view, "second");
        drain_agent_prompts(&app, &pane.network);
        exhaust_reconnect(&mut app, pane);

        // The replacement execution exists but has not connected yet.
        let replacement = attach_network(&mut app, &pane.view, Stage::BeforeJoined);
        pane.set_current_network(Some(replacement.clone()));
        subscribe_network_events(
            &mut app,
            &pane.view,
            &pane.model,
            &pane.current_network,
            &replacement,
        );
        flush(&mut app);
        assert!(
            drain_agent_prompts(&app, &replacement).is_empty(),
            "nothing may be sent before the replacement session has joined"
        );

        replacement.update(&mut app, |network, _| {
            network.stage = Stage::JoinedSuccessfully;
        });
        // Joining the replacement puts the pane back in an executor role. The fatal teardown had
        // moved an owned task to `NotShared`, and the real join path restores it via
        // `on_session_share_joined`.
        pane.model
            .lock()
            .set_shared_session_status(SharedSessionStatus::executor());
        pane.view.update(&mut app, |view, ctx| {
            let session_id = replacement.read(ctx, |network, _| network.session_id());
            view.drain_disconnected_viewer_queue_after_replacement_join(session_id, ctx);
        });
        flush(&mut app);

        let sent = drain_agent_prompts(&app, &replacement);
        assert_eq!(sent.len(), 1, "the join drains exactly one row");
        assert_eq!(sent[0].prompt, "second");

        // Re-delivering the join must not resend the row already in flight.
        pane.view.update(&mut app, |view, ctx| {
            let session_id = replacement.read(ctx, |network, _| network.session_id());
            view.drain_disconnected_viewer_queue_after_replacement_join(session_id, ctx);
        });
        flush(&mut app);
        assert!(
            drain_agent_prompts(&app, &replacement).is_empty(),
            "a repeated join must not produce a duplicate send"
        );
    });
}

#[test]
fn a_failed_attachment_upload_restores_the_whole_row() {
    // Criterion 15. A queued cloud follow-up uploads its attachments to the task before it
    // dispatches, because the replacement VM downloads them at startup. If that upload fails the
    // follow-up must not start, and the row must come back whole — text and attachments — rather
    // than being consumed with its attachments silently dropped.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _image_flag = FeatureFlag::CloudModeImageContext.override_enabled(true);
        let ambient = ambient_viewer_pane(
            &mut app,
            reconnecting_stage(),
            AmbientTaskOwner::CurrentUser,
        );
        let pane = &ambient.pane;

        // End the execution so the pane is in the follow-up-eligible state, then queue a row that
        // carries an attachment.
        exhaust_reconnect(&mut app, pane);
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(
                pane.conversation_id,
                QueuedQuery::new_with_attachments(
                    "look at this".to_owned(),
                    QueuedQueryOrigin::QueueSlashCommand,
                    vec![PendingAttachment::Image(ImageContext {
                        data: String::new(),
                        mime_type: "image/png".to_owned(),
                        file_name: "diagram.png".to_owned(),
                        is_figma: false,
                    })],
                ),
                ctx,
            );
        });

        let (query_id, text) = QueuedQueryModel::handle(&app).read(&app, |model, _| {
            let row = &model.queue(pane.conversation_id)[0];
            (row.id(), row.text().to_owned())
        });
        pane.view.update(&mut app, |view, ctx| {
            view.input().clone().update(ctx, |input, ctx| {
                input.submit_queued_prompt_for_active_pane(
                    text,
                    pane.conversation_id,
                    query_id,
                    ctx,
                );
            });
        });
        // The upload cannot succeed against the test server. Poll for the failure to land rather
        // than sleeping a fixed amount, so the test does not depend on how loaded the machine is.
        for _ in 0..200 {
            let settled = QueuedQueryModel::handle(&app).read(&app, |model, _| {
                !model.queue(pane.conversation_id).is_empty()
            });
            if settled {
                break;
            }
            warpui::r#async::Timer::after(Duration::from_millis(10)).await;
            flush(&mut app);
        }

        let pending_followup = pane.view.read(&app, |view, ctx| {
            view.ambient_agent_view_model().and_then(|model| {
                model
                    .as_ref(ctx)
                    .pending_followup_prompt()
                    .map(str::to_owned)
            })
        });
        assert_eq!(
            pending_followup, None,
            "a follow-up must not start when its attachments never reached the task"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            let queue = queue_model.queue(pane.conversation_id);
            assert_eq!(queue.len(), 1, "the row must be restored, not consumed");
            assert_eq!(queue[0].text(), "look at this");
            assert_eq!(
                queue[0].attachments().len(),
                1,
                "the attachment must survive with the row rather than being dropped"
            );
        });
    });
}

#[test]
fn a_rejoin_flushes_buffered_input_before_it_retries_the_prompt() {
    // Criterion 8, ordering half. The editor clear that accompanied the original submission is
    // buffered while disconnected. If the retried prompt overtook it, the sharer would apply the
    // clear after the new prompt and wipe it.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());

        submit_viewer_prompt(&mut app, &pane.view, "retry me");
        // Buffer an input update while disconnected, as the editor does during a reconnect.
        pane.network.update(&mut app, |network, _| {
            network.send_input_update(&Default::default(), std::iter::empty());
        });
        drain_agent_prompts(&app, &pane.network);

        inject_downstream(&mut app, &pane.network, rejoined_message());

        // Read the channel in order: every buffered input update must precede the prompt.
        let ws_proxy_rx = pane
            .network
            .read(&app, |network, _| network.ws_proxy_rx.clone());
        let mut saw_prompt = false;
        let mut input_after_prompt = false;
        while let Ok(message) = ws_proxy_rx.try_recv() {
            match message {
                UpstreamMessage::SendAgentPrompt(_) => saw_prompt = true,
                UpstreamMessage::UpdateInput(_) if saw_prompt => input_after_prompt = true,
                _ => {}
            }
        }
        assert!(saw_prompt, "the rejoin must retry the prompt");
        assert!(
            !input_after_prompt,
            "buffered input updates must be flushed before the retried prompt"
        );
    });
}

#[test]
fn every_drain_trigger_together_accepts_each_prompt_at_most_once() {
    // Criterion 13. The three entrypoints can fire in quick succession around one fatal
    // disconnect: the rejoin attempt, the fatal end, and the replacement join. Each prompt may be
    // accepted once and only once across all of them, which is what the per-row claim buys.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let ambient = ambient_viewer_pane(
            &mut app,
            reconnecting_stage(),
            AmbientTaskOwner::CurrentUser,
        );
        let pane = &ambient.pane;

        for prompt in ["one", "two", "three"] {
            submit_viewer_prompt(&mut app, &pane.view, prompt);
        }
        drain_agent_prompts(&app, &pane.network);

        let mut accepted: Vec<String> = Vec::new();

        // Trigger 1: the session briefly comes back and takes the head.
        inject_downstream(&mut app, &pane.network, rejoined_message());
        for request in drain_agent_prompts(&app, &pane.network) {
            accepted.push(request.prompt.clone());
            inject_downstream(
                &mut app,
                &pane.network,
                DownstreamMessage::AgentPromptRequestInFlight(request.id),
            );
        }

        // Trigger 2: it then dies for good, handing the next row to a cloud follow-up.
        pane.network.update(&mut app, |network, _| {
            network.stage = reconnecting_stage();
        });
        exhaust_reconnect(&mut app, pane);
        if let Some(followup) = pane.view.read(&app, |view, ctx| {
            view.ambient_agent_view_model().and_then(|model| {
                model
                    .as_ref(ctx)
                    .pending_followup_prompt()
                    .map(str::to_owned)
            })
        }) {
            accepted.push(followup);
        }

        // Trigger 3: the replacement joins and continues the queue.
        let replacement = attach_network(&mut app, &pane.view, Stage::JoinedSuccessfully);
        pane.set_current_network(Some(replacement.clone()));
        subscribe_network_events(
            &mut app,
            &pane.view,
            &pane.model,
            &pane.current_network,
            &replacement,
        );
        pane.model
            .lock()
            .set_shared_session_status(SharedSessionStatus::executor());
        pane.view.update(&mut app, |view, ctx| {
            let session_id = replacement.read(ctx, |network, _| network.session_id());
            view.drain_disconnected_viewer_queue_after_replacement_join(session_id, ctx);
        });
        flush(&mut app);
        for request in drain_agent_prompts(&app, &replacement) {
            accepted.push(request.prompt.clone());
        }

        assert_eq!(
            accepted,
            vec!["one", "two", "three"],
            "across all three drain triggers each prompt is accepted once, in FIFO order"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(
                queue_model.queue(pane.conversation_id).is_empty(),
                "nothing may be left behind once every trigger has run"
            );
        });
    });
}

#[test]
fn a_rejoin_after_the_conversation_was_removed_sends_nothing() {
    // Criterion 9, final arm. If the conversation the prompt belonged to is gone by the time the
    // session returns, there is nothing to retry into — the prompt must not be redirected to
    // whatever conversation happens to be active now.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let pane = viewer_pane(&mut app, reconnecting_stage());

        submit_viewer_prompt(&mut app, &pane.view, "orphaned");
        drain_agent_prompts(&app, &pane.network);
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(queue_model.queue(pane.conversation_id).len(), 1);
        });

        let terminal_view_id = pane.view.id();
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
            history.delete_conversation(pane.conversation_id, Some(terminal_view_id), ctx);
        });

        inject_downstream(&mut app, &pane.network, rejoined_message());

        assert!(
            drain_agent_prompts(&app, &pane.network).is_empty(),
            "a removed conversation leaves nothing to retry"
        );
        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(queue_model.queue(pane.conversation_id).is_empty());
        });
    });
}

#[test]
fn ending_ambient_session_refreshes_shared_session_link_surfaces() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(Manager::new);

        let terminal_view = add_window_with_terminal(&mut app, None);
        let model = terminal_view.read(&app, |view, _| view.model.clone());
        model
            .lock()
            .set_shared_session_status(SharedSessionStatus::ActiveViewer {
                role: Default::default(),
            });

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
        let ended_session_id = ended_network.read(&app, |network, _| network.session_id());
        Manager::handle(&app).update(&mut app, |manager, ctx| {
            manager.joined_share(terminal_view.downgrade(), ended_session_id, ctx);
        });

        let link_change_events = Arc::new(FairMutex::new(0));
        let link_change_events_for_subscription = link_change_events.clone();
        let pane_configuration =
            terminal_view.read(&app, |view, _| view.pane_configuration().clone());
        app.update(|ctx| {
            ctx.subscribe_to_model(&pane_configuration, move |_, event, _| {
                if matches!(event, PaneConfigurationEvent::SharedSessionLinkChanged) {
                    *link_change_events_for_subscription.lock() += 1;
                }
            });
        });

        let current_network = Arc::new(FairMutex::new(Some(ended_network.clone())));
        let handled = app.update(|ctx| {
            TerminalManager::end_current_ambient_session(
                &terminal_view,
                model.clone(),
                &current_network,
                &ended_network,
                ctx,
            )
        });

        assert!(handled);
        assert_eq!(*link_change_events.lock(), 1);
        terminal_view.read(&app, |view, ctx| {
            let status = view.model.lock().shared_session_status().clone();
            assert_eq!(
                Manager::as_ref(ctx).session_id_for_link(&view.id(), &status),
                None,
                "an ended id must not stay exposed while the status is still ActiveViewer"
            );
        });
    });
}
