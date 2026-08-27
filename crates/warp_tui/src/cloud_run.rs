//! Startup and retained-link state for a TUI cloud-child session.
//!
//! Ongoing run lifecycle remains authoritative in `BlocklistAIHistoryModel`;
//! this model covers the pre-run states that exist before history has a run ID.
use warp::tui_export::{
    AIConversationId, AmbientAgentTaskId, CloudAgentStartupBlocker, CloudAgentStartupFailure,
};
use warpui_core::{Entity, ModelContext};

/// Startup presentation before shared conversation lifecycle becomes authoritative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiCloudRunStartup {
    Dispatching,
    Blocked(CloudAgentStartupBlocker),
    Failed(CloudAgentStartupFailure),
    Spawned,
}

/// Per-session metadata for a cloud child session.
pub(crate) struct TuiCloudRunState {
    conversation_id: Option<AIConversationId>,
    startup: TuiCloudRunStartup,
    task_id: Option<AmbientAgentTaskId>,
    run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiCloudRunStateEvent {
    Updated,
}

impl TuiCloudRunState {
    pub(crate) fn new() -> Self {
        Self {
            conversation_id: None,
            startup: TuiCloudRunStartup::Dispatching,
            task_id: None,
            run_id: None,
        }
    }

    /// Builds the retained state for a cloud child restored from history.
    ///
    /// A restored child already has a stable task/run identity, so it starts in
    /// [`TuiCloudRunStartup::Spawned`] and never renders the "Starting cloud
    /// run…" dispatching state. Displayed lifecycle status is still derived from
    /// the restored `AIConversation` by [`crate::cloud_run_view::TuiCloudRunView`].
    pub(crate) fn new_restored(
        conversation_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        run_id: String,
    ) -> Self {
        Self {
            conversation_id: Some(conversation_id),
            startup: TuiCloudRunStartup::Spawned,
            task_id: Some(task_id),
            run_id: Some(run_id),
        }
    }

    pub(crate) fn conversation_id(&self) -> Option<AIConversationId> {
        self.conversation_id
    }

    pub(crate) fn startup(&self) -> &TuiCloudRunStartup {
        &self.startup
    }

    /// The server-assigned run ID for this cloud child, once spawned or restored. The web
    /// destination is resolved fresh from this at render/click time (see
    /// [`crate::cloud_run_view::TuiCloudRunView::display_state`]) rather than cached here, so a
    /// viewer's Factory access resolving after spawn still takes effect (APP-5583).
    pub(crate) fn run_id(&self) -> Option<&str> {
        self.run_id.as_deref()
    }

    pub(crate) fn set_conversation_id(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.conversation_id = Some(conversation_id);
        ctx.emit(TuiCloudRunStateEvent::Updated);
    }

    pub(crate) fn set_blocked(
        &mut self,
        blocker: CloudAgentStartupBlocker,
        ctx: &mut ModelContext<Self>,
    ) {
        self.startup = TuiCloudRunStartup::Blocked(blocker);
        ctx.emit(TuiCloudRunStateEvent::Updated);
    }

    pub(crate) fn set_failed(
        &mut self,
        failure: CloudAgentStartupFailure,
        ctx: &mut ModelContext<Self>,
    ) {
        self.startup = TuiCloudRunStartup::Failed(failure);
        ctx.emit(TuiCloudRunStateEvent::Updated);
    }

    pub(crate) fn set_spawned(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.task_id = Some(task_id);
        self.run_id = Some(run_id);
        self.startup = TuiCloudRunStartup::Spawned;
        ctx.emit(TuiCloudRunStateEvent::Updated);
    }
}

impl Entity for TuiCloudRunState {
    type Event = TuiCloudRunStateEvent;
}
