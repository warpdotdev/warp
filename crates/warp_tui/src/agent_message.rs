//! Rich TUI rendering for messages received from orchestration participants.

use warp::tui_export::{
    AIConversationId, BlocklistAIHistoryModel, ConversationStatus, MessageId,
    ReceivedMessageDisplay,
};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{tui_collapsible, Modifier, TuiContainer, TuiElement, TuiText};
use warpui_core::AppContext;

use crate::agent_block::{CollapsibleSectionStates, TuiAIBlockAction};
use crate::orchestrated_agent_identity_styling::{stable_hash, AgentIdentity};
use crate::orchestration_model::{TuiOrchestrationModel, TuiOrchestrationParticipantKind};
use crate::status::TuiStatusState;
use crate::tui_builder::TuiUiBuilder;

/// Render-ready identity and lifecycle presentation for a message sender.
struct AgentMessagePresentation {
    name: String,
    status: TuiStatusState,
    identity: AgentIdentity,
}
/// Maps a shared conversation lifecycle into the common TUI status contract.
fn agent_status(status: &ConversationStatus) -> TuiStatusState {
    match status {
        ConversationStatus::InProgress
        | ConversationStatus::TransientError
        | ConversationStatus::WaitingForEvents => TuiStatusState::Running,
        ConversationStatus::Success => TuiStatusState::Succeeded,
        ConversationStatus::Error => TuiStatusState::Failed,
        ConversationStatus::Cancelled => TuiStatusState::Cancelled,
        ConversationStatus::Blocked { .. } => TuiStatusState::Blocked,
    }
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
    let participant = TuiOrchestrationModel::as_ref(app).participant_snapshot(
        current_conversation_id,
        sender_agent_id,
        palette.len(),
        app,
    );
    let sender = participant
        .conversation_id
        .and_then(|conversation_id| history.conversation(&conversation_id));
    let name = participant.display_name;
    let status = sender
        .map(|conversation| agent_status(conversation.status()))
        .unwrap_or(TuiStatusState::Running);
    let fallback_identity_index = (!palette.is_empty()).then(|| {
        usize::try_from(stable_hash(sender_agent_id) % palette.len() as u64).unwrap_or_default()
    });
    let identity = match participant.kind {
        TuiOrchestrationParticipantKind::Orchestrator => AgentIdentity::default(),
        TuiOrchestrationParticipantKind::Child => participant
            .identity_index
            .or(fallback_identity_index)
            .and_then(|index| palette.get(index))
            .cloned()
            .unwrap_or_default(),
        TuiOrchestrationParticipantKind::Unknown => fallback_identity_index
            .and_then(|index| palette.get(index))
            .cloned()
            .unwrap_or_default(),
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
            format!("{} ", presentation.status.glyph()),
            presentation.status.glyph_style(&builder),
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
