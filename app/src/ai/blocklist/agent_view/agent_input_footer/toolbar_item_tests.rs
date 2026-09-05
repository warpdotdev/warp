use super::*;

/// The File explorer chip is offered to the agent view toolbelt editor.
#[test]
fn file_explorer_is_offered_in_the_agent_view() {
    assert!(
        AgentToolbarItemKind::FileExplorer
            .available_in()
            .is_available_for_agent_view()
    );
    assert!(AgentToolbarItemKind::all_available().contains(&AgentToolbarItemKind::FileExplorer));
}

/// ...but it is opt-in, so it must stay out of the default agent view layout.
#[test]
fn file_explorer_is_not_an_agent_view_default() {
    assert!(!AgentToolbarItemKind::default_left().contains(&AgentToolbarItemKind::FileExplorer));
    assert!(!AgentToolbarItemKind::default_right().contains(&AgentToolbarItemKind::FileExplorer));
}
