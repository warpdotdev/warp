use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_channel::Sender;
use byte_unit::Byte;
use futures::channel::mpsc;
use futures_util::future::BoxFuture;
use futures_util::stream::AbortHandle;
use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _, future, sink, stream};
use instant::Instant;
use parking_lot::FairMutex;
use session_sharing_protocol::common::{
    ActivePrompt, FeatureSupport, InputOperationId, InputOperationSeqNo, InputUpdate,
    OrderedTerminalEvent, OrderedTerminalEventType, ParticipantId, Selection, SelectionUpdate,
    SessionId, UserID,
};
use session_sharing_protocol::sharer::{
    DownstreamMessage, FailedToInitializeSessionReason, QuotaType, ReconnectPayload,
    ReconnectToken, ReconnectionFailedReason, SessionEndedReason, SessionTerminatedReason,
    UpstreamMessage,
};
use warp_server_client::iap::IapManager;
#[cfg(not(target_family = "wasm"))]
use warpui::r#async::executor::Foreground;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{App, ModelHandle, RetryOption};
use websocket::{Error as WebsocketError, Message, Sink, Stream, WebsocketMessage as _};

use super::{
    AMBIENT_CREATE_SESSION_MAX_ATTEMPTS, ConfirmedReconnection, MAX_PRE_RECONNECT_BYTES,
    MAX_PRE_RECONNECT_MESSAGES, Network, PTY_READS_BATCH_THRESHOLD, PtyBytesBatchStatus, Stage,
    StartupFailure, StartupRetryState, confirm_reconnection, share_with_team_uid_for_init_payload,
    startup_max_attempts,
};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::terminal::TerminalModel;
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

fn discard_sink() -> impl Sink {
    sink::drain().sink_map_err(|error: Infallible| match error {})
}

fn reconnect_payload() -> ReconnectPayload {
    ReconnectPayload {
        session_secret: Default::default(),
        reconnect_token: ReconnectToken::new(),
        user_id: UserID {
            anonymous_id: "anonymous".to_string(),
            access_token: None,
        },
        latest_block_id: "block".to_string().into(),
        selection: Selection::None,
        feature_support: FeatureSupport {
            supports_agent_view: false,
            supports_full_role: true,
            supports_full_role_for_real: true,
        },
    }
}

fn reconnected_message() -> Message {
    Message::new(
        DownstreamMessage::SessionReconnected {
            last_received_event_no: None,
            participant_list: Default::default(),
        }
        .to_json()
        .unwrap(),
    )
}

type MockReconnection = ConfirmedReconnection<Pin<Box<dyn Sink>>, Pin<Box<dyn Stream>>>;
type ReconnectAttempt = BoxFuture<'static, anyhow::Result<MockReconnection>>;

fn mock_reconnect(stream: impl Stream) -> ReconnectAttempt {
    mock_reconnect_with_sink(discard_sink(), stream)
}

fn mock_reconnect_with_sink(sink: impl Sink, stream: impl Stream) -> ReconnectAttempt {
    let sink: Pin<Box<dyn Sink>> = Box::pin(sink);
    let stream: Pin<Box<dyn Stream>> = Box::pin(stream);
    confirm_reconnection(sink, stream, reconnect_payload()).boxed()
}

fn confirmed_reconnect() -> ReconnectAttempt {
    mock_reconnect(stream::iter([Ok(reconnected_message())]).chain(stream::pending()))
}

fn start_scripted_reconnect(
    app: &mut App,
    script: Vec<ReconnectAttempt>,
    retry_strategy: RetryOption,
    attempt_timeout: Duration,
    cycle_timeout: Duration,
) -> (ModelHandle<Network>, Arc<AtomicUsize>) {
    let (network, _) = create_network(app, true);
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut script = script.into_iter();
    network.update(app, |network, ctx| {
        let attempts = attempts.clone();
        network.start_reconnect_task(
            move || {
                attempts.fetch_add(1, Ordering::SeqCst);
                script.next().expect("Unexpected reconnect attempt")
            },
            retry_strategy,
            attempt_timeout,
            cycle_timeout,
            ctx,
        );
    });
    (network, attempts)
}

