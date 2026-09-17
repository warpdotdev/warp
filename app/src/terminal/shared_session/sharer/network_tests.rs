use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use byte_unit::Byte;
use futures::SinkExt as _;
use futures::channel::mpsc;
use futures_util::StreamExt as _;
use futures_util::stream::AbortHandle;
use instant::Instant;
use parking_lot::FairMutex;
use session_sharing_protocol::common::{
    ActivePrompt, ActivePromptUpdate, OrderedTerminalEvent, OrderedTerminalEventType,
    ParticipantId, Selection, SelectionUpdate, SessionId,
};
use session_sharing_protocol::sharer::{
    DownstreamMessage, FailedToInitializeSessionReason, QuotaType, ReconnectToken, UpstreamMessage,
};
use warp_multi_agent_api::client_action::{Action, AddMessagesToTask};
use warp_multi_agent_api::message::{AgentOutput, Message as MessageKind};
use warp_multi_agent_api::response_event::{ClientActions, Type};
use warp_multi_agent_api::{ClientAction, ResponseEvent};
use warp_server_client::iap::IapManager;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};
use websocket::{Message, WebsocketMessage as _};

use super::super::message_size::{
    MAX_ORDERED_TERMINAL_EVENT_BYTES, MAX_PTY_BATCH_RAW_BYTES, SERVER_MAX_WEBSOCKET_MESSAGE_BYTES,
};
use super::{
    AMBIENT_CREATE_SESSION_MAX_ATTEMPTS, Network, PTY_READS_BATCH_THRESHOLD, PtyBytesBatchStatus,
    Stage, StartupFailure, StartupRetryState, share_with_team_uid_for_init_payload,
    startup_max_attempts,
};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::terminal::TerminalModel;
use crate::terminal::shared_session::ai_agent::{
    decode_agent_response_event, encode_agent_response_event,
};
use crate::terminal::shared_session::{
    MAX_BYTES_SHAREABLE, SELECTION_THROTTLE_PERIOD, SharedSessionSource,
};
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
fn test_share_with_team_uid_for_init_payload_includes_team_scoped_view() {
    let team_uid = crate::server::ids::ServerId::from(123);
    let scope = crate::workspaces::user_workspaces::TeamContextForOperation::new_for_test(team_uid);
    assert_eq!(
        share_with_team_uid_for_init_payload(&scope),
        Some(String::from(team_uid))
    );
}

