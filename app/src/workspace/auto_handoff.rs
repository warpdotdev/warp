use std::collections::HashSet;

use warp_core::send_telemetry_from_ctx;
use warpui::{
    AppContext, Entity, EntityId, ModelContext, SingletonEntity, TypedActionView, ViewHandle,
    WindowId,
};

use super::{
    AutoCloudHandoffTrigger, OneTimeModalModel, ToastStack, Workspace, WorkspaceAction,
    WorkspaceRegistry,
};
use crate::ai::active_agent_views_model::{ActiveAgentViewsModel, ConversationOrTaskId};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::ambient_agents::telemetry::CloudAgentTelemetryEvent;
use crate::ai::blocklist::orchestration_topology::has_local_orchestrated_children;
use crate::settings::AISettings;
use crate::system::{SystemStats, SystemStatsEvent};
use crate::terminal::view::TerminalView;
use crate::view_components::DismissibleToast;
use crate::BlocklistAIHistoryModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoCloudHandoffSkipReason {
    EmptyConversation,
    NotInProgress,
    MissingServerConversationToken,
    SharedSessionViewer,
    CloudHandoffUnavailable,
    OrchestratorWithLocalChildren,
    AlreadyAttempted,
    NoFocusedConversation,
    TerminalNotFound { terminal_view_id: EntityId },
    CloudPane,
    LongRunningCommand,
    ConversationNotLoaded { conversation_id: AIConversationId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AutoCloudHandoffEligibility {
    pub(crate) is_empty: bool,
    pub(crate) is_in_progress: bool,
    pub(crate) has_server_conversation_token: bool,
    pub(crate) is_viewing_shared_session: bool,
    pub(crate) can_handoff_to_cloud: bool,
    pub(crate) already_attempted: bool,
    /// True when the focused conversation is an orchestrator with at least one
    /// active local child agent. Handing such a session off to the cloud would
    /// fork only the parent and orphan its local children, so we skip it.
    pub(crate) has_local_orchestrated_children: bool,
}

impl AutoCloudHandoffEligibility {
    pub(crate) fn from_conversation(
        conversation: &AIConversation,
        can_handoff_to_cloud: bool,
        already_attempted: bool,
        has_local_orchestrated_children: bool,
    ) -> Self {
        Self {
            is_empty: conversation.is_empty(),
            is_in_progress: conversation.status().is_in_progress(),
            has_server_conversation_token: conversation.server_conversation_token().is_some(),
            is_viewing_shared_session: conversation.is_viewing_shared_session(),
            can_handoff_to_cloud,
            already_attempted,
            has_local_orchestrated_children,
        }
    }

    pub(crate) fn skip_reason(self) -> Option<AutoCloudHandoffSkipReason> {
        if self.already_attempted {
            return Some(AutoCloudHandoffSkipReason::AlreadyAttempted);
        }
        if self.is_viewing_shared_session {
            return Some(AutoCloudHandoffSkipReason::SharedSessionViewer);
        }
        if self.is_empty {
            return Some(AutoCloudHandoffSkipReason::EmptyConversation);
        }
        if !self.is_in_progress {
            return Some(AutoCloudHandoffSkipReason::NotInProgress);
        }
        if self.has_local_orchestrated_children {
            return Some(AutoCloudHandoffSkipReason::OrchestratorWithLocalChildren);
        }
        if !self.has_server_conversation_token {
            return Some(AutoCloudHandoffSkipReason::MissingServerConversationToken);
        }
        if !self.can_handoff_to_cloud {
            return Some(AutoCloudHandoffSkipReason::CloudHandoffUnavailable);
        }
        None
    }
}

pub(crate) struct AutoCloudHandoffRequest {
    workspace: ViewHandle<Workspace>,
    terminal_view_id: EntityId,
    conversation_id: AIConversationId,
    trigger: AutoCloudHandoffTrigger,
}

/// A focused local agent conversation that passed every auto-handoff
/// precondition, resolved to the views needed to dispatch the handoff.
struct AutoCloudHandoffCandidate {
    window_id: WindowId,
    workspace: ViewHandle<Workspace>,
    terminal_view_id: EntityId,
    conversation_id: AIConversationId,
}

impl AutoCloudHandoffRequest {
    fn dispatch(&self, ctx: &mut AppContext) {
        self.workspace.update(ctx, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::AutoHandoffActiveAgentToCloud {
                    terminal_view_id: self.terminal_view_id,
                    conversation_id: self.conversation_id,
                    trigger: self.trigger,
                },
                ctx,
            );
        });
    }
}
pub(crate) struct AutoCloudHandoffController {
    attempted_conversation_ids: HashSet<AIConversationId>,
    /// Set at sleep time when an eligible in-progress local agent run would have
    /// been handed off but `auto_handoff_on_sleep_enabled` is off. Consumed on
    /// wake to surface the discoverability modal.
    pending_sleep_prompt: bool,
    /// True between `CpuWillSleep` and `CpuWasAwakened`. Used to decide whether
    /// a handoff success toast can be shown right away or must wait for wake.
    is_system_sleeping: bool,
    /// Window of an automatic handoff that succeeded while the system was
    /// sleeping. Consumed on wake to show the success toast once the user can
    /// actually see it.
    pending_success_toast_window: Option<WindowId>,
}

