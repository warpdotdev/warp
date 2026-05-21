//! Action buttons row for conversation details panel.

use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::elements::{ChildView, CrossAxisAlignment, Empty, Flex, ParentElement};
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

use crate::view_components::copyable_text_field::COPY_FEEDBACK_DURATION;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::{AgentConversationEntryId, AgentRunDisplayStatus};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, SecondaryTheme};
use crate::workspace::WorkspaceAction;

const BUTTON_SPACING: f32 = 4.;

/// Per-button config for the action buttons row.
/// Each field controls one button independently.
#[derive(Debug, Clone, Default)]
pub struct ActionButtonsConfig {
    pub open_action: Option<WorkspaceAction>,
    pub cancel_task_id: Option<AmbientAgentTaskId>,
    pub fork_conversation_id: Option<AIConversationId>,
    /// Shows an info button for viewing more details.
    /// Only used in management view hover toolbelt.
    pub view_details_item_id: Option<AgentConversationEntryId>,
    /// Conversation link URL (either to the transcript or live session) for copy link button.
    pub copy_link_url: Option<String>,
}

impl ActionButtonsConfig {
    /// Returns true if no buttons will be rendered.
    pub fn is_empty(&self) -> bool {
        self.open_action.is_none()
            && self.cancel_task_id.is_none()
            && self.fork_conversation_id.is_none()
            && self.view_details_item_id.is_none()
            && self.copy_link_url.is_none()
    }

    /// Create config for a task.
    /// - `display_status`: used to determine if cancel button should show.
    /// - `open_action`: pass `Some(action)` to show open button, `None` to hide
    /// - `copy_link_url`: conversation link URL, or `None` to hide
    pub fn for_task(
        task_id: AmbientAgentTaskId,
        display_status: &AgentRunDisplayStatus,
        open_action: Option<WorkspaceAction>,
        copy_link_url: Option<String>,
    ) -> Self {
        Self {
            open_action,
            cancel_task_id: if display_status.is_cancellable() {
                Some(task_id)
            } else {
                None
            },
            fork_conversation_id: None,
            view_details_item_id: None,
            copy_link_url,
        }
    }

    /// Create config for a conversation.
    /// - `open_action`: pass `Some(action)` to show open button, `None` to hide
    /// - `copy_link_url`: conversation link URL, or `None` to hide
    pub fn for_conversation(
        conversation_id: AIConversationId,
        open_action: Option<WorkspaceAction>,
        copy_link_url: Option<String>,
    ) -> Self {
        Self {
            open_action,
            cancel_task_id: None,
            fork_conversation_id: Some(conversation_id),
            view_details_item_id: None,
            copy_link_url,
        }
    }
}

/// Events emitted by the action buttons.
#[derive(Debug, Clone)]
pub enum AgentDetailsButtonEvent {
    Open,
    CancelTask { task_id: AmbientAgentTaskId },
    ForkConversation { conversation_id: AIConversationId },
    ViewDetails { item_id: AgentConversationEntryId },
    CopyLink { link: String },
}

/// Actions dispatched by button clicks (internal).
#[derive(Debug, Clone)]
pub enum AgentDetailsAction {
    Open,
    CancelTask,
    ForkConversation,
    ViewDetails,
    CopyLink,
}

/// Reusable action buttons row for details panel.
pub struct ConversationActionButtonsRow {
    config: ActionButtonsConfig,
    open_button: ViewHandle<ActionButton>,
    cancel_task_button: ViewHandle<ActionButton>,
    fork_conversation_button: ViewHandle<ActionButton>,
    view_details_button: ViewHandle<ActionButton>,
    copy_link_button: ViewHandle<ActionButton>,
}

impl ConversationActionButtonsRow {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let open_button = ctx.add_typed_action_view(|_| {
            Self::make_action_button(
                Icon::LinkExternal,
                "Open conversation",
                None,
                AgentDetailsAction::Open,
            )
        });

        let cancel_task_button = ctx.add_typed_action_view(|_| {
            Self::make_action_button(
                Icon::StopFilled,
                "Cancel task",
                Some(AnsiColorIdentifier::Red),
                AgentDetailsAction::CancelTask,
            )
        });

        let fork_conversation_button = ctx.add_typed_action_view(|_| {
            Self::make_action_button(
                Icon::ArrowSplit,
                "Fork conversation",
                None,
                AgentDetailsAction::ForkConversation,
            )
        });

