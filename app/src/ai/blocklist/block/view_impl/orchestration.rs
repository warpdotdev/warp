//! Rendering functions for orchestration-related output items (messaging & agent management).

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use warp_errors::report_error;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, FormattedTextElement,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
    Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, SingletonEntity};

use super::WithContentItemSpacing;
use super::common::render_scrollable_collapsible_content;
use super::output::{Props, action_icon};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResultType, MessageId, ReceivedMessageDisplay,
    SendMessageToAgentResult,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::action_model::AIActionStatus;
use crate::ai::blocklist::agent_view::orchestration_avatar::OrchestrationAvatar;
use crate::ai::blocklist::agent_view::orchestration_conversation_links::dispatch_focus_or_open_child_agent_pane;
use crate::ai::blocklist::block::model::AIBlockModelHelper;
use crate::ai::blocklist::block::{
    AIBlockAction, CollapsibleExpansionState, received_message_collapsible_id,
};
use crate::ai::blocklist::inline_action::inline_action_header::{
    ICON_MARGIN, INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::{self, icon_size};
use crate::ai::blocklist::inline_action::requested_action::{
    render_requested_action_row, render_requested_action_row_for_text,
};
use crate::ai::blocklist::orchestration_topology::{
    OrchestrationParticipantKind, orchestrator_agent_id_for_conversation,
    resolve_orchestration_participant,
};
use crate::ai::conversation_status_ui::render_status_element;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

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
    let participant = resolve_orchestration_participant(
        BlocklistAIHistoryModel::as_ref(app),
        agent_id,
        orchestrator_agent_id,
    );
    let display_name = participant.kind.display_name().to_string();
    let avatar = match &participant.kind {
        OrchestrationParticipantKind::Orchestrator => OrchestrationAvatar::Orchestrator,
        OrchestrationParticipantKind::Agent { .. } | OrchestrationParticipantKind::Unknown => {
            OrchestrationAvatar::agent(display_name.clone())
        }
    };
    OrchestrationParticipant {
        display_name,
        avatar,
        conversation_id: match &participant.kind {
            OrchestrationParticipantKind::Orchestrator | OrchestrationParticipantKind::Unknown => {
                None
            }
            OrchestrationParticipantKind::Agent { .. } => participant.conversation_id,
        },
    }
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

/// Whether a child conversation status should show an activity indicator on
/// the spawned-agent transcript row (REMOTE-2409).
pub(super) fn transcript_row_shows_activity_indicator(status: &ConversationStatus) -> bool {
    status.is_in_progress() || status.is_transient_error() || status.is_waiting_for_events()
}

fn participant_conversation_status(
    conversation_id: Option<AIConversationId>,
    app: &AppContext,
) -> Option<ConversationStatus> {
    let conversation_id = conversation_id?;
    BlocklistAIHistoryModel::as_ref(app)
        .conversation(&conversation_id)
        .map(|conversation| conversation.status().clone())
}

/// Labeled "View" control that opens/focuses the child agent pane. Uses a
/// persistent mouse handle so hover state survives re-renders.
fn render_view_child_button(
    conversation_id: AIConversationId,
    mouse_state: MouseStateHandle,
    self_terminal_view_id: warpui::EntityId,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.monospace_font_size() - 1.;
    let label_color = blended_colors::text_main(theme, theme.background());

    Hoverable::new(mouse_state, move |hover_state| {
        let background = if hover_state.is_hovered() || hover_state.is_clicked() {
            blended_colors::fg_overlay_2(theme)
        } else {
            blended_colors::fg_overlay_1(theme)
        };
        Container::new(
            Text::new("View".to_string(), font_family, font_size)
                .with_color(label_color)
                .soft_wrap(false)
                .finish(),
        )
        .with_horizontal_padding(8.)
        .with_vertical_padding(2.)
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, app, _| {
        dispatch_focus_or_open_child_agent_pane(conversation_id, self_terminal_view_id, ctx, app);
    })
    .finish()
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
    let child_status = participant_conversation_status(data.participant.conversation_id, app);
    let show_activity = child_status
        .as_ref()
        .is_some_and(transcript_row_shows_activity_indicator);

    // Name is display-only (not the expand target). Chevron alone expands the
    // body; a separate View control opens the child conversation (REMOTE-2409).
    let name = FormattedTextFragment::bold(&data.participant.display_name);
    let name_element = render_formatted_text_element(vec![name], app)
        .set_selectable(false)
        .finish();

    let mut header_leading = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_child(Shrinkable::new(1., name_element).finish());

    if show_activity && let Some(status) = child_status.as_ref() {
        header_leading = header_leading.with_child(
            Container::new(render_status_element(
                status,
                icon_size(app) - 4.,
                appearance,
            ))
            .with_margin_left(6.)
            .finish(),
        );
    }

    if let Some(state) = collapsible_state {
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
        // Chevron-only expand target so the agent name is not the sole path
        // into the collapsible body (users need a clear open-conversation path).
        let chevron = Hoverable::new(toggle_mouse_state, move |_| {
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
        .finish();
        header_leading =
            header_leading.with_child(Container::new(chevron).with_margin_left(6.).finish());
    }

    let mut header_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_child(Shrinkable::new(1., header_leading.finish()).finish());

    if let (Some(conversation_id), Some(view_mouse_state)) = (
        data.participant.conversation_id,
        props
            .state_handles
            .transcript_view_handles
            .get(data.message_id),
    ) {
        header_row = header_row.with_child(
            Container::new(render_view_child_button(
                conversation_id,
                view_mouse_state.clone(),
                props.terminal_view_id,
                appearance,
            ))
            .with_margin_left(8.)
            .finish(),
        );
    }

    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    content.add_child(header_row.finish());
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
        // Keep avatar open as an additive path; labeled View is the discoverable
        // control (REMOTE-2409).
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
    let orchestrator_agent_id = props.model.conversation(app).and_then(|conversation| {
        orchestrator_agent_id_for_conversation(BlocklistAIHistoryModel::as_ref(app), conversation)
    });
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
    let orchestrator_agent_id = props.model.conversation(app).and_then(|conversation| {
        orchestrator_agent_id_for_conversation(BlocklistAIHistoryModel::as_ref(app), conversation)
    });
    let recipient_participants =
        participant_for_agent_ids(address, orchestrator_agent_id.as_deref(), app);
    let recipients = participant_display_names(&recipient_participants);

    if let Some(AIActionStatus::Finished(result)) = &status {
        let AIAgentActionResultType::SendMessageToAgent(result) = &result.result else {
            report_error!(
                "Unexpected action result type for send message action",
                extra: { "result_type" => ?result.result }
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
        report_error!(
            "Missing collapsible state for orchestration message",
            extra: { "message_id" => ?message_id }
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

#[cfg(test)]
#[path = "orchestration_tests.rs"]
mod tests;
