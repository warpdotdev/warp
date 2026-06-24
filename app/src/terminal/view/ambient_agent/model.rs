use instant::Instant;
use session_sharing_protocol::common::SessionId;
use warp_cli::agent::Harness;
use warpui::{AppContext, Entity, EntityId, ModelContext};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::AmbientAgentTaskId;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::handoff::touched_repos::TouchedWorkspace;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::blocklist::handoff::PendingCloudLaunch;
use crate::server::ids::SyncId;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::server::server_api::ai::InitialSnapshotToken;
use crate::server::server_api::ai::{AgentConfigSnapshot, AttachmentInput, SpawnAgentRequest};
use crate::terminal::view::ambient_agent::{
    AmbientAgentProgressUIState, SetupCommandGroupId, SetupCommandState,
};
use crate::terminal::CLIAgent;

#[derive(Debug, Clone)]
pub struct AgentProgress {
    pub spawned_at: Instant,
    pub claimed_at: Option<Instant>,
    pub harness_started_at: Option<Instant>,
    pub stopped_at: Option<Instant>,
}

impl AgentProgress {
    pub fn setup_status_text(&self) -> &'static str {
        "Connecting to Host (Step 1/3)"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStartupKind {
    InitialRun,
    Followup,
}

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum HandoffSubmissionState {
    #[default]
    Idle,
    Queued,
    Starting,
}

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum SnapshotUploadStatus {
    #[default]
    Pending,
    SkippedEmptyWorkspace,
    Uploaded(InitialSnapshotToken),
    Failed(String),
}

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
impl SnapshotUploadStatus {
    pub(crate) fn is_settled(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
#[derive(Debug, Clone)]
pub(crate) struct PendingHandoff {
    pub(crate) forked_conversation_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) touched_workspace: Option<TouchedWorkspace>,
    pub(crate) snapshot_upload: SnapshotUploadStatus,
    pub(crate) submission_state: HandoffSubmissionState,
    pub(crate) auto_submit: Option<PendingCloudLaunch>,
    pub(crate) orchestration_handoff: Option<bool>,
    pub(crate) should_inject_continue: bool,
}

#[derive(Debug, Clone)]
pub enum Status {
    Setup,
    Composing,
    WaitingForSession {
        progress: AgentProgress,
        kind: SessionStartupKind,
    },
    AgentRunning,
    Failed {
        progress: AgentProgress,
        error_message: String,
    },
    NeedsGithubAuth {
        progress: AgentProgress,
        error_message: String,
        auth_url: String,
    },
    Cancelled {
        progress: AgentProgress,
    },
}

pub struct AmbientAgentViewModel {
    status: Status,
    setup_commands_state: SetupCommandState,
    conversation_id: Option<AIConversationId>,
    pub ui_state: AmbientAgentProgressUIState,
    selected_harness: Harness,
    selected_harness_model_id: Option<String>,
    selected_harness_reasoning_level: Option<String>,
    selected_harness_auth_secret_name: Option<String>,
    worker_host: Option<String>,
    pending_followup_prompt: Option<String>,
    harness_command_started: bool,
}

impl AmbientAgentViewModel {
    pub fn new(_terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) -> Self {
        Self {
            status: Status::Composing,
            setup_commands_state: SetupCommandState::default(),
            conversation_id: None,
            ui_state: AmbientAgentProgressUIState::new(ctx),
            selected_harness: Harness::Codex,
            selected_harness_model_id: None,
            selected_harness_reasoning_level: None,
            selected_harness_auth_secret_name: None,
            worker_host: None,
            pending_followup_prompt: None,
            harness_command_started: false,
        }
    }

