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
    run_url: Option<String>,
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
            run_url: None,
        }
    }

    pub(crate) fn conversation_id(&self) -> Option<AIConversationId> {
        self.conversation_id
    }

    pub(crate) fn startup(&self) -> &TuiCloudRunStartup {
        &self.startup
    }

    pub(crate) fn run_url(&self) -> Option<&str> {
        self.run_url.as_deref()
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
        run_url: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.task_id = Some(task_id);
        self.run_id = Some(run_id);
        self.run_url = Some(run_url);
        self.startup = TuiCloudRunStartup::Spawned;
        ctx.emit(TuiCloudRunStateEvent::Updated);
    }
}

impl Entity for TuiCloudRunState {
    type Event = TuiCloudRunStateEvent;
}
