use std::collections::HashMap;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WindowId};

use crate::settings::AISettings;

use crate::ai::active_agent_views_model::{ActiveAgentViewsEvent, ActiveAgentViewsModel};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent_management::notifications::{
    NotificationCategory, NotificationId, NotificationItem, NotificationItems, NotificationOrigin,
    NotificationSourceAgent,
};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, ConversationStatusUpdate};
use crate::server::telemetry::TelemetryEvent;
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::{CLIAgent, TerminalView};
use crate::workspace::util::is_terminal_view_in_same_tab;
use crate::workspace::{Workspace, WorkspaceRegistry};
use crate::BlocklistAIHistoryModel;
use warp_core::send_telemetry_from_ctx;

/// Singleton model responsible for triggering in-app notifications on blocking conversation
/// status updates and tracking/storing these notifications for the notifications mailbox.
/// Tracks and stores notifications for both warp agent conversations and other supported
/// cli agent sessions.
pub struct AgentNotificationsModel {
    notifications: NotificationItems,
    /// Artifacts accumulated during the current turn for each conversation.
    /// Drained into the notification when a terminal state fires, cleared on InProgress.
    pub(crate) pending_artifacts: HashMap<AIConversationId, Vec<Artifact>>,
}

impl Entity for AgentNotificationsModel {
    type Event = AgentManagementEvent;
}

impl SingletonEntity for AgentNotificationsModel {}

impl AgentNotificationsModel {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        let cli_sessions_model = CLIAgentSessionsModel::handle(ctx);
        ctx.subscribe_to_model(&cli_sessions_model, |me, event, ctx| {
            me.handle_cli_agent_session_event(event, ctx);
        });

        let active_views_model = ActiveAgentViewsModel::handle(ctx);
        ctx.subscribe_to_model(&active_views_model, |me, event, ctx| {
            me.handle_active_agent_views_changed(event, ctx);
        });

