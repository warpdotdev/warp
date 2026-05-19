//! Rendering functions for orchestration-related output items (messaging & agent management).

use pathfinder_color::ColorU;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, Hoverable,
    MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, SingletonEntity};

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warpui::elements::FormattedTextElement;

use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, ConversationStatus, StatusColorStyle,
};
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResultType, MessageId, ReceivedMessageDisplay,
    SendMessageToAgentResult, StartAgentExecutionMode, StartAgentResult,
};
use crate::ai::blocklist::action_model::AIActionStatus;
use crate::ai::blocklist::agent_view::orchestration_avatar::OrchestrationAvatar;
use crate::ai::blocklist::agent_view::orchestration_conversation_links::{
    conversation_id_for_agent_id, conversation_navigation_card_with_icon,
    dispatch_focus_or_open_child_agent_pane,
};
use crate::ai::blocklist::block::model::AIBlockModelHelper;
use crate::ai::blocklist::block::{
    received_message_collapsible_id, AIBlockAction, CollapsibleExpansionState,
};
use crate::ai::blocklist::inline_action::inline_action_header::{
    ICON_MARGIN, INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::{self, icon_size};
use crate::ai::blocklist::inline_action::requested_action::{
    render_requested_action_row, render_requested_action_row_for_text,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

use super::common::render_scrollable_collapsible_content;
use super::output::{action_icon, Props};
use super::WithContentItemSpacing;

const GENERATING_TITLE_PLACEHOLDER: &str = "Generating title...";
const ORCHESTRATION_COLLAPSED_MAX_HEIGHT: f32 = 200.;
#[derive(Clone, Debug, PartialEq, Eq)]
struct OrchestrationParticipant {
    display_name: String,
    avatar: OrchestrationAvatar,
    /// The participant's conversation, when resolved. `None` for the
    /// orchestrator and unknown agents (avatar stays non-clickable).
    conversation_id: Option<AIConversationId>,
}

impl OrchestrationParticipant {
    fn orchestrator() -> Self {
        Self {
            display_name: "Orchestrator".to_string(),
            avatar: OrchestrationAvatar::Orchestrator,
            conversation_id: None,
        }
    }

    fn unknown_child() -> Self {
        Self {
            display_name: "Unknown agent".to_string(),
            avatar: OrchestrationAvatar::agent("Unknown agent".to_string()),
            conversation_id: None,
        }
    }

    fn is_orchestrator(&self) -> bool {
        matches!(&self.avatar, OrchestrationAvatar::Orchestrator)
    }
}

#[cfg(test)]
fn agent_display_name_from_id(
    agent_id: &str,
    orchestrator_agent_id: Option<&str>,
    app: &AppContext,
) -> String {
    participant_for_agent_id(agent_id, orchestrator_agent_id, app).display_name
}

fn participant_for_agent_id(
    agent_id: &str,
    orchestrator_agent_id: Option<&str>,
    app: &AppContext,
) -> OrchestrationParticipant {
    if let Some(conversation_id) = conversation_id_for_agent_id(agent_id, app) {
        if let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(app).conversation(&conversation_id)
        {
            return participant_for_conversation(
                conversation,
                orchestrator_agent_id,
                Some(agent_id),
            );
        }
    }
    if orchestrator_agent_id.is_some_and(|id| id == agent_id) {
        return OrchestrationParticipant::orchestrator();
    }
    OrchestrationParticipant::unknown_child()
}

fn participant_for_conversation(
    conversation: &AIConversation,
    orchestrator_agent_id: Option<&str>,
    agent_id: Option<&str>,
) -> OrchestrationParticipant {
    let is_orchestrator = agent_id
        .map(|id| {
            orchestrator_agent_id.is_some_and(|orchestrator_id| id == orchestrator_id)
                || (orchestrator_agent_id.is_none()
                    && conversation.parent_conversation_id().is_none())
        })
        .unwrap_or_else(|| conversation.parent_conversation_id().is_none());
    if is_orchestrator {
        return OrchestrationParticipant::orchestrator();
    }

    let display_name = conversation.agent_name().unwrap_or("Agent").to_string();
    OrchestrationParticipant {
        display_name: display_name.clone(),
        avatar: OrchestrationAvatar::agent(display_name),
        conversation_id: Some(conversation.id()),
    }
}

fn orchestrator_agent_id_for_conversation(
    conversation: &AIConversation,
    app: &AppContext,
) -> Option<String> {
    match conversation.parent_conversation_id() {
        Some(parent_id) => BlocklistAIHistoryModel::as_ref(app)
            .conversation(&parent_id)
            .and_then(|parent| parent.orchestration_agent_id()),
        None => conversation.orchestration_agent_id(),
    }
}

fn participant_for_current_conversation(
    props: Props,
    orchestrator_agent_id: Option<&str>,
    app: &AppContext,
) -> OrchestrationParticipant {
    props
        .model
        .conversation(app)
        .map(|conversation| {
            participant_for_conversation(
                conversation,
                orchestrator_agent_id,
                conversation.orchestration_agent_id().as_deref(),
            )
        })
        .unwrap_or_else(OrchestrationParticipant::orchestrator)
}

fn transcript_metadata(recipients: &[OrchestrationParticipant], subject: &str) -> Option<String> {
    let recipients = recipients
        .iter()
        .filter(|participant| !participant.is_orchestrator())
        .map(|participant| participant.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    match (recipients.is_empty(), subject.is_empty()) {
        (true, true) => None,
        (true, false) => Some(subject.to_string()),
        (false, true) => Some(format!("to {recipients}")),
        (false, false) => Some(format!("to {recipients} • {subject}")),
    }
}

struct TranscriptRowData<'a> {
    participant: &'a OrchestrationParticipant,
    recipients: &'a [OrchestrationParticipant],
    subject: &'a str,
    body: &'a str,
    message_id: &'a MessageId,
    is_streaming: bool,
}

fn render_transcript_row(
    data: TranscriptRowData<'_>,
    props: Props,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.monospace_font_size();
    let metadata_color = blended_colors::text_disabled(theme, theme.surface_2());
    let body_color: ColorU = theme.main_text_color(theme.background()).into();
    let collapsible_state = if data.body.is_empty() {
        None
    } else {
        props.collapsible_block_states.get(data.message_id)
    };

    let name = FormattedTextFragment::bold(&data.participant.display_name);
    let header_row_element: Box<dyn Element> = if let Some(state) = collapsible_state {
        // Wrap the name + chevron in one clickable element so either toggles
        // the section. Text is non-selectable + non-interactive so clicks
        // register and the pointing-hand cursor isn't reset by the text.
        let header = render_formatted_text_element(vec![name], app)
            .set_selectable(false)
            .disable_mouse_interaction()
            .finish();
        let text_color = theme.foreground();
        let icon_sz = icon_size(app);
        let is_expanded = matches!(
            state.expansion_state,
            CollapsibleExpansionState::Expanded { .. }
        );
        let chevron_icon = if is_expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let toggle_mouse_state = state.expansion_toggle_mouse_state.clone();
        let message_id_clone = data.message_id.clone();

        let expandable = Hoverable::new(toggle_mouse_state, move |_| {
            // Make the bold name a Shrinkable child so very long agent names
            // shrink within the available width instead of pushing the chevron
            // past the transcript column.
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., header).finish())
                .with_child(
                    Container::new(
                        ConstrainedBox::new(chevron_icon.to_warpui_icon(text_color).finish())
                            .with_width(icon_sz)
                            .with_height(icon_sz)
                            .finish(),
                    )
                    .with_margin_left(6.)
                    .finish(),
                )
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AIBlockAction::ToggleCollapsibleBlockExpanded(
                message_id_clone.clone(),
            ));
        });

        // Wrap the Hoverable in a Shrinkable inside an outer row so it
        // receives a bounded width constraint from the parent column. This
        // lets the inner Shrinkable around the bold name actually shrink
        // when the name is long, while the Hoverable's click bounds still
        // size to its content (bold name + chevron) when it fits.
        Flex::row()
            .with_child(Shrinkable::new(1., expandable.finish()).finish())
            .finish()
    } else {
        let header = render_formatted_text_element(vec![name], app).finish();
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., header).finish())
            .finish()
    };

    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    content.add_child(header_row_element);
    if let Some(metadata) = transcript_metadata(data.recipients, data.subject) {
        content.add_child(
            Container::new(
                Text::new(metadata, font_family, font_size)
                    .with_color(metadata_color)
                    .with_selectable(true)
                    .finish(),
            )
            .with_margin_top(2.)
            .finish(),
        );
    }
    if !data.body.is_empty() {
        let body_element = Container::new(
            Text::new(data.body.to_string(), font_family, font_size)
                .with_color(body_color)
                .with_selectable(true)
                .finish(),
        )
        .with_margin_top(8.)
        .finish();
        if let Some(body) =
            render_collapsible_body(data.message_id, body_element, data.is_streaming, props)
        {
            content.add_child(body);
        }
    }

    let avatar = data.participant.avatar.render(app);
    let avatar_element: Box<dyn Element> = if let (Some(conversation_id), Some(mouse_state)) = (
        data.participant.conversation_id,
        props
            .state_handles
            .transcript_avatar_handles
            .get(data.message_id),
    ) {
        // Navigate to the child's pane: focus if already open, otherwise
        // open a new pane.
        let mouse_state = mouse_state.clone();
        let self_terminal_view_id = props.terminal_view_id;
        Hoverable::new(mouse_state, move |_| avatar)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, app, _| {
                dispatch_focus_or_open_child_agent_pane(
                    conversation_id,
                    self_terminal_view_id,
                    ctx,
                    app,
                );
            })
            .finish()
    } else {
        avatar
    };

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Container::new(avatar_element)
                .with_margin_right(12.)
                .finish(),
        )
        .with_child(Shrinkable::new(1., content.finish()).finish())
        .finish()
}

