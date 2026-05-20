//! The model that interfaces with the network to
//! connect to and communicate with the shared session.
//! Adheres to the [`session-sharing-protocol`].

use anyhow::bail;
use async_channel::Receiver;
use instant::Instant;
use std::{pin::pin, sync::Arc};
use warpui::r#async::{SpawnedFutureHandle, Timer};

use futures_util::{stream::AbortHandle, SinkExt, StreamExt};

use parking_lot::FairMutex;
use session_sharing_protocol::{
    common::{
        ActivePrompt, ActivePromptUpdate, AddGuestsResponse, AgentAttachment,
        AgentPromptFailureReason, AgentPromptRequest, AgentPromptRequestId,
        CommandExecutionFailureReason, ControlAction, ControlActionFailureReason, FeatureSupport,
        InputOperationId, InputOperationSeqNo, InputUpdate, LinkAccessLevelUpdateResponse,
        ParticipantId, ParticipantList, ParticipantPresenceUpdate, RemoveGuestResponse, Role,
        RoleRequestId, RoleRequestResponse, Selection, SelectionUpdate, ServerConversationToken,
        SessionId, TeamAccessLevelUpdateResponse, TeamAclData, TelemetryContext,
        UniversalDeveloperInputContext, UniversalDeveloperInputContextUpdate,
        UpdatePendingUserRoleResponse, UserID, WindowSize, WriteToPtyFailureReason,
        WriteToPtyRequestId, WriteToPtySeqNo,
    },
    sharer::SessionSourceType,
    viewer::{
        DownstreamMessage, InitPayload, RoleUpdatedReason, SessionEndedReason, UpstreamMessage,
        ViewerRemovedReason,
    },
};

use std::time::Duration;
use warp_core::features::FeatureFlag;
use warpui::{
    Entity, ModelContext, ModelHandle, RequestState, RetryOption, SingletonEntity, WeakViewHandle,
};
use websocket::{Message, Sink, Stream, WebsocketMessage as _};

use crate::{
    auth::{auth_state::AuthState, AuthStateProvider, UserUid},
    editor::{CrdtOperation, ReplicaId},
    server::{
        server_api::{auth::AuthClient, ServerApiProvider},
        telemetry::telemetry_context,
    },
    terminal::{
        event_listener::ChannelEventListener,
        model::block::BlockId,
        shared_session::{
            connect_endpoint,
            network::heartbeat::{Event as HeartbeatEvent, Heartbeat},
            viewer::event_loop::{EventLoop, SharedSessionInitialLoadMode},
            EventNumber, SELECTION_THROTTLE_PERIOD,
        },
        TerminalModel, TerminalView,
    },
    throttle::throttle,
};

/// The amount of time we will wait to batch consecutive write to pty requests before sending an event to the server.
const PTY_WRITES_BATCH_THRESHOLD: Duration = if cfg!(test) {
    Duration::from_millis(5)
} else {
    Duration::from_millis(100)
};
/// Exponential backoff when retrying reconnection. This configuration has us retry for ~128 seconds before giving up,
/// where the last interval between retries is 26s.
/// The viewer can always close the window and rejoin using the same link, so we don't need to be super generous with the retries allowed.
const RECONNECT_RETRY_STRATEGY: RetryOption = RetryOption::exponential(
    Duration::from_millis(1000), /* interval */
    1.2,                         /* exponential factor */
    18,                          /* max retry count */
)
.with_jitter(0.2);

#[derive(Debug)]
enum Stage {
    BeforeJoined,
    JoinedSuccessfully,
    Reconnecting {
        abort_handle: AbortHandle,
    },
    /// The session was ended.
    Finished,
}

#[derive(Debug, Clone)]
enum PtyBytesBatchStatus {
    /// We're not currently batching PTY write events.
    NotBatching {
        /// The last time we sent a batch of PTY write events to the server.
        last_sent_at: Instant,
    },
    /// We're currently batching PTY write events.
    Batching {
        /// The set of PTY bytes accumulated so far.
        accumulated: Vec<u8>,
        /// The abort handle for the batch timer.
        abort_handle: SpawnedFutureHandle,
    },
}

/// Helper struct to group together the most up to date state that the server needs to know about.
/// Any event we send to the server where we only care about the latest value should be included here.
/// This is used to avoid sending duplicate updates, and to update the server with the latest state on reconnection.
struct CachedLatestState {
    selection: Selection,
    universal_developer_input_context: Option<UniversalDeveloperInputContext>,
}

/// The network interface to allow communication to and from the
/// cloud-backed shared session.
pub struct Network {
    heartbeat: ModelHandle<Heartbeat>,

    session_id: SessionId,
    /// [`None`] until the viewer receives the successful join ack.
    event_loop: Option<ModelHandle<EventLoop>>,

    terminal_view: WeakViewHandle<TerminalView>,

    channel_event_proxy: ChannelEventListener,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    initial_load_mode: SharedSessionInitialLoadMode,

    stage: Stage,

    /// Intermediate channel to queue up messages to send over
    /// over the websocket to the server.
    ws_proxy_tx: async_channel::Sender<UpstreamMessage>,
    selection_throttled_tx: async_channel::Sender<Selection>,

    #[cfg(test)]
    ws_proxy_rx: async_channel::Receiver<UpstreamMessage>,

    /// The participant ID we were assigned by the server.
    /// This is populated after successfully joining a session, and
    /// used if we need to reconnect.
    id: Option<ParticipantId>,