#[test]
fn test_share_with_team_uid_for_init_payload_omits_personal_view() {
    assert_eq!(
        share_with_team_uid_for_init_payload(
            &crate::workspaces::user_workspaces::TeamlessScopeForTest,
        ),
        None
    );
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

    assert!(
        !StartupFailure::ServerRejected(FailedToInitializeSessionReason::ScrollbackTooLarge {})
            .is_retryable()
    );
    assert!(
        !StartupFailure::ServerRejected(FailedToInitializeSessionReason::NoUserQuotaRemaining {
            quota_type: QuotaType::SessionsCreated,
        })
        .is_retryable()
    );
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

fn is_upstream_message_selection_update(
    message: UpstreamMessage,
    expected_event_no: usize,
    expected_selection: Selection,
) -> bool {
    matches!(
        message,
        UpstreamMessage::UpdateSelection(SelectionUpdate {
            selection,
            event_no,
        }) if event_no == expected_event_no.into() && selection == expected_selection
    )
}

fn create_network(
    app: &mut App,
    session_initialized: bool,
) -> (ModelHandle<Network>, Sender<OrderedTerminalEventType>) {
    create_network_with_max_session_size(app, session_initialized, MAX_BYTES_SHAREABLE)
}

fn create_network_with_max_session_size(
    app: &mut App,
    session_initialized: bool,
    max_session_size: usize,
) -> (ModelHandle<Network>, Sender<OrderedTerminalEventType>) {
    let (ordered_events_tx, ordered_events_rx) = async_channel::unbounded();
    let active_prompt = ActivePrompt::default();
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

    let network = app.add_model(|ctx| {
        Network::new_for_test(
            terminal_model,
            ordered_events_rx,
            active_prompt,
            Selection::None,
            Byte::from_u64(max_session_size as u64),
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

/// An `AgentResponseEvent` carrying `message_count` agent output messages of `text_len` bytes each
/// in a single `AddMessagesToTask`.
fn agent_response_event_with_messages(
    message_count: usize,
    text_len: usize,
) -> OrderedTerminalEventType {
    let messages = (0..message_count)
        .map(|i| warp_multi_agent_api::Message {
            id: format!("message-{i}"),
            message: Some(MessageKind::AgentOutput(AgentOutput {
                text: "x".repeat(text_len),
            })),
            ..Default::default()
        })
        .collect();
    let response = ResponseEvent {
        r#type: Some(Type::ClientActions(ClientActions {
            actions: vec![ClientAction {
                action: Some(Action::AddMessagesToTask(AddMessagesToTask {
                    task_id: "task".to_string(),
                    messages,
                })),
            }],
        })),
    };
    OrderedTerminalEventType::AgentResponseEvent {
        response_initiator: Some(ParticipantId::new()),
        response_event: encode_agent_response_event(&response),
        forked_from_conversation_token: None,
    }
}

fn decoded_message_ids(event_type: &OrderedTerminalEventType) -> Vec<String> {
    let OrderedTerminalEventType::AgentResponseEvent { response_event, .. } = event_type else {
        panic!("expected an AgentResponseEvent, got {event_type:?}");
    };
    let Some(Type::ClientActions(client_actions)) =
        decode_agent_response_event(response_event).unwrap().r#type
    else {
        panic!("expected ClientActions");
    };
    client_actions
        .actions
        .into_iter()
        .flat_map(|action| {
            let Some(Action::AddMessagesToTask(add)) = action.action else {
                return Vec::new();
            };
            add.messages.into_iter().map(|m| m.id).collect()
        })
        .collect()
}

/// Wires a mock websocket into the network's send task and returns the receiving end, so tests
/// can observe exactly what would be written to the wire.
fn connect_mock_websocket(
    network: &ModelHandle<Network>,
    app: &mut App,
) -> mpsc::UnboundedReceiver<Message> {
    let (wire_tx, wire_rx) = mpsc::unbounded();
    network.update(app, |network, ctx| {
        let (ws_proxy_tx, ws_proxy_rx) = async_channel::unbounded();
        network.ws_proxy_tx = ws_proxy_tx;
        network.ws_proxy_rx = ws_proxy_rx.clone();
        let sink = wire_tx.sink_map_err(|e| websocket::Error::from(anyhow::Error::new(e)));
        let stream = futures::stream::pending::<Result<Message, websocket::Error>>();
        network.on_websocket_connected(None, ws_proxy_rx, sink, stream, ctx);
    });
    wire_rx
}

async fn next_wire_message(wire_rx: &mut mpsc::UnboundedReceiver<Message>) -> UpstreamMessage {
    let message = wire_rx
        .next()
        .with_timeout(Duration::from_secs(5))
        .await
        .expect("A message should reach the wire before the timeout")
        .expect("The wire should remain open");
    UpstreamMessage::from_json(message.text().unwrap()).unwrap()
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
fn test_send_ordered_terminal_event_message_splits_oversized_agent_event() {
    App::test((), |mut app| async move {
        let network = create_network_with_max_session_size(&mut app, true, usize::MAX).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // Three messages that together exceed the cap but individually fit.
        let message_count = 3;
        let text_len = MAX_ORDERED_TERMINAL_EVENT_BYTES / 2;
        let event = agent_response_event_with_messages(message_count, text_len);
        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(event);
        });

        // Nothing was dropped: every message reaches the server, in order, across consecutively
        // numbered events that each fit under the cap.
        let sent = ws_proxy_rx.len();
        assert!(
            sent > 1,
            "expected the event to be split, got {sent} event(s)"
        );
        network.read(&app, |network, _ctx| {
            assert_eq!(usize::from(network.event_no), sent);
            assert_eq!(network.unacked_terminal_events.len(), sent);
        });
        let mut message_ids = Vec::new();
        for expected_event_no in 0..sent {
            let message = ws_proxy_rx.recv().await.unwrap();
            assert!(message.to_json().unwrap().len() <= MAX_ORDERED_TERMINAL_EVENT_BYTES);
            let UpstreamMessage::OrderedTerminalEvent(event) = message else {
                panic!("expected an ordered terminal event");
            };
            assert_eq!(event.event_no, expected_event_no);
            message_ids.extend(decoded_message_ids(&event.event_type));
        }
        assert_eq!(
            message_ids,
            (0..message_count)
                .map(|i| format!("message-{i}"))
                .collect::<Vec<_>>()
        );
    });
}

#[test]
fn test_send_ordered_terminal_event_message_drops_indivisible_oversized_message() {
    App::test((), |mut app| async move {
        let network = create_network_with_max_session_size(&mut app, true, usize::MAX).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // A single message that is itself larger than the cap can only be dropped; the event
        // number must not be consumed so the sequence stays contiguous for the next event.
        let event = agent_response_event_with_messages(1, MAX_ORDERED_TERMINAL_EVENT_BYTES);
        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(event);
        });
        assert_eq!(ws_proxy_rx.len(), 0);
        network.read(&app, |network, _ctx| {
            assert_eq!(usize::from(network.event_no), 0);
            assert!(network.unacked_terminal_events.is_empty());
        });

        network.update(&mut app, |network, _| {
            network.send_ordered_terminal_event_message(
                OrderedTerminalEventType::AgentConversationReplayEnded,
            );
        });
        let message = ws_proxy_rx.recv().await.unwrap();
        assert!(matches!(
            message,
            UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                event_no: 0,
                event_type: OrderedTerminalEventType::AgentConversationReplayEnded,
            })
        ));
    });
}

