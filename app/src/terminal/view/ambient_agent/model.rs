use std::time::Duration;

use instant::Instant;
use session_sharing_protocol::common::SessionId;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_terminal::model::BlockId;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::{conversation::AIConversationId, extract_user_query_mode};
use crate::ai::ambient_agents::github_auth_notifier::{GitHubAuthEvent, GitHubAuthNotifier};
use crate::ai::ambient_agents::github_auth_url;
use crate::ai::ambient_agents::spawn::{spawn_task, submit_run_followup, AmbientAgentEvent};
use crate::ai::ambient_agents::task::{HarnessAuthSecretsConfig, HarnessConfig};
use crate::ai::ambient_agents::telemetry::CloudAgentTelemetryEvent;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::{
    OUT_OF_CREDITS_TASK_FAILURE_MESSAGE, SERVER_OVERLOADED_TASK_FAILURE_MESSAGE,
};
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::handoff::touched_repos::TouchedWorkspace;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::handoff::PendingCloudLaunch;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::execution_profiles::{CloudAgentComputerUseState, ComputerUsePermission};
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::CloudObjectLookup as _;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ServerId, SyncId};
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::server::server_api::ai::InitialSnapshotToken;
use crate::server::server_api::ai::{
    AgentConfigSnapshot, AmbientAgentTaskState, AttachmentInput, SpawnAgentRequest,
};
use crate::server::server_api::{
    AIApiError, ClientError, CloudAgentCapacityError, ServerApiProvider,
};
use crate::settings::PrivacySettings;
use crate::terminal::view::ambient_agent::{SetupCommandGroupId, SetupCommandState};
use crate::terminal::CLIAgent;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::AdminEnablementSetting;

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

impl AgentProgress {
    fn new() -> Self {
        Self {
            spawned_at: Instant::now(),
            claimed_at: None,
            harness_started_at: None,
            stopped_at: None,
        }
    }

    pub fn setup_status_text(&self) -> &'static str {
        if self.harness_started_at.is_some() {
            "Starting Environment (Step 3/3)"
        } else if self.claimed_at.is_some() {
            "Creating Environment (Step 2/3)"
        } else {
            "Connecting to Host (Step 1/3)"
        }
    }
}

/// Identifies what kind of session startup the model is currently waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStartupKind {
    InitialRun,
    Followup,
}

/// Gates `submit_handoff` against double-submits.
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum HandoffSubmissionState {
    #[default]
    Idle,
    Queued,
    Starting,
}

/// Outcome of the chip-click async snapshot upload.
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum SnapshotUploadStatus {
    /// Upload is still in flight, or has not started yet.
    #[default]
    Pending,
    /// Touched workspace was empty so no upload happened. The cloud agent will
    /// start with no rehydration content.
    SkippedEmptyWorkspace,
    /// Upload succeeded; the inner token is sent to the server on spawn.
    Uploaded(InitialSnapshotToken),
    /// Upload failed. The error message is surfaced as a toast via
    /// `HandoffSnapshotUploadFailed`.
    Failed(String),
}

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
impl SnapshotUploadStatus {
    /// True when the upload has resolved to a final state. `Pending` blocks
    /// submit; all other variants (including `Failed`) allow it to proceed.
    /// On `Failed`, `initial_snapshot_token()` returns `None` so the cloud
    /// run starts without a local file snapshot.
    fn is_settled(&self) -> bool {
        !matches!(self, Self::Pending)
    }

    /// Returns the initial snapshot token to send on spawn, if any.
    fn initial_snapshot_token(&self) -> Option<InitialSnapshotToken> {
        match self {
            Self::Uploaded(token) => Some(token.clone()),
            Self::SkippedEmptyWorkspace | Self::Pending | Self::Failed(_) => None,
        }
    }
}

/// Per-pane handoff context. Seeded by the chip / slash command's open path on a
/// fresh cloud-mode pane and consumed by `submit_handoff`. Its presence is the
/// single source of truth for "this pane is in handoff mode" via
/// `is_local_to_cloud_handoff()`.
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
#[derive(Debug, Clone)]
pub(crate) struct PendingHandoff {
    /// Forked conversation id minted by `POST /agent/conversations/{conversation_id}/fork`.
    /// Sent under `conversation_id` on the subsequent `POST /agent/runs` request.
    /// `None` for fresh cloud launches that have no source conversation to fork.
    pub(crate) forked_conversation_id: Option<String>,
    /// Title override for the cloud run (e.g. "<title> (Moved to cloud)").
    pub(crate) title: Option<String>,
    /// `None` until `derive_touched_workspace` completes.
    pub(crate) touched_workspace: Option<TouchedWorkspace>,
    /// Outcome of the async snapshot upload.
    pub(crate) snapshot_upload: SnapshotUploadStatus,
    /// Gates submit — prevents double-submitting while the spawn is in flight.
    pub(crate) submission_state: HandoffSubmissionState,
    /// When the user types `& query` or `/handoff query`, the launch payload is
    /// stashed here so `maybe_auto_submit_handoff` can consume it once
    /// the touched workspace and snapshot upload have settled.
    pub(crate) auto_submit: Option<PendingCloudLaunch>,
}