    cached_latest_state: CachedLatestState,

    selection_event_no: EventNumber,

    /// The parameters for the next input operation to send.
    next_buffer_seq_no: (BlockId, InputOperationSeqNo),

    /// The next event number to use when sending a write to pty request to the server.
    write_to_pty_event_no: WriteToPtySeqNo,
    pty_bytes_batch_status: PtyBytesBatchStatus,
}

impl Network {
    pub fn new(
        session_id: SessionId,
        channel_event_proxy: ChannelEventListener,
        terminal_view: WeakViewHandle<TerminalView>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        write_to_pty_events_rx: Receiver<Vec<u8>>,
        initial_load_mode: SharedSessionInitialLoadMode,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (ws_proxy_tx, ws_proxy_rx) = async_channel::unbounded();
        let (selection_throttled_tx, selection_rx) = async_channel::unbounded();
        let selection_throttled_rx = throttle(SELECTION_THROTTLE_PERIOD, selection_rx);
        let heartbeat = ctx.add_model(|_| Heartbeat::default());
        ctx.subscribe_to_model(&heartbeat, Self::handle_heartbeat_event);

        let model = Network {
            heartbeat,
            session_id,
            event_loop: None,
            ws_proxy_tx,
            #[cfg(test)]
            ws_proxy_rx: ws_proxy_rx.clone(),
            channel_event_proxy,
            terminal_model,
            initial_load_mode,
            terminal_view,
            stage: Stage::BeforeJoined,
            id: None,
            cached_latest_state: CachedLatestState {
                selection: Selection::None,
                universal_developer_input_context: None,
            },
            selection_throttled_tx,
            selection_event_no: EventNumber::new(),
            next_buffer_seq_no: (BlockId::new(), InputOperationSeqNo::zero()),
            write_to_pty_event_no: WriteToPtySeqNo::zero(),
            pty_bytes_batch_status: PtyBytesBatchStatus::NotBatching {
                last_sent_at: Instant::now(),
            },
        };

        model.start_write_to_pty_events_listener(write_to_pty_events_rx, ctx);
        model.start_websocket(session_id, ws_proxy_rx, ctx);
        ctx.spawn_stream_local(
            selection_throttled_rx,
            |network, selection, _ctx| {
                let event_no = network.selection_event_no.advance();
                network.send_message_to_server(UpstreamMessage::UpdateSelection(SelectionUpdate {
                    selection,
                    event_no: event_no.into(),
                }));
            },
            |_, _| {},
        );
        model
    }

    /// Creates a model that artifically declares that a shared session has been joined.
    #[cfg(test)]
    pub fn new_for_test(
        channel_event_proxy: ChannelEventListener,
        terminal_view: WeakViewHandle<TerminalView>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        write_to_pty_events_rx: Receiver<Vec<u8>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        use session_sharing_protocol::common::SessionId;

        let (ws_proxy_tx, ws_proxy_rx) = async_channel::unbounded();
        let (selection_throttled_tx, selection_rx) = async_channel::unbounded();
        let selection_throttled_rx = throttle(SELECTION_THROTTLE_PERIOD, selection_rx);
        let heartbeat = ctx.add_model(|_| Heartbeat::default());
        ctx.subscribe_to_model(&heartbeat, Self::handle_heartbeat_event);

        let session_id = SessionId::new();
        let viewer_id = ParticipantId::new();
        let viewer_firebase_uid = UserUid::new("mock_firebase_uid");
        let active_prompt = ActivePrompt::WarpPrompt("test warp prompt".to_owned());

        let model = Network {
            heartbeat,
            session_id,
            event_loop: None,
            ws_proxy_tx,
            ws_proxy_rx,
            channel_event_proxy,
            terminal_model,
            initial_load_mode: SharedSessionInitialLoadMode::ReplaceFromSessionScrollback,
            terminal_view,
            stage: Stage::BeforeJoined,
            id: Some(viewer_id.clone()),
            cached_latest_state: CachedLatestState {
                selection: Selection::None,
                universal_developer_input_context: None,
            },
            selection_throttled_tx,
            selection_event_no: EventNumber::new(),
            next_buffer_seq_no: (BlockId::new(), InputOperationSeqNo::zero()),
            write_to_pty_event_no: WriteToPtySeqNo::zero(),
            pty_bytes_batch_status: PtyBytesBatchStatus::NotBatching {
                last_sent_at: Instant::now(),
            },
        };

        ctx.emit(NetworkEvent::JoinedSuccessfully {
            active_prompt,
            viewer_id,
            viewer_firebase_uid,
            participant_list: Default::default(),
            input_replica_id: ReplicaId::random(),
            universal_developer_input_context: None,
            source_type: SessionSourceType::default(),
        });

        model.start_write_to_pty_events_listener(write_to_pty_events_rx, ctx);
        ctx.spawn_stream_local(
            selection_throttled_rx,
            |network, selection, _ctx| {
                let event_no = network.selection_event_no.advance();
                network.send_message_to_server(UpstreamMessage::UpdateSelection(SelectionUpdate {
                    selection,
                    event_no: event_no.into(),
                }));
            },
            |_, _| {},
        );
        model
    }

