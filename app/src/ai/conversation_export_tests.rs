use tempfile::tempdir;

use super::*;

#[test]
fn explicit_filename_is_trimmed_and_gets_markdown_extension() {
    assert_eq!(
        conversation_export_filename_at(
            Some("  notes/session  "),
            Some("ignored"),
            "20260710_120000"
        ),
        "notes/session.md"
    );
    assert_eq!(
        conversation_export_filename_at(
            Some("conversation.md"),
            Some("ignored"),
            "20260710_120000"
        ),
        "conversation.md"
    );
}

#[test]
fn default_filename_sanitizes_the_conversation_title() {
    assert_eq!(
        conversation_export_filename_at(None, Some("Fix: slash commands / TUI"), "20260710_120000"),
        "20260710_120000-Fix__slash_commands___TUI.md"
    );
    assert_eq!(
        conversation_export_filename_at(None, None, "20260710_120000"),
        "20260710_120000-conversation.md"
    );
}

#[test]
fn export_writes_markdown_and_reports_overwrites() {
    let directory = tempdir().unwrap();
    let directory_path = directory.path().to_string_lossy();

    let first =
        export_conversation_markdown(Some(&directory_path), Some("conversation"), None, "# First")
            .unwrap();
    assert_eq!(first.path(), directory.path().join("conversation.md"));
    assert!(!first.overwrote_existing());
    assert_eq!(std::fs::read_to_string(first.path()).unwrap(), "# First");

    let second = export_conversation_markdown(
        Some(&directory_path),
        Some("conversation.md"),
        None,
        "# Second",
    )
    .unwrap();
    assert!(second.overwrote_existing());
    assert_eq!(std::fs::read_to_string(second.path()).unwrap(), "# Second");
}

#[test]
fn missing_subdirectory_returns_a_friendly_error() {
    let directory = tempdir().unwrap();
    let directory_path = directory.path().to_string_lossy();
    let error = export_conversation_markdown(
        Some(&directory_path),
        Some("missing/conversation.md"),
        None,
        "content",
    )
    .unwrap_err();

    assert_eq!(
        error.path(),
        directory.path().join("missing/conversation.md")
    );
    assert_eq!(
        error.user_message(),
        format!(
            "Directory not found: {}",
            directory.path().join("missing").display()
        )
    );
}