/// Status of the ambient agent run.
#[derive(Debug, Clone)]
pub enum Status {
    /// First-time environment setup for cloud agents.
    Setup,
    /// The user is composing their ambient agent prompt.
    Composing,
    /// Waiting for the ambient agent run to be ready.
    WaitingForSession {
        progress: AgentProgress,
        kind: SessionStartupKind,
    },
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

    /// Selected cloud environment to launch the ambient agent with.
    environment_id: Option<SyncId>,
    /// True when `environment_id` came from an existing run config rather than from local
    /// environment selection/defaulting. Existing runs may reference an environment before the
    /// local CloudModel has loaded it, so initial-load validation should not clear it.
    environment_id_from_viewed_task: bool,

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
    /// Selected worker host for the cloud agent run. Populated from the HostSelector
    /// (which resolves env var > workspace setting) and read by `spawn_agent`.
    worker_host: Option<String>,
    /// Selected model id for a third-party harness (e.g. `"opus"` for Claude).
    harness_model_id: Option<String>,
    /// Optional reasoning level for the selected harness model.
    harness_reasoning_level: Option<String>,
    /// Name of the selected auth secret for the current non-Oz harness.
    harness_auth_secret_name: Option<String>,
    /// Whether the harness CLI
    /// Used to transition the cloud-mode setup UI out of the pre-first-exchange phase when
    /// there is no oz `AppendedExchange` to key off of.
    harness_command_started: bool,

    /// Session ID for the currently running ambient execution, if the run has attached to a live
    /// shared session.
    active_execution_session_id: Option<SessionId>,
    /// Session ID for the most recently finished ambient execution.
    /// Used as the previous session ID when submitting a follow-up so polling can wait for a
    /// different fresh session after the prior execution has ended.
    last_ended_execution_session_id: Option<SessionId>,

    /// Prompt text for a follow-up that has been submitted but not yet attached to a new session.
    pending_followup_prompt: Option<String>,

    /// See [`PendingHandoff`].
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pending_handoff: Option<PendingHandoff>,
}

