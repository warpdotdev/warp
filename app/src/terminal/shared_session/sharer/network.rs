//! The "sender" of a shared session represents the sharer's end.
//!
//! Currently there is no way to share a session from wasm.
#![cfg_attr(target_family = "wasm", allow(dead_code))]

use std::collections::HashMap;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Receiver;
use byte_unit::{Byte, UnitType};
use futures_util::stream::AbortHandle;
use futures_util::{SinkExt, StreamExt};
use instant::Instant;
use parking_lot::FairMutex;
use session_sharing_protocol::common::{
    ActivePrompt, ActivePromptUpdate, AgentPromptFailureReason, AgentPromptRequest,
    AgentPromptRequestId, CommandExecutionFailureReason, CommandExecutionRequestId, ControlAction,
    ControlActionFailureReason, ControlActionRequestId, FeatureSupport, InputOperationId,
    InputOperationSeqNo, InputUpdate, OrderedTerminalEvent, OrderedTerminalEventType,
    ParticipantId, ParticipantList, ParticipantPresenceUpdate, Role, RoleRequestId,
    RoleRequestResponse, Selection, SelectionUpdate, SessionId, UniversalDeveloperInputContext,
    UniversalDeveloperInputContextUpdate, UserID, WindowSize, WriteToPtyFailureReason,
    WriteToPtyRequestId,
};
use session_sharing_protocol::sharer::{
    AddGuestsResponse, DownstreamMessage, FailedToAddGuestsReason, FailedToInitializeSessionReason,
    LinkAccessLevelUpdateResponse, ReconnectPayload, ReconnectToken, RemoveGuestResponse,
    RoleUpdateReason, SessionEndedReason, SessionSourceType, SessionTerminatedReason,
    TeamAccessLevelUpdateResponse, UpdatePendingUserRoleResponse, UpstreamMessage,
};
use warp_core::features::FeatureFlag;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, ModelHandle, RequestState, RetryOption, SingletonEntity};
use websocket::{Message, Sink, Stream, WebSocket, WebsocketMessage as _};
#[cfg(not(any(test, feature = "integration_tests")))]
use {
    crate::{report_error, server::telemetry::telemetry_context},
    session_sharing_protocol::common::{Scrollback, TelemetryContext},
    session_sharing_protocol::sharer::{InitPayload, Lifetime},
};

use crate::auth::{AuthStateProvider, UserUid};
use crate::editor::{CrdtOperation, ReplicaId};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::block::BlockId;
use crate::terminal::shared_session::network::heartbeat::{Event as HeartbeatEvent, Heartbeat};
use crate::terminal::shared_session::{
    connect_endpoint, max_session_size, EventNumber, SharedSessionScrollbackType,
    SharedSessionSource, SELECTION_THROTTLE_PERIOD,
};
use crate::terminal::TerminalModel;
use crate::throttle::throttle;

/// The amount of time we will wait to batch consecutive PTY read events before sending an event to the server
const PTY_READS_BATCH_THRESHOLD: Duration = Duration::from_millis(50);
/// Exponential backoff when retrying reconnection. This configuration has us retry for ~128 seconds before giving up,
/// where the last interval between retries is 26s.
/// We should be somewhat generous with the amount of retries allowed when a sharer wants to recover their session,
/// since they have the choice of giving up early by closing the window/stopping sharing.
const RECONNECT_RETRY_STRATEGY: RetryOption = RetryOption::exponential(
    Duration::from_millis(1000), /* interval */
    1.2,                         /* exponential factor */
    18,                          /* max retry count */
)
.with_jitter(0.2);

/// How far along the starting process we are.
#[derive(Debug)]
enum Stage {
    /// The server is not ready to receive messages from us.
    BeforeStarted,
    /// The server is ready to receive messages from us.
    StartedSuccessfully,
    /// The server disconnected after the session was started successfully and we are trying to reconnect.
    Reconnecting { abort_handle: AbortHandle },
    /// The session was ended.
    Finished,
}

enum PtyBytesBatchStatus {
    /// We're not currently batching PTY read events.
    NotBatching {
        /// The last time we sent a batch of PTY read events to the server.
        last_sent_at: Instant,
    },
    /// We're currently batch PTY read events.
    Batching {
        /// The set of PTY bytes accumulated so far.
        accumulated: Vec<u8>,
        /// The abort handle for the batch timer.
        abort_handle: AbortHandle,
    },
}

/// Helper struct to group together the most up to date state that the server needs to know about.
/// Any event we send to the server where we only care about the latest value should be included here.
/// This is used to avoid sending duplicate updates, and to update the server with the latest state on reconnection.
struct CachedLatestState {
    prompt: ActivePrompt,
    selection: Selection,
    universal_developer_input_context: Option<UniversalDeveloperInputContext>,
}

pub struct Network {
    model: Arc<FairMutex<TerminalModel>>,
    stage: Stage,
    heartbeat: ModelHandle<Heartbeat>,

    /// The next event number to use when sending an event to the server.
    event_no: EventNumber,
    /// The next event number to use when sending an presence selection update to the server.
    selection_event_no: EventNumber,
    /// Intermediate channel to queue up messages to send over
    /// over the websocket to the server.
    ws_proxy_tx: async_channel::Sender<UpstreamMessage>,
    /// The number of bytes shared for this session so far.
    num_bytes_shared: Byte,
    max_session_size: Byte,

    pty_bytes_batch_status: PtyBytesBatchStatus,

    // TODO (suraj): figure out how to better structure the
    // Network model for testing so that we don't need stuff like this.
    #[allow(dead_code)]
    ws_proxy_rx: async_channel::Receiver<UpstreamMessage>,

    selection_throttled_tx: async_channel::Sender<Selection>,

    cached_latest_state: CachedLatestState,

    // These fields are Some once we successfully connect and create the shared session.
    session_id: Option<SessionId>,
    reconnect_token: Option<ReconnectToken>,
    sharer_id: Option<ParticipantId>,
    source: Option<SharedSessionSource>,

    /// HashMap from event_no to the event. We keep these in memory to support reconnections
    /// until the server acks that they have been processed and are safe to remove.
    unacked_terminal_events: HashMap<usize, OrderedTerminalEvent>,