    /// We need to ensure we're maintaining a heartbeat with the server.
    /// This helps us detect if the server has gone away silently and helps
    /// the server detect if we (the client) have disconnected quietly.
    fn handle_heartbeat_event(&mut self, event: &HeartbeatEvent, ctx: &mut ModelContext<Self>) {
        match event {
            HeartbeatEvent::Ping => {
                self.send_message_to_server(UpstreamMessage::Ping { data: vec![] });
            }
            HeartbeatEvent::Idle => {
                log::info!("Viewer reconnecting: heartbeat idle timeout");
                self.reconnect_websocket(ctx);
            }
        }
    }

    async fn get_user_id(
        auth_client: Arc<dyn AuthClient>,
        auth_state: &AuthState,
    ) -> anyhow::Result<UserID> {
        let user_id = UserID {
            anonymous_id: auth_state.anonymous_id(),
            access_token: auth_client
                .get_or_refresh_access_token()
                .await
                .ok()
                .and_then(|token| token.bearer_token()),
        };
        anyhow::Ok(user_id)
    }

    async fn connect_websocket_and_get_user_id(
        session_id: SessionId,
        auth_client: Arc<dyn AuthClient>,
        auth_state: Arc<AuthState>,
    ) -> anyhow::Result<((impl Sink, impl Stream), UserID)> {
        let Some(join_endpoint) = connect_endpoint(format!("/sessions/join/{session_id}")) else {
            bail!("This channel does not support session-sharing.");
        };
        let user_id = Self::get_user_id(auth_client, &auth_state).await?;
        let socket = websocket::WebSocket::connect(join_endpoint, None /* protocols */).await?;
        anyhow::Ok(((socket.split().await), user_id))
    }

    fn on_websocket_connected(
        &mut self,
        ws_proxy_rx: async_channel::Receiver<UpstreamMessage>,
        mut sink: impl Sink,
        stream: impl Stream,
        ctx: &mut ModelContext<Self>,
    ) {
        self.heartbeat.update(ctx, |heartbeat, ctx| {
            heartbeat.start(ctx);
        });

        // Receive messages from the server.
        ctx.spawn_stream_local(
            stream,
            |network, item, ctx| match item {
                Ok(message) => {
                    network.heartbeat.update(ctx, |heartbeat, ctx| {
                        heartbeat.reset_idle_timeout(ctx);
                    });
                    network.process_websocket_message(message, ctx);
                }
                Err(e) => {
                    log::error!("Got error from shared session viewer websocket: {e}");
                }
            },
            |network, ctx| {
                log::info!("Websocket to session sharing server ended");
                // Close our current websocket proxy, because we may try to reconnect and that will create a new websocket proxy.
                // This must be done before trying to reconnect.
                network.close();
                if matches!(network.stage, Stage::JoinedSuccessfully) {
                    // The connection may have timed out or the server restarted.
                    log::info!("Viewer reconnecting: websocket closed by server");
                    network.reconnect_websocket(ctx);
                }
            },
        );

        // Send messages back up the websocket to the server.
        ctx.spawn(async move {
            let mut ws_proxy_rx = pin!(ws_proxy_rx);
            while let Some(message) = ws_proxy_rx.next().await {
                let serialized = message.to_json();
                match serialized {
                    Ok(serialized) => {
                        if let Err(e) = sink.send(Message::new(serialized)).await {
                            log::warn!("Failed to send message over shared session websocket: {e}");
                            break;
                        }
                    }
                    Err(e) => log::warn!("Failed to serialize message to send over shared session websocket: {e}")
                }
            }
            log::info!("Closing websocket to session sharing server as viewer");
            if let Err(e) = sink.close().await {
                log::error!("Failed to close session sharing websocket due to {e}");
            }
        }, |_, _, _| {});
    }

    fn start_websocket(
        &self,
        session_id: SessionId,
        ws_proxy_rx: async_channel::Receiver<UpstreamMessage>,
        ctx: &mut ModelContext<Self>,
    ) {
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        // Open a websocket to the server to join the session.
        ctx.spawn(
            Self::connect_websocket_and_get_user_id(session_id, auth_client, auth_state.clone()),
            |network, conn, ctx| match conn {
                Ok(((sink, stream), user_id)) => {
                    let initialize_message = UpstreamMessage::Initialize(InitPayload {
                        viewer_id: network.id.clone(),
                        user_id,
                        last_received_event_no: None,
                        latest_block_id: None,
                        telemetry_context: Some(TelemetryContext(telemetry_context().as_value())),
                        feature_support: FeatureSupport {
                            supports_agent_view: FeatureFlag::AgentView.is_enabled(),
                            supports_full_role: true,
                            supports_full_role_for_real: true,
                        },
                    });
                    if let Err(e) = network.ws_proxy_tx.try_send(initialize_message) {
                        log::error!("Failed to send initialize message for viewer: {e}");
                        return;
                    }

                    network.on_websocket_connected(ws_proxy_rx, sink, stream, ctx)
                }
                Err(e) => {
                    log::error!("Failed to join shared session: {e}");
                    ctx.emit(NetworkEvent::FailedToJoin {
                        reason: FailedToJoinReason::FailedToConnectToServer,
                    });
                }
            },
        );
    }

