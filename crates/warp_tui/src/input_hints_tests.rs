use super::{ASK_AGENT_HINT, COMMANDS_HINT, CONVERSATIONS_HINT, SHELL_MODE_HINT, agent_input_hint};

#[test]
fn transcript_state_selects_the_applicable_hint_segments() {
    let zero_state = agent_input_hint(true, false);
    assert!(zero_state.contains(COMMANDS_HINT));
    assert!(zero_state.contains(CONVERSATIONS_HINT));
    assert!(!zero_state.contains(ASK_AGENT_HINT));
    assert!(!zero_state.contains(SHELL_MODE_HINT));

    let started = agent_input_hint(false, false);
    assert!(started.contains(ASK_AGENT_HINT));
    assert!(started.contains(SHELL_MODE_HINT));
    assert!(started.contains(COMMANDS_HINT));
    assert!(!started.contains(CONVERSATIONS_HINT));
}
