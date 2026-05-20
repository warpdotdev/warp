use super::*;

// Pre-order traversal correctness for the descendant walker is exercised in
// `app/src/ai/blocklist/orchestration_topology_tests.rs`. These tests stay
// focused on the pill bar's own dispatch behavior.

#[test]
fn navigation_action_for_child_pill_reveals_existing_child_pane() {
    let conversation_id = AIConversationId::new();

    assert!(matches!(
        navigation_action_for_pill(PillKind::Child, conversation_id),
        TerminalAction::RevealChildAgent {
            conversation_id: actual_id,
        } if actual_id == conversation_id
    ));
}

#[test]
fn navigation_action_for_orchestrator_pill_switches_in_place() {
    let conversation_id = AIConversationId::new();

    assert!(matches!(
        navigation_action_for_pill(PillKind::Orchestrator, conversation_id),
        TerminalAction::SwitchAgentViewToConversation {
            conversation_id: actual_id,
        } if actual_id == conversation_id
    ));
}