    /// Initiates attempts to reconnect to the server, with retries.
    /// Successfully connecting does not mean we joined the session successfully.
    /// We must wait for DownstreamMessage::JoinedSuccessfully to confirm that.
    /// We also will not initiate an attempt if the session has been explicitly ended or
    /// is already attempting to reconnect.
    pub fn reconnect_websocket(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(self.stage, Stage::Finished | Stage::Reconnecting { .. }) {
            return;
        }
        let Some(event_loop) = self.event_loop.clone() else {
            log::error!("Cannot reconnect to server as viewer when event loop does not exist");
            return;
        };
        let session_id = self.session_id;
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let abort_handle = ctx.spawn_with_retry_on_error(
            move || {
                log::info!("Attempting to reconnect to session sharing server as viewer");
                Self::connect_websocket_and_get_user_id(session_id, auth_client.clone(), auth_state.clone())
            },
            RECONNECT_RETRY_STRATEGY,
            move |network, conn, ctx| match conn {
                RequestState::RequestSucceeded(((sink, stream), user_id)) => {
                    log::info!("Successfully reconnected to server as viewer");
                    let last_received_event_no = event_loop.as_ref(ctx).last_received_event_no();
                    let latest_block_id = network.terminal_model.lock().block_list().active_block_id().clone();
                    let initialize_message = UpstreamMessage::Initialize(InitPayload {
                        viewer_id: network.id.clone(),
                        user_id,
                        last_received_event_no,
                        latest_block_id: Some(latest_block_id.into()),
                        telemetry_context: Some(TelemetryContext(telemetry_context().as_value())),
                        feature_support: FeatureSupport {
                            supports_agent_view: FeatureFlag::AgentView.is_enabled(),
                            supports_full_role: true,
                            supports_full_role_for_real: true,
                        },
                    });
                    let (ws_proxy_tx, ws_proxy_rx) = async_channel::unbounded();
                    network.ws_proxy_tx = ws_proxy_tx;
                    if let Err(e) = network.ws_proxy_tx.try_send(initialize_message) {
                        log::error!("Failed to send initialize message for viewer when reconnecting: {e}");
                        return;
                    }

                    network.on_websocket_connected(ws_proxy_rx, sink, stream, ctx)
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to reconnect to shared session as viewer, will retry: {e}");
                }
                RequestState::RequestFailed(e) => {
                    log::warn!(
                        "Failed to reconnect to shared session as viewer, and retries exhausted: {e}"
                    );
                    network.close_without_reconnection();
                    ctx.emit(NetworkEvent::FailedToReconnect);
                }
            },
        ).abort_handle();
        ctx.emit(NetworkEvent::Reconnecting);
        self.stage = Stage::Reconnecting { abort_handle };
    }

