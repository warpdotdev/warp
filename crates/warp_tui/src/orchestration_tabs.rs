use std::collections::HashMap;

use warp::tui_export::{AIConversationId, ConversationStatus};
use warpui_core::elements::tui::{TuiElement, TuiStyle, TuiText};

use crate::agent_message::{conversation_status_glyph, conversation_status_glyph_style};
use crate::orchestrated_agent_identity_styling::{AgentIdentity, assign_agent_identity_indices};
use crate::orchestration_model::TuiOrchestrationSnapshot;
use crate::tab_bar::{TuiTab, TuiTabBarConfig};
use crate::tui_builder::TuiUiBuilder;

pub(crate) const ORCHESTRATION_TAB_BAR_FOCUSED_FLAG: &str = "TuiOrchestrationTabBarFocused";
const ORCHESTRATION_TAB_LABEL_MAX_COLUMNS: u16 = 20;

pub(crate) fn orchestration_tab_bar_config(
    snapshot: &TuiOrchestrationSnapshot,
    focused: bool,
    builder: &TuiUiBuilder,
) -> TuiTabBarConfig {
    let palette = builder.agent_identity_palette();
    let mut children_in_spawn_order = snapshot.children.iter().collect::<Vec<_>>();
    children_in_spawn_order.sort_by_key(|child| child.spawn_index);
    let identity_indices = assign_agent_identity_indices(
        children_in_spawn_order
            .iter()
            .map(|child| child.label.as_str()),
        palette.len(),
    );
    let identity_by_conversation = children_in_spawn_order
        .into_iter()
        .map(|child| child.conversation_id)
        .zip(identity_indices)
        .collect::<HashMap<AIConversationId, usize>>();
    let tabs = snapshot
        .children
        .iter()
        .map(|child| {
            let identity = palette
                .get(
                    identity_by_conversation
                        .get(&child.conversation_id)
                        .copied()
                        .unwrap_or_default(),
                )
                .or_else(|| palette.first())
                .cloned()
                .unwrap_or_default();
            let (icon_glyph, icon_style) =
                orchestration_tab_icon(&child.status, &identity, builder);
            TuiTab::new(child.conversation_id.to_string(), child.label.clone())
                .with_leading_text(icon_glyph, icon_style)
        })
        .collect();
    let mut config = TuiTabBarConfig::new(tabs);
    config.leading = Some("   Agents:   ".to_owned());
    config.main_tab = Some(TuiTab::new(
        snapshot.root_conversation_id.to_string(),
        "orchestrator",
    ));
    config.selected_key = Some(snapshot.selected_conversation_id.to_string());
    config.focused = focused;
    config.page_anchor = snapshot.page_anchor.map(|id| id.to_string());
    config.reveal_selected = snapshot.reveal_selected;
    config.maximum_label_columns = Some(ORCHESTRATION_TAB_LABEL_MAX_COLUMNS);
    config.secondary_gap_columns = 3;
    config.styles = builder.orchestration_tab_bar_styles();
    config
}

pub(crate) fn render_orchestration_tab_footer(builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    let primary = builder.primary_text_style();
    let muted = builder.muted_text_style();
    TuiText::from_spans([
        ("Tab or ← →".to_string(), primary),
        (" to navigate  ".to_string(), muted),
        ("Shift + ← →".to_string(), primary),
        (" to go to start/end  ".to_string(), muted),
        ("Shift + ↓".to_string(), primary),
        (" to send a message".to_string(), muted),
    ])
    .truncate()
    .finish()
}

pub(crate) fn render_cloud_orchestration_tab_footer(builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    let primary = builder.primary_text_style();
    let muted = builder.muted_text_style();
    TuiText::from_spans([
        ("Tab or ← →".to_string(), primary),
        (" to navigate  ".to_string(), muted),
        ("Shift + ← →".to_string(), primary),
        (" to go to start/end".to_string(), muted),
    ])
    .truncate()
    .finish()
}

pub(crate) fn orchestration_tab_icon(
    status: &ConversationStatus,
    identity: &AgentIdentity,
    builder: &TuiUiBuilder,
) -> (&'static str, TuiStyle) {
    match status {
        ConversationStatus::InProgress
        | ConversationStatus::TransientError
        | ConversationStatus::WaitingForEvents
        | ConversationStatus::Blocked { .. } => (
            conversation_status_glyph(status),
            conversation_status_glyph_style(status, builder),
        ),
        ConversationStatus::Success | ConversationStatus::Error | ConversationStatus::Cancelled => {
            (identity.glyph, identity.style)
        }
    }
}
