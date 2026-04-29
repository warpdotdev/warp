use std::time::Duration;

use instant::Instant;
use session_sharing_protocol::common::SessionId;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::spawn::{spawn_task, AmbientAgentEvent};
use crate::ai::ambient_agents::task::HarnessConfig;
use crate::ai::ambient_agents::telemetry::CloudAgentTelemetryEvent;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::{
    OUT_OF_CREDITS_TASK_FAILURE_MESSAGE, SERVER_OVERLOADED_TASK_FAILURE_MESSAGE,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::execution_profiles::{CloudAgentComputerUseState, ComputerUsePermission};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ai::{
    AgentConfigSnapshot, AmbientAgentTaskState, AttachmentInput, SpawnAgentRequest,
};
use crate::server::server_api::{AIApiError, CloudAgentCapacityError, ServerApiProvider};
use crate::terminal::view::ambient_agent::SetupCommandState;

use super::AmbientAgentProgressUIState;

/// Tracks progress timestamps for each step during ambient agent spawning.
#[derive(Debug, Clone)]
pub struct AgentProgress {
    /// When the agent run was requested.
    pub spawned_at: Instant,
    /// When the run was claimed by a worker.
    pub claimed_at: Option<Instant>,
    /// When the agent harness began executing.
    pub harness_started_at: Option<Instant>,
    /// When the agent stopped.
    pub stopped_at: Option<Instant>,
}

/// Status of the ambient agent run.
#[derive(Debug, Clone)]
pub enum Status {
    /// Not in an ambient agent session.
    NotAmbientAgent,
    /// First-time environment setup for cloud agents.
    Setup,
    /// The user is composing their ambient agent prompt.
    Composing,
    /// Waiting for the ambient agent run to be ready.
    WaitingForSession { progress: AgentProgress },
    /// The agent is running and the session is ready.
    AgentRunning,
    /// The agent failed.
    Failed {
        progress: AgentProgress,
        error_message: String,
    },
    /// The user needs to authenticate with GitHub.
    NeedsGithubAuth {
        progress: AgentProgress,
        error_message: String,
        auth_url: String,
    },
    /// The agent was cancelled.
    Cancelled { progress: AgentProgress },
}

/// Model to track the state of an ambient agent run.
pub struct AmbientAgentViewModel {
    status: Status,

    /// The request with which the cloud agent was spawned, if it was spawned.
    request: Option<SpawnAgentRequest>,

    /// The terminal view this model is part of.
    terminal_view_id: EntityId,

    /// Whether this ambient agent view has a parent terminal view to return to.
    /// `false` for standalone views (e.g., from "New Cloud Conversation").
    /// `true` for nested views (pushed onto an existing terminal's pane stack).
    has_parent_terminal: bool,

    /// Selected cloud environment to launch the ambient agent with.
    environment_id: Option<SyncId>,

    /// Handle for the periodic timer that updates progress durations.
    progress_timer_handle: Option<SpawnedFutureHandle>,

    /// UI state for rendering the ambient agent progress screen.
    pub ui_state: AmbientAgentProgressUIState,

    setup_commands_state: SetupCommandState,

    /// The task ID for the current cloud agent task, if one has been spawned.
    task_id: Option<AmbientAgentTaskId>,

    /// The local conversation associated with this cloud agent run, if any.
    /// Set for remote child agents spawned via `start_agent` so the `run_id`
    /// from the server response can be wired back to the conversation.
    conversation_id: Option<AIConversationId>,

    /// Selected execution harness for the cloud agent run.
    /// Defaults to `Harness::Oz`. Used to populate `AgentConfigSnapshot.harness` on spawn.
    harness: Harness,
    /// Whether the optimistic InitialUserQuery block has been inserted for the current run.
    has_inserted_cloud_mode_user_query_block: bool,
    /// Whether the harness CLI (e.g. `claude`, `gemini`) has started running for a non-oz run.
    /// Used to transition the cloud-mode setup UI out of the pre-first-exchange phase when
    /// there is no oz `AppendedExchange` to key off of.
    harness_command_started: bool,
}

impl AmbientAgentViewModel {
    pub fn new(
        terminal_view_id: EntityId,
        has_parent_terminal: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        // Validate the default environment once Warp Drive sync completes.
        // The environment ID may be restored from settings before environments are synced,
        // so we need to validate it once the initial load is complete.
        let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
        ctx.spawn(initial_load_complete, |me, _, ctx| {
            me.validate_environment_after_initial_load(ctx);
        });

        let ui_state = AmbientAgentProgressUIState::new(ctx);

        Self {
            status: Status::NotAmbientAgent,
            request: None,
            terminal_view_id,
            has_parent_terminal,
            environment_id: None,
            progress_timer_handle: None,
            ui_state,
            setup_commands_state: Default::default(),
            task_id: None,
            conversation_id: None,
            harness: Harness::default(),
            has_inserted_cloud_mode_user_query_block: false,
            harness_command_started: false,
        }
    }

    pub fn request(&self) -> Option<&SpawnAgentRequest> {
        self.request.as_ref()
    }

    pub fn setup_command_state(&self) -> &SetupCommandState {
        &self.setup_commands_state
    }

    pub fn setup_command_state_mut(&mut self) -> &mut SetupCommandState {
        &mut self.setup_commands_state
    }

    pub(super) fn set_setup_command_visibility(
        &mut self,
        is_visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if is_visible != self.setup_commands_state.should_expand() {
            self.setup_commands_state.set_should_expand(is_visible);
            ctx.emit(AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility);
        }
    }

    /// Handles CloudModel events to keep environment_id in sync.
    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            // If the selected environment is deleted, clear the selection.
            CloudModelEvent::ObjectTrashed { type_and_id, .. }
            | CloudModelEvent::ObjectDeleted { type_and_id, .. } => {
                if type_and_id.as_generic_string_object_id() == self.environment_id
                    && self.environment_id.is_some()
                {
                    self.environment_id = None;
                    ctx.emit(AmbientAgentViewModelEvent::EnvironmentSelected);
                }
            }
            // When an environment syncs and gets a ServerId, update our stored ID.
            CloudModelEvent::ObjectSynced {
                client_id,
                server_id,
                ..
            } => {
                if let Some(current_id) = &self.environment_id {
                    // Check if this is our environment by comparing with the ClientId
                    if current_id == &SyncId::ClientId(*client_id) {
                        self.environment_id = Some(SyncId::ServerId(*server_id));
                        ctx.emit(AmbientAgentViewModelEvent::EnvironmentSelected);
                    }
                }
            }
            _ => (),
        }
    }

    /// Validates the environment ID after Warp Drive initial load completes.
    /// If the environment no longer exists, clears the selection.
    fn validate_environment_after_initial_load(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(id) = &self.environment_id {
            if CloudAmbientAgentEnvironment::get_by_id(id, ctx).is_none() {
                log::warn!(
                    "Environment {id:?} no longer exists after initial load, clearing selection"
                );
                self.environment_id = None;
                ctx.emit(AmbientAgentViewModelEvent::EnvironmentSelected);
            }
        }
    }

    /// Returns the agent progress for tracking spawn steps.
    /// Returns `None` if not in the `WaitingForSession`, `Failed`, `NeedsGithubAuth`, or `Cancelled` state.
    pub fn agent_progress(&self) -> Option<&AgentProgress> {
        match &self.status {
            Status::WaitingForSession { progress }
            | Status::Failed { progress, .. }
            | Status::NeedsGithubAuth { progress, .. }
            | Status::Cancelled { progress } => Some(progress),
            _ => None,
        }
    }

    /// Returns the currently selected environment ID.
    pub fn selected_environment_id(&self) -> Option<&SyncId> {
        self.environment_id.as_ref()
    }

    pub fn selected_harness(&self) -> Harness {
        self.harness
    }

    pub fn set_harness(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        if self.harness == harness {
            return;
        }
        self.harness = harness;
        ctx.emit(AmbientAgentViewModelEvent::HarnessSelected);
    }

    /// True when the run is configured to use a non-Oz execution harness and the
    /// required feature flags are enabled.
    pub(super) fn is_third_party_harness(&self) -> bool {
        FeatureFlag::AgentHarness.is_enabled() && self.harness != Harness::Oz
    }

    /// Whether the harness CLI has started running. Only meaningful for non-oz runs.
    pub(super) fn harness_command_started(&self) -> bool {
        self.harness_command_started
    }

    /// Marks the harness CLI as started and emits `HarnessCommandStarted`.
    /// Idempotent: subsequent calls after the first are no-ops and do not re-emit.
    pub(super) fn mark_harness_command_started(&mut self, ctx: &mut ModelContext<Self>) {
        debug_assert!(
            self.harness != Harness::Oz,
            "harness_command_started is only meaningful for non-oz runs"
        );
        if self.harness_command_started {
            return;
        }
        self.harness_command_started = true;
        ctx.emit(AmbientAgentViewModelEvent::HarnessCommandStarted);
    }

    /// Sets the selected environment ID.
    /// If the given ID does not exist in CloudModel, the environment ID is not changed.
    pub fn set_environment_id(
        &mut self,
        environment_id: Option<SyncId>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(id) = &environment_id {
            if CloudAmbientAgentEnvironment::get_by_id(id, ctx).is_none() {
                log::warn!("Tried to select unknown environment {id:?}");
                return;
            }
        }
        self.environment_id = environment_id;
        ctx.emit(AmbientAgentViewModelEvent::EnvironmentSelected);
    }

    /// Whether or not this terminal session is for an ambient agent.
    pub fn is_ambient_agent(&self) -> bool {
        !matches!(self.status, Status::NotAmbientAgent)
    }

    /// Returns the task ID for the current cloud agent task, if one has been spawned.
    pub fn task_id(&self) -> Option<AmbientAgentTaskId> {
        self.task_id
    }

    pub fn has_inserted_cloud_mode_user_query_block(&self) -> bool {
        self.has_inserted_cloud_mode_user_query_block
    }

    pub fn set_has_inserted_cloud_mode_user_query_block(&mut self, has_inserted: bool) {
        self.has_inserted_cloud_mode_user_query_block = has_inserted;
    }

    /// Returns whether this ambient agent view has a parent terminal to return to.
    pub fn has_parent_terminal(&self) -> bool {
        self.has_parent_terminal
    }

    /// Sets whether this ambient agent view has a parent terminal to return to.
    /// This should be called when pushing the view onto an existing pane stack.
    pub fn set_has_parent_terminal(&mut self, has_parent: bool) {
        self.has_parent_terminal = has_parent;
    }

    /// Whether or not this terminal session is in the setup state (first-time environment creation).
    pub fn is_in_setup(&self) -> bool {
        matches!(self.status, Status::Setup)
    }

    /// Whether or not this terminal session is currently setting up an ambient agent run.
    pub fn is_configuring_ambient_agent(&self) -> bool {
        matches!(self.status, Status::Composing)
    }

    /// Whether or not this terminal session is waiting for an ambient agent session to be ready.
    pub fn is_waiting_for_session(&self) -> bool {
        matches!(self.status, Status::WaitingForSession { .. })
    }

    /// Whether or not the ambient agent failed to spawn.
    pub fn is_failed(&self) -> bool {
        matches!(self.status, Status::Failed { .. })
    }

    /// Whether or not the ambient agent was cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self.status, Status::Cancelled { .. })
    }

    /// Whether or not the ambient agent needs GitHub authentication.
    pub fn is_needs_github_auth(&self) -> bool {
        matches!(self.status, Status::NeedsGithubAuth { .. })
    }

    /// Whether or not the ambient agent is currently running.
    pub fn is_agent_running(&self) -> bool {
        matches!(self.status, Status::AgentRunning)
    }

    /// Whether or not we should show a status footer (loading, error, auth, or cancelled).
    pub fn should_show_status_footer(&self) -> bool {
        if FeatureFlag::CloudModeSetupV2.is_enabled() {
            return false;
        }

        self.is_waiting_for_session()
            || self.is_failed()
            || self.is_needs_github_auth()
            || self.is_cancelled()
    }

    /// Returns the error message if the agent is in a failed state.
    pub fn error_message(&self) -> Option<&str> {
        match &self.status {
            Status::Failed { error_message, .. } => Some(error_message),
            _ => None,
        }
    }

    /// Returns the GitHub auth URL if the agent needs GitHub authentication.
    pub fn github_auth_url(&self) -> Option<&str> {
        match &self.status {
            Status::NeedsGithubAuth { auth_url, .. } => Some(auth_url),
            _ => None,
        }
    }

    /// Returns the error message for GitHub authentication failures.
    pub fn github_auth_error_message(&self) -> Option<&str> {
        match &self.status {
            Status::NeedsGithubAuth { error_message, .. } => Some(error_message),
            _ => None,
        }
    }

    /// Enter the setup state for first-time environment creation.
    pub fn enter_setup(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = Status::Setup;
        ctx.emit(AmbientAgentViewModelEvent::EnteredSetupState);
    }

    /// Transition from Setup to Composing state.
    pub fn enter_composing_from_setup(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = Status::Composing;
        ctx.emit(AmbientAgentViewModelEvent::EnteredComposingState);
    }

    /// This is used when we join an already-running ambient agent shared session (e.g. from the
    /// agent management view). We want the ambient agent UI affordances (like the environment
    /// selector) to be visible even though we did not spawn the agent from this view model.
    pub fn enter_viewing_existing_session(
        &mut self,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        // Store the task ID for later use
        self.task_id = Some(task_id);

        if matches!(self.status, Status::NotAmbientAgent) {
            self.status = Status::AgentRunning;
        }

        // Fetch the task so we can set the correct environment (instead of defaulting to the most
        // recently-used one) and the correct harness (so non-oz viewers know to use the
        // queued-prompt / harness-command-started flow).
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            |me, result, ctx| match result {
                Ok(task) => {
                    let snapshot = task.agent_config_snapshot.as_ref();
                    let environment_id = snapshot
                        .and_then(|s| s.environment_id.as_deref())
                        .and_then(|id| ServerId::try_from(id).ok())
                        .map(SyncId::ServerId);
                    let harness = snapshot
                        .and_then(|s| s.harness.as_ref())
                        .map(|h| h.harness_type)
                        .unwrap_or(Harness::Oz);

                    me.set_environment_id(environment_id, ctx);
                    me.set_harness(harness, ctx);
                }
                Err(err) => {
                    log::warn!("Failed to fetch ambient agent task for shared session: {err}");
                    me.set_environment_id(None, ctx);
                }
            },
        );
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Reset the status back to NotAmbientAgent.
    pub fn reset_status(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = Status::NotAmbientAgent;
        self.environment_id = None;
        self.task_id = None;
        self.conversation_id = None;
        self.has_inserted_cloud_mode_user_query_block = false;
        self.harness_command_started = false;
        self.stop_progress_timer();
        ctx.notify();
    }

    /// Sets the local conversation ID associated with this cloud agent run.
    pub fn set_conversation_id(&mut self, id: Option<AIConversationId>) {
        self.conversation_id = id;
    }

    /// Spawn an ambient agent with the given prompt and current session configuration.
    pub fn spawn_agent(
        &mut self,
        prompt: String,
        attachments: Vec<AttachmentInput>,
        ctx: &mut ModelContext<Self>,
    ) {
        let model_id = LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, Some(self.terminal_view_id))
            .id
            .to_string();

        // Determine computer_use_enabled based on workspace AI autonomy settings
        let CloudAgentComputerUseState { enabled, .. } =
            ComputerUsePermission::resolve_cloud_agent_state(ctx);
        let computer_use_enabled = Some(enabled);

        let default_host = std::env::var("WARP_CLOUD_MODE_DEFAULT_HOST")
            .ok()
            .filter(|s| !s.is_empty());

        let harness_override =
            (self.harness != Harness::Oz).then(|| HarnessConfig::from_harness_type(self.harness));

        let config = Some(AgentConfigSnapshot {
            environment_id: self.environment_id.as_ref().map(|id| id.to_string()),
            model_id: Some(model_id),
            computer_use_enabled,
            worker_host: default_host,
            harness: harness_override,
            ..Default::default()
        });

        let request = SpawnAgentRequest {
            prompt,
            config,
            title: None,
            team: None,
            skill: None,
            attachments,
            interactive: None,
            parent_run_id: None,
            runtime_skills: vec![],
            referenced_attachments: vec![],
        };

        self.spawn_internal(request, ctx);
    }

    /// Spawn an ambient agent with a fully-constructed request.
    pub fn spawn_agent_with_request(
        &mut self,
        request: SpawnAgentRequest,
        ctx: &mut ModelContext<Self>,
    ) {
        // Apply pane settings from the request.
        if let Some(config) = request.config.as_ref() {
            self.environment_id = config
                .environment_id
                .as_deref()
                .and_then(|id| ServerId::try_from(id).ok())
                .map(SyncId::ServerId);

            if let Some(model_id) = config.model_id.as_deref() {
                LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                    prefs.update_preferred_agent_mode_llm(
                        &LLMId::from(model_id),
                        self.terminal_view_id,
                        ctx,
                    )
                });
            }
        }

        self.spawn_internal(request, ctx);
    }

    /// Spawn an ambient agent given `request`.
    fn spawn_internal(&mut self, mut request: SpawnAgentRequest, ctx: &mut ModelContext<Self>) {
        request.interactive = Some(true);
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        self.request = Some(request.clone());
        let stream = spawn_task(request, ai_client, None);

        ctx.spawn_stream_local(
            stream,
            |me, event_result, ctx| {
                // If we're in Cancelled or Failed state, ignore most events from the stream
                // except for TaskSpawned (which we need to handle for early cancellation).
                let ignore_events = matches!(me.status, Status::Cancelled { .. } | Status::Failed { .. });

                match event_result {
                Ok(event) => match event {
                    AmbientAgentEvent::TaskSpawned { task_id, run_id } => {
                        // Store the task ID for later use (e.g., populating details panel)
                        me.task_id = Some(task_id);

                        // If we already transitioned to Cancelled state (because user cancelled
                        // before we received the task_id), send the cancellation to the server now.
                        if matches!(me.status, Status::Cancelled { .. }) {
                            log::info!(
                                "Received task_id after cancellation, sending server cancellation for task {}",
                                task_id
                            );
                            let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
                            ctx.spawn(
                                async move {
                                    if let Err(e) = ai_client.cancel_ambient_agent_task(&task_id).await {
                                        log::error!("Failed to cancel ambient agent task {}: {:?}", task_id, e);
                                    }
                                },
                                |_, _, _| {},
                            );
                            return;
                        }

                        // Wire the run_id to the associated conversation for
                        // orchestration v2. This unblocks the parent agent's
                        // pending start_agent tool call.
                        if let Some(conversation_id) = me.conversation_id {
                            let terminal_view_id = me.terminal_view_id;
                            let spawned_task_id = Some(task_id);
                            BlocklistAIHistoryModel::handle(ctx).update(
                                ctx,
                                |history, ctx| {
                                    history.assign_run_id_for_conversation(
                                        conversation_id,
                                        run_id,
                                        spawned_task_id,
                                        terminal_view_id,
                                        ctx,
                                    );
                                },
                            );
                        }

                        // Mark the task as manually opened so it appears in the conversation list
                        // even though its server-side source may not be user-initiated.
                        AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                            model.mark_task_as_manually_opened(task_id, ctx);
                        });

                        // Mark this task as active immediately so it renders under the Active section
                        // (and doesn't briefly appear under Past before the shared session join completes).
                        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
                            model.register_ambient_session(me.terminal_view_id, task_id, ctx);
                        });

                        // Emit event so terminal view knows to show the info button
                        ctx.emit(AmbientAgentViewModelEvent::ProgressUpdated);
                    }
                    AmbientAgentEvent::StateChanged {
                        state,
                        status_message,
                    } => {
                        // Ignore state changes if we're already in a terminal state
                        if ignore_events {
                            return;
                        }

                        if let Status::WaitingForSession { progress } = &mut me.status {
                            match state {
                                AmbientAgentTaskState::Cancelled => {
                                    me.handle_cancellation(ctx);
                                }
                                AmbientAgentTaskState::Queued | AmbientAgentTaskState::Pending => {
                                    // Clear later states in case the agent failed to start and was retried.
                                    progress.claimed_at = None;
                                    progress.harness_started_at = None;
                                    ctx.emit(AmbientAgentViewModelEvent::ProgressUpdated);
                                }
                                AmbientAgentTaskState::Claimed => {
                                    if progress.claimed_at.is_none() {
                                        progress.claimed_at = Some(Instant::now());
                                        progress.harness_started_at = None;
                                        ctx.emit(AmbientAgentViewModelEvent::ProgressUpdated);
                                    }
                                }
                                AmbientAgentTaskState::InProgress => {
                                    if progress.harness_started_at.is_none() {
                                        progress.harness_started_at = Some(Instant::now());
                                        ctx.emit(AmbientAgentViewModelEvent::ProgressUpdated);
                                    }
                                }
                                AmbientAgentTaskState::Succeeded => {}
                                AmbientAgentTaskState::Failed
                                | AmbientAgentTaskState::Error
                                | AmbientAgentTaskState::Blocked
                                | AmbientAgentTaskState::Unknown => {
                                    let error = status_message
                                        .map(|msg| msg.message)
                                        .unwrap_or_else(|| "Cloud agent failed".to_string());
                                    me.handle_spawn_error(error, ctx);
                                }
                            }
                        }
                    }
                    AmbientAgentEvent::SessionStarted { session_join_info } => {
                        // Ignore session started if we're already in a terminal state
                        if ignore_events {
                            return;
                        }

                        if let Some(session_id) = session_join_info.session_id {
                            me.stop_progress_timer();
                            me.status = Status::AgentRunning;
                            ctx.emit(AmbientAgentViewModelEvent::SessionReady { session_id });
                        }
                    }
                    AmbientAgentEvent::AtCapacity => {
                        if ignore_events {
                            return;
                        }

                        if matches!(me.status, Status::WaitingForSession { .. }) {
                            ctx.emit(AmbientAgentViewModelEvent::ShowCloudAgentCapacityModal);
                        }
                    }
                    AmbientAgentEvent::TimedOut => {}
                },
                Err(err) => {
                    // Ignore errors if we're already in a terminal state
                    if ignore_events {
                        return;
                    }
                    let error_message = err.to_string();
                    send_telemetry_from_ctx!(
                        CloudAgentTelemetryEvent::DispatchFailed {
                            error: error_message.clone()
                        },
                        ctx
                    );

                    // Check if this is a ClientError with an auth_url
                    use crate::server::server_api::ClientError;
                    if let Some(client_error) = err.downcast_ref::<ClientError>() {
                        if let Some(auth_url) = &client_error.auth_url {
                            me.handle_needs_github_auth(
                                auth_url.clone(),
                                client_error.error.clone(),
                                ctx,
                            );
                            return;
                        }
                    }
                    if let Some(capacity_error) = err.downcast_ref::<CloudAgentCapacityError>() {
                        me.handle_spawn_error(capacity_error.error.clone(), ctx);
                        ctx.emit(AmbientAgentViewModelEvent::ShowCloudAgentCapacityModal);
                        return;
                    }
                    if let Some(ai_api_error) = err.downcast_ref::<AIApiError>() {
                        match ai_api_error {
                            AIApiError::QuotaLimit => {
                                me.handle_spawn_error(
                                    OUT_OF_CREDITS_TASK_FAILURE_MESSAGE.to_string(),
                                    ctx,
                                );
                                ctx.emit(AmbientAgentViewModelEvent::ShowAICreditModal);
                                return;
                            }
                            AIApiError::ServerOverloaded => {
                                me.handle_spawn_error(
                                    SERVER_OVERLOADED_TASK_FAILURE_MESSAGE.to_string(),
                                    ctx,
                                );
                                return;
                            }
                            _ => {}
                        }
                    }
                    me.handle_spawn_error(error_message, ctx);
                }
            }
            },
            |_me, _ctx| {},
        );

        self.status = Status::WaitingForSession {
            progress: AgentProgress {
                spawned_at: Instant::now(),
                claimed_at: None,
                harness_started_at: None,
                stopped_at: None,
            },
        };
        self.start_progress_timer(ctx);
        ctx.emit(AmbientAgentViewModelEvent::DispatchedAgent);
    }

    /// Starts the periodic timer that updates the progress UI while waiting for a session.
    fn start_progress_timer(&mut self, ctx: &mut ModelContext<Self>) {
        // Don't start a new timer if one is already running.
        if self.progress_timer_handle.is_some() {
            return;
        }

        let handle = ctx.spawn(
            async move {
                Timer::after(Duration::from_millis(200)).await;
            },
            |me, _unit, ctx| {
                me.progress_timer_handle = None;

                // Check if still waiting for session.
                if matches!(me.status, Status::WaitingForSession { .. }) {
                    ctx.emit(AmbientAgentViewModelEvent::ProgressUpdated);
                    me.start_progress_timer(ctx);
                }
            },
        );

        self.progress_timer_handle = Some(handle);
    }

    fn stop_progress_timer(&mut self) {
        if let Some(handle) = self.progress_timer_handle.take() {
            handle.abort();
        }
    }

    /// Handles a spawn error by transitioning to the Failed state.
    fn handle_spawn_error(&mut self, error_message: String, ctx: &mut ModelContext<Self>) {
        self.stop_progress_timer();

        let now = Instant::now();

        // Extract or create progress tracking.
        let progress = if let Status::WaitingForSession { mut progress } =
            std::mem::replace(&mut self.status, Status::NotAmbientAgent)
        {
            progress.stopped_at = Some(now);
            progress
        } else {
            // If not in WaitingForSession, create a new progress with current time.
            AgentProgress {
                spawned_at: now,
                claimed_at: None,
                harness_started_at: None,
                stopped_at: Some(now),
            }
        };

        self.status = Status::Failed {
            progress,
            error_message: error_message.clone(),
        };
        ctx.emit(AmbientAgentViewModelEvent::Failed { error_message });
    }

    /// Handles the need for GitHub authentication by transitioning to the NeedsGithubAuth state.
    fn handle_needs_github_auth(
        &mut self,
        auth_url: String,
        error_message: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.stop_progress_timer();

        let now = Instant::now();

        // Extract or create progress tracking.
        let progress = if let Status::WaitingForSession { mut progress } =
            std::mem::replace(&mut self.status, Status::NotAmbientAgent)
        {
            progress.stopped_at = Some(now);
            progress
        } else {
            // If not in WaitingForSession, create a new progress with current time.
            AgentProgress {
                spawned_at: now,
                claimed_at: None,
                harness_started_at: None,
                stopped_at: Some(now),
            }
        };

        self.status = Status::NeedsGithubAuth {
            progress,
            error_message,
            auth_url,
        };

        ctx.emit(AmbientAgentViewModelEvent::NeedsGithubAuth);
    }

    /// Handles cancellation by transitioning to the Cancelled state.
    fn handle_cancellation(&mut self, ctx: &mut ModelContext<Self>) {
        self.stop_progress_timer();

        let now = Instant::now();

        // Extract or create progress tracking.
        let progress = if let Status::WaitingForSession { mut progress } =
            std::mem::replace(&mut self.status, Status::NotAmbientAgent)
        {
            progress.stopped_at = Some(now);
            progress
        } else {
            // If not in WaitingForSession, create a new progress with current time.
            AgentProgress {
                spawned_at: now,
                claimed_at: None,
                harness_started_at: None,
                stopped_at: Some(now),
            }
        };

        self.status = Status::Cancelled { progress };

        ctx.emit(AmbientAgentViewModelEvent::Cancelled);
    }

    /// Cancels the ambient agent task if one is currently running.
    /// Sends a cancellation request to the server (if task_id is available) and transitions to the Cancelled state.
    pub fn cancel_task(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_waiting_for_session() {
            log::warn!("Attempted to cancel ambient agent task but not in WaitingForSession state");
            return;
        }

        // If we have a task_id, send cancellation request to the server
        if let Some(task_id) = self.task_id {
            let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
            ctx.spawn(
                async move { ai_client.cancel_ambient_agent_task(&task_id).await },
                |_me, result, _ctx| {
                    if let Err(err) = result {
                        log::error!("Failed to cancel ambient agent task: {err}");
                    }
                },
            );
        } else {
            // No task_id yet, but we can still cancel locally.
            // The spawn stream will handle the cancellation when it receives the TaskSpawned event
            // and sees we're no longer in WaitingForSession state.
            log::info!("Cancelling ambient agent task before task_id was received");
        }

        // Always transition to cancelled state immediately, regardless of whether we have a task_id.
        // This provides immediate UI feedback to the user.
        self.handle_cancellation(ctx);
    }
}

