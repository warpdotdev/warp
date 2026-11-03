use vim::vim::VimMode;
use warp_core::features::FeatureFlag;
use warpui::App;
use warpui::keymap::Keystroke;
use warpui::platform::WindowStyle;

use super::initialize_app;
use crate::editor::{DisplayPoint, EditorOptions, EditorView};

#[test]
fn test_set_marked_text() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _guard = FeatureFlag::ImeMarkedText.override_enabled(true);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("", Default::default(), ctx);

            // Simulate typing in "nihao" into the IME and then selecting "你好" as the candidate.
            editor.set_marked_text("nihao", &(5..5), ctx);
            assert_eq!(editor.selected_text(ctx), "nihao");
            editor.ime_commit("你好", ctx);
            assert_eq!(editor.buffer_text(ctx), "你好");

            editor.user_insert(", I am Teddy ", ctx);
            assert_eq!(editor.buffer_text(ctx), "你好, I am Teddy ".to_owned());

            // Simulate typing in "xiong" into the IME and selecting "熊" as the candidate.
            editor.set_marked_text("xiong", &(5..5), ctx);
            assert_eq!(editor.selected_text(ctx), "xiong");
            editor.ime_commit("熊", ctx);
            assert_eq!(editor.buffer_text(ctx), "你好, I am Teddy 熊".to_owned());

            editor
        });
    });
}

#[test]
fn test_set_marked_text_multiple_empty_selections() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _guard = FeatureFlag::ImeMarkedText.override_enabled(true);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(" is ", Default::default(), ctx);

            // Set two cursors: one at the beginning and one at the end.
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0),
                        DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4),
                    ],
                    ctx,
                )
                .unwrap();
            assert_eq!(editor.selections(ctx).len(), 2);

            // Simulate typing in "pyaar" into the IME and then selecting "प्यार" as the candidate.
            editor.set_marked_text("pyaar", &(5..5), ctx);
            for selected_text in editor.selected_text_strings(ctx).iter() {
                assert_eq!(selected_text, "pyaar");
            }
            editor.ime_commit("प्यार", ctx);
            assert_eq!(editor.buffer_text(ctx), "प्यार is प्यार".to_owned());

            editor
        });
    });
}

#[test]
fn test_set_marked_text_multiple_nonempty_selections() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _guard = FeatureFlag::ImeMarkedText.override_enabled(true);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("love is love", Default::default(), ctx);

            // Select both instances of "love" in the buffer text.
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 0)..DisplayPoint::new(0, 4),
                        DisplayPoint::new(0, 8)..DisplayPoint::new(0, 12),
                    ],
                    ctx,
                )
                .unwrap();
            assert_eq!(editor.selections(ctx).len(), 2);

            // Simulate typing in "pyaar" into the IME and then selecting "प्यार" as the candidate.
            editor.set_marked_text("pyaar", &(5..5), ctx);
            for selected_text in editor.selected_text_strings(ctx).iter() {
                assert_eq!(selected_text, "pyaar");
            }
            editor.ime_commit("प्यार", ctx);
            assert_eq!(editor.buffer_text(ctx), "प्यार is प्यार".to_owned());

            editor
        });
    });
}

#[test]
fn test_set_marked_text_vim_normal_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _guard = FeatureFlag::ImeMarkedText.override_enabled(true);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let editor_options = EditorOptions {
                supports_vim_mode: true,
                ..Default::default()
            };
            let mut editor = EditorView::new_with_base_text(
                "This text should remain unchanged",
                editor_options,
                ctx,
            );

            editor
                .select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();

            // Set vim to normal mode.
            editor.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));

            // Simulate typing in "Om Shanti Om" into the IME and then selecting "ॐ शांति ॐ" as the candidate.
            // Since we're in normal mode, we don't expect the text to change at all.
            editor.set_marked_text("om shanti om", &(10..10), ctx);
            assert_eq!(editor.selected_text(ctx), "");
            assert_eq!(
                editor.buffer_text(ctx),
                "This text should remain unchanged".to_owned()
            );
            editor.ime_commit("ॐ शांति ॐ", ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "This text should remain unchanged".to_owned()
            );

            editor
        });
    });
}