#[test]
fn test_reconnect_retries_eof_before_ack() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![mock_reconnect(stream::empty()), confirmed_reconnect()],
            RetryOption::linear(Duration::from_millis(1), 1),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| network.is_connected()),
            "EOF before acknowledgement should retry and reconnect"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn test_reconnect_retries_socket_error_before_ack() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![
                mock_reconnect(stream::iter([Err(anyhow::anyhow!("socket error").into())])),
                confirmed_reconnect(),
            ],
            RetryOption::linear(Duration::from_millis(1), 1),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| network.is_connected()),
            "Socket error before acknowledgement should retry and reconnect"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn test_reconnect_retries_ack_timeout() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![mock_reconnect(stream::pending()), confirmed_reconnect()],
            RetryOption::linear(Duration::from_millis(1), 1),
            Duration::from_millis(20),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| network.is_connected()),
            "Missing acknowledgement should time out and retry"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn test_reconnect_transport_timeout_retries_within_cycle() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![future::pending().boxed(), confirmed_reconnect()],
            RetryOption::linear(Duration::from_millis(1), 1),
            Duration::from_millis(20),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| network.is_connected()),
            "A stalled transport connection should time out and retry"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn test_reconnect_send_error_is_retryable() {
    App::test((), |mut app| async move {
        let failed_sink =
            discard_sink().with(|_| future::err(anyhow::anyhow!("send failed").into()));
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![
                mock_reconnect_with_sink(failed_sink, stream::pending()),
                confirmed_reconnect(),
            ],
            RetryOption::linear(Duration::from_millis(1), 1),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| network.is_connected()),
            "A failed reconnect send should retry"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn test_reconnect_send_timeout_is_retryable() {
    App::test((), |mut app| async move {
        let stalled_sink =
            discard_sink().with(|_| future::pending::<Result<Message, WebsocketError>>());
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![
                mock_reconnect_with_sink(stalled_sink, stream::pending()),
                confirmed_reconnect(),
            ],
            RetryOption::linear(Duration::from_millis(1), 1),
            Duration::from_millis(20),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| network.is_connected()),
            "A stalled reconnect send should time out and retry"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn test_reconnect_attempt_budget_exhaustion_finishes_session() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![
                mock_reconnect(stream::empty()),
                mock_reconnect(stream::empty()),
                mock_reconnect(stream::empty()),
            ],
            RetryOption::linear(Duration::from_millis(1), 2),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| matches!(network.stage, Stage::Finished)),
            "Exhausting reconnect retries should finish rather than strand the session"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        network.read(&app, |network, _| assert!(network.ws_proxy_tx.is_closed()));
    });
}

#[test]
fn test_reconnect_cycle_deadline_includes_backoff() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![mock_reconnect(stream::empty())],
            RetryOption::linear(Duration::from_secs(1), 18),
            Duration::from_secs(1),
            Duration::from_millis(20),
        );
        assert_eventually!(
            network.read(&app, |network, _| matches!(network.stage, Stage::Finished)),
            "Cycle deadline should end reconnect during backoff"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn test_reconnect_cycle_deadline_includes_transport() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![future::pending().boxed()],
            RetryOption::linear(Duration::from_secs(1), 18),
            Duration::from_secs(1),
            Duration::from_millis(20),
        );
        assert_eventually!(
            network.read(&app, |network, _| matches!(network.stage, Stage::Finished)),
            "Cycle deadline should end reconnect during transport connection"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn test_end_session_cancels_reconnect_backoff() {
    App::test((), |mut app| async move {
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![mock_reconnect(stream::empty())],
            RetryOption::linear(Duration::from_millis(100), 18),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eventually!(
            attempts.load(Ordering::SeqCst) == 1,
            "First attempt should start"
        );
        network.update(&mut app, |network, _| {
            network.end_session(SessionEndedReason::EndedBySharer);
        });
        Timer::after(Duration::from_millis(250)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished))
        });
    });
}