        Self {
            notifications: NotificationItems::default(),
            pending_artifacts: HashMap::new(),
        }
    }

    pub(crate) fn notifications(&self) -> &NotificationItems {
        &self.notifications
    }

    pub(crate) fn mark_item_read(&mut self, id: NotificationId, ctx: &mut ModelContext<Self>) {
        if self.notifications.mark_item_read(id) {
            ctx.emit(AgentManagementEvent::NotificationUpdated);
        }
    }

    pub(crate) fn mark_all_items_read(&mut self, ctx: &mut ModelContext<Self>) {
        if self.notifications.mark_all_items_read() {
            ctx.emit(AgentManagementEvent::AllNotificationsMarkedRead);
        }
    }

    /// Marks all notifications from the given terminal view as read.
    pub(crate) fn mark_items_from_terminal_view_read(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::HOANotifications.is_enabled() {
            return;
        }
        if self
            .notifications
            .mark_all_terminal_view_items_as_read(terminal_view_id)
        {
            ctx.emit(AgentManagementEvent::NotificationUpdated);
        }
    }

    fn handle_active_agent_views_changed(
        &mut self,
        event: &ActiveAgentViewsEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::HOANotifications.is_enabled() {
            return;
        }

        match event {
            ActiveAgentViewsEvent::ConversationClosed { conversation_id } => {
                // When a conversation is closed, clean up its notifications
                // (as there's no conversation to navigate to when you click said notifications).
                if self
                    .notifications
                    .remove_by_origin(NotificationOrigin::Conversation(*conversation_id))
                {
                    ctx.emit(AgentManagementEvent::NotificationUpdated);
                }
            }
            ActiveAgentViewsEvent::TerminalViewFocused
            | ActiveAgentViewsEvent::WindowClosed
            | ActiveAgentViewsEvent::AmbientSessionOpened { .. }
            | ActiveAgentViewsEvent::AmbientSessionClosed { .. } => {}
        }
    }

    fn handle_cli_agent_session_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::HOANotifications.is_enabled() {
            return;
        }

        match event {
            CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            } => {
                self.remove_notification_by_source(
                    NotificationOrigin::CLISession(*terminal_view_id),
                    ctx,
                );
            }
            CLIAgentSessionsModelEvent::Started { .. }
            | CLIAgentSessionsModelEvent::InputSessionChanged { .. }
            | CLIAgentSessionsModelEvent::SessionUpdated { .. } => {}
            CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status,
                session_context,
            } => match status {
                // When the agent resumes its work we can assume that the previous notification is stale.
                CLIAgentSessionStatus::InProgress => {
                    self.remove_notification_by_source(
                        NotificationOrigin::CLISession(*terminal_view_id),
                        ctx,
                    );
                }
                CLIAgentSessionStatus::Success => {
                    let title = session_context
                        .display_title()
                        .unwrap_or_else(|| format!("{} completed", agent.display_name()));
                    let message = match agent {
                        CLIAgent::Codex => "Notification from Codex",
                        _ => "Task completed.",
                    };
                    let metadata = TerminalViewMetadata::lookup(*terminal_view_id, ctx);
                    self.add_notification(
                        title,
                        message.to_owned(),
                        NotificationCategory::Complete,
                        NotificationSourceAgent::CLI {
                            agent: *agent,
                            is_ambient: metadata.is_ambient,
                        },
                        NotificationOrigin::CLISession(*terminal_view_id),
                        *terminal_view_id,
                        vec![],
                        metadata.branch,
                        ctx,
                    );
                }
                CLIAgentSessionStatus::Blocked { message } => {
                    let title = session_context
                        .display_title()
                        .unwrap_or_else(|| format!("{} needs attention", agent.display_name()));
                    let metadata = TerminalViewMetadata::lookup(*terminal_view_id, ctx);
                    self.add_notification(
                        title,
                        message
                            .clone()
                            .unwrap_or_else(|| "Waiting for input.".to_owned()),
                        NotificationCategory::Request,
                        NotificationSourceAgent::CLI {
                            agent: *agent,
                            is_ambient: metadata.is_ambient,
                        },
                        NotificationOrigin::CLISession(*terminal_view_id),
                        *terminal_view_id,
                        vec![],
                        metadata.branch,
                        ctx,
                    );
                }
            },
        }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // When a conversation is deleted or removed, clean up its notification and pending artifacts.
        if let BlocklistAIHistoryEvent::DeletedConversation {
            conversation_id, ..
        }
        | BlocklistAIHistoryEvent::RemoveConversation {
            conversation_id, ..
        } = event
        {
            if FeatureFlag::HOANotifications.is_enabled() {
                self.pending_artifacts.remove(conversation_id);
                self.remove_notification_by_source(
                    NotificationOrigin::Conversation(*conversation_id),
                    ctx,
                );
            }
            return;
        }

        // Accumulate artifacts as they arrive during the conversation.
        if let BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
            conversation_id,
            artifact,
            ..
        } = event
        {
            if FeatureFlag::HOANotifications.is_enabled() {
                self.pending_artifacts
                    .entry(*conversation_id)
                    .or_default()
                    .push(artifact.clone());
            }
            return;
        }

        let BlocklistAIHistoryEvent::UpdatedConversationStatus {
            terminal_view_id,
            conversation_id,
            // We shouldn't trigger toasts when restoring conversations on startup.
            update: ConversationStatusUpdate::Changed { .. },
            ..
        } = event
        else {
            return;
        };

        let ai_history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(updated_conversation) = ai_history_model.conversation(conversation_id) else {
            return;
        };

        if updated_conversation.should_exclude_from_navigation() {
            return;
        }

        let status = updated_conversation.status().clone();
        let latest_query = updated_conversation.latest_user_query();
        if FeatureFlag::HOANotifications.is_enabled() {
            self.handle_history_event_for_mailbox(
                &status,
                *conversation_id,
                latest_query,
                *terminal_view_id,
                ctx,
            );
            // The new mailbox path handled the event — skip the legacy toast path below.
            return;
        }

        if !status.should_trigger_notification() {
            return;
        }

        if is_terminal_view_visible(*terminal_view_id, ctx) {
            return;
        }

        let Some((window_id, tab_index)) =
            window_and_tab_idx_id_for_conversation(*conversation_id, ctx)
        else {
            return;
        };

        ctx.emit(AgentManagementEvent::ConversationNeedsAttention {
            window_id,
            tab_index,
            terminal_view_id: *terminal_view_id,
            conversation_id: *conversation_id,
        });
    }

    fn handle_history_event_for_mailbox(
        &mut self,
        status: &ConversationStatus,
        conversation_id: AIConversationId,
        latest_query: Option<String>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let origin = NotificationOrigin::Conversation(conversation_id);

        // If the conversation view is no longer open, don't create notifications for it
        // (there's nothing to navigate to when clicking them).
        if !ActiveAgentViewsModel::as_ref(ctx).is_conversation_open(conversation_id, ctx) {
            self.pending_artifacts.remove(&conversation_id);
            self.remove_notification_by_source(origin, ctx);
            return;
        }

        let title = latest_query.unwrap_or_else(|| "Agent task".to_owned());
        let metadata = TerminalViewMetadata::lookup(terminal_view_id, ctx);
        let oz_agent = NotificationSourceAgent::Oz {
            is_ambient: metadata.is_ambient,
        };

        match status {
            // When the agent resumes its work, clear stale notifications.
            ConversationStatus::InProgress => {
                self.remove_notification_by_source(origin, ctx);
            }
            ConversationStatus::Success => {
                let artifacts = self.flush_pending_artifacts(conversation_id);
                self.add_notification(
                    title,
                    "Task completed.".to_owned(),
                    NotificationCategory::Complete,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    artifacts,
                    metadata.branch,
                    ctx,
                );
            }
            ConversationStatus::Cancelled => {
                let artifacts = self.flush_pending_artifacts(conversation_id);
                self.add_notification(
                    title,
                    "Task was cancelled.".to_owned(),
                    NotificationCategory::Complete,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    artifacts,
                    metadata.branch,
                    ctx,
                );
            }
            ConversationStatus::Blocked { blocked_action } => {
                self.add_notification(
                    title,
                    blocked_action.clone(),
                    NotificationCategory::Request,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    vec![],
                    metadata.branch,
                    ctx,
                );
            }
            ConversationStatus::Error => {
                let artifacts = self.flush_pending_artifacts(conversation_id);
                self.add_notification(
                    title,
                    "Something went wrong.".to_owned(),
                    NotificationCategory::Error,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    artifacts,
                    metadata.branch,
                    ctx,
                );
            }
        }
    }

    /// Removes the existing notification for the given source (if any) and emits an update event.
    fn remove_notification_by_source(
        &mut self,
        origin: NotificationOrigin,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.notifications.remove_by_origin(origin) {
            ctx.emit(AgentManagementEvent::NotificationUpdated);
        }
    }

    /// Drains and returns the pending artifacts for a conversation.
    pub(crate) fn flush_pending_artifacts(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Vec<Artifact> {
        self.pending_artifacts
            .remove(&conversation_id)
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn add_notification(
        &mut self,
        title: String,
        message: String,
        category: NotificationCategory,
        agent: NotificationSourceAgent,
        origin: NotificationOrigin,
        terminal_view_id: EntityId,
        artifacts: Vec<Artifact>,
        branch: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !*AISettings::as_ref(ctx).show_agent_notifications {
            return;
        }

        let is_visible = is_terminal_view_visible(terminal_view_id, ctx);
        let item = NotificationItem::new(
            title,
            message,
            category,
            agent,
            origin,
            is_visible,
            terminal_view_id,
            artifacts,
            branch,
        );
        send_telemetry_from_ctx!(
            TelemetryEvent::AgentNotificationShown {
                agent_variant: agent.into(),
            },
            ctx
        );

        let id = item.id;
        self.notifications.push(item);
        ctx.emit(AgentManagementEvent::NotificationAdded { id });
    }
}

#[derive(Clone, Debug)]
pub enum AgentManagementEvent {
    /// A Warp-native conversation needs attention and is not visible in the current window/tab.
    ConversationNeedsAttention {
        window_id: WindowId,
        tab_index: usize,
        terminal_view_id: EntityId,
        conversation_id: AIConversationId,
    },
    /// A new notification was added to the persistent notification center.
    NotificationAdded { id: NotificationId },
    /// A notification's read state changed.
    NotificationUpdated,
    /// All notifications were marked as read.
    AllNotificationsMarkedRead,
}

impl ConversationStatus {
    /// Returns true if the updating the conversation with this status should trigger some
    /// notification to the user.
    pub fn should_trigger_notification(&self) -> bool {
        matches!(
            self,
            ConversationStatus::Success
                | ConversationStatus::Blocked { .. }
                | ConversationStatus::Error
        )
    }
}

fn is_terminal_view_visible(terminal_view_id: EntityId, app: &AppContext) -> bool {
    let Some(active_id) = active_focused_terminal_id(app) else {
        return false;
    };
    active_id == terminal_view_id
        || is_terminal_view_in_same_tab(&active_id, &terminal_view_id, app)
}

fn window_and_tab_idx_id_for_conversation(
    conversation_id: AIConversationId,
    app: &AppContext,
) -> Option<(WindowId, usize)> {
    WorkspaceRegistry::as_ref(app)
        .all_workspaces(app)
        .iter()
        .find_map(|(window_id, workspace_handle)| {
            workspace_handle
                .as_ref(app)
                .tab_views()
                .enumerate()
                .find_map(|(tab_idx, pane_group)| {
                    pane_group
                        .as_ref(app)
                        .terminal_pane_ids()
                        .filter_map(|pane_id| {
                            pane_group
                                .as_ref(app)
                                .terminal_view_from_pane_id(pane_id, app)
                        })
                        .find_map(|terminal_view| {
                            let terminal_view_conversation_id =
                                terminal_view.as_ref(app).active_conversation_id(app)?;
                            (terminal_view_conversation_id == conversation_id)
                                .then_some((*window_id, tab_idx))
                        })
                })
        })
}

/// Per-notification metadata derived from a single [`TerminalView`] lookup. Both fields
/// are read on the same emit path, so we resolve the view once and pass the projection
/// down rather than walking the workspace tree for each.
struct TerminalViewMetadata {
    is_ambient: bool,
    branch: Option<String>,
}

impl TerminalViewMetadata {
    fn lookup(terminal_view_id: EntityId, app: &AppContext) -> Self {
        let Some(terminal_view) = find_terminal_view_by_id(terminal_view_id, app) else {
            return Self {
                is_ambient: false,
                branch: None,
            };
        };
        let view = terminal_view.as_ref(app);
        Self {
            is_ambient: view.is_ambient_agent_session(app),
            branch: view.current_git_branch(app),
        }
    }
}

fn find_terminal_view_by_id(
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Option<ViewHandle<TerminalView>> {
    for (_, workspace_handle) in WorkspaceRegistry::as_ref(app).all_workspaces(app) {
        for pane_group in workspace_handle.as_ref(app).tab_views() {
            let pane_group = pane_group.as_ref(app);
            for pane_id in pane_group.terminal_pane_ids() {
                if let Some(terminal_view) = pane_group.terminal_view_from_pane_id(pane_id, app) {
                    if terminal_view.id() == terminal_view_id {
                        return Some(terminal_view);
                    }
                }
            }
        }
    }
    None
}

fn active_focused_terminal_id(app: &AppContext) -> Option<EntityId> {
    let active_window = app.windows().active_window()?;
    let workspace = app
        .views_of_type::<Workspace>(active_window)
        .and_then(|views| views.first().cloned())?;

    let workspace = workspace.as_ref(app);
    workspace.active_terminal_id(app)
}

#[cfg(test)]
#[path = "agent_management_model_tests.rs"]
mod tests;
