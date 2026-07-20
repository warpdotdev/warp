//! Rich TUI rendering for messages received from orchestration participants.

use warp::tui_export::{
    AIConversationId, BlocklistAIHistoryModel, ConversationStatus, MessageId,
    OrchestrationParticipantKind, ReceivedMessageDisplay, orchestrator_agent_id_for_conversation,
    resolve_orchestration_participant,
};
use warpui::SingletonEntity;
use warpui_core::AppContext;
use warpui_core::elements::tui::{
    Modifier, TuiContainer, TuiElement, TuiStyle, TuiText, tui_collapsible,
};

use crate::agent_block::{CollapsibleSectionStates, TuiAIBlockAction};
use crate::orchestrated_agent_identity_styling::{
    AgentIdentity, assign_agent_identity_indices, stable_hash,
};
use crate::tui_builder::TuiUiBuilder;

/// Render-ready identity and lifecycle presentation for a message sender.
struct AgentMessagePresentation {
    name: String,
    status: ConversationStatus,
    identity: AgentIdentity,
}

/// Compact glyph for a conversation's lifecycle status.
fn conversation_status_glyph(status: &ConversationStatus) -> &'static str {
    match status {
        ConversationStatus::InProgress
        | ConversationStatus::TransientError
        | ConversationStatus::WaitingForEvents => "●",
        ConversationStatus::Success => "✓",
        ConversationStatus::Error => "×",
        ConversationStatus::Cancelled | ConversationStatus::Blocked { .. } => "■",
    }
}

/// Semantic theme style for a conversation's lifecycle glyph.
fn conversation_status_glyph_style(
    status: &ConversationStatus,
    builder: &TuiUiBuilder,
) -> TuiStyle {
    match status {
        ConversationStatus::InProgress
        | ConversationStatus::TransientError
        | ConversationStatus::WaitingForEvents
        | ConversationStatus::Blocked { .. } => builder.attention_glyph_style(),
        ConversationStatus::Success => builder.success_glyph_style(),
        ConversationStatus::Error => builder.error_text_style(),
        ConversationStatus::Cancelled => builder.muted_text_style(),
    }
}

/// Returns a child's stable identity index among its siblings.
fn child_identity_index(
    history: &BlocklistAIHistoryModel,
    conversation_id: AIConversationId,
    palette_len: usize,
) -> Option<usize> {
    let conversation = history.conversation(&conversation_id)?;
    let parent_id = history.resolved_parent_conversation_id_for_conversation(conversation)?;
    let siblings = history.child_conversations_of(parent_id);
    let sender_index = siblings
        .iter()
        .position(|sibling| sibling.id() == conversation_id)?;
    assign_agent_identity_indices(
        siblings
            .iter()
            .map(|sibling| sibling.agent_name().unwrap_or("Agent")),
        palette_len,
    )
    .get(sender_index)
    .copied()
}

/// Resolves a sender's name and sibling-stable identity.
fn message_presentation(
    sender_agent_id: &str,
    current_conversation_id: AIConversationId,
    builder: &TuiUiBuilder,
    app: &AppContext,
) -> AgentMessagePresentation {
    let history = BlocklistAIHistoryModel::as_ref(app);
    let palette = builder.agent_identity_palette();
    let orchestrator_agent_id = history
        .conversation(&current_conversation_id)
        .and_then(|conversation| orchestrator_agent_id_for_conversation(history, conversation));
    let participant = resolve_orchestration_participant(
        history,
        sender_agent_id,
        orchestrator_agent_id.as_deref(),
    );
    let sender = participant
        .conversation_id
        .and_then(|conversation_id| history.conversation(&conversation_id));
    let status = sender
        .map(|conversation| conversation.status().clone())
        .unwrap_or(ConversationStatus::InProgress);
    let fallback_identity_index = (!palette.is_empty()).then(|| {
        usize::try_from(stable_hash(sender_agent_id) % palette.len() as u64).unwrap_or_default()
    });
    let (name, identity) = match participant.kind {
        OrchestrationParticipantKind::Orchestrator => {
            ("Orchestrator".to_owned(), AgentIdentity::default())
        }
        OrchestrationParticipantKind::Agent { name } => (
            name,
            participant
                .conversation_id
                .and_then(|conversation_id| {
                    child_identity_index(history, conversation_id, palette.len())
                })
                .or(fallback_identity_index)
                .and_then(|index| palette.get(index))
                .cloned()
                .unwrap_or_default(),
        ),
        OrchestrationParticipantKind::Unknown => (
            "Unknown agent".to_owned(),
            fallback_identity_index
                .and_then(|index| palette.get(index))
                .cloned()
                .unwrap_or_default(),
        ),
    };
    AgentMessagePresentation {
        name,
        status,
        identity,
    }
}

/// The persistent collapse-state key for one received message.
pub(crate) fn agent_message_section_id(message: &ReceivedMessageDisplay) -> MessageId {
    MessageId::new(format!("received-agent-message:{}", message.message_id))
}

/// Renders a received child message as a collapsed-by-default disclosure.
pub(crate) fn render_agent_message(
    states: &CollapsibleSectionStates,
    message: &ReceivedMessageDisplay,
    current_conversation_id: AIConversationId,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let presentation = message_presentation(
        &message.sender_agent_id,
        current_conversation_id,
        &builder,
        app,
    );
    let header_spans = [
        (
            format!("{} ", conversation_status_glyph(&presentation.status)),
            conversation_status_glyph_style(&presentation.status, &builder),
        ),
        (
            format!("{} ", presentation.identity.glyph),
            presentation.identity.style,
        ),
        (
            presentation.name,
            presentation.identity.style.add_modifier(Modifier::BOLD),
        ),
        // The helper adds another separating space with its chevron.
        (" ".to_owned(), builder.primary_text_style()),
    ];
    let preview = if message.message_body.trim().is_empty() {
        message.subject.as_str()
    } else {
        message.message_body.as_str()
    }
    .to_owned();
    let preview_style = builder.muted_text_style();
    let message_id = agent_message_section_id(message);
    let collapsed = states.is_collapsed(&message_id, true);
    let toggle_message_id = message_id.clone();
    tui_collapsible(
        collapsed,
        header_spans,
        builder.primary_text_style(),
        states.hover_state(&message_id),
        move || {
            TuiContainer::new(
                TuiText::new(preview.clone())
                    .with_style(preview_style)
                    .finish(),
            )
            .with_padding_left(4)
            .finish()
        },
        move |event_ctx, _app| {
            event_ctx.dispatch_typed_action(TuiAIBlockAction::SetSectionCollapsed {
                message_id: toggle_message_id.clone(),
                collapsed: !collapsed,
            });
        },
    )
}

#[cfg(test)]
#[path = "agent_message_tests.rs"]
mod tests;