pub(super) fn render_messages_received_from_agents(
    messages: &[ReceivedMessageDisplay],
    props: Props,
    app: &AppContext,
) -> Box<dyn Element> {
    if messages.is_empty() {
        return Empty::new().finish();
    }
    let orchestrator_agent_id = props
        .model
        .conversation(app)
        .and_then(|conversation| orchestrator_agent_id_for_conversation(conversation, app));
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (index, msg) in messages.iter().enumerate() {
        let sender =
            participant_for_agent_id(&msg.sender_agent_id, orchestrator_agent_id.as_deref(), app);
        let recipients = msg
            .addresses
            .iter()
            .map(|agent_id| {
                participant_for_agent_id(agent_id, orchestrator_agent_id.as_deref(), app)
            })
            .collect::<Vec<_>>();
        let row_message_id = received_message_collapsible_id(&msg.message_id);
        let row = render_transcript_row(
            TranscriptRowData {
                participant: &sender,
                recipients: &recipients,
                subject: &msg.subject,
                body: &msg.message_body,
                message_id: &row_message_id,
                is_streaming: false,
            },
            props,
            app,
        );
        let mut row_container = Container::new(row);
        if index > 0 {
            row_container = row_container.with_margin_top(12.);
        }
        column.add_child(row_container.finish());
    }

    column.finish().with_agent_output_item_spacing(app).finish()
}

