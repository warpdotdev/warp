use warp::tui_export::{export_conversation_markdown, PtyIntent, PtyIntentEvent};

use super::{export_file_success_message, raw_prompt_if_not_blank, TuiTerminalSessionEvent};

#[test]
fn interrupt_event_projects_to_high_level_pty_intent() {
    let event = TuiTerminalSessionEvent::InterruptPty;
    assert!(matches!(event.pty_intent(), Some(PtyIntent::Interrupt)));
}

#[test]
fn non_command_prompt_preserves_leading_whitespace() {
    assert_eq!(raw_prompt_if_not_blank("  /compact"), Some("  /compact"));
}

#[test]
fn whitespace_only_prompt_is_ignored() {
    assert_eq!(raw_prompt_if_not_blank(" \t\n"), None);
}

#[test]
fn file_export_success_message_includes_destination_path() {
    let directory = tempfile::tempdir().expect("temp directory");
    let export = export_conversation_markdown(
        Some(directory.path().to_str().expect("UTF-8 temp path")),
        Some("conversation.md"),
        None,
        "# Conversation",
    )
    .expect("conversation export");

    assert_eq!(
        export_file_success_message(&export),
        format!("Conversation exported to {}", export.path().display())
    );
}