    /// Fetches the new user id and reconnectes to the websocket.
    pub fn reauthenticate_viewer(&mut self, ctx: &mut ModelContext<Self>) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        ctx.spawn(
            async move { Self::get_user_id(server_api, &auth_state).await },
            |network, res, ctx| match res {
                Ok(user_id) => {
                    let message = UpstreamMessage::Reauthenticated { user_id };
                    network.send_message_to_server(message);
                    log::info!("Viewer reconnecting: reauthentication completed");
                    network.reconnect_websocket(ctx);
                }
                Err(e) => {
                    log::warn!("Failed to reauthenticate viewer: {e}");
                }
            },
        );
    }

    fn process_websocket_message(&mut self, message: Message, ctx: &mut ModelContext<Self>) {
        let Some(msg) = message
            .text()
            .and_then(|t| DownstreamMessage::from_json(t).ok())
        else {
            log::warn!("Got unexpected message from shared session viewer websocket");
            return;
        };
        match msg {
            DownstreamMessage::JoinedSuccessfully {
                scrollback,
                latest_event_no,
                active_prompt,
                window_size,
                participant_list,
                viewer_id,
                viewer_firebase_uid,
                input_replica_id,
                universal_developer_input_context,
                // We use the more detailed source type here,
                // ignoring the legacy source_type field (which was kept around for backwards compatibility).
                detailed_source_type: source_type,
                ..
            } => {
                if matches!(self.stage, Stage::JoinedSuccessfully) {
                    log::warn!(
                        "Received unexpected JoinedSuccessfully message when we've already joined"
                    );
                    return;
                }
                log::info!("Successfully joined shared session.");
                self.id = Some(viewer_id.clone());
                self.stage = Stage::JoinedSuccessfully;

                // Initialize the cache with the server's initial context so that subsequent
                // local changes are correctly compared against what the server knows.
                self.cached_latest_state.universal_developer_input_context =
                    universal_developer_input_context.clone();

                // Create the event loop now that we've joined and are ready to receive events from the server.
                let event_loop = ctx.add_model(|ctx| {
                    EventLoop::new(
                        self.terminal_model.clone(),
                        self.terminal_view.clone(),
                        self.channel_event_proxy.clone(),
                        window_size,
                        *scrollback,
                        latest_event_no,
                        self.initial_load_mode,
                        ctx,
                    )
                });
                self.event_loop = Some(event_loop);
                ctx.emit(NetworkEvent::JoinedSuccessfully {
                    active_prompt,
                    viewer_id,
                    viewer_firebase_uid: UserUid::new(viewer_firebase_uid.as_str()),
                    participant_list: Box::new(*participant_list),
                    input_replica_id: input_replica_id.into(),
                    universal_developer_input_context,
                    source_type,
                });
            }
            DownstreamMessage::RejoinedSuccessfully { participant_list } => {
                if matches!(self.stage, Stage::JoinedSuccessfully) {
                    log::warn!("Received unexpected RejoinedSuccessfully message when we've already joined");
                    return;
                }
                log::info!("Successfully reconnected to shared session as viewer.");
                self.stage = Stage::JoinedSuccessfully;
                // Events where we only care about the latest value were dropped before we reconnected.
                self.send_latest_state_to_server();
                ctx.emit(NetworkEvent::ReconnectedSuccessfully);
                ctx.emit(NetworkEvent::ParticipantListUpdated(Box::new(
                    *participant_list,
                )));
            }
            DownstreamMessage::OrderedTerminalEvent(event) => {
                if let Some(event_loop) = &self.event_loop {
                    event_loop.update(ctx, |event_loop, ctx| {
                        event_loop.process_ordered_terminal_event(event, ctx);
                    })
                } else {
                    log::error!(
                        "Received OrderedTerminalEvent before event_loop was initialized. This can mean events were dropped."
                    );
                }
            }
            DownstreamMessage::SessionEnded { reason } => {
                self.close_without_reconnection();
                ctx.emit(NetworkEvent::SessionEnded { reason });
            }
            DownstreamMessage::ViewerRemoved { reason } => {
                self.close_without_reconnection();
                ctx.emit(NetworkEvent::ViewerRemoved { reason })
            }
            DownstreamMessage::ActivePromptUpdated(active_prompt_update) => {
                ctx.emit(NetworkEvent::SharerActivePromptUpdated(
                    active_prompt_update,
                ));
            }
            DownstreamMessage::UniversalDeveloperInputContextUpdated(context_update) => {
                // Update our cache to stay in sync with what the server knows.
                self.apply_context_update_to_cache(context_update.clone());
                ctx.emit(NetworkEvent::UniversalDeveloperInputContextUpdated(
                    context_update,
                ));
            }
            DownstreamMessage::FailedToJoin { reason } => {
                log::warn!("Failed to join shared session: {reason:?}");

                if let Stage::Reconnecting { abort_handle } = &self.stage {
                    abort_handle.abort();
                    self.close_without_reconnection();
                    ctx.emit(NetworkEvent::FailedToReconnect);
                } else {
                    ctx.emit(NetworkEvent::FailedToJoin {
                        reason: reason.into(),
                    })
                }
            }
            DownstreamMessage::ParticipantListUpdated(participant_list) => {
                ctx.emit(NetworkEvent::ParticipantListUpdated(Box::new(
                    participant_list,
                )));
            }
            DownstreamMessage::ParticipantPresenceUpdated(update) => {
                ctx.emit(NetworkEvent::ParticipantPresenceUpdated(update));
            }
            DownstreamMessage::RoleRequestInFlight(role_request_id) => {
                ctx.emit(NetworkEvent::RoleRequestInFlight(role_request_id));
            }
            DownstreamMessage::RoleRequestResponse(role_request_response) => {
                ctx.emit(NetworkEvent::RoleRequestResponse(role_request_response));
            }
            DownstreamMessage::ParticipantRoleChanged {
                participant_id,
                reason,
                role,
            } => {
                ctx.emit(NetworkEvent::ParticipantRoleChanged {
                    participant_id,
                    reason,
                    role,
                });
            }
            DownstreamMessage::InputUpdated(update) => {
                // Deserialize the operations, failing if any of the operations can't be deserialized.
                let operations = update
                    .ops
                    .into_iter()
                    .map(|o| serde_json::from_slice(o.0.as_slice()))
                    .collect();
                let operations = match operations {
                    Ok(operations) => operations,
                    Err(e) => {
                        log::warn!("Failed to deserialize CRDT operations from server: {e}");
                        return;
                    }
                };

                ctx.emit(NetworkEvent::InputUpdated {
                    block_id: update.id.buffer_id.into(),
                    operations,
                });
            }
            DownstreamMessage::InputUpdateRejected { .. } => {
                // TODO
            }
            DownstreamMessage::CommandExecutionRequestInFlight { .. } => {
                // TODO
            }
            DownstreamMessage::CommandExecutionRequestFailed { reason, .. } => {
                ctx.emit(NetworkEvent::CommandExecutionRequestFailed { reason });
            }
            DownstreamMessage::AgentPromptRequestInFlight(id) => {
                ctx.emit(NetworkEvent::AgentPromptRequestInFlight(id));
            }
            DownstreamMessage::AgentPromptRequestFailed { reason } => {
                ctx.emit(NetworkEvent::AgentPromptRequestFailed { reason });
            }
            DownstreamMessage::WriteToPtyRequestFailed { reason } => {
                ctx.emit(NetworkEvent::WriteToPtyRequestFailed { reason });
            }
            DownstreamMessage::ControlActionRequestFailed { reason } => {
                ctx.emit(NetworkEvent::ControlActionRequestFailed { reason });
            }
            DownstreamMessage::LinkAccessLevelUpdated { role } => {
                ctx.emit(NetworkEvent::LinkAccessLevelUpdated { role });
            }
            // We don't use the `team_uid` field yet because currently, sessions
            // can only have a single team ACL.
            DownstreamMessage::TeamAccessLevelUpdated { team_acl, .. } => {
                ctx.emit(NetworkEvent::TeamAccessLevelUpdated { team_acl });
            }
            DownstreamMessage::LinkAccessLevelUpdateResponse(response) => {
                ctx.emit(NetworkEvent::LinkAccessLevelUpdateResponse { response });
            }
            DownstreamMessage::AddGuestsResponse(response) => {
                ctx.emit(NetworkEvent::AddGuestsResponse { response });
            }
            DownstreamMessage::RemoveGuestResponse(response) => {
                ctx.emit(NetworkEvent::RemoveGuestResponse { response });
            }
            DownstreamMessage::UpdatePendingUserRoleResponse(response) => {
                ctx.emit(NetworkEvent::UpdatePendingUserRoleResponse { response });
            }
            DownstreamMessage::TeamAccessLevelUpdateResponse(response) => {
                ctx.emit(NetworkEvent::TeamAccessLevelUpdateResponse { response });
            }
            DownstreamMessage::Pong { .. } => {}
        }
    }

    /// Start a process to listen for and batch pty write events.
    fn start_write_to_pty_events_listener(
        &self,
        events_rx: Receiver<Vec<u8>>,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn_stream_local(
            events_rx,
            move |network, bytes, ctx| {
                match &mut network.pty_bytes_batch_status {
                    PtyBytesBatchStatus::NotBatching { last_sent_at } => {
                        // Start batching
                        let next_send_time = last_sent_at
                            .checked_add(PTY_WRITES_BATCH_THRESHOLD)
                            .expect("Can add durations");
                        let wait_time = next_send_time.saturating_duration_since(Instant::now());
                        let abort_handle = ctx.spawn_abortable(
                            async move {
                                Timer::after(wait_time).await;
                            },
                            |network, _, _| {
                                network.send_write_to_pty();
                            },
                            |_, _| {},
                        );

                        // Update the batch status and initialize the accumulated bytes with the current write event.
                        network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                            accumulated: bytes,
                            abort_handle,
                        };
                    }
                    PtyBytesBatchStatus::Batching { accumulated, .. } => {
                        accumulated.extend(bytes);
                    }
                }
            },
            |_network, _ctx| {},
        );
    }

    /// Close the websocket to the session-sharing-server.
    pub fn close(&mut self) {
        if let Stage::Reconnecting { abort_handle } = &self.stage {
            abort_handle.abort();
        }
        // Closing this channel will close the websocket.
        self.ws_proxy_tx.close();
    }

    /// Close the websocket and don't try to reconnect.
    pub fn close_without_reconnection(&mut self) {
        self.close();
        self.stage = Stage::Finished;
    }

    fn send_message_to_server(&self, message: UpstreamMessage) {
        let Stage::JoinedSuccessfully = self.stage else {
            return;
        };
        if let Err(e) = self.ws_proxy_tx.try_send(message) {
            log::warn!("Failed to send message over ws_proxy channel in viewer network: {e}");
        }
    }

    /// Send the presence selection to the server if it changed, with a throttle period.
    pub fn send_presence_selection_if_changed(&mut self, selection: Selection) {
        if selection == self.cached_latest_state.selection {
            return;
        }

        self.send_presence_selection(selection);
    }

    /// Send the presence selection to the server, with a throttle period.
    pub fn send_presence_selection(&mut self, selection: Selection) {
        self.cached_latest_state.selection = selection.clone();
        if let Err(e) = self.selection_throttled_tx.try_send(selection) {
            log::warn!(
                "Failed to send message over selection_throttled_tx channel in viewer network: {e}"
            );
        }
    }

    pub fn send_input_update<'a>(
        &mut self,
        block_id: &BlockId,
        operations: impl Iterator<Item = &'a CrdtOperation>,
    ) {
        let Some(viewer_id) = self.id.clone() else {
            return;
        };

        // Set the right block ID. The block IDs that we call this function
        // with are monotonically increasing.
        if block_id != &self.next_buffer_seq_no.0 {
            self.next_buffer_seq_no = (block_id.to_owned(), InputOperationSeqNo::zero());
        }

        let operations = operations
            .map(|o| serde_json::to_vec(o).map(session_sharing_protocol::common::CrdtOperation))
            .collect();

        let ops = match operations {
            Ok(operations) => operations,
            Err(e) => {
                log::warn!("Failed to serialize CRDT operations to send to server: {e}");
                return;
            }
        };

        let id = InputOperationId {
            participant_id: viewer_id,
            buffer_id: block_id.to_owned().into(),
            op_no: self.next_buffer_seq_no.1,
        };
        self.next_buffer_seq_no.1.advance();

        self.send_message_to_server(UpstreamMessage::UpdateInput(InputUpdate { id, ops }));
    }

    pub fn send_write_to_pty(&mut self) {
        let Some(viewer_id) = self.id.clone() else {
            return;
        };

        if let PtyBytesBatchStatus::Batching {
            accumulated,
            abort_handle,
        } = &self.pty_bytes_batch_status
        {
            abort_handle.abort();

            // Flush the accumulated PTY writes into a single [`WriteToPty`] request
            let request_id = WriteToPtyRequestId {
                participant_id: viewer_id,
                op_no: self.write_to_pty_event_no,
            };
            self.write_to_pty_event_no.advance();
            let message = UpstreamMessage::WriteToPty {
                request_id,
                bytes: accumulated.to_vec(),
            };
            self.send_message_to_server(message);

            // Update batch status
            self.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: Instant::now(),
            };
        }
    }

    pub fn send_command_execution_request(&mut self, block_id: &BlockId, command: String) {
        let buffer_id = block_id.to_owned().into();
        self.send_message_to_server(UpstreamMessage::ExecuteCommand { buffer_id, command });
    }

    pub fn send_agent_prompt_request(
        &mut self,
        server_conversation_token: Option<ServerConversationToken>,
        prompt: String,
        attachments: Vec<AgentAttachment>,
    ) {
        let request = AgentPromptRequest {
            id: AgentPromptRequestId::new(),
            server_conversation_token,
            prompt,
            attachments,
        };
        self.send_message_to_server(UpstreamMessage::SendAgentPrompt(request));
    }

    pub fn send_cancel_control_action(
        &mut self,
        server_conversation_token: ServerConversationToken,
    ) {
        let action = ControlAction::CancelConversation {
            server_conversation_token,
        };
        self.send_message_to_server(UpstreamMessage::SendControlAction(action));
    }

    pub fn send_link_permission_update(&mut self, role: Option<Role>) {
        self.send_message_to_server(UpstreamMessage::UpdateLinkAccessLevel { role });
    }

    pub fn send_team_permission_update(&mut self, role: Option<Role>, team_uid: String) {
        self.send_message_to_server(UpstreamMessage::UpdateTeamAccessLevel { team_uid, role });
    }

    pub fn send_add_guests(&mut self, emails: Vec<String>, role: Role) {
        self.send_message_to_server(UpstreamMessage::AddGuests { emails, role });
    }

    pub fn send_remove_guest(&mut self, user_uid: UserUid) {
        self.send_message_to_server(UpstreamMessage::RemoveGuest {
            user_uid: user_uid.as_string(),
        });
    }

    pub fn send_remove_pending_guest(&mut self, email: String) {
        self.send_message_to_server(UpstreamMessage::RemovePendingGuest { email });
    }

    pub fn send_user_role_update(&mut self, user_uid: UserUid, role: Role) {
        self.send_message_to_server(UpstreamMessage::UpdateUserRole {
            user_uid: user_uid.as_string(),
            role,
        });
    }

    pub fn send_pending_user_role_update(&mut self, email: String, role: Role) {
        self.send_message_to_server(UpstreamMessage::UpdatePendingUserRole { email, role });
    }

    pub fn send_report_terminal_size(&mut self, window_size: WindowSize) {
        self.send_message_to_server(UpstreamMessage::ReportTerminalSize { window_size });
    }

    /// Send everything in `self.cached_latest_state` to the server.
    /// This is needed when we reconnect to the server, since all values were dropped before we were connected.
    fn send_latest_state_to_server(&mut self) {
        self.send_presence_selection(self.cached_latest_state.selection.clone())
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.stage, Stage::JoinedSuccessfully)
    }

    pub fn send_role_request(&mut self, role: Role) {
        let message = UpstreamMessage::RequestRole(role);
        self.send_message_to_server(message);
    }

    pub fn send_cancel_role_request(&mut self, role_request_id: RoleRequestId) {
        let message = UpstreamMessage::CancelRoleRequest(role_request_id);
        self.send_message_to_server(message);
    }

    pub fn send_universal_developer_input_context_update(
        &mut self,
        update: UniversalDeveloperInputContextUpdate,
    ) {
        // Skip update if nothing would change
        if let Some(ref cached) = self.cached_latest_state.universal_developer_input_context {
            if !update.changes_cached_context(cached) {
                return;
            }
        }

        self.apply_context_update_to_cache(update.clone());
        self.send_message_to_server(UpstreamMessage::UpdateUniversalDeveloperInputContext(
            update,
        ));
    }

    /// Merges an update into the cached context.
    fn apply_context_update_to_cache(&mut self, update: UniversalDeveloperInputContextUpdate) {
        let current = self
            .cached_latest_state
            .universal_developer_input_context
            .take()
            .unwrap_or_default();

        self.cached_latest_state.universal_developer_input_context =
            Some(update.merge_into(current));
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }
}