#[test]
fn test_send_pty_read_event_splits_large_batches() {
    App::test((), |mut app| async move {
        let network = create_network_with_max_session_size(&mut app, true, usize::MAX).0;
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        // Incompressible bytes, so compression cannot mask the raw size.
        let accumulated = (0..MAX_PTY_BATCH_RAW_BYTES * 2 + 1)
            .map(|i| (i * 7919 % 251) as u8)
            .collect::<Vec<_>>();
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: accumulated.clone(),
                abort_handle: AbortHandle::new_pair().0,
            };
        });
        network.update(&mut app, |network, _| {
            network.send_pty_bytes_read_message();
        });

        assert_eq!(ws_proxy_rx.len(), 3);
        let mut reassembled = Vec::new();
        for expected_event_no in 0..3 {
            let message = ws_proxy_rx.recv().await.unwrap();
            assert!(message.to_json().unwrap().len() <= MAX_ORDERED_TERMINAL_EVENT_BYTES);
            let UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                event_no,
                event_type: OrderedTerminalEventType::PtyBytesRead { bytes },
            }) = message
            else {
                panic!("expected a PtyBytesRead event");
            };
            assert_eq!(event_no, expected_event_no);
            reassembled.extend(lz4_flex::block::decompress_size_prepended(&bytes).unwrap());
        }
        assert_eq!(reassembled, accumulated);
    });
}

#[test]
fn test_oversized_unacked_event_is_replaced_on_reconnect_replay() {
    App::test((), |mut app| async move {
        let network = create_network_with_max_session_size(&mut app, true, usize::MAX).0;

        // An unacked event that is over the server's frame limit, as if it had been queued by a
        // path that bypassed source-side bounding.
        let oversized_payload = "A".repeat(SERVER_MAX_WEBSOCKET_MESSAGE_BYTES + 1);
        let response_initiator = Some(ParticipantId::new());
        network.update(&mut app, |network, _| {
            network.unacked_terminal_events.insert(
                0,
                OrderedTerminalEvent {
                    event_no: 0,
                    event_type: OrderedTerminalEventType::AgentResponseEvent {
                        response_initiator: response_initiator.clone(),
                        response_event: oversized_payload,
                        forked_from_conversation_token: None,
                    },
                },
            );
            network.unacked_terminal_events.insert(
                1,
                OrderedTerminalEvent {
                    event_no: 1,
                    event_type: OrderedTerminalEventType::AgentConversationReplayEnded,
                },
            );
            network.stage = Stage::Reconnecting {
                abort_handle: AbortHandle::new_pair().0,
            };
        });
        let mut wire_rx = connect_mock_websocket(&network, &mut app);

        // The server confirms the reconnect without having received anything, which replays
        // the whole unacked queue.
        network.update(&mut app, |network, ctx| {
            let downstream_message = DownstreamMessage::SessionReconnected {
                last_received_event_no: None,
                participant_list: Default::default(),
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
        });

        // The oversized event reaches the wire as a same-numbered placeholder rather than
        // verbatim, and the events after it are unaffected.
        let UpstreamMessage::OrderedTerminalEvent(first) = next_wire_message(&mut wire_rx).await
        else {
            panic!("expected an ordered terminal event");
        };
        assert_eq!(first.event_no, 0);
        let OrderedTerminalEventType::AgentResponseEvent {
            response_initiator: placeholder_initiator,
            response_event,
            ..
        } = first.event_type
        else {
            panic!("expected an AgentResponseEvent placeholder");
        };
        assert_eq!(placeholder_initiator, response_initiator);
        assert!(response_event.len() < SERVER_MAX_WEBSOCKET_MESSAGE_BYTES);
        assert!(
            decoded_message_ids(&OrderedTerminalEventType::AgentResponseEvent {
                response_initiator: None,
                response_event,
                forked_from_conversation_token: None,
            })
            .is_empty()
        );

        assert!(matches!(
            next_wire_message(&mut wire_rx).await,
            UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
                event_no: 1,
                event_type: OrderedTerminalEventType::AgentConversationReplayEnded,
            })
        ));
        assert!(matches!(
            next_wire_message(&mut wire_rx).await,
            UpstreamMessage::UpdateActivePrompt(_)
        ));
    });
}