#[test]
fn test_reconnect_buffers_pre_ack_messages_and_preserves_remaining_stream() {
    App::test((), |_| async move {
        let buffered = DownstreamMessage::EventsProcessedAck {
            latest_processed_event_no: 1,
        };
        let connection = mock_reconnect(stream::iter([
            Ok(Message::new_binary(vec![1])),
            Ok(Message::new(buffered.to_json().unwrap())),
            Ok(reconnected_message()),
            Ok(Message::new(buffered.to_json().unwrap())),
        ]))
        .await
        .unwrap();
        assert_eq!(connection.buffered_messages.len(), 1);
        assert_eq!(
            connection.buffered_messages[0].text(),
            Some(buffered.to_json().unwrap().as_str())
        );
        let mut remaining = connection.stream;
        assert!(remaining.next().await.unwrap().is_ok());
    });
}

#[test]
fn test_reconnect_pre_ack_buffer_rejects_message_count_over_limit() {
    App::test((), |_| async move {
        let messages = (0..=MAX_PRE_RECONNECT_MESSAGES).map(|_| Ok(Message::new("{}".to_string())));
        let error = mock_reconnect(stream::iter(messages)).await.err().unwrap();
        assert!(error.to_string().contains("Too many messages"));
    });
}

#[test]
fn test_reconnect_pre_ack_buffer_accepts_message_count_at_limit() {
    App::test((), |_| async move {
        let messages = (0..MAX_PRE_RECONNECT_MESSAGES).map(|_| Ok(Message::new("{}".to_string())));
        let connection =
            mock_reconnect(stream::iter(messages).chain(stream::iter([Ok(reconnected_message())])))
                .await
                .unwrap();
        assert_eq!(
            connection.buffered_messages.len(),
            MAX_PRE_RECONNECT_MESSAGES
        );
    });
}

#[test]
fn test_reconnect_pre_ack_buffer_rejects_byte_count_over_limit() {
    App::test((), |_| async move {
        let error = mock_reconnect(stream::iter([Ok(Message::new(
            "x".repeat(MAX_PRE_RECONNECT_BYTES + 1),
        ))]))
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("Too many messages"));
    });
}

#[test]
fn test_reconnect_pre_ack_buffer_accepts_byte_count_at_limit() {
    App::test((), |_| async move {
        let connection = mock_reconnect(stream::iter([
            Ok(Message::new("x".repeat(MAX_PRE_RECONNECT_BYTES))),
            Ok(reconnected_message()),
        ]))
        .await
        .unwrap();
        assert_eq!(connection.buffered_messages.len(), 1);
        assert_eq!(
            connection.buffered_messages[0].text().unwrap().len(),
            MAX_PRE_RECONNECT_BYTES
        );
    });
}

#[test]
fn test_explicit_rejection_does_not_retry() {
    App::test((), |mut app| async move {
        let response = DownstreamMessage::FailedToReconnect {
            reason: ReconnectionFailedReason::SessionNotFound,
        };
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![mock_reconnect(stream::iter([Ok(Message::new(
                response.to_json().unwrap(),
            ))]))],
            RetryOption::linear(Duration::from_millis(1), 18),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| matches!(network.stage, Stage::Finished)),
            "Explicit rejection must be terminal"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn test_explicit_termination_does_not_retry() {
    App::test((), |mut app| async move {
        let response = DownstreamMessage::SessionTerminated {
            reason: SessionTerminatedReason::ExceededSizeLimit,
        };
        let (network, attempts) = start_scripted_reconnect(
            &mut app,
            vec![mock_reconnect(stream::iter([Ok(Message::new(
                response.to_json().unwrap(),
            ))]))],
            RetryOption::linear(Duration::from_millis(1), 18),
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eventually!(
            network.read(&app, |network, _| matches!(network.stage, Stage::Finished)),
            "Explicit termination must be terminal"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    });
}

fn network_with_stale_websocket(
    app: &mut App,
) -> (
    ModelHandle<Network>,
    mpsc::UnboundedSender<Result<Message, WebsocketError>>,
) {
    let (network, ordered_events_tx) = create_network(app, true);
    drop(ordered_events_tx);
    let (old_tx, old_rx) = mpsc::unbounded();
    network.update(app, |network, ctx| {
        network.selection_throttled_tx.close();
        let (_, old_proxy_rx) = async_channel::unbounded();
        network.on_websocket_connected(None, old_proxy_rx, discard_sink(), old_rx, ctx);
        network.close();
        let (replacement_tx, replacement_rx) = async_channel::unbounded();
        network.ws_proxy_tx = replacement_tx;
        network.ws_proxy_rx = replacement_rx;
    });
    (network, old_tx)
}

#[cfg(not(target_family = "wasm"))]
async fn finish_foreground_tasks(app: &App) {
    let foreground = app.foreground_executor();
    let Foreground::Test { executor } = foreground.as_ref() else {
        panic!("Expected the test foreground executor");
    };
    // The foreground stream task remains registered until both on_item and on_done return.
    // Closing the unrelated test sources lets executor emptiness prove callback completion.
    assert_eventually!(400 => executor.is_empty(), "Old websocket callbacks should finish");
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn test_stale_websocket_message_does_not_terminate_replacement() {
    App::test((), |mut app| async move {
        let (network, old_tx) = network_with_stale_websocket(&mut app);
        old_tx
            .unbounded_send(Ok(Message::new(
                DownstreamMessage::SessionTerminated {
                    reason: SessionTerminatedReason::ExceededSizeLimit,
                }
                .to_json()
                .unwrap(),
            )))
            .unwrap();
        drop(old_tx);
        finish_foreground_tasks(&app).await;
        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::StartedSuccessfully { .. }));
            assert!(!network.ws_proxy_tx.is_closed());
        });
    });
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn test_stale_websocket_eof_does_not_close_replacement() {
    App::test((), |mut app| async move {
        let (network, old_tx) = network_with_stale_websocket(&mut app);
        drop(old_tx);
        finish_foreground_tasks(&app).await;
        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::StartedSuccessfully { .. }));
            assert!(!network.ws_proxy_tx.is_closed());
        });
    });
}

