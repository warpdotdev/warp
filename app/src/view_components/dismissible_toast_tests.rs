use std::time::Duration;

use warp_core::ui::appearance::Appearance;
use warpui::keymap::Keystroke;
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView, ViewHandle};

use super::{
    COLLAPSED_MAX_CHARS, COLLAPSED_MAX_LINES, DismissibleToast, DismissibleToastAction,
    DismissibleToastStack, ToastFlavor, is_expand_toggle_keystroke, toast_message_is_truncated,
    truncate_toast_message,
};

fn stack_handle(app: &mut App) -> ViewHandle<DismissibleToastStack<()>> {
    app.add_singleton_model(|_| Appearance::mock());
    let (_, stack) = app.add_window(WindowStyle::NotStealFocus, |_| {
        DismissibleToastStack::new(Duration::from_secs(30))
    });
    stack
}
#[test]
fn newline_heavy_message_is_truncated_to_collapsed_lines() {
    let message = "line one\nline two\nline three";

    assert!(toast_message_is_truncated(message));
    let collapsed = truncate_toast_message(message);
    assert_eq!(collapsed.lines().count(), COLLAPSED_MAX_LINES);
    assert!(collapsed.ends_with('…'));
}

#[test]
fn expand_toggle_accepts_enter_and_space_without_modifiers() {
    assert!(is_expand_toggle_keystroke(
        &Keystroke::parse("enter").expect("enter should parse")
    ));
    assert!(is_expand_toggle_keystroke(
        &Keystroke::parse("space").expect("space should parse")
    ));
    assert!(!is_expand_toggle_keystroke(
        &Keystroke::parse("ctrl-enter").expect("ctrl-enter should parse")
    ));
}

fn toast(text: impl Into<String>) -> DismissibleToast<()> {
    DismissibleToast::new(text.into(), ToastFlavor::Default)
}

#[test]
fn add_ephemeral_toasts_caps_at_three_newest() {
    App::test((), |mut app| async move {
        let stack = stack_handle(&mut app);
        stack.update(&mut app, |stack, ctx| {
            for index in 0..5 {
                stack.add_ephemeral_toast(toast(format!("toast {index}")), ctx);
            }
        });

        stack.read(&app, |stack, _| {
            assert_eq!(stack.toasts.len(), 3);
            assert_eq!(
                stack
                    .toasts
                    .iter()
                    .map(|toast| toast.dismissible_toast.main_text.as_str())
                    .collect::<Vec<_>>(),
                ["toast 2", "toast 3", "toast 4"]
            );
        });
    });
}

#[test]
fn add_persistent_toasts_caps_at_three_newest() {
    App::test((), |mut app| async move {
        let stack = stack_handle(&mut app);
        stack.update(&mut app, |stack, ctx| {
            for index in 0..5 {
                stack.add_persistent_toast(toast(format!("toast {index}")), ctx);
            }
        });

        stack.read(&app, |stack, _| {
            assert_eq!(stack.toasts.len(), 3);
            assert_eq!(
                stack
                    .toasts
                    .iter()
                    .map(|toast| toast.dismissible_toast.main_text.as_str())
                    .collect::<Vec<_>>(),
                ["toast 2", "toast 3", "toast 4"]
            );
        });
    });
}

#[test]
fn evicted_ephemeral_toast_aborts_timer() {
    App::test((), |mut app| async move {
        let stack = stack_handle(&mut app);
        let evicted_abort_handle = stack.update(&mut app, |stack, ctx| {
            stack.add_ephemeral_toast(toast("oldest"), ctx);
            stack.add_persistent_toast(toast("second"), ctx);
            stack.add_persistent_toast(toast("third"), ctx);
            stack
                .toasts
                .first()
                .and_then(|toast| toast.abort_handle.as_ref())
                .expect("oldest toast should have an ephemeral timer")
                .abort_handle()
        });
        stack.update(&mut app, |stack, ctx| {
            stack.add_persistent_toast(toast("newest"), ctx);
        });
        assert!(evicted_abort_handle.is_aborted());

        stack.read(&app, |stack, _| {
            assert_eq!(stack.toasts.len(), 3);
            assert!(
                stack
                    .toasts
                    .iter()
                    .all(|toast| toast.dismissible_toast.main_text != "oldest")
            );
        });
    });
}