#[test]
fn test_oversized_non_ordered_message_is_not_written_to_the_wire() {
    App::test((), |mut app| async move {
        let network = create_network(&mut app, true).0;
        let mut wire_rx = connect_mock_websocket(&network, &mut app);

        network.update(&mut app, |network, _| {
            network.send_message_to_server(UpstreamMessage::UpdateActivePrompt(
                ActivePromptUpdate {
                    active_prompt: ActivePrompt::WarpPrompt(
                        "p".repeat(SERVER_MAX_WEBSOCKET_MESSAGE_BYTES + 1),
                    ),
                    last_event_no: 0,
                },
            ));
            network.send_message_to_server(UpstreamMessage::UpdateActivePrompt(
                ActivePromptUpdate {
                    active_prompt: ActivePrompt::WarpPrompt("small".to_string()),
                    last_event_no: 0,
                },
            ));
        });

        // Only the small update makes it to the wire; the oversized one is skipped rather than
        // sent and rejected.
        assert!(matches!(
            next_wire_message(&mut wire_rx).await,
            UpstreamMessage::UpdateActivePrompt(ActivePromptUpdate {
                active_prompt: ActivePrompt::WarpPrompt(prompt),
                ..
            }) if prompt == "small"
        ));
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

/// Waits until the mock terminal model reports its active block as bootstrapped.
///
/// `start_ordered_terminal_events_listener` silently drops ordered events until this is
/// true, so callers must wait for it instead of racing it: sending an event beforehand can
/// flake if the listener task hasn't observed the bootstrapped state yet. Uses the same
/// generous 2s budget as the `recv()` timeouts below it, rather than the default
/// `assert_eventually!` tick budget, so this wait can't reintroduce a fixed-window race of
/// its own.
async fn wait_for_bootstrapped(network: &ModelHandle<Network>, app: &App) {
    assert_eventually!(
        400 =>
        network.read(app, |network, _ctx| network
            .model
            .lock()
            .is_active_block_bootstrapped()),
        "Mock terminal model should report the active block as bootstrapped"
    );
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

        wait_for_bootstrapped(&network, &app).await;

        // Send a non PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // Await each flush directly rather than polling a fixed tick budget, so a scheduling
        // delay under load can't race a fixed timeout window (which flaked on Windows CI).
        // Make sure that we flush the PtyBytesRead message first.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("PtyBytesRead flush message should be sent before the timeout");
        assert!(is_upstream_message_pty_bytes_read(
            item.unwrap(),
            0,
            "a".into()
        ));

        // And that the non PtyBytesRead message follows suit.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("Non-PtyBytesRead message should be sent before the timeout");
        assert!(is_upstream_message_command_executed(&item.unwrap(), 1));

        assert_eq!(ws_proxy_rx.len(), 0);

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

        wait_for_bootstrapped(&network, &app).await;

        // Send a non PtyBytesRead event to the Network model.
        let event = OrderedTerminalEventType::CommandExecutionStarted {
            participant_id: Default::default(),
            ai_metadata: None,
        };
        ordered_events_tx
            .try_send(event)
            .expect("Can send event over ordered_events_tx");

        // Await the flush directly rather than polling a fixed tick budget; see
        // test_handle_non_pty_read_event_while_batching for why.
        let item = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("Message should be sent before the timeout");
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
        network.update(&mut app, |network, _ctx| {
            for i in 0..5 {
                network.send_presence_selection_if_changed(Selection::Blocks {
                    block_ids: vec![format!("block{i}").to_string().into()],
                });
            }
        });
        let first_update = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("First selection update should be sent before the timeout")
            .expect("Selection update channel should remain open");
        assert!(is_upstream_message_selection_update(
            first_update,
            0,
            Selection::Blocks {
                block_ids: vec!["block0".to_string().into()]
            }
        ));

        let trailing_update = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("Trailing selection update should be sent before the timeout")
            .expect("Selection update channel should remain open");
        assert!(is_upstream_message_selection_update(
            trailing_update,
            1,
            Selection::Blocks {
                block_ids: vec!["block4".to_string().into()]
            }
        ));
        network.update(&mut app, |network, _ctx| {
            network.send_presence_selection_if_changed(Selection::Blocks {
                block_ids: vec!["block4".to_string().into()],
            });
        });
        assert!(
            ws_proxy_rx
                .recv()
                .with_timeout(SELECTION_THROTTLE_PERIOD * 2)
                .await
                .is_err(),
            "Duplicate selection updates should be ignored"
        );

        // Different selection update should go through.
        network.update(&mut app, |network, _ctx| {
            network.send_presence_selection_if_changed(Selection::None);
        });
        let distinct_update = ws_proxy_rx
            .recv()
            .with_timeout(Duration::from_secs(2))
            .await
            .expect("Distinct selection update should be sent before the timeout")
            .expect("Selection update channel should remain open");
        assert!(is_upstream_message_selection_update(
            distinct_update,
            2,
            Selection::None
        ));
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
