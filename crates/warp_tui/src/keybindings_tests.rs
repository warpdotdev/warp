use warpui_core::App;

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

/// Registering every TUI binding — including the orchestration card's
/// enter/ctrl-e/escape/ctrl-c and Tab/Left/Right navigation set — must satisfy the debug-time
/// cross-surface validators, which panic on any keystroke binding matching
/// a TUI view's context that is not TUI-owned.
#[test]
fn tui_binding_registration_passes_the_cross_surface_validators() {
    App::test((), |mut app| async move {
        app.update(super::init);
    });
}
