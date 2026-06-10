use std::collections::HashMap;

use warp_core::features::FeatureFlag;
use warpui::{
    AppContext, Entity, EntityId, ModelContext, SingletonEntity, TypedActionView, ViewHandle,
    WindowId,
};

use super::{AutoCloudHandoffTrigger, Workspace, WorkspaceAction, WorkspaceRegistry};
use crate::ai::active_agent_views_model::{ActiveAgentViewsModel, ConversationOrTaskId};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent_management::AgentNotificationsModel;
use crate::settings::AISettings;
use crate::system::{SystemStats, SystemStatsEvent};
use crate::terminal::view::TerminalView;
use crate::BlocklistAIHistoryModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoCloudHandoffSkipReason {
    EmptyConversation,
    NotInProgress,
    MissingServerConversationToken,
    SharedSessionViewer,
    CloudHandoffUnavailable,
    AlreadyAttempted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoCloudHandoffAttemptState {
    InFlight,
    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    Succeeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AutoCloudHandoffEligibility {
    pub(crate) is_empty: bool,
    pub(crate) is_in_progress: bool,
    pub(crate) has_server_conversation_token: bool,
    pub(crate) is_viewing_shared_session: bool,
    pub(crate) can_handoff_to_cloud: bool,
    pub(crate) already_attempted: bool,
}

impl AutoCloudHandoffEligibility {
    pub(crate) fn from_conversation(
        conversation: &AIConversation,
        can_handoff_to_cloud: bool,
        already_attempted: bool,
    ) -> Self {
        Self {
            is_empty: conversation.is_empty(),
            is_in_progress: conversation.status().is_in_progress(),
            has_server_conversation_token: conversation.server_conversation_token().is_some(),
            is_viewing_shared_session: conversation.is_viewing_shared_session(),
            can_handoff_to_cloud,
            already_attempted,
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
    attempted_conversation_ids: HashMap<AIConversationId, AutoCloudHandoffAttemptState>,
    /// Set at sleep time when an eligible in-progress local agent run would have
    /// been handed off but `auto_handoff_on_sleep_enabled` is off. Consumed on
    /// wake to surface the discoverability prompt.
    pending_sleep_prompt: Option<EntityId>,
}

impl AutoCloudHandoffController {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&SystemStats::handle(ctx), |controller, event, ctx| {
            controller.handle_system_stats_event(event, ctx);
        });

        Self {
            attempted_conversation_ids: HashMap::new(),
            pending_sleep_prompt: None,
        }
    }

    #[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
    pub(crate) fn record_handoff_succeeded(&mut self, conversation_id: AIConversationId) {
        self.attempted_conversation_ids
            .insert(conversation_id, AutoCloudHandoffAttemptState::Succeeded);
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
                self.trigger(AutoCloudHandoffTrigger::MacOsSleep, ctx);
                self.maybe_record_sleep_prompt(ctx);
            }
            SystemStatsEvent::CpuWasAwakened => {
                self.maybe_show_sleep_prompt(ctx);
            }
        }
    }

    /// At sleep time, records a pending discoverability prompt when an eligible
    /// in-progress local agent run *would* have auto-handed-off but the sleep
    /// setting is off. The actual notification is surfaced on wake by
    /// [`Self::maybe_show_sleep_prompt`].
    fn maybe_record_sleep_prompt(&mut self, ctx: &mut ModelContext<Self>) {
        self.pending_sleep_prompt = None;

        if !FeatureFlag::AutoHandoffSleepPrompt.is_enabled() {
            log::info!("auto-handoff sleep prompt: skipping at sleep, feature flag disabled");
            return;
        }

        let (setting_on, can_handoff_to_cloud) = {
            let ai_settings = AISettings::as_ref(ctx);
            (
                ai_settings.is_auto_handoff_on_sleep_enabled(ctx),
                ai_settings.is_cloud_handoff_enabled(ctx),
            )
        };

        // Only prompt when the setting is off and cloud handoff is otherwise
        // available (so enabling the setting actually helps). The prompt is not
        // permanently dismissable: it resurfaces on every sleep interruption
        // until the user enables auto-handoff-on-sleep.
        if setting_on {
            log::info!(
                "auto-handoff sleep prompt: skipping at sleep, auto-handoff-on-sleep is already enabled"
            );
            return;
        }
        if !can_handoff_to_cloud {
            log::info!(
                "auto-handoff sleep prompt: skipping at sleep, cloud handoff is unavailable"
            );
            return;
        }

        let Some((terminal_view_id, conversation_id)) = Self::last_focused_local_conversation(ctx)
        else {
            log::info!(
                "auto-handoff sleep prompt: skipping at sleep, no focused local agent conversation"
            );
            return;
        };

        let Some((_window_id, _workspace, terminal_view)) =
            Self::find_workspace_and_terminal(terminal_view_id, ctx)
        else {
            log::info!(
                "auto-handoff sleep prompt: skipping at sleep, terminal view {terminal_view_id:?} not found"
            );
            return;
        };

        if terminal_view
            .as_ref(ctx)
            .ambient_agent_view_model()
            .is_some()
        {
            log::info!("auto-handoff sleep prompt: skipping at sleep, terminal is a cloud pane");
            return;
        }

        if terminal_view.as_ref(ctx).has_active_long_running_command() {
            log::info!(
                "auto-handoff sleep prompt: skipping at sleep, terminal has a long-running command"
            );
            return;
        }

        let skip_reason = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(conversation) = history.conversation(&conversation_id) else {
                log::info!(
                    "auto-handoff sleep prompt: skipping at sleep, conversation {conversation_id:?} is not loaded"
                );
                return;
            };
            AutoCloudHandoffEligibility::from_conversation(
                conversation,
                can_handoff_to_cloud,
                false,
            )
            .skip_reason()
        };

        if let Some(reason) = skip_reason {
            log::info!(
                "auto-handoff sleep prompt: skipping at sleep, conversation {conversation_id:?} ineligible: {reason:?}"
            );
            return;
        }

        log::info!(
            "auto-handoff sleep prompt: recorded pending prompt for conversation {conversation_id:?} in terminal {terminal_view_id:?}"
        );
        self.pending_sleep_prompt = Some(terminal_view_id);
    }

    /// On wake, surfaces the discoverability prompt recorded at sleep time, as
    /// long as the setting is still off and not dismissed.
    fn maybe_show_sleep_prompt(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(terminal_view_id) = self.pending_sleep_prompt.take() else {
            log::info!(
                "auto-handoff sleep prompt: nothing to show on wake, no pending prompt was recorded at sleep"
            );
            return;
        };

        let should_show = {
            let ai_settings = AISettings::as_ref(ctx);
            FeatureFlag::AutoHandoffSleepPrompt.is_enabled()
                && !ai_settings.is_auto_handoff_on_sleep_enabled(ctx)
        };
        if !should_show {
            log::info!(
                "auto-handoff sleep prompt: not showing on wake, auto-handoff-on-sleep was enabled in the meantime"
            );
            return;
        }

        log::info!(
            "auto-handoff sleep prompt: showing prompt on wake for terminal {terminal_view_id:?}"
        );
        AgentNotificationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.add_auto_handoff_sleep_prompt(terminal_view_id, ctx);
        });
    }

    fn trigger(&mut self, trigger: AutoCloudHandoffTrigger, ctx: &mut ModelContext<Self>) {
        if let Some(request) = self.prepare_handoff_request(trigger, ctx) {
            ctx.emit(request);
        }
    }

    fn prepare_handoff_request(
        &mut self,
        trigger: AutoCloudHandoffTrigger,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AutoCloudHandoffRequest> {
        if !Self::is_trigger_enabled(trigger, ctx) {
            return None;
        }

        let (terminal_view_id, conversation_id) = Self::last_focused_local_conversation(ctx)?;

        let (window_id, workspace, terminal_view) =
            Self::find_workspace_and_terminal(terminal_view_id, ctx)?;

        if terminal_view
            .as_ref(ctx)
            .ambient_agent_view_model()
            .is_some()
        {
            return None;
        }

        if terminal_view.as_ref(ctx).has_active_long_running_command() {
            return None;
        }

        let skip_reason = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let conversation = history.conversation(&conversation_id)?;
            let can_handoff_to_cloud = AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx);
            AutoCloudHandoffEligibility::from_conversation(
                conversation,
                can_handoff_to_cloud,
                self.attempted_conversation_ids
                    .contains_key(&conversation_id),
            )
            .skip_reason()
        };

        if skip_reason.is_some() {
            return None;
        }

        self.attempted_conversation_ids
            .insert(conversation_id, AutoCloudHandoffAttemptState::InFlight);

        log::info!(
            "Triggering auto handoff to cloud for conversation {conversation_id:?} in window {window_id:?} via {trigger:?}"
        );
        Some(AutoCloudHandoffRequest {
            workspace,
            terminal_view_id,
            conversation_id,
            trigger,
        })
    }

    fn last_focused_local_conversation(
        ctx: &ModelContext<Self>,
    ) -> Option<(EntityId, AIConversationId)> {
        let active_agent_views = ActiveAgentViewsModel::as_ref(ctx);
        let terminal_view_id = active_agent_views.get_last_focused_terminal_id()?;
        let conversation_id = match active_agent_views.get_last_focused_conversation()? {
            ConversationOrTaskId::ConversationId(conversation_id) => conversation_id,
            ConversationOrTaskId::TaskId(_) => return None,
        };
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
