use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::appearance::Appearance;

use crate::terminal::shared_session::replay_agent_conversations::reconstruct_response_events_from_conversations;
use crate::terminal::shared_session::role_change_modal::TEXT_FONT_SIZE;
use crate::terminal::shared_session::{
    ai_agent::encode_agent_response_event, max_session_size, SharedSessionActionSource,
    SharedSessionScrollbackType,
};
use crate::terminal::TerminalModel;
use byte_unit::Byte;
use warp_core::features::FeatureFlag;

use std::default::Default;
use std::sync::Arc;

use parking_lot::FairMutex;

use warpui::elements::{
    Container, Flex, MainAxisSize, MouseStateHandle, ParentElement, Shrinkable, Text,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::radio_buttons::{
    RadioButtonItem, RadioButtonLayout, RadioButtonStateHandle,
};

use super::style::{self, BUTTON_GAP, MODAL_MARGIN};
use warpui::{
    platform::Cursor, AppContext, Element, Entity, SingletonEntity, TypedActionView, View,
    ViewContext,
};

#[derive(Default)]
struct ButtonMouseStateHandles {
    cancel_button: MouseStateHandle,
    start_sharing_button: MouseStateHandle,
}

#[derive(Default)]
struct RadioButtonGroupState {
    group_state_handle: RadioButtonStateHandle,
    items: Vec<ScrollbackOption>,
}

struct ScrollbackOption {
    label: &'static str,
    scrollback_type: SharedSessionScrollbackType,
    mouse_state_handle: MouseStateHandle,
    is_disabled: bool,
}

pub struct Body {
    button_mouse_states: ButtonMouseStateHandles,
    radio_button_mouse_states: RadioButtonGroupState,
    has_agent_conversations: bool,
}

#[derive(Debug)]
pub enum BodyAction {
    StartSharing,
    Cancel,
}

pub enum BodyEvent {
    Close,
    StartSharing {
        scrollback_type: SharedSessionScrollbackType,
    },
}

impl Body {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            button_mouse_states: Default::default(),
            radio_button_mouse_states: Default::default(),
            has_agent_conversations: false,
        }
    }

    /// Calculate the total size of agent conversation response events that will be sent
    /// during session initialization. This is important because these events count toward
    /// the session size quota, but are separate from the scrollback blocks.
    fn calculate_agent_conversations_size(
        terminal_view_id: warpui::EntityId,
        ctx: &ViewContext<Self>,
    ) -> Byte {
        let conversations: Vec<_> = BlocklistAIHistoryModel::as_ref(ctx)
            .all_live_conversations_for_terminal_view(terminal_view_id)
            .filter(|conv| conv.exchange_count() > 0)
            .cloned()
            .collect();

        let total_bytes: usize = reconstruct_response_events_from_conversations(&conversations)
            .iter()
            .map(|event| encode_agent_response_event(event).len())
            .sum();

        Byte::from_u64(total_bytes as u64)
    }
}

impl Body {
    pub fn open(
        &mut self,
        open_source: SharedSessionActionSource,
        model: Arc<FairMutex<TerminalModel>>,
        terminal_view_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = model.lock();
        let max_session_size = max_session_size(ctx);

        // TODO: serializing the blocks to compute their sizes is
        // inefficient but it matches how the server checks limits.
        // Consider caching size in the blocklist as blocks are added
        // so we can compute this more efficiently if latency becomes an issue.
        // This is not an issue in release mode.

        // TODO: technically, the size of the long-running block might change while the modal
        // is open. We might want to watch for changes on the terminal model and recompute
        // the size here accordingly. That being said, we still have guardrails on both
        // client and server to ensure that the actual share won't be started if the size is
        // too large.

        // Check if agent shared sessions is enabled and there are active conversations
        self.has_agent_conversations = if FeatureFlag::AgentSharedSessions.is_enabled() {
            BlocklistAIHistoryModel::as_ref(ctx)
                .all_live_conversations_for_terminal_view(terminal_view_id)
                .any(|conv| conv.exchange_count() > 0)
        } else {
            false
        };

        // Calculate the size of agent conversation response events that will be sent during initialization.
        // Only include this if the feature flag is enabled, since the events won't be sent otherwise.
        let agent_conversations_size =
            if FeatureFlag::AgentSharedSessions.is_enabled() && self.has_agent_conversations {
                Self::calculate_agent_conversations_size(terminal_view_id, ctx)
            } else {
                Byte::from_u64(0)
            };

        let scrollback_from_active_block = SharedSessionScrollbackType::None.to_scrollback(&model);
        let mut is_scrollback_from_active_block_disabled = scrollback_from_active_block
            .num_bytes()
            .as_u64()
            .saturating_add(agent_conversations_size.as_u64())
            > max_session_size.as_u64();

        // Disable the "without scrollback" option if there are agent conversations
        if self.has_agent_conversations {
            is_scrollback_from_active_block_disabled = true;
        }

        let all_scrollback = SharedSessionScrollbackType::All.to_scrollback(&model);
        let is_all_scrollback_disabled = all_scrollback
            .num_bytes()
            .as_u64()
            .saturating_add(agent_conversations_size.as_u64())
            > max_session_size.as_u64();

        let scrollback_from_active_block_message = if model.is_alt_screen_active() {
            "Share from current screen"
        } else if model
            .block_list()
            .active_block()
            .is_active_and_long_running()
        {
            "Share from current block"
        } else {
            "Share without scrollback"
        };

        let mut options = vec![
            ScrollbackOption {
                label: scrollback_from_active_block_message,
                scrollback_type: SharedSessionScrollbackType::None,
                mouse_state_handle: Default::default(),
                is_disabled: is_scrollback_from_active_block_disabled,
            },
            ScrollbackOption {
                label: "Share from start of session",
                scrollback_type: SharedSessionScrollbackType::All,
                mouse_state_handle: Default::default(),
                is_disabled: is_all_scrollback_disabled,
            },
        ];

        if let SharedSessionActionSource::BlocklistContextMenu {
            block_index: Some(block_index),
        } = open_source
        {
            // Context menu from blocklist can be opened with or without block selection
            // Add option only if a block is selected
            let scrollback_type = SharedSessionScrollbackType::FromBlock { block_index };
            let mut is_disabled = if !is_all_scrollback_disabled {
                false
            } else {
                let block_scrollback = scrollback_type.to_scrollback(&model);
                block_scrollback
                    .num_bytes()
                    .as_u64()
                    .saturating_add(agent_conversations_size.as_u64())
                    > max_session_size.as_u64()
            };

            // Disable this option if there are agent conversations in the current session.
            if self.has_agent_conversations {
                is_disabled = true;
            }

            options.insert(
                0,
                ScrollbackOption {
                    label: "Share from selected block and onwards",
                    scrollback_type,
                    mouse_state_handle: Default::default(),
                    is_disabled,
                },
            );
        }

        self.radio_button_mouse_states.items = options;
        ctx.notify();
    }
}