#[test]
fn test_set_marked_text_vim_insert_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _guard = FeatureFlag::ImeMarkedText.override_enabled(true);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let editor_options = EditorOptions {
                supports_vim_mode: true,
                ..Default::default()
            };
            let mut editor = EditorView::new_with_base_text(
                " is the best Bollywood movie ever created.",
                editor_options,
                ctx,
            );

            editor
                .select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();

            // Set vim to normal mode.
            assert_eq!(editor.vim_mode(ctx), Some(VimMode::Insert));

            // Simulate typing in "Om Shanti Om" into the IME and then selecting "ॐ शांति ॐ" as the candidate.
            // Since we're in insert mode, we don't expect the text to be inserted.
            editor.set_marked_text("om shanti om", &(10..10), ctx);
            assert_eq!(editor.selected_text(ctx), "om shanti om");
            assert_eq!(
                editor.buffer_text(ctx),
                "om shanti om is the best Bollywood movie ever created.".to_owned()
            );
            editor.ime_commit("ॐ शांति ॐ", ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "ॐ शांति ॐ is the best Bollywood movie ever created.".to_owned()
            );

            editor
        });
    });
}

/// Regression test for #13631: the macOS press-and-hold accent popup commits the base
/// character first and then commits the accented character as a replacement for it. The
/// replacement must land as a single grapheme, not appended after the base character.
#[test]
fn test_replace_preceding_characters_accent_popup() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            // Typed rather than seeded as base text, so the caret ends up after the
            // text the way it does when a user is typing a command.
            let mut editor = EditorView::new_with_base_text("", Default::default(), ctx);
            editor.user_insert("ech", ctx);

            // Holding `o` commits a literal `o` through the ordinary typed-character path.
            editor.user_insert("o", ctx);
            assert_eq!(editor.buffer_text(ctx), "echo");

            // Picking `ò` from the popup replaces that `o` rather than following it.
            editor.replace_preceding_characters("ò", ctx);
            assert_eq!(editor.buffer_text(ctx), "echò");

            editor
        });
    });
}

/// The replacement costs no extra undo step: undoing an accented word behaves exactly
/// like undoing the same word typed directly, rather than first uncovering the
/// intermediate `echo` state that only existed because of how the popup commits.
///
/// The directly-typed editor is the control — without it, a single `undo` reaching an
/// empty buffer would prove nothing, since that is also what typing coalescing alone
/// would produce.
#[test]
fn test_replace_preceding_characters_costs_no_extra_undo_step() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut control = EditorView::new_with_base_text("", Default::default(), ctx);
            control.user_insert("ech", ctx);
            control.user_insert("ò", ctx);
            assert_eq!(control.buffer_text(ctx), "echò");
            control.undo(ctx);
            let control_after_one_undo = control.buffer_text(ctx);
            assert_eq!(control_after_one_undo, "");

            control
        });

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("", Default::default(), ctx);
            editor.user_insert("ech", ctx);
            editor.user_insert("o", ctx);
            editor.replace_preceding_characters("ò", ctx);
            assert_eq!(editor.buffer_text(ctx), "echò");

            // One undo, same as the control. Two would mean the popup commit left an
            // extra edit behind, and the second undo would surface `echo`.
            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "");

            editor
        });
    });
}

/// At the start of the buffer there is nothing to replace, so the commit degrades to a
/// plain insert instead of eating a character that belongs to the user.
#[test]
fn test_replace_preceding_characters_at_buffer_start_inserts() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("", Default::default(), ctx);

            editor.replace_preceding_characters("ò", ctx);
            assert_eq!(editor.buffer_text(ctx), "ò");

            editor
        });
    });
}