    pub fn request(&self) -> Option<&SpawnAgentRequest> {
        None
    }
    pub fn setup_command_state(&self) -> &SetupCommandState {
        &self.setup_commands_state
    }
    pub fn setup_command_state_mut(&mut self) -> &mut SetupCommandState {
        &mut self.setup_commands_state
    }
    pub fn start_new_setup_command_group(&mut self, ctx: &mut ModelContext<Self>) {
        self.setup_commands_state.start_new_group();
        ctx.emit(AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility);
    }
    pub fn finish_setup_command_group(
        &mut self,
        group_id: SetupCommandGroupId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.setup_commands_state.finish_group(group_id);
        ctx.emit(AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility);
    }
    pub fn set_setup_command_group_visibility(
        &mut self,
        group_id: SetupCommandGroupId,
        visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.setup_commands_state
            .set_should_expand(group_id, visible);
        ctx.emit(AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility);
    }
    pub fn set_setup_command_visibility(&mut self, visible: bool, ctx: &mut ModelContext<Self>) {
        let group_id = self.setup_commands_state.current_group_id();
        self.set_setup_command_group_visibility(group_id, visible, ctx);
    }
    pub(crate) fn tear_down_active_setup_command_group(&mut self, _ctx: &mut ModelContext<Self>) {}
    pub fn agent_progress(&self) -> Option<&AgentProgress> {
        None
    }
    pub fn selected_environment_id(&self) -> Option<&SyncId> {
        None
    }
    pub fn selected_harness(&self) -> Harness {
        self.selected_harness
    }
    pub fn set_harness(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        if self.selected_harness != harness {
            self.selected_harness = harness;
            ctx.emit(AmbientAgentViewModelEvent::HarnessSelected);
        }
    }
    pub fn set_worker_host(&mut self, worker_host: Option<String>) {
        self.worker_host = worker_host;
    }
    pub fn selected_harness_model_id(&self) -> Option<&str> {
        self.selected_harness_model_id.as_deref()
    }
    pub fn selected_harness_reasoning_level(&self) -> Option<&str> {
        self.selected_harness_reasoning_level.as_deref()
    }
    pub fn set_harness_model_selection(
        &mut self,
        model_id: Option<String>,
        reasoning_level: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selected_harness_model_id = model_id;
        self.selected_harness_reasoning_level = reasoning_level;
        ctx.emit(AmbientAgentViewModelEvent::HarnessModelSelected);
    }
    pub fn selected_harness_auth_secret_name(&self) -> Option<&str> {
        self.selected_harness_auth_secret_name.as_deref()
    }
    pub fn set_harness_auth_secret_name(
        &mut self,
        name: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selected_harness_auth_secret_name = name;
        ctx.emit(AmbientAgentViewModelEvent::AuthSecretSelected);
    }
    pub fn is_third_party_harness(&self) -> bool {
        !matches!(self.selected_harness, Harness::Unknown)
    }
    pub fn selected_third_party_cli_agent(&self) -> Option<CLIAgent> {
        CLIAgent::from_harness(self.selected_harness)
            .filter(|agent| !matches!(agent, CLIAgent::Unknown))
    }
    pub(crate) fn is_local_to_cloud_handoff(&self) -> bool {
        false
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn is_handoff_ready_to_submit(&self) -> bool {
        false
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn set_pending_handoff(
        &mut self,
        _pending: Option<PendingHandoff>,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn set_pending_handoff_workspace(
        &mut self,
        _workspace: TouchedWorkspace,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn set_pending_handoff_snapshot_upload(
        &mut self,
        _status: SnapshotUploadStatus,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn record_handoff_snapshot_upload_failed(
        &mut self,
        _error_message: String,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn queue_handoff_auto_submit(&mut self, _ctx: &mut ModelContext<Self>) -> bool {
        false
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn maybe_auto_submit_handoff(&mut self, _ctx: &mut ModelContext<Self>) -> bool {
        false
    }
    pub fn set_environment_id(&mut self, _id: Option<SyncId>, _ctx: &mut ModelContext<Self>) {}
    pub fn is_ambient_agent(&self) -> bool {
        false
    }
    pub fn task_id(&self) -> Option<AmbientAgentTaskId> {
        None
    }
    pub fn is_in_setup(&self) -> bool {
        false
    }
    pub fn is_configuring_ambient_agent(&self) -> bool {
        false
    }
    pub fn is_waiting_for_session(&self) -> bool {
        false
    }
    pub fn is_failed(&self) -> bool {
        false
    }
    pub fn is_cancelled(&self) -> bool {
        false
    }
    pub fn is_needs_github_auth(&self) -> bool {
        false
    }
    pub fn is_agent_running(&self) -> bool {
        false
    }
    pub fn is_ready_for_cloud_followup_prompt(&self) -> bool {
        false
    }
    pub fn should_show_status_footer(&self) -> bool {
        false
    }
    pub fn error_message(&self) -> Option<&str> {
        None
    }
    pub fn github_auth_url(&self) -> Option<&str> {
        None
    }
    pub fn github_auth_error_message(&self) -> Option<&str> {
        None
    }
    pub fn enter_setup(&mut self, _ctx: &mut ModelContext<Self>) {}
    pub fn enter_composing_from_setup(&mut self, _ctx: &mut ModelContext<Self>) {}
    pub fn enter_viewing_existing_session(
        &mut self,
        _task_id: AmbientAgentTaskId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    pub fn record_ambient_execution_ended(
        &mut self,
        _session_id: SessionId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    pub fn blocks_cloud_followups(&self) -> bool {
        false
    }
    pub fn attach_execution_session(
        &mut self,
        _session_id: SessionId,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    pub fn submit_cloud_followup(&mut self, prompt: String, ctx: &mut ModelContext<Self>) {
        self.pending_followup_prompt = Some(prompt);
        ctx.emit(AmbientAgentViewModelEvent::FollowupDispatched);
    }
    pub fn status(&self) -> &Status {
        &self.status
    }
    pub fn pending_followup_prompt(&self) -> Option<&str> {
        self.pending_followup_prompt.as_deref()
    }
    pub fn should_show_followup_progress(&self) -> bool {
        false
    }
    pub fn reset_for_new_cloud_prompt(&mut self, _ctx: &mut ModelContext<Self>) {
        self.pending_followup_prompt = None;
        self.harness_command_started = false;
    }
    pub fn set_conversation_id(&mut self, id: Option<AIConversationId>) {
        self.conversation_id = id;
    }
    pub(crate) fn build_default_spawn_config(&self, _ctx: &AppContext) -> AgentConfigSnapshot {
        AgentConfigSnapshot::default()
    }
    pub fn spawn_agent(
        &mut self,
        _prompt: String,
        _attachments: Vec<AttachmentInput>,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    pub fn spawn_agent_with_request(
        &mut self,
        _request: SpawnAgentRequest,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn submit_handoff(
        &mut self,
        _prompt: String,
        _attachments: Vec<AttachmentInput>,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    #[cfg(not(all(feature = "local_fs", not(target_family = "wasm"))))]
    pub(crate) fn submit_handoff(
        &mut self,
        _prompt: String,
        _attachments: Vec<AttachmentInput>,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
    pub fn cancel_task(&mut self, _ctx: &mut ModelContext<Self>) {}
    pub fn harness_command_started(&self) -> bool {
        self.harness_command_started
    }
    pub fn mark_harness_command_started(
        &mut self,
        block_id: warp_terminal::model::BlockId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.harness_command_started = true;
        ctx.emit(AmbientAgentViewModelEvent::HarnessCommandStarted { block_id });
    }
}

impl Entity for AmbientAgentViewModel {
    type Event = AmbientAgentViewModelEvent;
}

#[derive(Debug)]
pub enum AmbientAgentViewModelEvent {
    EnteredSetupState,
    EnteredComposingState,
    DispatchedAgent,
    FollowupDispatched,
    ProgressUpdated,
    EnvironmentSelected,
    Failed {
        error_message: String,
    },
    ShowAICreditModal,
    NeedsGithubAuth,
    Cancelled,
    HarnessSelected,
    ViewerHarnessResolved,
    HostSelected,
    HarnessModelSelected,
    SessionReady {
        session_id: SessionId,
    },
    ExecutionSessionReady {
        session_id: SessionId,
    },
    HarnessCommandStarted {
        block_id: warp_terminal::model::BlockId,
    },
    PendingHandoffChanged,
    HandoffSnapshotUploadFailed {
        error_message: String,
    },
    UpdatedSetupCommandVisibility,
    AuthSecretSelected,
    RunLifecycleChanged,
}

pub(crate) fn should_disable_snapshot(_ctx: &AppContext) -> bool {
    true
}
