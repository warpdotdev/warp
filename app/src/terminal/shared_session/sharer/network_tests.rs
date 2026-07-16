use std::sync::Arc;

use async_channel::Sender;
use futures_util::stream::AbortHandle;
use instant::Instant;
use parking_lot::FairMutex;
use session_sharing_protocol::common::{
    ActivePrompt, OrderedTerminalEvent, OrderedTerminalEventType, ParticipantId, Selection,
    SessionId,
};
use session_sharing_protocol::sharer::{
    DownstreamMessage, FailedToInitializeSessionReason, QuotaType, ReconnectToken, UpstreamMessage,
};
use warp_server_client::iap::IapManager;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};
use websocket::{Message, WebsocketMessage as _};

use super::{
    startup_max_attempts, Network, PtyBytesBatchStatus, Stage, StartupFailure, StartupRetryState,
    AMBIENT_CREATE_SESSION_MAX_ATTEMPTS, PTY_READS_BATCH_THRESHOLD,
};
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::editor::ReplicaId;
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::terminal::shared_session::{
    SharedSessionScrollbackType, SharedSessionSource, MAX_BYTES_SHAREABLE,
};
use crate::terminal::TerminalModel;
use crate::test_util::assert_eventually;

fn is_upstream_message_pty_bytes_read(
    message: UpstreamMessage,
    expected_event_no: usize,
    expected_bytes: Vec<u8>,
) -> bool {
    let compressed_bytes = lz4_flex::block::compress_prepend_size(&expected_bytes);
    matches!(message, UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
        event_no,
        event_type: OrderedTerminalEventType::PtyBytesRead { bytes },
    }) if event_no == expected_event_no && bytes == compressed_bytes)
}

#[test]
fn test_startup_max_attempts_only_retries_ambient_agent_sources() {
    assert_eq!(
        startup_max_attempts(&SharedSessionSource::ambient_agent(Some(
            "task-id".to_string()
        ))),
        AMBIENT_CREATE_SESSION_MAX_ATTEMPTS
    );
    assert_eq!(startup_max_attempts(&SharedSessionSource::user(None)), 1);
}

#[test]
fn test_startup_failure_retryability() {
    assert!(StartupFailure::Transport.is_retryable());
    assert!(StartupFailure::InitializeSend.is_retryable());
    assert!(StartupFailure::WebsocketClosedBeforeStarted.is_retryable());
    assert!(StartupFailure::WebsocketError.is_retryable());
    assert!(StartupFailure::Timeout.is_retryable());
    assert!(
        StartupFailure::ServerRejected(FailedToInitializeSessionReason::InternalServerError {
            details: "transient".to_string(),
        })
        .is_retryable()
    );

    assert!(!StartupFailure::ServerRejected(
        FailedToInitializeSessionReason::ScrollbackTooLarge {}
    )
    .is_retryable());
    assert!(!StartupFailure::ServerRejected(
        FailedToInitializeSessionReason::NoUserQuotaRemaining {
            quota_type: QuotaType::SessionsCreated,
        }
    )
    .is_retryable());
    assert!(
        !StartupFailure::ServerRejected(FailedToInitializeSessionReason::UserNotFound)
            .is_retryable()
    );
}

#[test]
fn test_should_retry_startup_failure_respects_attempt_budget() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, false).0;

        network.update(&mut app, |network, _| {
            network.stage = Stage::BeforeStarted {
                startup_retry: StartupRetryState {
                    current_attempt: 1,
                    max_attempts: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    timeout_abort_handle: None,
                    transport_abort_handle: None,
                },
            };
            assert!(network.should_retry_startup_failure(&StartupFailure::Timeout));

            network.stage = Stage::BeforeStarted {
                startup_retry: StartupRetryState {
                    current_attempt: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    max_attempts: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    timeout_abort_handle: None,
                    transport_abort_handle: None,
                },
            };
            assert!(!network.should_retry_startup_failure(&StartupFailure::Timeout));

            let mut startup_retry = StartupRetryState::new(1);
            startup_retry.current_attempt = 1;
            network.stage = Stage::BeforeStarted { startup_retry };
            assert!(
                !network.should_retry_startup_failure(&StartupFailure::ServerRejected(
                    FailedToInitializeSessionReason::InternalServerError {
                        details: "transient".to_string(),
                    }
                ))
            );
        });
    });
}