#[derive(Debug)]
pub enum FailedToJoinReason {
    Unknown,
    FailedToConnectToServer,
    SessionNotFound,
    WrongPassword,
    MaxNumberOfParticipantsReached,
    SessionNotAccessible,
}

impl FailedToJoinReason {
    /// This error message will be displayed to the user.
    pub fn user_facing_error_message(&self) -> &str {
        match self {
            FailedToJoinReason::Unknown => "Failed to join shared session.",
            FailedToJoinReason::FailedToConnectToServer => {
                "Failed to connect. Please try again later."
            }
            FailedToJoinReason::SessionNotFound => "Shared session not found.",
            FailedToJoinReason::WrongPassword => "Invalid session sharing link.",
            FailedToJoinReason::MaxNumberOfParticipantsReached => {
                "The maximum number of participants for this shared session has been reached."
            }
            FailedToJoinReason::SessionNotAccessible => "You don't have access to this link.",
        }
    }
}

impl From<session_sharing_protocol::viewer::FailedToJoinReason> for FailedToJoinReason {
    fn from(reason: session_sharing_protocol::viewer::FailedToJoinReason) -> Self {
        match reason {
            session_sharing_protocol::viewer::FailedToJoinReason::Invalid |
            session_sharing_protocol::viewer::FailedToJoinReason::InternalServerError => Self::Unknown,
            session_sharing_protocol::viewer::FailedToJoinReason::SessionNotFound => Self::SessionNotFound,
            session_sharing_protocol::viewer::FailedToJoinReason::WrongPassword => Self::WrongPassword,
            session_sharing_protocol::viewer::FailedToJoinReason::MaxNumberOfParticipantsReached => Self::MaxNumberOfParticipantsReached,
            session_sharing_protocol::viewer::FailedToJoinReason::SessionNotAccessible => Self::SessionNotAccessible,
        }
    }
}

