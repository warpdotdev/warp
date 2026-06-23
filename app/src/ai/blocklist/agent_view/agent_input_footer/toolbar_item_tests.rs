use super::AgentToolbarItemKind;
use crate::features::FeatureFlag;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::ui_components::icons::Icon;

#[test]
fn fetched_memories_is_agent_view_only() {
    let item = AgentToolbarItemKind::FetchedMemories;

    assert!(item.available_in().is_available_for_agent_view());
    assert!(!item.available_in().is_available_for_cli());
}

#[test]
fn fetched_memories_is_visible_to_session_viewers() {
    let item = AgentToolbarItemKind::FetchedMemories;

    assert!(item.available_to_session_viewer(&SharedSessionStatus::reader(), false));
    assert!(item.available_to_session_viewer(&SharedSessionStatus::NotShared, false));
}

#[test]
fn fetched_memories_display_metadata() {
    let item = AgentToolbarItemKind::FetchedMemories;

    assert_eq!(item.display_label(), "Memories");
    assert_eq!(item.icon(), Some(Icon::Cognition));
    assert!(!item.is_available_during_handoff_compose());
}

#[test]
fn default_right_inserts_fetched_memories_after_context_usage_when_flag_enabled() {
    let _flag = FeatureFlag::FetchedMemoriesChip.override_enabled(true);

    let items = AgentToolbarItemKind::default_right();
    let context_usage_index = items
        .iter()
        .position(|item| matches!(item, AgentToolbarItemKind::ContextWindowUsage))
        .expect("default_right should contain ContextWindowUsage");

    assert_eq!(
        items.get(context_usage_index + 1),
        Some(&AgentToolbarItemKind::FetchedMemories)
    );
    assert!(AgentToolbarItemKind::all_available().contains(&AgentToolbarItemKind::FetchedMemories));
}

#[test]
fn default_right_excludes_fetched_memories_when_flag_disabled() {
    let _flag = FeatureFlag::FetchedMemoriesChip.override_enabled(false);

    assert!(!AgentToolbarItemKind::default_right().contains(&AgentToolbarItemKind::FetchedMemories));
    assert!(
        !AgentToolbarItemKind::all_available().contains(&AgentToolbarItemKind::FetchedMemories)
    );
}