/// Events emitted by the ambient agent view model.
#[derive(Debug, Clone)]
pub enum AmbientAgentViewModelEvent {
    /// The user has entered the setup state (first-time environment creation).
    EnteredSetupState,
    /// The user has entered the composing state (typing their prompt).
    EnteredComposingState,
    /// The ambient agent run has been dispatched.
    DispatchedAgent,
    /// The spawn progress has been updated (e.g., task claimed or in-progress).
    ProgressUpdated,
    /// The ambient agent has started sharing its session.
    SessionReady {
        session_id: SessionId,
    },
    /// An environment was selected.
    EnvironmentSelected,
    /// The ambient agent failed.
    Failed {
        error_message: String,
    },
    /// Request to show the cloud agent concurrency/capacity modal.
    ShowCloudAgentCapacityModal,
    /// Request to show the cloud agent AI credits modal.
    ShowAICreditModal,
    /// The ambient agent needs GitHub authentication.
    NeedsGithubAuth,
    /// The ambient agent was cancelled.
    Cancelled,
    /// The selected execution harness (Oz / Claude Code) changed.
    HarnessSelected,
    /// The harness CLI (for non-oz runs) has started executing in the shared session.
    /// Fires once per run and signals the transition out of the pre-first-exchange phase
    /// for claude / gemini / other third-party harnesses.
    HarnessCommandStarted,

    UpdatedSetupCommandVisibility,
}

impl Entity for AmbientAgentViewModel {
    type Event = AmbientAgentViewModelEvent;
}