fn participant_display_names(participants: &[OrchestrationParticipant]) -> String {
    participants
        .iter()
        .map(|participant| participant.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn participant_for_agent_ids(
    agent_ids: &[String],
    orchestrator_agent_id: Option<&str>,
    app: &AppContext,
) -> Vec<OrchestrationParticipant> {
    agent_ids
        .iter()
        .map(|agent_id| participant_for_agent_id(agent_id, orchestrator_agent_id, app))
        .collect()
}

fn render_transcript_row_with_spacing(
    data: TranscriptRowData<'_>,
    props: Props,
    app: &AppContext,
) -> Box<dyn Element> {
    render_transcript_row(data, props, app)
        .with_agent_output_item_spacing(app)
        .finish()
}

pub(super) fn render_send_message(
    props: Props,
    action_id: &AIAgentActionId,
    address: &[String],
    subject: &str,
    message: &str,
    message_id: &MessageId,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let status = props.action_model.as_ref(app).get_action_status(action_id);
    let orchestrator_agent_id = props
        .model
        .conversation(app)
        .and_then(|conversation| orchestrator_agent_id_for_conversation(conversation, app));
    let recipient_participants =
        participant_for_agent_ids(address, orchestrator_agent_id.as_deref(), app);
    let recipients = participant_display_names(&recipient_participants);

    if let Some(AIActionStatus::Finished(result)) = &status {
        let AIAgentActionResultType::SendMessageToAgent(result) = &result.result else {
            log::error!(
                "Unexpected action result type for send message action: {:?}",
                result.result
            );
            return Empty::new().finish();
        };
        match result {
            SendMessageToAgentResult::Success { .. } => {
                let sender = participant_for_current_conversation(
                    props,
                    orchestrator_agent_id.as_deref(),
                    app,
                );
                return render_transcript_row_with_spacing(
                    TranscriptRowData {
                        participant: &sender,
                        recipients: &recipient_participants,
                        subject,
                        body: message,
                        message_id,
                        is_streaming: false,
                    },
                    props,
                    app,
                );
            }
            SendMessageToAgentResult::Error(error) => {
                let label = format!("Failed to send message to {recipients}: {error}");
                let status_icon = inline_action_icons::red_x_icon(appearance).finish();
                return render_requested_action_row_for_text(
                    label.into(),
                    appearance.ui_font_family(),
                    Some(status_icon),
                    None,
                    false,
                    false,
                    app,
                )
                .with_agent_output_item_spacing(app)
                .with_background_color(blended_colors::neutral_2(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish();
            }
            SendMessageToAgentResult::Cancelled => {
                let label = format!("Send message to {recipients} cancelled.");
                let status_icon = inline_action_icons::cancelled_icon(appearance).finish();
                return render_requested_action_row_for_text(
                    label.into(),
                    appearance.ui_font_family(),
                    Some(status_icon),
                    None,
                    false,
                    false,
                    app,
                )
                .with_agent_output_item_spacing(app)
                .with_background_color(blended_colors::neutral_2(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish();
            }
        };
    }

    // Non-finished (streaming/queued) state.
    let dimmed_text_color = blended_colors::text_disabled(theme, theme.surface_2());
    let should_dim_text = (props.model.status(app).is_streaming()
        && !props.model.is_first_action_in_output(action_id, app))
        || status.as_ref().is_some_and(|s| s.is_queued());

    let label_fragments = vec![
        FormattedTextFragment::plain_text("Sending message to "),
        FormattedTextFragment::bold(&recipients),
        FormattedTextFragment::plain_text(format!(": {subject}")),
    ];
    let mut header_text = render_formatted_text_element(label_fragments, app);
    if should_dim_text {
        header_text = header_text.with_color(dimmed_text_color);
    }

    let has_message = !message.is_empty();
    let chevron = if has_message {
        render_collapse_chevron(message_id, props, app)
    } else {
        None
    };

    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    column.add_child(render_requested_action_row(
        header_text.into(),
        Some(action_icon(action_id, props.action_model, props.model, app).finish()),
        chevron,
        false,
        false,
        app,
    ));

    // Collapsible body: message text with max height
    if has_message {
        let message_color = if should_dim_text {
            dimmed_text_color
        } else {
            blended_colors::text_disabled(theme, theme.surface_2())
        };
        let message_element = render_collapsible_text_body(message, message_color, true, app);
        if let Some(body) = render_collapsible_body(
            message_id,
            message_element,
            props.model.status(app).is_streaming(),
            props,
        ) {
            column.add_child(body);
        }
    }

    column
        .finish()
        .with_agent_output_item_spacing(app)
        .with_background_color(blended_colors::neutral_2(theme))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish()
}

pub(super) fn render_start_agent(
    props: Props,
    action_id: &AIAgentActionId,
    name: &str,
    prompt: &str,
    execution_mode: &StartAgentExecutionMode,
    message_id: &MessageId,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let status = props.action_model.as_ref(app).get_action_status(action_id);

    if let Some(AIActionStatus::Finished(result)) = &status {
        let AIAgentActionResultType::StartAgent(result) = &result.result else {
            log::error!(
                "Unexpected action result type for start agent action: {:?}",
                result.result
            );
            return Empty::new().finish();
        };
        let child_conversation_card_data = child_conversation_card_data_for_result(result, app);
        let (label_fragments, status_icon) = match result {
            StartAgentResult::Success { .. } => (
                vec![
                    FormattedTextFragment::plain_text("Started agent "),
                    FormattedTextFragment::bold(name),
                    FormattedTextFragment::plain_text(start_agent_success_suffix(execution_mode)),
                ],
                inline_action_icons::green_check_icon(appearance).finish(),
            ),
            StartAgentResult::Error { error, .. } => (
                vec![
                    FormattedTextFragment::plain_text(start_agent_error_prefix(execution_mode)),
                    FormattedTextFragment::bold(name),
                    FormattedTextFragment::plain_text(format!(": {error}")),
                ],
                inline_action_icons::red_x_icon(appearance).finish(),
            ),
            StartAgentResult::Cancelled { .. } => (
                vec![
                    FormattedTextFragment::plain_text(start_agent_cancelled_prefix(execution_mode)),
                    FormattedTextFragment::bold(name),
                    FormattedTextFragment::plain_text(" cancelled."),
                ],
                inline_action_icons::cancelled_icon(appearance).finish(),
            ),
        };

        let has_prompt = !prompt.is_empty();
        let chevron = if has_prompt {
            render_collapse_chevron(message_id, props, app)
        } else {
            None
        };

        let header_text = render_formatted_text_element(label_fragments, app);
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(render_requested_action_row(
            header_text.into(),
            Some(status_icon),
            chevron,
            false,
            false,
            app,
        ));

        if has_prompt {
            let prompt_element = render_collapsible_text_body(
                prompt,
                blended_colors::text_disabled(theme, theme.surface_2()),
                true,
                app,
            );
            if let Some(body) = render_collapsible_body(message_id, prompt_element, false, props) {
                column.add_child(body);
            }
        }
        if let Some(card_data) = child_conversation_card_data {
            let navigation_card_handle = props
                .state_handles
                .orchestration_navigation_card_handles
                .get(action_id)
                .cloned()
                .unwrap_or_else(|| {
                    log::error!(
                        "Missing orchestration navigation card handle for StartAgent action {:?}",
                        action_id
                    );
                    MouseStateHandle::default()
                });
            let status_icon = card_data
                .status
                .status_icon_and_color(theme, StatusColorStyle::Standard);
            column.add_child(render_conversation_navigation_card_row(
                &card_data.agent_name,
                Some(&card_data.title),
                Some(status_icon),
                card_data.conversation_id,
                navigation_card_handle,
                true,
                app,
            ));
        }

        return column
            .finish()
            .with_agent_output_item_spacing(app)
            .with_background_color(blended_colors::neutral_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish();
    }

    // Non-finished (streaming/queued) state.
    let dimmed_text_color = blended_colors::text_disabled(theme, theme.surface_2());
    let should_dim_text = (props.model.status(app).is_streaming()
        && !props.model.is_first_action_in_output(action_id, app))
        || status.as_ref().is_some_and(|s| s.is_queued());

    let label_fragments = vec![
        FormattedTextFragment::plain_text(start_agent_in_progress_prefix(execution_mode)),
        FormattedTextFragment::bold(name),
        FormattedTextFragment::plain_text(" ..."),
    ];
    let mut header_text = render_formatted_text_element(label_fragments, app);
    if should_dim_text {
        header_text = header_text.with_color(dimmed_text_color);
    }

    let has_prompt = !prompt.is_empty();
    let chevron = if has_prompt {
        render_collapse_chevron(message_id, props, app)
    } else {
        None
    };

    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    column.add_child(render_requested_action_row(
        header_text.into(),
        Some(action_icon(action_id, props.action_model, props.model, app).finish()),
        chevron,
        false,
        false,
        app,
    ));

    // Collapsible body: prompt text with max height
    if has_prompt {
        let prompt_color = if should_dim_text {
            dimmed_text_color
        } else {
            blended_colors::text_disabled(theme, theme.surface_2())
        };
        let prompt_element = render_collapsible_text_body(prompt, prompt_color, true, app);
        if let Some(body) = render_collapsible_body(
            message_id,
            prompt_element,
            props.model.status(app).is_streaming(),
            props,
        ) {
            column.add_child(body);
        }
    }

    column
        .finish()
        .with_agent_output_item_spacing(app)
        .with_background_color(blended_colors::neutral_2(theme))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish()
}

fn start_agent_success_suffix(execution_mode: &StartAgentExecutionMode) -> &'static str {
    match execution_mode {
        StartAgentExecutionMode::Local { .. } => " locally.",
        StartAgentExecutionMode::Remote { .. } => " remotely.",
    }
}

fn start_agent_error_prefix(execution_mode: &StartAgentExecutionMode) -> &'static str {
    match execution_mode {
        StartAgentExecutionMode::Local { .. } => "Failed to start agent ",
        StartAgentExecutionMode::Remote { .. } => "Failed to start remote agent ",
    }
}

fn start_agent_cancelled_prefix(execution_mode: &StartAgentExecutionMode) -> &'static str {
    match execution_mode {
        StartAgentExecutionMode::Local { .. } => "Start agent ",
        StartAgentExecutionMode::Remote { .. } => "Start remote agent ",
    }
}

fn start_agent_in_progress_prefix(execution_mode: &StartAgentExecutionMode) -> &'static str {
    match execution_mode {
        StartAgentExecutionMode::Local { .. } => "Starting agent ",
        StartAgentExecutionMode::Remote { .. } => "Starting remote agent ",
    }
}

/// Renders a selectable text block below an orchestration action header, using a muted color.
/// Used for both StartAgent prompts and SendMessageToAgent message bodies.
fn render_collapsible_text_body(
    text: &str,
    text_color: ColorU,
    align_with_status_row_text: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let mut container = Container::new(
        Text::new(
            text.to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(text_color)
        .with_selectable(true)
        .finish(),
    )
    .with_margin_top(4.);

    if align_with_status_row_text {
        container = container
            .with_margin_left(INLINE_ACTION_HORIZONTAL_PADDING + icon_size(app) + ICON_MARGIN)
            .with_margin_right(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_margin_bottom(INLINE_ACTION_HEADER_VERTICAL_PADDING);
    }

    container.finish()
}

/// Card data for a child conversation navigation link.
#[derive(Debug, PartialEq)]
struct ChildConversationCardData {
    conversation_id: AIConversationId,
    agent_name: String,
    title: String,
    status: ConversationStatus,
}

fn child_conversation_card_data_for_result(
    result: &StartAgentResult,
    app: &AppContext,
) -> Option<ChildConversationCardData> {
    match result {
        StartAgentResult::Success { agent_id, .. } => {
            let conversation_id = conversation_id_for_agent_id(agent_id, app)?;
            let conversation =
                BlocklistAIHistoryModel::as_ref(app).conversation(&conversation_id)?;
            let agent_name = conversation.agent_name().unwrap_or("Agent").to_string();
            let status = conversation.status().clone();
            let title = available_conversation_title_for_id(conversation_id, app)?;
            Some(ChildConversationCardData {
                conversation_id,
                agent_name,
                title,
                status,
            })
        }
        StartAgentResult::Error { .. } | StartAgentResult::Cancelled { .. } => None,
    }
}

fn available_conversation_title_for_id(
    conversation_id: AIConversationId,
    app: &AppContext,
) -> Option<String> {
    let conversation = BlocklistAIHistoryModel::as_ref(app).conversation(&conversation_id)?;
    let title = conversation.title().filter(|title| !title.is_empty());
    match title {
        Some(title) if conversation.initial_query().as_deref() != Some(title.as_str()) => {
            Some(title)
        }
        _ => Some(GENERATING_TITLE_PLACEHOLDER.to_string()),
    }
}

/// Renders a chevron toggle for collapsing/expanding orchestration block bodies.
fn render_collapse_chevron(
    message_id: &MessageId,
    props: Props,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let state = props.collapsible_block_states.get(message_id)?;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let text_color = theme.foreground();
    let icon_sz = icon_size(app);

    let is_expanded = matches!(
        state.expansion_state,
        CollapsibleExpansionState::Expanded { .. }
    );
    let chevron_icon = if is_expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };

    let toggle_mouse_state = state.expansion_toggle_mouse_state.clone();
    let message_id_clone = message_id.clone();

    Some(
        Hoverable::new(toggle_mouse_state, move |_| {
            ConstrainedBox::new(chevron_icon.to_warpui_icon(text_color).finish())
                .with_width(icon_sz)
                .with_height(icon_sz)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AIBlockAction::ToggleCollapsibleBlockExpanded(
                message_id_clone.clone(),
            ));
        })
        .finish(),
    )
}

/// Renders the collapsible body content with max height and scroll, or None if collapsed.
fn render_collapsible_body(
    message_id: &MessageId,
    body: Box<dyn Element>,
    is_streaming: bool,
    props: Props,
) -> Option<Box<dyn Element>> {
    let Some(state) = props.collapsible_block_states.get(message_id) else {
        log::error!(
            "Missing collapsible state for orchestration message {:?}",
            message_id
        );
        return None;
    };
    render_scrollable_collapsible_content(
        message_id,
        state,
        body,
        is_streaming,
        ORCHESTRATION_COLLAPSED_MAX_HEIGHT,
    )
}

/// Builds a `FormattedTextElement` from a list of mixed plain/bold fragments.
fn render_formatted_text_element(
    fragments: Vec<FormattedTextFragment>,
    app: &AppContext,
) -> FormattedTextElement {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let formatted_text = FormattedText::new(vec![FormattedTextLine::Line(fragments)]);
    FormattedTextElement::new(
        formatted_text,
        appearance.monospace_font_size(),
        appearance.ui_font_family(),
        appearance.ui_font_family(),
        blended_colors::text_main(theme, theme.background()),
        Default::default(),
    )
    .set_selectable(true)
}

fn render_conversation_navigation_card_row(
    title: &str,
    subtitle: Option<&str>,
    icon: Option<(Icon, pathfinder_color::ColorU)>,
    conversation_id: AIConversationId,
    mouse_state: MouseStateHandle,
    align_with_status_row_text: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let card = conversation_navigation_card_with_icon(
        icon,
        title.to_string(),
        subtitle.map(|s| s.to_string()),
        move |ctx, _, _| {
            ctx.dispatch_typed_action(TerminalAction::RevealChildAgent { conversation_id });
        },
        mouse_state,
        true,
        None,
        app,
    );

    let mut container = Container::new(card).with_margin_top(6.);

    if align_with_status_row_text {
        container = container
            .with_margin_left(INLINE_ACTION_HORIZONTAL_PADDING + icon_size(app) + ICON_MARGIN)
            .with_margin_right(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_margin_bottom(INLINE_ACTION_HEADER_VERTICAL_PADDING);
    }

    container.finish()
}

#[cfg(test)]
#[path = "orchestration_tests.rs"]
mod tests;
