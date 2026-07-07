use super::{is_tui_owned, TUI_BINDING_GROUP};

#[test]
fn tui_ownership_is_by_name_prefix_or_group() {
    // Editable TUI bindings are owned by name prefix.
    assert!(is_tui_owned("tui:input:submit", None));
    // Fixed TUI bindings have no name; they are owned by group.
    assert!(is_tui_owned("", Some(TUI_BINDING_GROUP)));

    // GUI bindings — named, unnamed, or grouped differently — are not.
    assert!(!is_tui_owned("terminal:cancel_command", None));
    assert!(!is_tui_owned("", None));
    assert!(!is_tui_owned("", Some("workspace")));
    assert!(!is_tui_owned("input:clear_screen", None));
}
