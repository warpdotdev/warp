use warpui::keymap::Context;
use warpui::App;

use crate::util::bindings::keybinding_name_to_keystroke;

/// The branch-selector shortcut is registered as an *editable* binding with no
/// default keystroke, so it appears in Settings → Keyboard shortcuts but does
/// nothing until the user assigns a key.
#[test]
fn test_open_branch_selector_binding_is_editable_with_no_default() {
    App::test((), |mut app| async move {
        app.update(crate::workspace::register_code_review_branch_selector_binding);

        app.update(|ctx| {
            let binding = ctx
                .editable_bindings()
                .find(|binding| binding.name == crate::code_review::OPEN_BRANCH_SELECTOR_BINDING_NAME)
                .expect(
                    "code_review:open_branch_selector should be registered as an editable binding",
                );

            // No default keystroke: the trigger stays empty until the user
            // assigns one in keyboard settings.
            assert!(
                binding.trigger.is_empty(),
                "code_review:open_branch_selector should have no default keystroke"
            );
            assert_eq!(
                None,
                keybinding_name_to_keystroke(
                    crate::code_review::OPEN_BRANCH_SELECTOR_BINDING_NAME,
                    ctx
                ),
                "an unassigned editable binding should resolve to no keystroke"
            );

            let inactive_context = Context::default();
            assert!(
                !binding.in_context(&inactive_context),
                "code_review:open_branch_selector should not be active without an open code review panel"
            );

            let mut open_panel_context = Context::default();
            open_panel_context.set.insert("Workspace");
            open_panel_context
                .set
                .insert(crate::workspace::WORKSPACE_CODE_REVIEW_PANEL_OPEN_NOT_EDITING);
            assert!(
                binding.in_context(&open_panel_context),
                "code_review:open_branch_selector should be active when code review is open and not editing"
            );
        });
    });
}
