use std::env;
use std::io::{self, Write};

use warp::tui_export::{
    AIAgentActionResultType, AIAgentActionType, AIConversationId, ActiveSession,
    BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel, ConversationSelectionHandle, ConversationStatus,
    ConversationStatusUpdate,
};
use warp_core::cli_agent_protocol::{CLI_AGENT_NOTIFICATION_SENTINEL, CLIAgentNotification};
use warp_terminal::model::escape_sequences::{C0, C1, tmux_passthrough};
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _};

const WARP_TUI_AGENT_NAME: &str = "warp-tui";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StatusOscEvent {
    event: &'static str,
    error_type: Option<&'static str>,
}

/// Publishes this TUI session's agent lifecycle through the CLI-agent OSC protocol.
pub(crate) struct CliAgentOscEventPublisher {
    terminal_surface_id: EntityId,
    active_session: ModelHandle<ActiveSession>,
    conversation_selection: ConversationSelectionHandle,
}

impl Entity for CliAgentOscEventPublisher {
    type Event = ();
}

impl CliAgentOscEventPublisher {
    pub(crate) fn new(
        terminal_surface_id: EntityId,
        active_session: ModelHandle<ActiveSession>,
        conversation_selection: ConversationSelectionHandle,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(action_model, |publisher, action_model, event, ctx| {
            publisher.handle_action_event(&action_model, event, ctx);
        });
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |publisher, _, event, ctx| publisher.handle_history_event(event, ctx),
        );
        Self {
            terminal_surface_id,
            active_session,
            conversation_selection,
        }
    }

    pub(crate) fn publish_session_start(&self, ctx: &AppContext) {
        self.publish(self.notification("session_start", ctx));
    }

    pub(crate) fn publish_prompt_submit(&self, prompt: String, ctx: &AppContext) {
        let mut notification = self.notification("prompt_submit", ctx);
        notification.query = Some(prompt);
        self.publish(notification);
    }

    fn handle_action_event(
        &self,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        event: &BlocklistAIActionEvent,
        ctx: &AppContext,
    ) {
        match event {
            BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(action_id) => {
                let Some(selected_conversation_id) = self
                    .conversation_selection
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                else {
                    return;
                };
                let action_model = action_model.as_ref(ctx);
                let Some(action) = action_model
                    .get_pending_actions_for_conversation(&selected_conversation_id)
                    .find(|action| &action.id == action_id)
                else {
                    return;
                };
                if matches!(&action.action, AIAgentActionType::AskUserQuestion { .. }) {
                    let mut notification = self.notification("question_asked", ctx);
                    notification.summary = Some("Waiting for your answer".to_owned());
                    self.publish(notification);
                } else {
                    let mut notification = self.notification("permission_request", ctx);
                    notification.tool_name = Some(action.action.user_friendly_name());
                    self.publish(notification);
                }
            }
            BlocklistAIActionEvent::FinishedAction {
                action_id,
                conversation_id,
                ..
            } => {
                let selected_conversation_id = self
                    .conversation_selection
                    .as_ref(ctx)
                    .selected_conversation_id(ctx);
                let finished_asking_question = action_model
                    .as_ref(ctx)
                    .get_action_result(action_id)
                    .is_some_and(|result| {
                        matches!(&result.result, AIAgentActionResultType::AskUserQuestion(_))
                    });
                if finished_asking_question && selected_conversation_id == Some(*conversation_id) {
                    self.publish(self.notification("tool_complete", ctx));
                }
            }
            BlocklistAIActionEvent::QueuedAction(_)
            | BlocklistAIActionEvent::ExecutingAction(_)
            | BlocklistAIActionEvent::InitProject(_)
            | BlocklistAIActionEvent::ToggleCodeReview(_)
            | BlocklistAIActionEvent::InsertCodeReviewComments { .. } => {}
        }
    }

    fn handle_history_event(&self, event: &BlocklistAIHistoryEvent, ctx: &AppContext) {
        if event
            .terminal_surface_id()
            .is_some_and(|id| id != self.terminal_surface_id)
        {
            return;
        }
        let BlocklistAIHistoryEvent::UpdatedConversationStatus {
            conversation_id,
            update,
            new_status,
            ..
        } = event
        else {
            return;
        };
        let selected_conversation_id = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx);
        let Some(status_event) = status_osc_event(
            selected_conversation_id,
            *conversation_id,
            update,
            new_status,
        ) else {
            return;
        };
        let mut notification = self.notification(status_event.event, ctx);
        notification.error_type = status_event.error_type.map(str::to_owned);
        self.publish(notification);
    }

    fn notification(&self, event: &str, ctx: &AppContext) -> CLIAgentNotification {
        CLIAgentNotification {
            session_id: Some(self.terminal_surface_id.to_string()),
            cwd: self
                .active_session
                .as_ref(ctx)
                .current_working_directory()
                .cloned()
                .or_else(|| {
                    env::current_dir()
                        .ok()
                        .map(|cwd| cwd.to_string_lossy().into_owned())
                }),
            ..CLIAgentNotification::new(WARP_TUI_AGENT_NAME, event)
        }
    }

    fn publish(&self, notification: CLIAgentNotification) {
        let json =
            serde_json::to_string(&notification).expect("CLI-agent notification is serializable");
        let osc = C1::to_utf8(C1::OSC);
        let bell = char::from(C0::BEL);
        let sequence = format!("{osc}777;notify;{CLI_AGENT_NOTIFICATION_SENTINEL};{json}{bell}");
        let sequence = if env::var_os("TMUX").is_some() {
            tmux_passthrough(&sequence)
        } else {
            sequence
        };

        // Lifecycle notifications are best-effort and must not disrupt the TUI
        // if its stdout is unavailable.
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(sequence.as_bytes());
        let _ = stdout.flush();
    }
}

fn status_osc_event(
    selected_conversation_id: Option<AIConversationId>,
    conversation_id: AIConversationId,
    update: &ConversationStatusUpdate,
    new_status: &ConversationStatus,
) -> Option<StatusOscEvent> {
    if selected_conversation_id != Some(conversation_id)
        || matches!(update, ConversationStatusUpdate::Restored)
    {
        return None;
    }
    match new_status {
        ConversationStatus::Success => Some(StatusOscEvent {
            event: "stop",
            error_type: None,
        }),
        ConversationStatus::Error => Some(StatusOscEvent {
            event: "stop_failure",
            error_type: Some("error"),
        }),
        ConversationStatus::Cancelled => Some(StatusOscEvent {
            event: "stop_failure",
            error_type: Some("cancelled"),
        }),
        ConversationStatus::InProgress
            if matches!(
                update,
                ConversationStatusUpdate::Changed {
                    prev_status: ConversationStatus::Blocked { .. }
                }
            ) =>
        {
            Some(StatusOscEvent {
                event: "permission_replied",
                error_type: None,
            })
        }
        ConversationStatus::InProgress
        | ConversationStatus::Blocked { .. }
        | ConversationStatus::TransientError
        | ConversationStatus::WaitingForEvents => None,
    }
}

#[cfg(test)]
#[path = "cli_agent_osc_event_publisher_tests.rs"]
mod tests;