#[test]
fn toggle_message_expanded_flips_per_toast_state() {
    App::test((), |mut app| async move {
        let stack = stack_handle(&mut app);
        let uuid = stack.update(&mut app, |stack, ctx| {
            stack.add_persistent_toast(toast("long message"), ctx);
            stack.toasts[0].uuid
        });

        stack.update(&mut app, |stack, ctx| {
            stack.handle_action(&DismissibleToastAction::ToggleMessageExpanded(uuid), ctx);
        });
        stack.read(&app, |stack, _| assert!(stack.toasts[0].message_expanded));

        stack.update(&mut app, |stack, ctx| {
            stack.handle_action(&DismissibleToastAction::ToggleMessageExpanded(uuid), ctx);
        });
        stack.read(&app, |stack, _| assert!(!stack.toasts[0].message_expanded));
    });
}

#[test]
fn expand_state_is_per_toast() {
    App::test((), |mut app| async move {
        let stack = stack_handle(&mut app);
        let first_uuid = stack.update(&mut app, |stack, ctx| {
            stack.add_persistent_toast(toast("first"), ctx);
            stack.add_persistent_toast(toast("second"), ctx);
            stack.toasts[0].uuid
        });

        stack.update(&mut app, |stack, ctx| {
            stack.handle_action(
                &DismissibleToastAction::ToggleMessageExpanded(first_uuid),
                ctx,
            );
        });
        stack.read(&app, |stack, _| {
            assert!(stack.toasts[0].message_expanded);
            assert!(!stack.toasts[1].message_expanded);
        });
    });
}

#[test]
fn truncation_predicate_is_correct() {
    let short = "short message";
    let long = "x".repeat(COLLAPSED_MAX_CHARS);

    assert!(!toast_message_is_truncated(short));
    assert!(toast_message_is_truncated(&long));
    assert_eq!(
        truncate_toast_message(&long).chars().count(),
        COLLAPSED_MAX_CHARS - 2
    );
    assert!(truncate_toast_message(&long).ends_with('…'));
}

#[test]
fn object_id_dedup_then_cap() {
    App::test((), |mut app| async move {
        let stack = stack_handle(&mut app);
        stack.update(&mut app, |stack, ctx| {
            stack.add_persistent_toast(toast("old x").with_object_id("x".to_string()), ctx);
            stack.add_persistent_toast(toast("new x").with_object_id("x".to_string()), ctx);
            stack.add_persistent_toast(toast("third x").with_object_id("x".to_string()), ctx);
            stack.add_persistent_toast(toast("distinct 1"), ctx);
            stack.add_persistent_toast(toast("distinct 2"), ctx);
            stack.add_persistent_toast(toast("distinct 3"), ctx);
        });

        stack.read(&app, |stack, _| {
            assert_eq!(stack.toasts.len(), 3);
            assert_eq!(
                stack
                    .toasts
                    .iter()
                    .map(|toast| toast.dismissible_toast.main_text.as_str())
                    .collect::<Vec<_>>(),
                ["distinct 1", "distinct 2", "distinct 3"]
            );
        });
    });
}

#[test]
fn manual_dismiss_and_clear_paths_unchanged() {
    App::test((), |mut app| async move {
        let stack = stack_handle(&mut app);
        let uuid = stack.update(&mut app, |stack, ctx| {
            stack.add_persistent_toast(toast("dismiss me"), ctx);
            stack.toasts[0].uuid
        });
        stack.update(&mut app, |stack, ctx| {
            stack.dismiss_toast_by_uuid(&uuid, ctx);
            assert!(stack.toasts.is_empty());
            stack.add_persistent_toast(toast("clear me"), ctx);
            stack.clear_toasts(ctx);
            assert!(stack.toasts.is_empty());
        });
    });
}