#[test]
fn test_startup_attempt_stale_filtering() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, false).0;

        network.update(&mut app, |network, _| {
            network.stage = Stage::BeforeStarted {
                startup_retry: StartupRetryState {
                    current_attempt: 1,
                    max_attempts: AMBIENT_CREATE_SESSION_MAX_ATTEMPTS,
                    timeout_abort_handle: None,
                    transport_abort_handle: None,
                },
            };
            assert!(!network.should_ignore_startup_attempt_websocket_callback(1));
            assert!(network.should_ignore_startup_attempt_websocket_callback(0));
            network.stage = Stage::StartedSuccessfully {
                startup_attempt: Some(1),
            };
            assert!(!network.should_ignore_startup_attempt_websocket_callback(1));
            assert!(network.should_ignore_startup_attempt_websocket_callback(0));

            network.stage = Stage::StartedSuccessfully {
                startup_attempt: None,
            };
            assert!(!network.should_ignore_startup_attempt_websocket_callback(0));
        });
    });
}

fn is_upstream_message_command_executed(
    message: &UpstreamMessage,
    expected_event_no: usize,
) -> bool {
    matches!(message, UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
        event_no,
        event_type: OrderedTerminalEventType::CommandExecutionStarted { .. },
    }) if *event_no == expected_event_no)
}

fn create_network(
    app: &mut App,
    session_initialized: bool,
) -> (ModelHandle<Network>, Sender<OrderedTerminalEventType>) {
    let (ordered_events_tx, ordered_events_rx) = async_channel::unbounded();
    let scrollback_type = SharedSessionScrollbackType::None;
    let active_prompt = ActivePrompt::default();
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

    let network = app.add_model(|ctx| {
        Network::new_for_test(
            terminal_model,
            ordered_events_rx,
            scrollback_type,
            active_prompt,
            Selection::None,
            ReplicaId::random(),
            ctx,
        )
    });

    if session_initialized {
        network.update(app, |network, _| {
            network.stage = Stage::StartedSuccessfully {
                startup_attempt: None,
            };
        });
    }

    (network, ordered_events_tx)
}

#[test]
fn test_send_ordered_terminal_event_message_advances_event_no() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;

        // Make sure the event no starts at 0.
        network.read(&app, |network, _ctx| {
            assert_eq!(usize::from(network.event_no), 0);
        });

        // Try to send an ordered terminal event message to the server.
        let event = OrderedTerminalEventType::PtyBytesRead { bytes: "a".into() };
        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(event);
        });

        // The event no should be 1 now.
        network.read(&app, |network, _ctx| {
            assert_eq!(usize::from(network.event_no), 1);
        });
    });
}

#[test]
fn test_send_ordered_terminal_event_message_max_reached() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // Make sure the ws_proxy_tx is open.
        let ws_proxy_tx = network.read(&app, |network, _ctx| network.ws_proxy_tx.clone());
        assert!(!ws_proxy_tx.is_closed());

        // Try to send an ordered terminal event that would exceed the max bytes allowed limit.
        let overflow_event = OrderedTerminalEventType::PtyBytesRead {
            bytes: "a".repeat(MAX_BYTES_SHAREABLE + 1).into(),
        };
        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(overflow_event);
        });

        // Make sure the item we put on the ws_proxy_tx was correct.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        assert!(matches!(item.unwrap(), UpstreamMessage::EndSession { .. }));

        // Make sure the ws_proxy_tx is closed and nothing was sent.
        assert!(ws_proxy_tx.is_closed());
    });
}

#[test]
fn test_send_pty_read_event_while_batching() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle: AbortHandle::new_pair().0,
            };
        });

        // Try to send a PtyBytesRead message to the server.
        network.update(&mut app, |network, _| {
            network.send_pty_bytes_read_message();
        });

        // Make sure the item we put on the ws_proxy_tx was correct.
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "a".into()
        ));

        // The batch status should be NotBatching now and the last_sent_at should be updated.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time ));
        });
    });
}

#[test]
fn test_send_pty_read_event_while_not_batching() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            }
        });

        // Try to send a PtyBytesRead message to the server.
        network.update(&mut app, |network, _| {
            network.send_pty_bytes_read_message();
        });

        // Make sure we didn't try to send anything to the server..
        assert_eq!(ws_proxy_rx.len(), 0);

        // The batch status should be unchanged.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at == init_time));
        });
    });
}