    /// The parameters for the next input operation to send.
    next_buffer_seq_no: (BlockId, InputOperationSeqNo),
}

impl Network {
    /// Creates a model that artifically declares that a shared session has been started.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn new_for_test(
        model: Arc<FairMutex<TerminalModel>>,
        ordered_events_rx: Receiver<OrderedTerminalEventType>,
        _scrollback_type: SharedSessionScrollbackType,
        active_prompt: ActivePrompt,
        selection: Selection,
        _input_replica_id: ReplicaId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (ws_proxy_tx, ws_proxy_rx) = async_channel::unbounded();
        let session_id = SessionId::new();
        let (selection_throttled_tx, selection_rx) = async_channel::unbounded();
        let selection_throttled_rx = throttle(SELECTION_THROTTLE_PERIOD, selection_rx);
        let init_block_id = model.lock().block_list().active_block_id().clone();
        let heartbeat = ctx.add_model(|_| Heartbeat::default());
        ctx.subscribe_to_model(&heartbeat, Self::handle_heartbeat_event);

        let network = Network {
            heartbeat,
            event_no: EventNumber::new(),
            selection_event_no: EventNumber::new(),
            model: model.clone(),
            ws_proxy_tx,
            num_bytes_shared: Byte::from_u64(0),
            max_session_size: max_session_size(ctx),
            pty_bytes_batch_status: PtyBytesBatchStatus::NotBatching {
                last_sent_at: Instant::now(),
            },
            ws_proxy_rx,
            selection_throttled_tx,
            cached_latest_state: CachedLatestState {
                prompt: active_prompt,
                selection,
                universal_developer_input_context: None,
            },
            stage: Stage::BeforeStarted,
            session_id: None,
            reconnect_token: None,
            sharer_id: None,
            source: None,
            unacked_terminal_events: HashMap::new(),
            next_buffer_seq_no: (init_block_id, InputOperationSeqNo::zero()),
        };
        let sharer_firebase_uid = UserUid::new("mock_firebase_uid");
        ctx.emit(NetworkEvent::SharedSessionCreatedSuccessfully {
            session_id,
            sharer_id: ParticipantId::new(),
            sharer_firebase_uid,
        });
        network.start_ordered_terminal_events_listener(ordered_events_rx, ctx);
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
        network
    }

    /// Initializes the Network interface for the shared session (creator-side) and
    /// tries to establish a websocket connection against the server.
    #[cfg(not(any(test, feature = "integration_tests")))]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Arc<FairMutex<TerminalModel>>,
        ordered_events_rx: Receiver<OrderedTerminalEventType>,
        scrollback_type: SharedSessionScrollbackType,
        active_prompt: ActivePrompt,
        selection: Selection,
        input_replica_id: ReplicaId,
        terminal_view_id: warpui::EntityId,
        universal_developer_input_context: UniversalDeveloperInputContext,
        lifetime: Lifetime,
        source: SharedSessionSource,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let (ws_proxy_tx, ws_proxy_rx) = async_channel::unbounded();
        let scrollback = scrollback_type.to_scrollback(&model.lock());
        let num_bytes_scrollback = scrollback.num_bytes();
        let max_session_size = max_session_size(ctx);
        let (selection_throttled_tx, selection_rx) = async_channel::unbounded();
        let selection_throttled_rx = throttle(SELECTION_THROTTLE_PERIOD, selection_rx);
        let init_block_id = model.lock().block_list().active_block_id().clone();
        let heartbeat = ctx.add_model(|_| Heartbeat::default());
        ctx.subscribe_to_model(&heartbeat, Self::handle_heartbeat_event);

        let network = Network {
            heartbeat,
            event_no: EventNumber::new(),
            selection_event_no: EventNumber::new(),
            model: model.clone(),
            ws_proxy_tx,
            ws_proxy_rx: ws_proxy_rx.clone(),
            selection_throttled_tx,
            num_bytes_shared: num_bytes_scrollback,
            max_session_size,
            pty_bytes_batch_status: PtyBytesBatchStatus::NotBatching {
                last_sent_at: Instant::now(),
            },
            cached_latest_state: CachedLatestState {
                prompt: active_prompt.clone(),
                selection: selection.clone(),
                universal_developer_input_context: Some(universal_developer_input_context.clone()),
            },
            stage: Stage::BeforeStarted,
            session_id: None,
            reconnect_token: None,
            sharer_id: None,
            source: Some(source.clone()),
            unacked_terminal_events: HashMap::new(),
            next_buffer_seq_no: (init_block_id.clone(), InputOperationSeqNo::zero()),
        };

        let window_size = {
            let size_info = *network.model.lock().block_list().size();
            WindowSize {
                num_rows: size_info.rows(),
                num_cols: size_info.columns(),
            }
        };

        // We should validate the scrollback is under the limit before creating the Network, but check here just to be safe.
        if num_bytes_scrollback > network.max_session_size {
            network.log_diagnostic("local_policy_rejected_start", "reason=scrollback_too_large");
            log::warn!("Session sharing scrollback exceeds max session size; failing startup");
            ctx.emit(NetworkEvent::FailedToCreateSharedSession {
                reason: FailedToInitializeSessionReason::ScrollbackTooLarge {},
                cause: None,
            });
        } else {
            network.start_ordered_terminal_events_listener(ordered_events_rx, ctx);
            network.start_websocket(
                ws_proxy_rx,
                scrollback,
                active_prompt,
                selection,
                window_size,
                init_block_id,
                input_replica_id,
                terminal_view_id,
                universal_developer_input_context,
                lifetime,
                source,
                ctx,
            );
        }
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
        network
    }

    /// Close the websocket to the session-sharing-server.
    fn close(&mut self) {
        if let Stage::Reconnecting { abort_handle, .. } = &self.stage {
            abort_handle.abort();
        }
        // Closing this channel will close the websocket.
        self.ws_proxy_tx.close();
    }

    /// Close the websocket to the session-sharing-server,
    /// and set the stage to Finished to ensure we don't try to reconnect.
    fn close_without_reconnection(&mut self) {
        self.close();
        self.stage = Stage::Finished;
    }

    pub fn max_session_size(&self) -> Byte {
        self.max_session_size
    }
    fn diagnostic_source_context(&self) -> (&'static str, Option<&str>) {
        match self.source.as_ref() {
            Some(source) => {
                let source_type = match &source.source_type {
                    SessionSourceType::User => "user",
                    SessionSourceType::AmbientAgent { .. } => "ambient_agent",
                };
                (source_type, source.orchestrator_task_id())
            }
            None => ("unknown", None),
        }
    }

    fn diagnostic_stage(&self) -> &'static str {
        match self.stage {
            Stage::BeforeStarted => "before_started",
            Stage::StartedSuccessfully => "started_successfully",
            Stage::Reconnecting { .. } => "reconnecting",
            Stage::Finished => "finished",
        }
    }

    fn log_diagnostic(&self, event: &'static str, details: impl std::fmt::Display) {
        let (source_type, source_task_id) = self.diagnostic_source_context();
        log::info!(
            "Shared session sharer lifecycle: event={event} session_id={:?} source_type={source_type} source_task_id={source_task_id:?} {details}",
            self.session_id
        );
    }

    /// All attempts to end a shared session must go through this API!
    /// This is important to guarantee that we correctly close the socket and
    /// notify viewers with the session ended reason.
    pub fn end_session(&mut self, reason: SessionEndedReason) {
        self.log_diagnostic("end_session_requested", format_args!("reason={reason:?}"));
        let message = UpstreamMessage::EndSession { reason };
        self.send_message_to_server(message);
        self.close_without_reconnection();
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
                self.log_diagnostic("reconnect_triggered", "trigger=heartbeat_idle_timeout");
                log::info!("Sharer reconnecting: heartbeat idle timeout");
                self.reconnect_websocket(ctx);
            }
        }
    }

    pub fn send_active_prompt_update_if_changed(&mut self, active_prompt: ActivePrompt) {
        if active_prompt == self.cached_latest_state.prompt {
            return;
        }

        self.send_active_prompt_update(active_prompt);
    }

    fn send_active_prompt_update(&mut self, active_prompt: ActivePrompt) {
        let message = UpstreamMessage::UpdateActivePrompt(ActivePromptUpdate {
            active_prompt: active_prompt.clone(),
            last_event_no: self.event_no.into(),
        });
        self.send_message_to_server(message);
        self.cached_latest_state.prompt = active_prompt;
    }

    /// Send the presence selection to the server if it changed, with a throttle period.
    pub fn send_presence_selection_if_changed(&mut self, selection: Selection) {
        if selection == self.cached_latest_state.selection {
            return;
        }

        self.send_presence_selection(selection);
    }

    /// Send the presence selection to the server, with a throttle period.
    fn send_presence_selection(&mut self, selection: Selection) {
        self.cached_latest_state.selection = selection.clone();
        if let Err(e) = self.selection_throttled_tx.try_send(selection) {
            log::warn!(
                "Failed to send message over selection_throttled_tx channel in sharer network: {e}"
            );
        }
    }

    pub fn send_role_update(&mut self, participant_id: ParticipantId, role: Role) {
        let message = UpstreamMessage::UpdateRole {
            participant_id,
            role,
        };
        self.send_message_to_server(message);
    }

    pub fn send_user_role_update(&mut self, user_uid: UserUid, role: Role) {
        let message = UpstreamMessage::UpdateUserRole {
            user_uid: user_uid.as_string(),
            role,
        };
        self.send_message_to_server(message);
    }

    pub fn send_pending_user_role_update(&mut self, email: String, role: Role) {
        let message = UpstreamMessage::UpdatePendingUserRole { email, role };
        self.send_message_to_server(message);
    }

    pub fn send_add_guests(&mut self, emails: Vec<String>, role: Role) {
        let message = UpstreamMessage::AddGuests { emails, role };
        self.send_message_to_server(message);
    }

    pub fn send_remove_guest(&mut self, user_uid: UserUid) {
        let message = UpstreamMessage::RemoveGuest {
            user_uid: user_uid.as_string(),
        };
        self.send_message_to_server(message);
    }

    pub fn send_remove_pending_guest(&mut self, email: String) {
        let message = UpstreamMessage::RemovePendingGuest { email };
        self.send_message_to_server(message);
    }

    pub fn send_make_all_participants_readers(&mut self, reason: RoleUpdateReason) {
        let message = UpstreamMessage::UpdateAllRolesToReader { reason };
        self.send_message_to_server(message);
    }

    pub fn send_role_request_response(
        &mut self,
        participant_id: ParticipantId,
        request_id: RoleRequestId,
        response: RoleRequestResponse,
    ) {
        let message = UpstreamMessage::RespondToRoleRequest {
            participant_id,
            request_id,
            response,
        };
        self.send_message_to_server(message);
    }

    pub fn send_input_update<'a>(
        &mut self,
        block_id: &BlockId,
        operations: impl Iterator<Item = &'a CrdtOperation>,
    ) {
        let Some(sharer_id) = self.sharer_id.clone() else {
            return;
        };

        // Set the right block ID. The block IDs that we call this function
        // with are monotonically increasing.
        if block_id != &self.next_buffer_seq_no.0 {
            self.next_buffer_seq_no = (block_id.clone(), InputOperationSeqNo::zero());
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
            participant_id: sharer_id,
            buffer_id: block_id.to_owned().into(),
            op_no: self.next_buffer_seq_no.1,
        };
        self.next_buffer_seq_no.1.advance();

        self.send_message_to_server(UpstreamMessage::UpdateInput(InputUpdate { id, ops }));
    }

    pub fn send_command_execution_rejection(
        &mut self,
        id: CommandExecutionRequestId,
        participant_id: ParticipantId,
        reason: CommandExecutionFailureReason,
    ) {
        let message = UpstreamMessage::RejectCommandExecutionRequest {
            id,
            participant_id,
            reason,
        };
        self.send_message_to_server(message);
    }

    pub fn send_write_to_pty_rejection(
        &mut self,
        id: WriteToPtyRequestId,
        reason: WriteToPtyFailureReason,
    ) {
        let message = UpstreamMessage::RejectWriteToPtyRequest { id, reason };
        self.send_message_to_server(message);
    }

    pub fn send_agent_prompt_rejection(
        &mut self,
        id: AgentPromptRequestId,
        participant_id: ParticipantId,
        reason: AgentPromptFailureReason,
    ) {
        let message = UpstreamMessage::RejectAgentPromptRequest {
            id,
            participant_id,
            reason,
        };
        self.send_message_to_server(message);
    }

    pub fn send_control_action_rejection(
        &mut self,
        participant_id: ParticipantId,
        request_id: ControlActionRequestId,
        reason: ControlActionFailureReason,
    ) {
        let message = UpstreamMessage::RejectControlActionRequest {
            participant_id,
            request_id,
            reason,
        };
        self.send_message_to_server(message);
    }

    pub fn send_link_permission_update(&mut self, role: Option<Role>) {
        let message = UpstreamMessage::UpdateLinkAccessLevel { role };
        self.send_message_to_server(message);
    }

    pub fn send_team_permission_update(&mut self, role: Option<Role>, team_uid: String) {
        let message = UpstreamMessage::UpdateTeamAccessLevel { team_uid, role };
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

    #[cfg(not(any(test, feature = "integration_tests")))]
    #[allow(clippy::too_many_arguments)]
    fn start_websocket(
        &self,
        ws_proxy_rx: async_channel::Receiver<UpstreamMessage>,
        scrollback: Scrollback,
        active_prompt: ActivePrompt,
        selection: Selection,
        window_size: WindowSize,
        init_block_id: BlockId,
        input_replica_id: ReplicaId,
        terminal_view_id: warpui::EntityId,
        universal_developer_input_context: UniversalDeveloperInputContext,
        lifetime: Lifetime,
        source: SharedSessionSource,
        ctx: &mut ModelContext<Self>,
    ) {
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let anonymous_id = AuthStateProvider::as_ref(ctx).get().anonymous_id();

        // Get the selected model before spawning the async task
        let llm_prefs = crate::ai::llms::LLMPreferences::as_ref(ctx);
        let selected_model_id: String = llm_prefs
            .get_active_base_model(ctx, Some(terminal_view_id))
            .id
            .clone()
            .into();

        ctx.spawn(
            async move {
                log::info!("Connecting to session sharing server");
                let Some(create_endpoint) = connect_endpoint("/sessions/create".to_owned()) else {
                    anyhow::bail!("This channel does not support session-sharing.");
                };
                let user_id = UserID {
                    anonymous_id,
                    access_token: auth_client
                        .get_or_refresh_access_token()
                        .await
                        .ok()
                        .and_then(|token| token.bearer_token()),
                };
                let socket = WebSocket::connect(create_endpoint, None).await?;
                log::info!("Connected to session sharing server; preparing initialization");
                anyhow::Ok((socket.split().await, user_id))
            },
            move |network, conn, ctx| match conn {
                Ok(((sink, stream), user_id)) => {
                    // We don't use the `send_message_to_server` API here
                    // because we don't want to buffer this message.

                    use session_sharing_protocol::common::SelectedAgentModel;

                    let message = UpstreamMessage::Initialize(InitPayload {
                        scrollback,
                        active_prompt,
                        window_size,
                        user_id,
                        selection,
                        init_block_id: init_block_id.into(),
                        input_replica_id: input_replica_id.into(),
                        telemetry_context: Some(TelemetryContext(telemetry_context().as_value())),
                        universal_developer_input_context: Some(UniversalDeveloperInputContext {
                            selected_model: Some(SelectedAgentModel::new(selected_model_id)),
                            ..universal_developer_input_context
                        }),
                        lifetime,
                        source_type: source.source_type,
                        source_task_id: source.source_task_id,
                        feature_support: FeatureSupport {
                            supports_agent_view: FeatureFlag::AgentView.is_enabled(),
                            supports_full_role: true,
                            supports_full_role_for_real: true,
                        },
                    });
                    if let Err(e) = network.ws_proxy_tx.try_send(message) {
                        log::error!("Sharer failed to send initialization message: {e}");
                        return;
                    }
                    log::info!("Sent session sharing initialization message");

                    network.on_websocket_connected(ws_proxy_rx, sink, stream, ctx);
                }
                Err(e) => {
                    network.log_diagnostic("initial_websocket_connect_failed", "outcome=transport_error");
                    let cause = Arc::new(e.context("Failed to create shared session"));
                    report_error!(&*cause);
                    ctx.emit(NetworkEvent::FailedToCreateSharedSession {
                        reason: FailedToInitializeSessionReason::internal_server_error_without_details(),
                        cause: Some(cause),
                    });
                }
            },
        );
    }

    /// Initiates attempts to reconnect to the server, with retries.
    /// Successfully connecting to the server here does not mean we reconnected to the session, since the server could reply with an error.
    /// We must wait for DownstreamMessage::SessionReconnected to confirm successful reconnection to the session and update the stage.
    /// We also will not initiate an attempt if the session has been explicitly ended or is already attempting to reconnect.
    pub fn reconnect_websocket(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(self.stage, Stage::Finished | Stage::Reconnecting { .. }) {
            return;
        }

        let (Some(session_id), Some(reconnect_token)) =
            (self.session_id, self.reconnect_token.clone())
        else {
            log::error!(
                "Cannot reconnect to session as sharer without session_id, and reconnect_token"
            );
            return;
        };
        let Some(reconnect_endpoint) = connect_endpoint(format!("/sessions/{session_id}/resume"))
        else {
            log::error!("This channel does not support session-sharing.");
            return;
        };
        self.log_diagnostic(
            "reconnect_initiated",
            "outcome=attempting_transport_recovery",
        );

        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let (source_type, source_task_id) = self.diagnostic_source_context();
        let source_type = source_type.to_string();
        let source_task_id = source_task_id.map(str::to_owned);
        let abort_handle = ctx
            .spawn_with_retry_on_error(
                move || {
                    log::info!(
                        "Shared session sharer lifecycle: event=reconnect_attempt session_id={session_id:?} source_type={source_type} source_task_id={source_task_id:?}"
                    );
                    let reconnect_endpoint = reconnect_endpoint.clone();
                    let auth_state = auth_state.clone();
                    let auth_client = auth_client.clone();
                    async move {
                        let socket = WebSocket::connect(reconnect_endpoint, None).await?;
                        let user_id = UserID {
                            anonymous_id: auth_state.anonymous_id(),
                            access_token: auth_client
                                .get_or_refresh_access_token()
                                .await
                                .ok()
                                .and_then(|token| token.bearer_token()),
                        };
                        anyhow::Ok((socket.split().await, user_id))
                    }
                },
                RECONNECT_RETRY_STRATEGY,
                move |network, res, ctx| match res {
                    RequestState::RequestSucceeded(((sink, stream), user_id)) => {
                        network.log_diagnostic(
                            "reconnect_transport_connected",
                            "outcome=waiting_for_server_confirmation",
                        );
                        let (ws_proxy_tx, ws_proxy_rx) = async_channel::unbounded();
                        let latest_block_id =
                            network.model.lock().block_list().active_block_id().clone();

                        // Because we're going to start listening on a new receiver, we need to update the sender.
                        network.ws_proxy_tx = ws_proxy_tx;
                        // We don't use the `send_message_to_server` API here
                        // because we don't want to buffer this message.
                        let message = UpstreamMessage::Reconnect(ReconnectPayload {
                            session_secret: Default::default(),
                            reconnect_token: reconnect_token.clone(),
                            user_id,
                            latest_block_id: latest_block_id.into(),
                            selection: network.cached_latest_state.selection.clone(),
                            feature_support: FeatureSupport {
                                supports_agent_view: FeatureFlag::AgentView.is_enabled(),
                                supports_full_role: true,
                                supports_full_role_for_real: true,
                            },
                        });
                        if let Err(e) = network.ws_proxy_tx.try_send(message) {
                            log::error!("Sharer failed to send reconnect message: {e}");
                            return;
                        }

                        network.on_websocket_connected(ws_proxy_rx, sink, stream, ctx);
                    }
                    RequestState::RequestFailedRetryPending(e) => {
                        network.log_diagnostic(
                            "reconnect_attempt_failed",
                            "outcome=retry_pending",
                        );
                        log::warn!("Failed to reconnect to shared session, will retry: {e}");
                    }
                    RequestState::RequestFailed(e) => {
                        network.log_diagnostic(
                            "reconnect_failed",
                            "outcome=transport_retries_exhausted",
                        );
                        log::warn!(
                            "Failed to reconnect to shared session, and retries exhausted: {e}"
                        );
                        network.close_without_reconnection();
                        ctx.emit(NetworkEvent::FailedToReconnect);
                    }
                },
            )
            .abort_handle();
        ctx.emit(NetworkEvent::Reconnecting);
        self.stage = Stage::Reconnecting { abort_handle };
    }

    /// Prepare to send and receive messages over the websocket.
    /// ws_proxy_rx is an intermediate channel we use to buffer messages that we'll eventually send to the server through the sink.
    /// The stream is for receiving messages from the server.
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

        // Handle any messages we receive over the websocket.
        ctx.spawn_stream_local(
            stream,
            |network, message, ctx| match message {
                Ok(message) => {
                    network.heartbeat.update(ctx, |heartbeat, ctx| {
                        heartbeat.reset_idle_timeout(ctx);
                    });
                    network.process_websocket_message(message, ctx);
                }
                Err(e) => {
                    network.log_diagnostic("websocket_error", "direction=downstream");
                    log::error!("Got error from shared session sharer websocket: {e}");
                }
            },
            |network, ctx| {
                let stage = network.diagnostic_stage();
                network.log_diagnostic("websocket_closed", format_args!("stage={stage}"));
                log::info!("Session sharing server closed websocket to sharer");
                // Close our current websocket proxy, because we may try to reconnect and that will create a new websocket proxy.
                // This must be done before trying to reconnect.
                network.close();
                // The connection may have timed out or the server restarted.
                // We don't emit this event if we haven't started successfully to avoid an infinite retry loop.
                if matches!(network.stage, Stage::StartedSuccessfully) {
                    network.log_diagnostic("reconnect_triggered", "trigger=websocket_closed");
                    log::info!("Sharer reconnecting: websocket closed by server");
                    network.reconnect_websocket(ctx);
                } else if matches!(network.stage, Stage::BeforeStarted) {
                    // If the websocket is closed while we were waiting for it to start, emit an error.
                    // This is unexpected; we expect to get [`DownstreamMessage::FailedToInitializeSession`]
                    // to get a possibly-more explicit reason.
                    ctx.emit(NetworkEvent::FailedToCreateSharedSession {
                        reason: FailedToInitializeSessionReason::InternalServerError {
                            details: "Websocket closed before starting session".to_string(),
                        },
                        cause: None,
                    });
                }
            },
        );

        // Spawn a task to send messages back up the websocket to the server.
        ctx.spawn(async move {
            let mut ws_proxy_rx = pin!(ws_proxy_rx);
            while let Some(message) = ws_proxy_rx.next().await {
                let serialized = message.to_json();
                match serialized {
                    Ok(serialized) => {
                        if let Err(e) = sink.send(Message::new(serialized)).await {
                            // Errors are not typically retryable. For a case like no network connection, 
                            // sink.send will succeed and the message will actually be sent when connection is restored.
                            log::warn!("Failed to send message over shared session websocket as sharer: {e}. Terminating connection.");
                            break;
                        }
                    }
                    Err(e) => log::warn!("Failed to serialize message to send over shared session websocket as sharer: {e}")
                }
            }
            log::info!("Closing websocket to session sharing server as sharer");
            if let Err(e) = sink.close().await {
                log::error!("Failed to close session sharing websocket as sharer due to {e}");
            }
        }, |_, _, _| {});
    }

    fn process_websocket_message(&mut self, message: Message, ctx: &mut ModelContext<Self>) {
        let Some(downstream_message) = message
            .text()
            .and_then(|t| DownstreamMessage::from_json(t).ok())
        else {
            log::warn!("Received unexpected message from shared session websocket as sharer");
            return;
        };
        match downstream_message {
            DownstreamMessage::SessionInitialized {
                session_id,
                reconnect_token,
                sharer_id,
                sharer_firebase_uid,
                ..
            } => {
                if !matches!(self.stage, Stage::BeforeStarted) {
                    log::warn!("Received unexpected SessionInitialized message when we weren't in BeforeStarted stage");
                    return;
                }
                log::info!("Successfully created shared session.");

                self.session_id = Some(session_id);
                self.reconnect_token = Some(reconnect_token);
                self.sharer_id = Some(sharer_id.clone());

                self.stage = Stage::StartedSuccessfully;
                self.log_diagnostic("session_initialized", "outcome=active_sharer");

                // Flush all events starting from the very first event 0, since events were buffered before the session was initialized.
                self.flush_terminal_events_to_server(0);
                // Non terminal events where we only care about the latest value were dropped before we were connected.
                self.send_latest_state_to_server();

                ctx.emit(NetworkEvent::SharedSessionCreatedSuccessfully {
                    session_id,
                    sharer_id,
                    sharer_firebase_uid: UserUid::new(sharer_firebase_uid.as_str()),
                });
            }
            DownstreamMessage::FailedToInitializeSession { reason } => {
                log::warn!("Failed to initialize session: {reason:?}");
                self.close_without_reconnection();
                ctx.emit(NetworkEvent::FailedToCreateSharedSession {
                    reason,
                    cause: None,
                });
            }
            DownstreamMessage::SessionReconnected {
                last_received_event_no,
                participant_list,
            } => {
                if !matches!(self.stage, Stage::Reconnecting { .. }) {
                    log::warn!("Received unexpected SessionReconnected message when we weren't reconnecting");
                    return;
                }
                self.log_diagnostic("reconnect_succeeded", "outcome=session_resumed");
                log::info!("Successfully reconnected to shared session server as sharer.");
                self.stage = Stage::StartedSuccessfully;

                let start_event_no = last_received_event_no
                    .map_or(0, |last_received_event_no| last_received_event_no + 1);
                self.flush_terminal_events_to_server(start_event_no);
                // Non terminal events where we only care about the latest value were dropped while disconnected.
                self.send_latest_state_to_server();
                ctx.emit(NetworkEvent::ReconnectedSuccessfully);
                ctx.emit(NetworkEvent::ParticipantListUpdated(Box::new(
                    participant_list,
                )));
            }
            DownstreamMessage::FailedToReconnect { reason: _ } => {
                self.log_diagnostic("reconnect_failed", "outcome=server_rejected_resume");
                log::warn!("Failed to reconnect to shared session server as sharer");
                self.close_without_reconnection();
                ctx.emit(NetworkEvent::FailedToReconnect);
            }
            DownstreamMessage::SessionTerminated { reason } => {
                let reason_label = session_terminated_reason_diagnostic_label(&reason);
                self.log_diagnostic(
                    "server_terminated_session",
                    format_args!("reason={reason_label}"),
                );
                self.close_without_reconnection();
                ctx.emit(NetworkEvent::SessionTerminated { reason });
            }
            DownstreamMessage::EventsProcessedAck {
                latest_processed_event_no,
            } => {
                let mut event_no = latest_processed_event_no;
                // Remove all stored events before latest_processed_event_no to free up memory.
                while self.unacked_terminal_events.remove(&event_no).is_some() && event_no > 0 {
                    event_no -= 1;
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
            DownstreamMessage::RoleRequested {
                participant_id,
                request_id,
                role,
            } => {
                ctx.emit(NetworkEvent::RoleRequested {
                    participant_id,
                    role_request_id: request_id,
                    role,
                });
            }
            DownstreamMessage::RoleRequestCancelled {
                participant_id,
                request_id,
            } => {
                ctx.emit(NetworkEvent::RoleRequestCancelled {
                    participant_id,
                    role_request_id: request_id,
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
            DownstreamMessage::InputUpdateRejectedAck { .. } => {
                // TODO
            }
            DownstreamMessage::ParticipantRoleChanged {
                participant_id,
                role,
            } => {
                ctx.emit(NetworkEvent::ParticipantRoleChanged {
                    participant_id,
                    role,
                });
            }
            DownstreamMessage::CommandExecutionRequested {
                id,
                participant_id,
                buffer_id,
                command,
            } => {
                ctx.emit(NetworkEvent::CommandExecutionRequested {
                    id,
                    participant_id,
                    block_id: buffer_id.into(),
                    command,
                });
            }
            DownstreamMessage::WriteToPtyRequested { id, bytes } => {
                ctx.emit(NetworkEvent::WriteToPtyRequested { id, bytes })
            }
            DownstreamMessage::AgentPromptRequested {
                id,
                participant_id,
                request,
            } => {
                ctx.emit(NetworkEvent::AgentPromptRequested {
                    id,
                    participant_id,
                    request,
                });
            }
            DownstreamMessage::LinkAccessLevelUpdateResponse(response) => {
                ctx.emit(NetworkEvent::LinkAccessLevelUpdateResponse { response })
            }
            DownstreamMessage::AddGuestsResponse(response) => {
                ctx.emit(NetworkEvent::AddGuestsResponse { response })
            }
            DownstreamMessage::RemoveGuestResponse(response) => {
                ctx.emit(NetworkEvent::RemoveGuestResponse { response })
            }
            DownstreamMessage::UpdatePendingUserRoleResponse(response) => {
                ctx.emit(NetworkEvent::UpdatePendingUserRoleResponse { response })
            }
            DownstreamMessage::TeamAccessLevelUpdateResponse(response) => {
                ctx.emit(NetworkEvent::TeamAccessLevelUpdateResponse { response })
            }
            DownstreamMessage::UniversalDeveloperInputContextUpdated(context_update) => {
                // Update our cache to stay in sync with what the server knows.
                self.apply_context_update_to_cache(context_update.clone());
                ctx.emit(NetworkEvent::UniversalDeveloperInputContextUpdated(
                    context_update,
                ));
            }
            DownstreamMessage::ViewerTerminalSizeReported { window_size, .. } => {
                ctx.emit(NetworkEvent::ViewerTerminalSizeReported { window_size });
            }
            DownstreamMessage::ControlActionRequested {
                participant_id,
                request_id,
                action,
            } => {
                ctx.emit(NetworkEvent::ControlActionRequested {
                    participant_id,
                    request_id,
                    action,
                });
            }
            DownstreamMessage::Pong { .. } => {}
        }
    }

    fn start_ordered_terminal_events_listener(
        &self,
        events_rx: Receiver<OrderedTerminalEventType>,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn_stream_local(
            events_rx,
            move |network, event_type, ctx| {
                let should_send = {
                    let model = network.model.lock();
                    !model.is_receiving_in_band_command_output()
                        && model.is_active_block_bootstrapped()
                };
                if !should_send {
                    return;
                }

                match (&mut network.pty_bytes_batch_status, event_type) {
                    (
                        PtyBytesBatchStatus::NotBatching { last_sent_at },
                        OrderedTerminalEventType::PtyBytesRead { bytes },
                    ) => {
                        // If we're not batching currently, but we get a PtyBytesRead event, we should start batching.

                        // Calculate how much time we should be batching for.
                        let next_send_time = last_sent_at
                            .checked_add(PTY_READS_BATCH_THRESHOLD)
                            .expect("Can add durations");
                        let wait_time = next_send_time.saturating_duration_since(Instant::now());
                        let spawn_handle = ctx.spawn(
                            async move {
                                Timer::after(wait_time).await;
                            },
                            |network, _, _| {
                                network.send_pty_bytes_read_message();
                            },
                        );
                        // Set the batch status to batching and initialize the accumulated bytes with the bytes from the current read event.
                        network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                            accumulated: bytes,
                            abort_handle: spawn_handle.abort_handle(),
                        };
                    }
                    (
                        PtyBytesBatchStatus::Batching { accumulated, .. },
                        OrderedTerminalEventType::PtyBytesRead { bytes },
                    ) => {
                        // If we're batching and this is a PtyBytesRead event, just add it to the accumulation.
                        accumulated.extend(bytes);
                    }
                    (PtyBytesBatchStatus::NotBatching { .. }, event_type) => {
                        // We're not batching so just send the event type (which is _not_ a PtyBytesRead event).
                        network.send_ordered_terminal_event_message(event_type);
                    }
                    (PtyBytesBatchStatus::Batching { .. }, event_type) => {
                        // If we're batching and we get a non-PtyBytesRead event, we should flush it
                        // and send this other event right after.
                        network.send_pty_bytes_read_message();
                        network.send_ordered_terminal_event_message(event_type);
                    }
                }
            },
            |_network, _ctx| {},
        );
    }

    /// Flushes the accumulated PTY reads into a single [`OrderedTerminalEventType::PtyBytesRead`]
    /// which is then sent to the server.
    fn send_pty_bytes_read_message(&mut self) {
        // We need to check this since we might have flushed the PTY bytes read before the timer expired
        // (for example, when a non-pty bytes read eevnt is received while we're batching).
        // Since Rust can't infer that we'll replace the batch status with a new one if we're currently batching,
        // we need to swap the status for a temporary one to take ownership of the batch status.
        let mut current_batch_status = std::mem::replace(
            &mut self.pty_bytes_batch_status,
            PtyBytesBatchStatus::NotBatching {
                last_sent_at: Instant::now(),
            },
        );

        if let PtyBytesBatchStatus::Batching {
            accumulated,
            abort_handle,
        } = current_batch_status
        {
            // Abort the existing timer if it's running.
            abort_handle.abort();

            // Send the bytes as a single event.
            // TODO: think more deeply about the best compression algorithm for our use-case.
            let compressed = lz4_flex::block::compress_prepend_size(&accumulated);
            let pty_event_type = OrderedTerminalEventType::PtyBytesRead { bytes: compressed };
            self.send_ordered_terminal_event_message(pty_event_type);

            // Since we swapped the status already, the current `pty_bytes_batch_status`
            // will be the [`PtyBytesReadBatch::NotBatching`] status, as expected.
        } else {
            // If we weren't actually batching right now, swap the status back.
            std::mem::swap(&mut self.pty_bytes_batch_status, &mut current_batch_status);
        }
    }

    fn send_ordered_terminal_event_message(&mut self, event_type: OrderedTerminalEventType) {
        // If this send is going to exceed the max number of shareable bytes,
        // let's just end the session.
        let num_bytes = event_type.num_bytes();
        self.num_bytes_shared = self.num_bytes_shared.add(num_bytes).unwrap_or(Byte::MAX);
        if self.num_bytes_shared > self.max_session_size {
            self.log_diagnostic("local_policy_end_session", "reason=exceeded_size_limit");
            log::info!("Stopping shared session because max bytes exceeded.");
            self.end_session(SessionEndedReason::ExceededSizeLimit);
            return;
        }

        let event_no = self.event_no.advance();
        let message = UpstreamMessage::OrderedTerminalEvent(OrderedTerminalEvent {
            event_no,
            event_type,
        });

        self.send_message_to_server(message);
    }

    /// Stores the event if it's an OrderedTerminalEvent, and sends the message to the server if we're connected.
    /// If we're not connected, the event will be flushed to the server once we've connected.
    /// TODO(roland): non OrderedTerminalEvents (like warp prompt) can be dropped if we're not connected. For non OrderedTerminalEvents,
    /// we only need the latest value and can drop old values. We can send the latest value of needed events as part of reconnection.
    fn send_message_to_server(&mut self, message: UpstreamMessage) {
        if let UpstreamMessage::OrderedTerminalEvent(event) = &message {
            self.unacked_terminal_events
                .insert(event.event_no, event.clone());
        }

        if let Stage::StartedSuccessfully = self.stage {
            if let Err(e) = self.ws_proxy_tx.try_send(message) {
                log::warn!("Failed to send message over ws_proxy channel in session sharer: {e}");
            }
        }
    }

    /// Send all stored terminal events from [start_event_no, ...) to the server
    /// The events are not removed from memory.
    fn flush_terminal_events_to_server(&self, start_event_no: usize) {
        let mut event_no = start_event_no;
        while let Some(event) = self.unacked_terminal_events.get(&event_no) {
            if let Err(e) = self
                .ws_proxy_tx
                .try_send(UpstreamMessage::OrderedTerminalEvent(event.clone()))
            {
                // Failures to send are due to be full or closed, so it doesn't make sense to keep trying.
                log::warn!("Failed to send message over ws_proxy channel in session sharer: {e}");
                return;
            }
            event_no += 1;
        }
    }

    /// Send everything in `self.cached_latest_state` to the server.
    /// This is needed when we (re)connect to the server, since all values were dropped before we were connected.
    fn send_latest_state_to_server(&mut self) {
        self.send_active_prompt_update(self.cached_latest_state.prompt.clone());

        // Only send a selection update if we've sent selection updates before or the selection update is non-trivial.
        if self.selection_event_no != EventNumber::new()
            || !matches!(self.cached_latest_state.selection, Selection::None)
        {
            self.send_presence_selection(self.cached_latest_state.selection.clone())
        }

        // Flush the cached UDI context so any model/input-mode changes
        // that were dropped while the websocket was connecting are sent.
        if let Some(cached_context) = self
            .cached_latest_state
            .universal_developer_input_context
            .clone()
        {
            self.send_message_to_server(UpstreamMessage::UpdateUniversalDeveloperInputContext(
                cached_context.into(),
            ));
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.stage, Stage::StartedSuccessfully)
    }
}

const NO_QUOTA_REMAINING_MESSAGE: &str =
    "Session sharing usage exceeded for the day. Please try again later.";
fn session_terminated_reason_diagnostic_label(reason: &SessionTerminatedReason) -> &'static str {
    match reason {
        SessionTerminatedReason::NoUserQuotaRemaining {} => "no_user_quota_remaining",
        SessionTerminatedReason::ExceededSizeLimit => "exceeded_size_limit",
        SessionTerminatedReason::InternalServerError { .. } => "internal_server_error",
    }
}

/// Converts [`SessionTerminatedReason`] to a user-facing string.
pub fn session_terminated_reason_string(
    reason: &SessionTerminatedReason,
    max_session_size: Byte,
) -> String {
    match reason {
        SessionTerminatedReason::NoUserQuotaRemaining {} => {
            // TODO: we should pass down the next refresh time to tell the user.
            NO_QUOTA_REMAINING_MESSAGE.to_string()
        }
        SessionTerminatedReason::ExceededSizeLimit => {
            let max_bytes = max_session_size.get_appropriate_unit(UnitType::Decimal);
            format!("Session limit ({max_bytes}) exceeded. Please reshare to continue.")
        }
        SessionTerminatedReason::InternalServerError { .. } => {
            "Session ended due to an internal error. Please try sharing again.".to_string()
        }
    }
}

/// Converts [`FailedToInitializeSessionReason`] to a user-facing error message.
pub fn failed_to_initialize_session_user_error(reason: &FailedToInitializeSessionReason) -> String {
    match reason {
        FailedToInitializeSessionReason::InternalServerError { .. } => {
            "An internal error occurred. Please try sharing again."
        }
        FailedToInitializeSessionReason::ScrollbackTooLarge {} => {
            "Scrollback exceeds limit. Try sharing again without scrollback."
        }
        FailedToInitializeSessionReason::NoUserQuotaRemaining { .. } => {
            // TODO: we should pass down the next refresh time to tell the user.
            NO_QUOTA_REMAINING_MESSAGE
        }
        FailedToInitializeSessionReason::UserNotFound => "You must be logged in to share sessions.",
    }
    .to_string()
}

pub fn failed_to_add_guests_user_error(reason: &FailedToAddGuestsReason) -> String {
    match reason {
        FailedToAddGuestsReason::Invalid => "Something went wrong. Please try again.",
        FailedToAddGuestsReason::NotWarpUsers => {
            "One or more emails were not associated with Warp accounts."
        }
        FailedToAddGuestsReason::GuestAlreadyAdded => {
            "One or more emails have already been added to the session."
        }
    }
    .to_string()
}

pub enum NetworkEvent {
    SharedSessionCreatedSuccessfully {
        session_id: SessionId,
        sharer_id: ParticipantId,
        sharer_firebase_uid: UserUid,
    },
    FailedToCreateSharedSession {
        reason: FailedToInitializeSessionReason,
        /// Internal error cause not suitable for displaying to the user,
        /// but useful for diagnostics (e.g. agent error messages).
        cause: Option<Arc<anyhow::Error>>,
    },
    SessionTerminated {
        reason: SessionTerminatedReason,
    },
    Reconnecting,
    ParticipantListUpdated(Box<ParticipantList>),
    ParticipantPresenceUpdated(ParticipantPresenceUpdate),
    ReconnectedSuccessfully,
    FailedToReconnect,
    RoleRequested {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
        role: Role,
    },
    RoleRequestCancelled {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
    },
    ParticipantRoleChanged {
        participant_id: ParticipantId,
        role: Role,
    },
    InputUpdated {
        block_id: BlockId,
        operations: Vec<CrdtOperation>,
    },
    CommandExecutionRequested {
        id: CommandExecutionRequestId,
        participant_id: ParticipantId,
        block_id: BlockId,
        command: String,
    },
    WriteToPtyRequested {
        id: WriteToPtyRequestId,
        bytes: Vec<u8>,
    },
    AgentPromptRequested {
        id: AgentPromptRequestId,
        participant_id: ParticipantId,
        request: AgentPromptRequest,
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
    UniversalDeveloperInputContextUpdated(UniversalDeveloperInputContextUpdate),
    ControlActionRequested {
        participant_id: ParticipantId,
        request_id: ControlActionRequestId,
        action: ControlAction,
    },
    ViewerTerminalSizeReported {
        window_size: WindowSize,
    },
}

impl Entity for Network {
    type Event = NetworkEvent;
}

impl Drop for Network {
    fn drop(&mut self) {
        let stage = self.diagnostic_stage();
        self.log_diagnostic("network_dropped", format_args!("stage={stage}"));
        // This is needed to gracefully close the websocket when Network is dropped.
        self.close();
        // We keep the same selection_throttled_tx even if we reconnect and replace the internal ws_proxy_tx,
        // which is why we don't close it as part of [`Self::close`]
        self.selection_throttled_tx.close();
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