impl AmbientAgentViewModel {
    pub fn new(terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        ctx.subscribe_to_model(&HarnessAvailabilityModel::handle(ctx), |me, _event, ctx| {
            me.validate_selected_harness(ctx);
        });

        ctx.subscribe_to_model(&GitHubAuthNotifier::handle(ctx), |me, event, ctx| {
            if matches!(event, GitHubAuthEvent::AuthCompleted) {
                me.handle_github_auth_completed(ctx);
            }
        });

        // Validate the default environment once Warp Drive sync completes.
        // The environment ID may be restored from settings before environments are synced,
        // so we need to validate it once the initial load is complete.
        let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
        ctx.spawn(initial_load_complete, |me, _, ctx| {
            me.validate_environment_after_initial_load(ctx);
        });

        let ui_state = AmbientAgentProgressUIState::new(ctx);

        let harness = Harness::default();
        let availability = HarnessAvailabilityModel::as_ref(ctx);
        // If the default harness is not available, find the first available one.
        let harness = if !availability.is_harness_enabled(harness) {
            availability
                .available_harnesses()
                .iter()
                .find(|h| h.enabled)
                .map(|h| h.harness)
                .unwrap_or(harness)
        } else {
            harness
        };

        Self {
            status: Status::Composing,
            request: None,
            terminal_view_id,
            environment_id: None,
            environment_id_from_viewed_task: false,
            progress_timer_handle: None,
            ui_state,
            setup_commands_state: Default::default(),
            task_id: None,
            conversation_id: None,
            harness,
            worker_host: None,
            harness_model_id: None,
            harness_reasoning_level: None,
            harness_auth_secret_name: None,
            harness_command_started: false,
            active_execution_session_id: None,
            last_ended_execution_session_id: None,
            pending_followup_prompt: None,
            #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
            pending_handoff: None,
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

    pub(super) fn start_new_setup_command_group(&mut self, ctx: &mut ModelContext<Self>) {
        self.setup_commands_state.start_new_group();
        self.harness_command_started = false;
        ctx.emit(AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility);
    }

    pub(super) fn finish_setup_command_group(
        &mut self,
        group_id: SetupCommandGroupId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.setup_commands_state.is_running(group_id) {
            self.setup_commands_state.finish_group(group_id);
            ctx.emit(AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility);
        }
    }

    pub(super) fn set_setup_command_group_visibility(
        &mut self,
        group_id: SetupCommandGroupId,
        is_visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if is_visible != self.setup_commands_state.should_expand(group_id) {
            self.setup_commands_state
                .set_should_expand(group_id, is_visible);
            ctx.emit(AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility);
        }
    }

    pub(super) fn set_setup_command_visibility(
        &mut self,
        is_visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let group_id = self.setup_commands_state.current_group_id();
        self.set_setup_command_group_visibility(group_id, is_visible, ctx);
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
            if self.environment_id_from_viewed_task {
                return;
            }
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
            Status::WaitingForSession { progress, .. }
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
        if self.is_local_to_cloud_handoff() {
            Harness::Oz
        } else {
            self.harness
        }
    }

    pub fn set_harness(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        // for local to cloud handoff, oz is the only option
        // (we'll need to update this to lock to the correct 3p harness if/when
        // we implement local -> cloud handoff for non-oz conversations).
        let harness = if self.is_local_to_cloud_handoff() {
            Harness::Oz
        } else {
            harness
        };

        if self.harness == harness {
            return;
        }
        self.harness = harness;
        self.harness_model_id = None;
        self.harness_reasoning_level = None;
        self.harness_auth_secret_name = None;
        ctx.emit(AmbientAgentViewModelEvent::HarnessSelected);
    }

    pub fn set_worker_host(&mut self, worker_host: Option<String>) {
        self.worker_host = worker_host;
    }

    pub fn selected_harness_model_id(&self) -> Option<&str> {
        self.harness_model_id.as_deref()
    }

    pub fn selected_harness_reasoning_level(&self) -> Option<&str> {
        self.harness_reasoning_level.as_deref()
    }

    pub fn set_harness_model_selection(
        &mut self,
        harness_model_id: Option<String>,
        reasoning_level: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.harness_model_id == harness_model_id
            && self.harness_reasoning_level == reasoning_level
        {
            return;
        }
        self.harness_model_id = harness_model_id;
        self.harness_reasoning_level = reasoning_level;
        ctx.emit(AmbientAgentViewModelEvent::HarnessModelSelected);
    }

    pub fn selected_harness_auth_secret_name(&self) -> Option<&str> {
        self.harness_auth_secret_name.as_deref()
    }

    pub fn set_harness_auth_secret_name(
        &mut self,
        name: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.harness_auth_secret_name == name {
            return;
        }
        self.harness_auth_secret_name = name;
        ctx.emit(AmbientAgentViewModelEvent::AuthSecretSelected);
    }

    /// True when the run is configured to use a non-Oz execution harness and the
    /// required feature flags are enabled.
    pub(super) fn is_third_party_harness(&self) -> bool {
        FeatureFlag::AgentHarness.is_enabled() && self.selected_harness() != Harness::Oz
    }

    /// Returns the [`CLIAgent`] corresponding to the currently selected harness when it is a
    /// third-party harness (e.g. Claude, Gemini). Returns `None` for [`Harness::Oz`].
    /// Used to drive the correct tab icon for a cloud run as soon as a non-oz harness is
    /// selected, even before the CLI session is registered with [`CLIAgentSessionsModel`].
    pub fn selected_third_party_cli_agent(&self) -> Option<CLIAgent> {
        CLIAgent::from_harness(self.selected_harness())
    }

    /// True when this pane is a local-to-cloud handoff pane. Set when the handoff opens
    /// the pane and stays true through and past the spawn.
    pub(crate) fn is_local_to_cloud_handoff(&self) -> bool {
        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        {
            self.pending_handoff.is_some()
        }
        #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
        {
            false
        }
    }

    /// True when this pane is a handoff pane and the touched-workspace derivation +
    /// snapshot upload have both settled and no submission is in flight. Used by the
    /// input layer to gate clearing the editor buffer on submit.
    pub(crate) fn is_handoff_ready_to_submit(&self) -> bool {
        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        {
            let Some(handoff) = self.pending_handoff.as_ref() else {
                return false;
            };
            handoff.touched_workspace.is_some()
                && handoff.snapshot_upload.is_settled()
                && matches!(
                    handoff.submission_state,
                    HandoffSubmissionState::Idle | HandoffSubmissionState::Queued
                )
        }
        #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
        {
            false
        }
    }

    /// Seeds the handoff context onto this pane. Called by the workspace bootstrap
    /// after splitting in a fresh cloud-mode pane and entering agent view.
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn set_pending_handoff(
        &mut self,
        pending: Option<PendingHandoff>,
        ctx: &mut ModelContext<Self>,
    ) {
        let previous_harness = self.selected_harness();
        self.pending_handoff = pending;
        if self.selected_harness() != previous_harness {
            ctx.emit(AmbientAgentViewModelEvent::HarnessSelected);
        }
        ctx.emit(AmbientAgentViewModelEvent::PendingHandoffChanged);
    }

    /// Updates the touched workspace once async derivation completes.
    /// No-op when no handoff context is set.
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn set_pending_handoff_workspace(
        &mut self,
        workspace: TouchedWorkspace,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(handoff) = self.pending_handoff.as_mut() else {
            return;
        };
        handoff.touched_workspace = Some(workspace);
        ctx.emit(AmbientAgentViewModelEvent::PendingHandoffChanged);
    }

    /// Records the outcome of the async snapshot upload.
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn set_pending_handoff_snapshot_upload(
        &mut self,
        snapshot_upload: SnapshotUploadStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(handoff) = self.pending_handoff.as_mut() else {
            return;
        };
        handoff.snapshot_upload = snapshot_upload;
        ctx.emit(AmbientAgentViewModelEvent::PendingHandoffChanged);
    }

    /// Records a snapshot upload failure on the pending handoff. The run will
    /// proceed without a local file snapshot (the cloud agent still has the
    /// forked conversation context).
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn record_handoff_snapshot_upload_failed(
        &mut self,
        error_message: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_pending_handoff_snapshot_upload(
            SnapshotUploadStatus::Failed(error_message.clone()),
            ctx,
        );
        ctx.emit(AmbientAgentViewModelEvent::HandoffSnapshotUploadFailed { error_message });
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    fn build_handoff_spawn_request(
        &self,
        prompt: String,
        attachments: Vec<AttachmentInput>,
        forked_conversation_id: Option<String>,
        initial_snapshot_token: Option<InitialSnapshotToken>,
        ctx: &AppContext,
    ) -> SpawnAgentRequest {
        let config = Some(self.build_default_spawn_config(ctx));
        let (prompt, mode) = extract_user_query_mode(prompt);
        SpawnAgentRequest {
            prompt,
            mode,
            config,
            title: self.pending_handoff.as_ref().and_then(|h| h.title.clone()),
            team: None,
            skill: None,
            attachments,
            interactive: None,
            parent_run_id: None,
            runtime_skills: vec![],
            referenced_attachments: vec![],
            conversation_id: forked_conversation_id,
            initial_snapshot_token,
            agent_identity_uid: None,
            snapshot_disabled: should_disable_snapshot(ctx).then_some(true),
        }
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn queue_handoff_auto_submit(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        let Some((launch, forked_conversation_id)) = ({
            let handoff = self.pending_handoff.as_mut();
            handoff.and_then(|handoff| {
                if !matches!(handoff.submission_state, HandoffSubmissionState::Idle) {
                    return None;
                }
                let launch = handoff.auto_submit.as_ref()?.clone();
                handoff.submission_state = HandoffSubmissionState::Queued;
                Some((launch, handoff.forked_conversation_id.clone()))
            })
        }) else {
            return false;
        };

        self.request = Some(self.build_handoff_spawn_request(
            launch.prompt,
            launch.attachments.request_attachments,
            forked_conversation_id,
            None,
            ctx,
        ));
        self.status = Status::WaitingForSession {
            progress: AgentProgress::new(),
            kind: SessionStartupKind::InitialRun,
        };
        self.start_progress_timer(ctx);
        ctx.emit(AmbientAgentViewModelEvent::PendingHandoffChanged);
        ctx.emit(AmbientAgentViewModelEvent::DispatchedAgent);
        true
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn maybe_auto_submit_handoff(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<PendingCloudLaunch> {
        let handoff = self.pending_handoff.as_mut()?;
        if handoff.touched_workspace.is_none()
            || !handoff.snapshot_upload.is_settled()
            || !matches!(
                handoff.submission_state,
                HandoffSubmissionState::Idle | HandoffSubmissionState::Queued
            )
        {
            return None;
        }
        let launch = handoff.auto_submit.take()?;
        ctx.emit(AmbientAgentViewModelEvent::PendingHandoffChanged);
        Some(launch)
    }

    /// Whether the harness CLI has started running. Only meaningful for non-oz runs.
    pub(super) fn harness_command_started(&self) -> bool {
        self.harness_command_started
    }

    /// Marks the harness CLI as started and emits `HarnessCommandStarted`.
    /// Idempotent: subsequent calls after the first are no-ops and do not re-emit.
    pub(super) fn mark_harness_command_started(
        &mut self,
        block_id: BlockId,
        ctx: &mut ModelContext<Self>,
    ) {
        debug_assert!(
            self.harness != Harness::Oz,
            "harness_command_started is only meaningful for non-oz runs"
        );
        if self.harness_command_started {
            return;
        }
        self.harness_command_started = true;
        ctx.emit(AmbientAgentViewModelEvent::HarnessCommandStarted { block_id });
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
        self.environment_id_from_viewed_task = false;
        ctx.emit(AmbientAgentViewModelEvent::EnvironmentSelected);
    }

    /// Resets to the first enabled harness if the current selection is no longer enabled.
    fn validate_selected_harness(&mut self, ctx: &mut ModelContext<Self>) {
        let model = HarnessAvailabilityModel::as_ref(ctx);
        if !model.is_harness_enabled(self.harness) {
            if let Some(first_enabled) = model.available_harnesses().iter().find(|h| h.enabled) {
                self.set_harness(first_enabled.harness, ctx);
            }
        }
    }

    /// Whether or not this terminal session is for an ambient agent.
    pub fn is_ambient_agent(&self) -> bool {
        true
    }

    /// Returns the task ID for the current cloud agent task, if one has been spawned.
    pub fn task_id(&self) -> Option<AmbientAgentTaskId> {
        self.task_id
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

    /// Returns true when an existing ambient task can accept a follow-up prompt.
    ///
    /// `AgentRunning` means this pane has moved past setup/composition into an ambient task view;
    /// `active_execution_session_id` is the live-session signal. After a Cloud Mode execution ends,
    /// the status stays `AgentRunning` while the active session is cleared, which is the editable
    /// post-run state where follow-ups are allowed.
    pub fn is_ready_for_cloud_followup_prompt(&self) -> bool {
        self.task_id.is_some()
            && self.active_execution_session_id.is_none()
            && self.pending_followup_prompt.is_none()
            && matches!(self.status, Status::AgentRunning)
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

        self.status = Status::AgentRunning;
        ctx.emit(AmbientAgentViewModelEvent::RunLifecycleChanged);

        // Fetch the task so we can set the correct environment (instead of defaulting to the most
        // recently-used one), harness, and harness model (so non-oz viewers know to use the
        // queued-prompt / harness-command-started flow).
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            |me, result, ctx| match result {
                Ok(task) => {
                    me.apply_viewed_task_config_snapshot(task.agent_config_snapshot.as_ref(), ctx);
                    ctx.emit(AmbientAgentViewModelEvent::ViewerHarnessResolved);
                }
                Err(err) => {
                    log::warn!("Failed to fetch ambient agent task for shared session: {err}");
                    me.set_environment_id(None, ctx);
                }
            },
        );
    }

    /// Applies the run configuration for an existing shared ambient session.
    ///
    /// Viewed sessions can join before Warp Drive has loaded the referenced environment object,
    /// especially on web. Preserve the server-provided environment ID anyway so the selector does
    /// not fall back to an unrelated default while waiting for the environment object to arrive.
    fn apply_viewed_task_config_snapshot(
        &mut self,
        snapshot: Option<&AgentConfigSnapshot>,
        ctx: &mut ModelContext<Self>,
    ) {
        let environment_id = snapshot
            .and_then(|s| s.environment_id.as_deref())
            .and_then(|id| ServerId::try_from(id).ok())
            .map(SyncId::ServerId);
        self.set_environment_id_from_viewed_task(environment_id, ctx);

        if let Some(model_id) = snapshot.and_then(|s| s.model_id.as_deref()) {
            LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                prefs.update_preferred_agent_mode_llm(
                    &LLMId::from(model_id),
                    self.terminal_view_id,
                    ctx,
                )
            });
        }

        let harness_config = snapshot.and_then(|s| s.harness.as_ref());
        let harness = harness_config
            .map(|h| h.harness_type)
            .unwrap_or(Harness::Oz);
        let harness_model_id = harness_config.and_then(|h| h.model_id.clone());
        let harness_reasoning_level = harness_config.and_then(|h| h.reasoning_level.clone());

        self.set_harness(harness, ctx);
        self.set_harness_model_selection(harness_model_id, harness_reasoning_level, ctx);
    }

    fn set_environment_id_from_viewed_task(
        &mut self,
        environment_id: Option<SyncId>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.environment_id == environment_id {
            return;
        }
        self.environment_id_from_viewed_task = environment_id.is_some();
        self.environment_id = environment_id;
        ctx.emit(AmbientAgentViewModelEvent::EnvironmentSelected);
    }

    pub fn record_ambient_execution_ended(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.active_execution_session_id.as_ref() == Some(&session_id) {
            self.active_execution_session_id = None;
            ctx.emit(AmbientAgentViewModelEvent::RunLifecycleChanged);
        }
        self.last_ended_execution_session_id = Some(session_id);
    }

    /// Attach a new execution session to an existing ambient agent pane (e.g. when the
    /// owner reopens a cloud conversation whose orchestrator has spun up a follow-up
    /// session). The emitted `ExecutionSessionReady` event drives the view-side
    /// `TerminalManager::attach_execution_session` swap to the new shared session.
    pub fn attach_execution_session(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.stop_progress_timer();
        self.active_execution_session_id = Some(session_id);
        self.last_ended_execution_session_id = None;
        self.pending_followup_prompt = None;
        self.status = Status::AgentRunning;
        ctx.emit(AmbientAgentViewModelEvent::ExecutionSessionReady { session_id });
    }

    pub fn submit_cloud_followup(&mut self, prompt: String, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::HandoffCloudCloud.is_enabled() {
            log::warn!("Attempted to submit cloud follow-up while HandoffCloudCloud is disabled");
            return;
        }

        let Some(task_id) = self.task_id else {
            log::warn!("Attempted to submit cloud follow-up without an ambient task ID");
            return;
        };

        let previous_session_id = self
            .active_execution_session_id
            .or(self.last_ended_execution_session_id);
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let stream = submit_run_followup(
            prompt.clone(),
            task_id,
            previous_session_id,
            ai_client,
            None,
        );

        self.pending_followup_prompt = Some(prompt);
        self.status = Status::WaitingForSession {
            progress: AgentProgress::new(),
            kind: SessionStartupKind::Followup,
        };
        self.start_progress_timer(ctx);
        ctx.emit(AmbientAgentViewModelEvent::FollowupDispatched);

        ctx.spawn_stream_local(
            stream,
            |me, event_result, ctx| me.handle_ambient_agent_event_result(event_result, ctx),
            |_me, _ctx| {},
        );
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn pending_followup_prompt(&self) -> Option<&str> {
        self.pending_followup_prompt.as_deref()
    }

    pub fn should_show_followup_progress(&self) -> bool {
        self.pending_followup_prompt.is_some()
            && matches!(
                self.status,
                Status::WaitingForSession { .. }
                    | Status::Failed { .. }
                    | Status::NeedsGithubAuth { .. }
                    | Status::Cancelled { .. }
            )
    }

    /// Reset cloud-specific prompt state so a retained cloud view can compose a new task.
    pub fn reset_for_new_cloud_prompt(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = Status::Composing;
        self.environment_id = None;
        self.environment_id_from_viewed_task = false;
        self.task_id = None;
        self.conversation_id = None;
        self.harness_model_id = None;
        self.harness_reasoning_level = None;
        self.harness_command_started = false;
        self.active_execution_session_id = None;
        self.last_ended_execution_session_id = None;
        self.pending_followup_prompt = None;
        self.request = None;
        self.setup_commands_state = Default::default();
        self.stop_progress_timer();
        ctx.notify();
    }

    /// Sets the local conversation ID associated with this cloud agent run.
    pub fn set_conversation_id(&mut self, id: Option<AIConversationId>) {
        self.conversation_id = id;
    }

    /// Builds the default `AgentConfigSnapshot` for spawning a cloud agent from this pane.
    ///
    /// Reads the user's preferred model, computer-use autonomy, optional self-hosted
    /// host (`WARP_CLOUD_MODE_DEFAULT_HOST`), and the pane's currently-selected env
    /// and harness. Shared by `spawn_agent` and the local-to-cloud handoff path so
    /// both flows route to the same worker host and inherit the same defaults.
    pub(crate) fn build_default_spawn_config(&self, ctx: &AppContext) -> AgentConfigSnapshot {
        let selected_harness = self.selected_harness();
        let computer_use_enabled = if selected_harness == Harness::Oz {
            // If the harness is Oz, determine computer use based on workspace AI autonomy settings.
            let CloudAgentComputerUseState { enabled, .. } =
                ComputerUsePermission::resolve_cloud_agent_state(ctx);
            Some(enabled)
        } else {
            None
        };

        let oz_model = (selected_harness == Harness::Oz).then(|| {
            LLMPreferences::as_ref(ctx)
                .get_active_base_model(ctx, Some(self.terminal_view_id))
                .id
                .to_string()
        });
        let third_party_harness = (selected_harness != Harness::Oz).then(|| HarnessConfig {
            harness_type: selected_harness,
            model_id: self.harness_model_id.clone(),
            reasoning_level: self.harness_reasoning_level.clone(),
        });

        let harness_auth_secrets =
            self.harness_auth_secret_name
                .as_ref()
                .and_then(|name| match selected_harness {
                    Harness::Claude => Some(HarnessAuthSecretsConfig {
                        claude_auth_secret_name: Some(name.clone()),
                        codex_auth_secret_name: None,
                    }),
                    Harness::Codex => Some(HarnessAuthSecretsConfig {
                        claude_auth_secret_name: None,
                        codex_auth_secret_name: Some(name.clone()),
                    }),
                    _ => None,
                });

        AgentConfigSnapshot {
            environment_id: self.environment_id.as_ref().map(|id| id.to_string()),
            model_id: oz_model,
            computer_use_enabled,
            worker_host: self.worker_host.clone(),
            harness: third_party_harness,
            harness_auth_secrets,
            ..Default::default()
        }
    }

    /// Spawn an ambient agent with the given prompt and current session configuration.
    pub fn spawn_agent(
        &mut self,
        prompt: String,
        attachments: Vec<AttachmentInput>,
        ctx: &mut ModelContext<Self>,
    ) {
        let config = Some(self.build_default_spawn_config(ctx));

        let (prompt, mode) = extract_user_query_mode(prompt);
        let request = SpawnAgentRequest {
            prompt,
            mode,
            config,
            title: None,
            team: None,
            agent_identity_uid: None,
            skill: None,
            attachments,
            interactive: None,
            parent_run_id: None,
            runtime_skills: vec![],
            referenced_attachments: vec![],
            conversation_id: None,
            initial_snapshot_token: None,
            snapshot_disabled: should_disable_snapshot(ctx).then_some(true),
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
            self.environment_id_from_viewed_task = false;

            if let Some(model_id) = config.model_id.as_deref() {
                LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                    prefs.update_preferred_agent_mode_llm(
                        &LLMId::from(model_id),
                        self.terminal_view_id,
                        ctx,
                    )
                });
            }
            if let Some(harness) = config.harness.as_ref() {
                self.harness = harness.harness_type;
                self.harness_model_id = harness.model_id.clone();
                self.harness_reasoning_level = harness.reasoning_level.clone();
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
            |me, event_result, ctx| me.handle_ambient_agent_event_result(event_result, ctx),
            |_me, _ctx| {},
        );

        self.status = Status::WaitingForSession {
            progress: AgentProgress::new(),
            kind: SessionStartupKind::InitialRun,
        };
        self.start_progress_timer(ctx);
        ctx.emit(AmbientAgentViewModelEvent::DispatchedAgent);
    }

    fn handle_ambient_agent_event_result(
        &mut self,
        event_result: Result<AmbientAgentEvent, anyhow::Error>,
        ctx: &mut ModelContext<Self>,
    ) {
        let ignore_events = matches!(
            self.status,
            Status::Cancelled { .. } | Status::Failed { .. }
        );

        match event_result {
            Ok(event) => self.handle_ambient_agent_event(event, ignore_events, ctx),
            Err(err) => {
                if ignore_events {
                    return;
                }
                self.handle_ambient_agent_stream_error(err, ctx);
            }
        }
    }

    fn handle_ambient_agent_event(
        &mut self,
        event: AmbientAgentEvent,
        ignore_events: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            AmbientAgentEvent::TaskSpawned { task_id, run_id } => {
                self.task_id = Some(task_id);
                if matches!(self.status, Status::Cancelled { .. }) {
                    log::info!(
                        "Received task_id after cancellation, sending server cancellation for task {}",
                        task_id
                    );
                    let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
                    ctx.spawn(
                        async move {
                            if let Err(e) = ai_client.cancel_ambient_agent_task(&task_id).await {
                                log::error!(
                                    "Failed to cancel ambient agent task {}: {:?}",
                                    task_id,
                                    e
                                );
                            }
                        },
                        |_, _, _| {},
                    );
                    return;
                }

                if let Some(conversation_id) = self.conversation_id {
                    let terminal_view_id = self.terminal_view_id;
                    let spawned_task_id = Some(task_id);
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history.assign_run_id_for_conversation(
                            conversation_id,
                            run_id,
                            spawned_task_id,
                            terminal_view_id,
                            ctx,
                        );
                    });
                }

                ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.register_ambient_session(self.terminal_view_id, task_id, ctx);
                });

                ctx.emit(AmbientAgentViewModelEvent::ProgressUpdated);
            }
            AmbientAgentEvent::StateChanged {
                state,
                status_message,
            } => {
                if ignore_events {
                    return;
                }

                if let Status::WaitingForSession { progress, .. } = &mut self.status {
                    match state {
                        AmbientAgentTaskState::Cancelled => {
                            self.handle_cancellation(ctx);
                        }
                        AmbientAgentTaskState::Queued | AmbientAgentTaskState::Pending => {
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
                            self.handle_spawn_error(error, ctx);
                        }
                    }
                }
            }
            AmbientAgentEvent::SessionStarted { session_join_info } => {
                if ignore_events {
                    return;
                }

                if let Some(session_id) = session_join_info.session_id {
                    self.stop_progress_timer();
                    let event_session_id = session_id;
                    let event = match &self.status {
                        Status::WaitingForSession {
                            kind: SessionStartupKind::InitialRun,
                            ..
                        } => AmbientAgentViewModelEvent::SessionReady {
                            session_id: event_session_id,
                        },
                        Status::WaitingForSession {
                            kind: SessionStartupKind::Followup,
                            ..
                        }
                        | Status::AgentRunning => {
                            AmbientAgentViewModelEvent::ExecutionSessionReady {
                                session_id: event_session_id,
                            }
                        }
                        Status::Setup
                        | Status::Composing
                        | Status::Failed { .. }
                        | Status::NeedsGithubAuth { .. }
                        | Status::Cancelled { .. } => return,
                    };
                    self.active_execution_session_id = Some(session_id);
                    self.last_ended_execution_session_id = None;
                    self.pending_followup_prompt = None;
                    self.status = Status::AgentRunning;
                    ctx.emit(event);
                }
            }
            AmbientAgentEvent::AtCapacity => {
                if ignore_events {
                    return;
                }

                if matches!(self.status, Status::WaitingForSession { .. }) {
                    ctx.emit(AmbientAgentViewModelEvent::ShowCloudAgentCapacityModal);
                }
            }
            AmbientAgentEvent::TimedOut => {}
        }
    }

    fn handle_ambient_agent_stream_error(
        &mut self,
        err: anyhow::Error,
        ctx: &mut ModelContext<Self>,
    ) {
        let error_message = err.to_string();
        send_telemetry_from_ctx!(
            CloudAgentTelemetryEvent::DispatchFailed {
                error: error_message.clone()
            },
            ctx
        );

        if let Some(client_error) = err.downcast_ref::<ClientError>() {
            if let Some(auth_url) = &client_error.auth_url {
                self.handle_needs_github_auth(auth_url.clone(), client_error.error.clone(), ctx);
                return;
            }
        }
        if let Some(capacity_error) = err.downcast_ref::<CloudAgentCapacityError>() {
            self.handle_spawn_error(capacity_error.error.clone(), ctx);
            ctx.emit(AmbientAgentViewModelEvent::ShowCloudAgentCapacityModal);
            return;
        }
        if let Some(ai_api_error) = err.downcast_ref::<AIApiError>() {
            match ai_api_error {
                AIApiError::QuotaLimit {
                    user_display_message,
                } => {
                    let error_message = user_display_message
                        .clone()
                        .unwrap_or_else(|| OUT_OF_CREDITS_TASK_FAILURE_MESSAGE.to_string());
                    self.handle_spawn_error(error_message, ctx);
                    ctx.emit(AmbientAgentViewModelEvent::ShowAICreditModal);
                    return;
                }
                AIApiError::ServerOverloaded => {
                    self.handle_spawn_error(
                        SERVER_OVERLOADED_TASK_FAILURE_MESSAGE.to_string(),
                        ctx,
                    );
                    return;
                }
                _ => {}
            }
        }
        self.handle_spawn_error(error_message, ctx);
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
        let progress = if let Status::WaitingForSession { mut progress, .. } =
            std::mem::replace(&mut self.status, Status::Composing)
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
        self.pending_followup_prompt = None;
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
        let (progress, startup_kind) = if let Status::WaitingForSession { mut progress, kind } =
            std::mem::replace(&mut self.status, Status::Composing)
        {
            progress.stopped_at = Some(now);
            (progress, Some(kind))
        } else {
            // If not in WaitingForSession, create a new progress with current time.
            (
                AgentProgress {
                    spawned_at: now,
                    claimed_at: None,
                    harness_started_at: None,
                    stopped_at: Some(now),
                },
                None,
            )
        };

        if !matches!(startup_kind, Some(SessionStartupKind::InitialRun)) {
            self.request = None;
        };

        self.status = Status::NeedsGithubAuth {
            progress,
            error_message,
            auth_url: github_auth_url::cloud_setup_auth_url_with_next(&auth_url),
        };
        self.pending_followup_prompt = None;

        ctx.emit(AmbientAgentViewModelEvent::NeedsGithubAuth);
    }

    fn handle_github_auth_completed(&mut self, ctx: &mut ModelContext<Self>) {
        if !matches!(self.status, Status::NeedsGithubAuth { .. }) {
            return;
        }

        let Some(request) = self.request.clone() else {
            return;
        };

        self.spawn_internal(request, ctx);
    }

    /// Handles cancellation by transitioning to the Cancelled state.
    fn handle_cancellation(&mut self, ctx: &mut ModelContext<Self>) {
        self.stop_progress_timer();

        let now = Instant::now();

        // Extract or create progress tracking.
        let progress = if let Status::WaitingForSession { mut progress, .. } =
            std::mem::replace(&mut self.status, Status::Composing)
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
        self.pending_followup_prompt = None;
        #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
        if let Some(handoff) = self.pending_handoff.as_mut() {
            handoff.auto_submit = None;
        }

        ctx.emit(AmbientAgentViewModelEvent::Cancelled);
    }

    /// Drive the local-to-cloud handoff submission for this pane. Reads the cached
    /// forked conversation id and snapshot upload result off the pending handoff,
    /// then routes through `spawn_agent_with_request`. Caller must check
    /// `is_handoff_ready_to_submit`.
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn submit_handoff(
        &mut self,
        prompt: String,
        attachments: Vec<AttachmentInput>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(handoff) = self.pending_handoff.as_mut() else {
            log::warn!("submit_handoff called without pending_handoff set; ignoring");
            return;
        };
        if matches!(handoff.submission_state, HandoffSubmissionState::Starting) {
            // Double-submit guard: spawn already in flight.
            return;
        }
        if handoff.touched_workspace.is_none() {
            log::warn!("submit_handoff called before touched-workspace derivation completed");
            return;
        }
        if !handoff.snapshot_upload.is_settled() {
            log::warn!(
                "submit_handoff called with unsettled snapshot_upload: {:?}",
                handoff.snapshot_upload
            );
            return;
        }
        let initial_snapshot_token = handoff.snapshot_upload.initial_snapshot_token();
        let forked_conversation_id = handoff.forked_conversation_id.clone();
        handoff.submission_state = HandoffSubmissionState::Starting;
        ctx.emit(AmbientAgentViewModelEvent::PendingHandoffChanged);

        let request = self.build_handoff_spawn_request(
            prompt,
            attachments,
            forked_conversation_id,
            initial_snapshot_token,
            ctx,
        );
        self.spawn_agent_with_request(request, ctx);
    }

    #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
    pub(crate) fn submit_handoff(
        &mut self,
        prompt: String,
        attachments: Vec<AttachmentInput>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.spawn_agent(prompt, attachments, ctx);
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
    /// A follow-up execution has been submitted and is waiting for a new session.
    FollowupDispatched,
    /// The spawn progress has been updated (e.g., task claimed or in-progress).
    ProgressUpdated,
    /// The ambient agent has started sharing its session.
    SessionReady {
        session_id: SessionId,
    },
    /// An execution has started sharing a session for an already-canonical ambient pane.
    ExecutionSessionReady {
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
    /// A shared-session viewer resolved the run harness from the server task.
    ViewerHarnessResolved,
    /// The selected worker host changed via the HostSelector.
    HostSelected,
    /// The selected third-party harness model id changed (e.g. user picked `"opus"` for Claude).
    HarnessModelSelected,
    /// The harness CLI (for non-oz runs) has started executing in the shared session.
    /// Fires once per run and signals the transition out of the pre-first-exchange phase
    /// for claude / gemini / other third-party harnesses.
    HarnessCommandStarted {
        block_id: BlockId,
    },
    /// The pane's `pending_handoff` was updated.
    PendingHandoffChanged,
    /// The async handoff snapshot upload failed. The input layer subscribes to
    /// surface the error as a toast.
    HandoffSnapshotUploadFailed {
        error_message: String,
    },

    UpdatedSetupCommandVisibility,
    /// The selected harness auth secret changed.
    AuthSecretSelected,
    /// The run's task association or execution liveness changed in a way that
    /// may affect lock-dependent UI (e.g. the model selector). Fired when
    /// a task is attached to the view (transcript restore) or when an
    /// execution ends.
    RunLifecycleChanged,
}

pub(crate) fn should_disable_snapshot(ctx: &AppContext) -> bool {
    let privacy = PrivacySettings::as_ref(ctx);
    if !privacy.is_cloud_conversation_storage_enabled {
        return true;
    }
    matches!(
        UserWorkspaces::as_ref(ctx).get_cloud_conversation_storage_enablement_setting(),
        AdminEnablementSetting::Disable
    )
}

impl Entity for AmbientAgentViewModel {
    type Event = AmbientAgentViewModelEvent;
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