#[test]
fn test_reconnect_confirmation_flushes_pending_input_updates() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app, true);
        network.update(&mut app, |network, ctx| {
            network.stage = Stage::Reconnecting {
                abort_handle: AbortHandle::new_pair().0,
            };
            network.pending_input_updates.push(InputUpdate {
                id: InputOperationId {
                    participant_id: ParticipantId::new(),
                    buffer_id: network.next_buffer_seq_no.0.clone().into(),
                    op_no: InputOperationSeqNo::zero(),
                },
                ops: vec![],
            });
            network.process_websocket_message(reconnected_message(), ctx);
            assert!(network.pending_input_updates.is_empty());
            assert!(matches!(
                network.ws_proxy_rx.try_recv().unwrap(),
                UpstreamMessage::UpdateInput(_)
            ));
            assert!(matches!(
                network.ws_proxy_rx.try_recv().unwrap(),
                UpstreamMessage::UpdateActivePrompt(_)
            ));
        });
    });
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
            assert!(network.should_ignore_startup_attempt_websocket_callback(0));
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
    let (ordered_events_tx, ordered_events_rx) = async_channel::unbounded();
    let active_prompt = ActivePrompt::default();
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

    let network = app.add_model(|ctx| {
        Network::new_for_test(
            terminal_model,
            ordered_events_rx,
            active_prompt,
            Selection::None,
            Byte::from_u64(MAX_BYTES_SHAREABLE as u64),
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

        // The batching status should reflect the accumulated bytes. Use the same generous tick
        // budget as `test_handle_pty_read_event_while_not_batching`: the event is handled on the
        // test executor, and the default budget flaked under coarse scheduling on Windows CI.
        assert_eventually!(
            200 =>
            network.read(&app, |network, _ctx| {
                matches!(&network.pty_bytes_batch_status, PtyBytesBatchStatus::Batching { accumulated, .. } if accumulated == b"aa" )
            }),
            "Batching status should reflect accumulated bytes"
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

        // Simulate the replacement transport receiving the SessionReconnected message.
        let ws_proxy_rx = network.update(&mut app, |network, ctx| {
            let (tx, rx) = async_channel::unbounded();
            network.ws_proxy_tx = tx;
            network.ws_proxy_rx = rx.clone();
            let downstream_message = DownstreamMessage::SessionReconnected {
                last_received_event_no: None,
                participant_list: Default::default(),
            };
            let serialized = downstream_message.to_json().unwrap();
            network.process_websocket_message(Message::new(serialized), ctx);
            rx
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