/// Converts SessionEndedReason to a user-facing string
pub fn session_ended_reason_string(reason: &SessionEndedReason) -> String {
    match reason {
        SessionEndedReason::InternalServerError => {
            "Something went wrong. Please ask sharer to reshare to continue.".to_owned()
        }
        SessionEndedReason::InactivityLimitReached => {
            "Sharing ended due to sharer inactivity".to_owned()
        }
        _ => "Session ended.".to_owned(),
    }
}

pub fn viewer_removed_reason_string(reason: &ViewerRemovedReason) -> String {
    match reason {
        ViewerRemovedReason::LostAccess => {
            "Your access to the session was removed. Please ask sharer to reshare to continue."
                .to_owned()
        }
    }
}

/// Converts CommandExecutionFailureReason to a user-facing string
pub fn command_execution_failure_reason_string(reason: &CommandExecutionFailureReason) -> String {
    match reason {
        CommandExecutionFailureReason::InsufficientPermissions => {
            "Insufficient permissions. Please request edit access.".to_owned()
        }
        _ => "Failed to execute command. Please try again.".to_owned(),
    }
}

/// Converts WriteToPtyFailureReason to a user-facing string
pub fn write_to_pty_failure_reason_string(reason: &WriteToPtyFailureReason) -> String {
    match reason {
        WriteToPtyFailureReason::InsufficientPermissions => {
            "Insufficient permissions. Please request edit access.".to_owned()
        }
        _ => "Failed to make edit. Please try again.".to_owned(),
    }
}