impl AutoCloudHandoffController {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&SystemStats::handle(ctx), |controller, _, event, ctx| {
            controller.handle_system_stats_event(event, ctx);
        });

        Self {
            attempted_conversation_ids: HashSet::new(),
            pending_sleep_prompt: false,
            is_system_sleeping: false,
            pending_success_toast_window: None,
        }
    }

    /// Marks the attempt as succeeded and surfaces the success toast:
    /// immediately when the system is awake (e.g. the fork RPC resolved after
    /// wake), otherwise deferred until `CpuWasAwakened` so the ephemeral
    /// toast's dismissal timeout doesn't elapse while the user is away.
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn record_handoff_succeeded(
        &mut self,
        conversation_id: AIConversationId,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.attempted_conversation_ids.insert(conversation_id);

        if self.is_system_sleeping {
            self.pending_success_toast_window = Some(window_id);
        } else {
            Self::show_success_toast(window_id, ctx);
        }
    }
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn record_handoff_failed(&mut self, conversation_id: AIConversationId) {
        self.attempted_conversation_ids.remove(&conversation_id);
    }

    fn handle_system_stats_event(
        &mut self,
        event: &SystemStatsEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            SystemStatsEvent::CpuWillSleep => {
                self.is_system_sleeping = true;
                self.handle_cpu_will_sleep(ctx);
            }
            SystemStatsEvent::CpuWasAwakened => {
                self.is_system_sleeping = false;
                self.maybe_show_success_toast(ctx);
                self.maybe_show_sleep_prompt(ctx);
            }
        }
    }

    /// On wake, shows the success toast for an automatic handoff that
    /// completed while the system was sleeping.
    fn maybe_show_success_toast(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(window_id) = self.pending_success_toast_window.take() {
            Self::show_success_toast(window_id, ctx);
        }
    }

    fn show_success_toast(window_id: WindowId, ctx: &mut ModelContext<Self>) {
        log::info!("auto handoff: showing success toast in window {window_id:?}");
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(
                DismissibleToast::success("Handed session off to the cloud".to_owned()),
                window_id,
                ctx,
            );
        });
    }

    /// At sleep time, hands the focused eligible local agent run off to the
    /// cloud when `auto_handoff_on_sleep_enabled` is on. When the setting is
    /// off, records a pending discoverability prompt instead; the modal is
    /// surfaced on wake by [`Self::maybe_show_sleep_prompt`] and shown at most
    /// once per user (enforced by `OneTimeModalModel`).
    fn handle_cpu_will_sleep(&mut self, ctx: &mut ModelContext<Self>) {
        self.pending_sleep_prompt = false;

        let candidate = match self.evaluate_handoff_candidate(ctx) {
            Ok(candidate) => candidate,
            Err(reason) => {
                log::info!("auto handoff: skipping at sleep: {reason:?}");
                return;
            }
        };

        if AISettings::as_ref(ctx).is_auto_handoff_on_sleep_enabled(ctx) {
            self.dispatch_handoff(candidate, AutoCloudHandoffTrigger::MacOsSleep, ctx);
        } else {
            log::info!(
                "auto-handoff sleep prompt: recorded pending prompt for conversation {:?} in terminal {:?}",
                candidate.conversation_id,
                candidate.terminal_view_id,
            );
            self.pending_sleep_prompt = true;
        }
    }

    /// On wake, surfaces the discoverability modal recorded at sleep time, as
    /// long as the setting is still off. The modal itself is once-ever per
    /// user; `OneTimeModalModel` enforces that.
    fn maybe_show_sleep_prompt(&mut self, ctx: &mut ModelContext<Self>) {
        if !std::mem::take(&mut self.pending_sleep_prompt) {
            log::info!(
                "auto-handoff sleep prompt: nothing to show on wake, no pending prompt was recorded at sleep"
            );
            return;
        }

        if AISettings::as_ref(ctx).is_auto_handoff_on_sleep_enabled(ctx) {
            log::info!(
                "auto-handoff sleep prompt: not showing on wake, auto-handoff-on-sleep was enabled in the meantime"
            );
            return;
        }

        let shown = OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
            model.check_and_trigger_auto_handoff_sleep_modal(ctx)
        });
        if shown {
            log::info!("auto-handoff sleep prompt: showing modal on wake");
            send_telemetry_from_ctx!(CloudAgentTelemetryEvent::SleepPromptShown, ctx);
        } else {
            log::info!(
                "auto-handoff sleep prompt: not showing on wake, modal was already shown once"
            );
        }
    }

    fn trigger(&mut self, trigger: AutoCloudHandoffTrigger, ctx: &mut ModelContext<Self>) {
        if !Self::is_trigger_enabled(trigger, ctx) {
            log::info!(
                "auto handoff: skipping {trigger:?} trigger, auto-handoff-on-sleep is disabled"
            );
            return;
        }
        match self.evaluate_handoff_candidate(ctx) {
            Ok(candidate) => self.dispatch_handoff(candidate, trigger, ctx),
            Err(reason) => log::info!("auto handoff: skipping {trigger:?} trigger: {reason:?}"),
        }
    }

    /// Resolves the focused local agent conversation and checks every
    /// precondition shared by automatic handoff and the sleep discoverability
    /// prompt. Returns the resolved candidate, or the first reason it must be
    /// skipped.
    fn evaluate_handoff_candidate(
        &self,
        ctx: &ModelContext<Self>,
    ) -> Result<AutoCloudHandoffCandidate, AutoCloudHandoffSkipReason> {
        let Some((terminal_view_id, conversation_id)) = Self::last_focused_local_conversation(ctx)
        else {
            return Err(AutoCloudHandoffSkipReason::NoFocusedConversation);
        };

        let Some((window_id, workspace, terminal_view)) =
            Self::find_workspace_and_terminal(terminal_view_id, ctx)
        else {
            return Err(AutoCloudHandoffSkipReason::TerminalNotFound { terminal_view_id });
        };

        if terminal_view
            .as_ref(ctx)
            .ambient_agent_view_model()
            .is_some()
        {
            return Err(AutoCloudHandoffSkipReason::CloudPane);
        }

        if terminal_view.as_ref(ctx).has_active_long_running_command() {
            return Err(AutoCloudHandoffSkipReason::LongRunningCommand);
        }

        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(conversation) = history.conversation(&conversation_id) else {
            return Err(AutoCloudHandoffSkipReason::ConversationNotLoaded { conversation_id });
        };

        let can_handoff_to_cloud = AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx);
        if let Some(reason) = AutoCloudHandoffEligibility::from_conversation(
            conversation,
            can_handoff_to_cloud,
            self.attempted_conversation_ids.contains(&conversation_id),
            has_local_orchestrated_children(history, conversation_id),
        )
        .skip_reason()
        {
            return Err(reason);
        }

        Ok(AutoCloudHandoffCandidate {
            window_id,
            workspace,
            terminal_view_id,
            conversation_id,
        })
    }

    /// Marks the candidate as attempted and emits the handoff request.
    fn dispatch_handoff(
        &mut self,
        candidate: AutoCloudHandoffCandidate,
        trigger: AutoCloudHandoffTrigger,
        ctx: &mut ModelContext<Self>,
    ) {
        self.attempted_conversation_ids
            .insert(candidate.conversation_id);

        log::info!(
            "Triggering auto handoff to cloud for conversation {:?} in window {:?} via {trigger:?}",
            candidate.conversation_id,
            candidate.window_id,
        );
        ctx.emit(AutoCloudHandoffRequest {
            workspace: candidate.workspace,
            terminal_view_id: candidate.terminal_view_id,
            conversation_id: candidate.conversation_id,
            trigger,
        });
    }

    fn last_focused_local_conversation(
        ctx: &ModelContext<Self>,
    ) -> Option<(EntityId, AIConversationId)> {
        let active_agent_views = ActiveAgentViewsModel::as_ref(ctx);
        let conversation_id = match active_agent_views.get_last_focused_conversation()? {
            ConversationOrTaskId::ConversationId(conversation_id) => conversation_id,
            ConversationOrTaskId::TaskId(_) => return None,
        };
        // The last-focused terminal id can go stale (e.g. its pane was closed
        // or swapped) while the conversation lives on in another view. Prefer
        // the history model's owner mapping — it's the same mapping the
        // handoff flow validates against — then the agent-view registry, and
        // only fall back to the last-focused id.
        let terminal_view_id = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(&conversation_id)
            .or_else(|| {
                active_agent_views.get_terminal_view_id_for_conversation(conversation_id, ctx)
            })
            .or_else(|| active_agent_views.get_last_focused_terminal_id())?;
        Some((terminal_view_id, conversation_id))
    }

    fn is_trigger_enabled(trigger: AutoCloudHandoffTrigger, ctx: &ModelContext<Self>) -> bool {
        match trigger {
            AutoCloudHandoffTrigger::MacOsSleep | AutoCloudHandoffTrigger::Uri => {
                AISettings::as_ref(ctx).is_auto_handoff_on_sleep_enabled(ctx)
            }
        }
    }
    fn find_workspace_and_terminal(
        terminal_view_id: EntityId,
        ctx: &ModelContext<Self>,
    ) -> Option<(WindowId, ViewHandle<Workspace>, ViewHandle<TerminalView>)> {
        WorkspaceRegistry::as_ref(ctx)
            .all_workspaces(ctx)
            .into_iter()
            .find_map(|(window_id, workspace)| {
                let terminal_view = workspace.as_ref(ctx).terminal_view(terminal_view_id, ctx)?;
                Some((window_id, workspace, terminal_view))
            })
    }
}

impl Entity for AutoCloudHandoffController {
    type Event = AutoCloudHandoffRequest;
}

impl SingletonEntity for AutoCloudHandoffController {}

pub(crate) fn init(app: &mut AppContext) {
    let controller = app.add_singleton_model(AutoCloudHandoffController::new);
    app.subscribe_to_model(&controller, |_, request, ctx| {
        request.dispatch(ctx);
    });
}

/// Triggers an auto-handoff to the cloud. This is the entry point for the
/// `warp://.../auto_handoff_to_cloud` URI action; the real macOS sleep path
/// goes through the `SystemStats` subscription instead.
///
/// Callers that dispatch from inside an in-progress workspace view update
/// (e.g. the debug palette entry) must defer past that update before calling
/// this: `update_view` temporarily removes the dispatching workspace from its
/// window, so the synchronous workspace lookup here would otherwise miss it.
pub(crate) fn trigger_auto_handoff_to_cloud(
    trigger: AutoCloudHandoffTrigger,
    ctx: &mut AppContext,
) {
    AutoCloudHandoffController::handle(ctx).update(ctx, |controller, ctx| {
        controller.trigger(trigger, ctx);
    });
}

#[cfg(test)]
#[path = "auto_handoff_tests.rs"]
mod tests;