        let view_details_button = ctx.add_typed_action_view(|_| {
            Self::make_action_button(
                Icon::Info,
                "View details",
                None,
                AgentDetailsAction::ViewDetails,
            )
        });

        let copy_link_button = ctx.add_typed_action_view(|_| {
            Self::make_action_button(
                Icon::Link,
                "Copy link to run",
                None,
                AgentDetailsAction::CopyLink,
            )
        });

        Self {
            config: ActionButtonsConfig::default(),
            open_button,
            cancel_task_button,
            fork_conversation_button,
            view_details_button,
            copy_link_button,
        }
    }

    /// Set the config and rerender.
    pub fn set_config(&mut self, config: ActionButtonsConfig, ctx: &mut ViewContext<Self>) {
        self.config = config;
        ctx.notify();
    }

    /// Returns true if no buttons will be rendered.
    pub fn is_empty(&self) -> bool {
        self.config.is_empty()
    }

    fn make_action_button(
        icon: Icon,
        tooltip: &str,
        icon_color: Option<AnsiColorIdentifier>,
        action: AgentDetailsAction,
    ) -> ActionButton {
        let mut button = ActionButton::new("", SecondaryTheme)
            .with_icon(icon)
            .with_size(ButtonSize::Small)
            .with_tooltip(tooltip)
            .on_click(move |ctx| {
                ctx.dispatch_typed_action(action.clone());
            });
        if let Some(color) = icon_color {
            button = button.with_icon_ansi_color(color);
        }
        button
    }
}

impl Entity for ConversationActionButtonsRow {
    type Event = AgentDetailsButtonEvent;
}

impl View for ConversationActionButtonsRow {
    fn ui_name() -> &'static str {
        "ConversationActionButtonsRow"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        if self.config.is_empty() {
            return Empty::new().finish();
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(BUTTON_SPACING);

        if self.config.copy_link_url.is_some() {
            row.add_child(ChildView::new(&self.copy_link_button).finish());
        }

        if self.config.open_action.is_some() {
            row.add_child(ChildView::new(&self.open_button).finish());
        }
        if self.config.cancel_task_id.is_some() {
            row.add_child(ChildView::new(&self.cancel_task_button).finish());
        }
        if self.config.fork_conversation_id.is_some() && !cfg!(target_family = "wasm") {
            row.add_child(ChildView::new(&self.fork_conversation_button).finish());
        }
        if self.config.view_details_item_id.is_some() {
            row.add_child(ChildView::new(&self.view_details_button).finish());
        }

        row.finish()
    }
}

impl TypedActionView for ConversationActionButtonsRow {
    type Action = AgentDetailsAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentDetailsAction::Open => {
                if self.config.open_action.is_some() {
                    ctx.emit(AgentDetailsButtonEvent::Open);
                }
            }
            AgentDetailsAction::CancelTask => {
                if let Some(task_id) = self.config.cancel_task_id {
                    ctx.emit(AgentDetailsButtonEvent::CancelTask { task_id });
                }
            }
            AgentDetailsAction::ForkConversation => {
                if let Some(conversation_id) = self.config.fork_conversation_id {
                    ctx.emit(AgentDetailsButtonEvent::ForkConversation { conversation_id });
                }
            }
            AgentDetailsAction::ViewDetails => {
                if let Some(item_id) = &self.config.view_details_item_id {
                    ctx.emit(AgentDetailsButtonEvent::ViewDetails { item_id: *item_id });
                }
            }
            AgentDetailsAction::CopyLink => {
                if let Some(link) = &self.config.copy_link_url {
                    ctx.emit(AgentDetailsButtonEvent::CopyLink { link: link.clone() });
                    self.copy_link_button.update(ctx, |button, ctx| {
                        button.set_icon(Some(Icon::Check), ctx);
                    });
                    let duration = COPY_FEEDBACK_DURATION;
                    ctx.spawn(
                        async move {
                            warpui::r#async::Timer::after(duration).await;
                        },
                        |me, _, ctx| {
                            me.copy_link_button.update(ctx, |button, ctx| {
                                button.set_icon(Some(Icon::Link), ctx);
                            });
                        },
                    );
                }
            }
        }
    }
}