/// Converts AgentPromptFailureReason to a user-facing string
pub fn agent_prompt_failure_reason_string(reason: &AgentPromptFailureReason) -> String {
    match reason {
        AgentPromptFailureReason::InsufficientPermissions => {
            "Insufficient permissions. Please request edit access.".to_owned()
        }
        AgentPromptFailureReason::InvalidConversation => {
            "Invalid conversation. Please try again.".to_owned()
        }
        AgentPromptFailureReason::CommandInProgress => {
            "A long running command is currently in progress. Please wait for it to complete before sending an agent prompt.".to_owned()
        }
    }
}

/// Converts ControlActionFailureReason to a user-facing string
pub fn control_action_failure_reason_string(reason: &ControlActionFailureReason) -> String {
    match reason {
        ControlActionFailureReason::InsufficientPermissions => {
            "Insufficient permissions. Please request edit access.".to_owned()
        }
        _ => "Failed to perform action. Please try again.".to_owned(),
    }
}

pub enum NetworkEvent {
    JoinedSuccessfully {
        active_prompt: ActivePrompt,
        viewer_id: ParticipantId,
        viewer_firebase_uid: UserUid,
        participant_list: Box<ParticipantList>,
        input_replica_id: ReplicaId,
        universal_developer_input_context: Option<UniversalDeveloperInputContext>,
        source_type: SessionSourceType,
    },
    FailedToJoin {
        reason: FailedToJoinReason,
    },
    FailedToReconnect,
    SessionEnded {
        reason: SessionEndedReason,
    },
    SharerActivePromptUpdated(ActivePromptUpdate),
    UniversalDeveloperInputContextUpdated(UniversalDeveloperInputContextUpdate),
    Reconnecting,
    ParticipantListUpdated(Box<ParticipantList>),
    ParticipantPresenceUpdated(ParticipantPresenceUpdate),
    ReconnectedSuccessfully,
    ParticipantRoleChanged {
        participant_id: ParticipantId,
        reason: RoleUpdatedReason,
        role: Role,
    },
    InputUpdated {
        block_id: BlockId,
        operations: Vec<CrdtOperation>,
    },
    RoleRequestInFlight(RoleRequestId),
    RoleRequestResponse(RoleRequestResponse),
    CommandExecutionRequestFailed {
        reason: CommandExecutionFailureReason,
    },
    AgentPromptRequestInFlight(AgentPromptRequestId),
    AgentPromptRequestFailed {
        reason: AgentPromptFailureReason,
    },
    WriteToPtyRequestFailed {
        reason: WriteToPtyFailureReason,
    },
    ControlActionRequestFailed {
        reason: ControlActionFailureReason,
    },
    ViewerRemoved {
        reason: ViewerRemovedReason,
    },
    LinkAccessLevelUpdated {
        role: Option<Role>,
    },
    TeamAccessLevelUpdated {
        team_acl: Option<TeamAclData>,
    },
    LinkAccessLevelUpdateResponse {
        response: LinkAccessLevelUpdateResponse,
    },
    AddGuestsResponse {
        response: AddGuestsResponse,
    },
    RemoveGuestResponse {
        response: RemoveGuestResponse,
    },
    UpdatePendingUserRoleResponse {
        response: UpdatePendingUserRoleResponse,
    },
    TeamAccessLevelUpdateResponse {
        response: TeamAccessLevelUpdateResponse,
    },
}

impl Entity for Network {
    type Event = NetworkEvent;
}

impl Drop for Network {
    fn drop(&mut self) {
        self.close();
        // We keep the same selection_throttled_tx even if we reconnect and replace the internal ws_proxy_tx,
        // which is why we don't close it as part of [`Self::close`]
        self.selection_throttled_tx.close();
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