#[test]
fn test_handle_pty_read_event_while_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle: AbortHandle::new_pair().0,
            };
        });

        // Send a PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::PtyBytesRead { bytes: "a".into() };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // The batching status should reflect the accumulated bytes.
        assert_eventually!(
            network.read(&app, |network, _ctx| {
                matches!(&network.pty_bytes_batch_status, PtyBytesBatchStatus::Batching { accumulated, .. } if accumulated == b"aa" )
            }), "Batching status should reflect accumulated bytes"
        );

        // Technically, we didn't start a task to send the event to the server after a timer. So let's do it manually.
        network.update(&mut app, |network, _| {
            network.send_pty_bytes_read_message();
        });

        // Eventually, the accumulated event should be sent to the server.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "aa".into()
        ));

        // The batching status should be reset.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        });
    })
}

#[test]
fn test_handle_pty_read_event_while_not_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            }
        });

        // Send a PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::PtyBytesRead { bytes: "a".into() };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // The test executor uses real (async_io) timers with no mock clock, so this
        // test relies on the batch timer actually firing. Under test builds
        // PTY_READS_BATCH_THRESHOLD is larger than the ~50ms production value so the
        // transient `Batching` state below is reliably observable instead of racing the
        // timer under coarse scheduler granularity (which flaked on Windows CI).
        assert_eventually!(
            200 =>
            network.read(&app, |network, _ctx| {
                matches!(&network.pty_bytes_batch_status, PtyBytesBatchStatus::Batching { accumulated, .. } if accumulated == b"a" )
            }),
            "Batching status should be batching"
        );

        // When the batch timer fires, the accumulated event is flushed to the server.
        // Await the flush directly rather than polling a fixed tick budget, but bound the
        // wait (generously, relative to the test-build batch threshold) so a regression in
        // the timer/flush path fails this test promptly instead of hanging until the CI
        // timeout.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(PTY_READS_BATCH_THRESHOLD * 20)
            .await
            .expect("Accumulated event should be flushed before the timeout");
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "a".into()
        ));

        // The batching status should be reset.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        });
    });
}

#[test]
fn test_handle_non_pty_read_event_while_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle: AbortHandle::new_pair().0,
            };
        });

        // Send a non PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        assert_eventually!(
            ws_proxy_rx.len() == 2,
            "Two messages should be sent to the server; got {}",
            ws_proxy_rx.len()
        );

        // Make sure that we flush the PtyBytesRead message first.
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "a".into()
        ));

        // And that the non PtyBytesRead message follows suit.
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 1));

        // The batching status should be reset.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        })
    })
}

#[test]
fn test_handle_non_pty_read_event_while_not_batching() {
    App::test((), |mut app| async move {
        let (network, ordered_events_tx) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set the batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            }
        });

        // Send a non PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        assert_eventually!(
            ws_proxy_rx.len() == 1,
            "One message should be sent to the server; got {}",
            ws_proxy_rx.len()
        );

        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));

        // The batching status should be unchanged.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at == init_time));
        })
    });
}

#[test]
fn test_ignore_duplicate_prompt_updates() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        assert_eq!(ws_proxy_rx.len(), 0);
        // First prompt update should go through.
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "test warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 1);

        // Duplicate prompt updates should be ignored.
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "test warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 1);
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "test warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 1);

        // Different prompt should go through.
        network.update(&mut app, |network, _ctx| {
            network.send_active_prompt_update_if_changed(ActivePrompt::WarpPrompt(
                "different warp prompt".to_owned(),
            ));
        });
        assert_eq!(ws_proxy_rx.len(), 2);
    });
}

#[test]
fn test_selection_updates_throttled_and_duplicates_ignored() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, true);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        assert_eq!(ws_proxy_rx.len(), 0);
        // Rapid fire selection updates. Only the last should be sent up the websocket due to throttling.
        network.update(&mut app, |network, _ctx| {
            for i in 0..5 {
                network.send_presence_selection_if_changed(Selection::Blocks {
                    block_ids: vec![format!("block{i}").to_string().into()],
                });
            }
        });

        // Only the very first and the last updates should go through, but not any of the intermediate ones.
        assert_eventually!(
            ws_proxy_rx.len() == 2,
            "Selection updates should be throttled"
        );

        // Last sent block ID should be block4, and duplicate selection updates should be ignored.
        network.update(&mut app, |network, _ctx| {
            network.send_presence_selection_if_changed(Selection::Blocks {
                block_ids: vec!["block4".to_string().into()],
            });
        });
        assert_eventually!(
            ws_proxy_rx.len() == 2,
            "Duplicate selection updates should be ignored"
        );

        // Different selection update should go through.
        network.update(&mut app, |network, _ctx| {
            network.send_presence_selection_if_changed(Selection::None);
        });
        assert_eventually!(
            ws_proxy_rx.len() == 3,
            "Different selection updates should go through"
        );
    });
}