impl Entity for Body {
    type Event = BodyEvent;
}

impl View for Body {
    fn ui_name() -> &'static str {
        "ShareSessionModalBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut start_sharing_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.button_mouse_states.start_sharing_button.clone(),
            )
            .with_centered_text_label(String::from("Start sharing"))
            .with_style(style::button_styles());

        // If none of the scrollback options are available, the start sharing
        // button should be disabled.
        if self
            .radio_button_mouse_states
            .items
            .iter()
            .all(|item| item.is_disabled)
        {
            start_sharing_button = start_sharing_button.disabled();
        }

        let start_sharing_button = start_sharing_button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(BodyAction::StartSharing))
            .finish();

        let cancel_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Outlined,
                self.button_mouse_states.cancel_button.clone(),
            )
            .with_centered_text_label(String::from("Cancel"))
            .with_style(style::button_styles())
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(BodyAction::Cancel))
            .finish();

        // When agent conversations exist, default to "Share from start of session"
        let default_option = self
            .radio_button_mouse_states
            .items
            .iter()
            .position(|i| !i.is_disabled);

        let radio_buttons = appearance
            .ui_builder()
            .radio_buttons(
                self.radio_button_mouse_states
                    .items
                    .iter()
                    .map(|i| i.mouse_state_handle.clone())
                    .collect(),
                self.radio_button_mouse_states
                    .items
                    .iter()
                    .map(|i| RadioButtonItem::text(i.label).with_disabled(i.is_disabled))
                    .collect(),
                self.radio_button_mouse_states.group_state_handle.clone(),
                default_option,
                TEXT_FONT_SIZE,
                RadioButtonLayout::Column,
            )
            .with_style(style::radio_button_styles());

        let button_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Container::new(cancel_button)
                        .with_margin_right(BUTTON_GAP)
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.0,
                    Container::new(start_sharing_button)
                        .with_margin_left(BUTTON_GAP)
                        .finish(),
                )
                .finish(),
            );

        let mut column = Flex::column();
        // Determine which explanation message to show
        let disabled_count = self
            .radio_button_mouse_states
            .items
            .iter()
            .filter(|i| i.is_disabled)
            .count();

        let explanation_message = if disabled_count == 0 {
            None
        } else if disabled_count > 1 {
            // Multiple options disabled - mention both reasons if agent conversations exist
            if self.has_agent_conversations {
                Some("Some options are disabled due to sharing size limits and the presence of agent conversations in the session")
            } else {
                Some("Some options are disabled due to sharing size limits")
            }
        } else {
            // Only one option disabled - use specific message if it's due to agent conversations
            if self.has_agent_conversations {
                Some("Sharing without scrollback is disabled because this session has agent conversations")
            } else {
                Some("Some options are disabled due to sharing size limits")
            }
        };

        if let Some(message) = explanation_message {
            column.add_child(
                Container::new(
                    Text::new(
                        message,
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().background())
                            .into(),
                    )
                    .finish(),
                )
                .with_margin_top(-MODAL_MARGIN)
                .with_margin_bottom(MODAL_MARGIN)
                .finish(),
            );
        }
        column
            .with_child(radio_buttons.build().finish())
            .with_child(
                Container::new(button_row.finish())
                    .with_margin_top(MODAL_MARGIN)
                    .finish(),
            )
            .finish()
    }
}

impl TypedActionView for Body {
    type Action = BodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BodyAction::Cancel => ctx.emit(BodyEvent::Close),
            BodyAction::StartSharing => {
                if let Some(selected_option) = self
                    .radio_button_mouse_states
                    .group_state_handle
                    .get_selected_idx()
                    .and_then(|idx| self.radio_button_mouse_states.items.get(idx))
                {
                    ctx.emit(BodyEvent::StartSharing {
                        scrollback_type: selected_option.scrollback_type,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
