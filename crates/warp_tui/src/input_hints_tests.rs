use super::{AGENT_HINT, ZERO_STATE_AGENT_HINT, agent_input_hint};

#[test]
fn transcript_state_selects_the_hint_variant() {
    assert_eq!(agent_input_hint(true), ZERO_STATE_AGENT_HINT);
    assert_eq!(agent_input_hint(false), AGENT_HINT);
    assert_ne!(agent_input_hint(true), agent_input_hint(false));
}

#[test]
fn zero_state_hint_teaches_commands_and_conversations() {
    assert!(ZERO_STATE_AGENT_HINT.contains("/ for commands"));
    assert!(ZERO_STATE_AGENT_HINT.contains("← for conversations"));
}

#[test]
fn started_transcript_hint_teaches_shell_mode_and_commands() {
    assert!(AGENT_HINT.contains("! for shell mode"));
    assert!(AGENT_HINT.contains("/ for commands"));
}