#[test]
fn test_messages_are_buffered_before_session_initialized() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, false);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // The network should start in the BeforeStarted state with no events.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::BeforeStarted { .. }));
            assert_eq!(network.unacked_terminal_events.len(), 0);
        });

        // Try to send a message to the server.
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 0,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });

        // The message should not be sent to the server but should instead be buffered.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::BeforeStarted { .. }));
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&0).unwrap().clone()
                ),
                0
            ));
        });

        // Simulate receiving the SessionInitialized message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_string(),
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // The message should be flushed to the server and the stage should be advanced.
        // We should also re-send the active prompt.
        assert_eq!(ws_proxy_rx.len(), 2);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::StartedSuccessfully { .. }));
        });
    });
}

#[test]
fn test_messages_are_buffered_while_reconnecting() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        // Disabled (`None`) IapManager so the reconnect path, which reads the
        // singleton, doesn't panic; inert no-op in tests.
        app.add_singleton_model(|ctx| {
            IapManager::new(
                None,
                Box::new(|_| futures::FutureExt::boxed(futures::future::ready(None::<String>))),
                None,
                ctx,
            )
        });
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        let (network, _) = create_network(&mut app, false);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // The network should start in the BeforeStarted state with no events.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::BeforeStarted { .. }));
            assert_eq!(network.unacked_terminal_events.len(), 0);
        });

        // Simulate receiving the SessionInitialized message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_string(),
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // We should have sent the latest prompt on connection.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        // Simulate reconnecting to the server after server disconnects. Nothing we need to do in this test to disconnect first.
        network.update(&mut app, |network, ctx| {
            network.reconnect_websocket(ctx);
        });

        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::Reconnecting { .. }));
        });

        // Try to send a message to the server.
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 0,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });

        // The message should not be sent to the server but should instead be stored.
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::Reconnecting { .. }));
            assert_eq!(network.unacked_terminal_events.len(), 1);
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&0).unwrap().clone()
                ),
                0
            ));
        });

        // Simulate receiving the SessionReconnected message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionReconnected {
                last_received_event_no: None,
                participant_list: Default::default(),
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // The message should be flushed to the server and the stage should be advanced.
        // We should also re-send the active prompt.
        assert_eq!(ws_proxy_rx.len(), 2);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        network.read(&app, |network, _| {
            assert!(matches!(&network.stage, Stage::StartedSuccessfully { .. }));
        });
    });
}

#[test]
fn test_events_are_saved_on_send_and_removed_on_ack() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, false);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // Simulate receiving the SessionInitialized message from the server.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionInitialized {
                session_id: SessionId::new(),
                session_secret: Default::default(),
                reconnect_token: ReconnectToken::new(),
                sharer_id: ParticipantId::new(),
                sharer_firebase_uid: "mock_firebase_uid".to_string(),
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // We should have sent the latest prompt on connection.
        assert_eq!(ws_proxy_rx.len(), 1);
        let item = ws_proxy_rx.recv().await;
        matches!(item.unwrap(), UpstreamMessage::UpdateActivePrompt(_));

        // Try to send a couple messages to the server.
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 0,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });
        let event_type = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        let event = OrderedTerminalEvent {
            event_no: 1,
            event_type,
        };
        let message = UpstreamMessage::OrderedTerminalEvent(event);
        network.update(&mut app, |network, _ctx| {
            network.send_message_to_server(message)
        });

        // The messages should be both sent and stored.
        assert_eq!(ws_proxy_rx.len(), 2);
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 0));
        let item = ws_proxy_rx.recv().await;
        assert!(is_upstream_message_command_executed(&item.unwrap(), 1));
        network.read(&app, |network, _| {
            assert_eq!(network.unacked_terminal_events.len(), 2);
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&0).unwrap().clone()
                ),
                0
            ));
            assert!(is_upstream_message_command_executed(
                &UpstreamMessage::OrderedTerminalEvent(
                    network.unacked_terminal_events.get(&1).unwrap().clone()
                ),
                1
            ));
        });

        // Simulate receiving the EventsProcessedAck message from the server.
        network.update(
            &mut app,
            |network, ctx: &mut warpui::ModelContext<'_, Network>| {
                let downstream_message = DownstreamMessage::EventsProcessedAck {
                    latest_processed_event_no: 1,
                };
                let serialized = downstream_message.to_json().unwrap();
                network.process_websocket_message(Message::new(serialized), ctx);
            },
        );

        // Both messages should be removed from the stored events to free up memory.
        network.read(&app, |network, _| {
            assert_eq!(network.unacked_terminal_events.len(), 0);
        });
    });
}
