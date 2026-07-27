use std::collections::HashSet;

use warpui_core::keymap::{Context, Trigger};
use warpui_core::{App, TuiView};

use super::{ATTACHMENTS_AVAILABLE_FLAG, TUI_BINDING_GROUP, is_tui_owned};
use crate::attachment_bar::{FOCUS_ATTACHMENTS_BINDING_NAME, TuiAttachmentBar};
use crate::input::TuiInputView;
use crate::terminal_session_view::{
    PASTE_IMAGE_BINDING_NAME, SESSION_COMPOSER_OWNS_INPUT_FLAG, TuiTerminalSessionView,
};

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

#[test]
fn tui_binding_registration_passes_the_app_cross_platform_validator() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.set_default_binding_validator(warp::util::bindings::is_binding_cross_platform);
            super::init(ctx);
        });
    });
}

#[test]
fn attachment_bindings_are_scoped_to_available_and_focused_contexts() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            crate::terminal_session_view::init(ctx);
            crate::attachment_bar::init(ctx);

            let mut input_with_attachments = Context::default();
            input_with_attachments.set.insert(TuiInputView::ui_name());
            input_with_attachments
                .set
                .insert(TuiTerminalSessionView::ui_name());
            input_with_attachments
                .set
                .insert(ATTACHMENTS_AVAILABLE_FLAG);
            let mut plain_input = Context::default();
            plain_input.set.insert(TuiInputView::ui_name());
            plain_input.set.insert(TuiTerminalSessionView::ui_name());
            let focus_bindings = ctx
                .editable_bindings()
                .filter(|binding| binding.name == FOCUS_ATTACHMENTS_BINDING_NAME)
                .collect::<Vec<_>>();
            assert_eq!(focus_bindings.len(), 1);
            assert!(
                focus_bindings
                    .iter()
                    .all(|binding| binding.in_context(&input_with_attachments))
            );
            assert!(
                focus_bindings
                    .iter()
                    .all(|binding| !binding.in_context(&plain_input))
            );

            let mut bar_context = Context::default();
            bar_context.set.insert(TuiAttachmentBar::ui_name());
            let local_bindings = ctx
                .editable_bindings()
                .filter(|binding| binding.name.starts_with("tui:attachments:"))
                .collect::<Vec<_>>();
            assert!(!local_bindings.is_empty());
            assert!(
                local_bindings
                    .iter()
                    .all(|binding| binding.in_context(&bar_context))
            );
            assert!(
                local_bindings
                    .iter()
                    .all(|binding| !binding.in_context(&plain_input))
            );

            let paste_bindings = ctx
                .editable_bindings()
                .filter(|binding| binding.name == PASTE_IMAGE_BINDING_NAME)
                .collect::<Vec<_>>();
            assert!(!paste_bindings.is_empty());
            let mut composer_context = plain_input.clone();
            composer_context
                .set
                .insert(SESSION_COMPOSER_OWNS_INPUT_FLAG);
            assert!(
                paste_bindings
                    .iter()
                    .all(|binding| binding.in_context(&composer_context))
            );
            assert!(
                paste_bindings
                    .iter()
                    .all(|binding| !binding.in_context(&plain_input))
            );
            assert!(
                paste_bindings
                    .iter()
                    .all(|binding| !binding.in_context(&bar_context))
            );
            let paste_triggers = paste_bindings
                .iter()
                .filter_map(|binding| match binding.trigger {
                    Trigger::Keystrokes(keys) => keys.first().map(|key| key.normalized()),
                    Trigger::Empty | Trigger::Standard(_) | Trigger::Custom(_) => None,
                })
                .collect::<HashSet<_>>();
            assert!(paste_triggers.contains("ctrl-v"));
            assert!(paste_triggers.contains("ctrl-shift-V"));
        });
    });
}
