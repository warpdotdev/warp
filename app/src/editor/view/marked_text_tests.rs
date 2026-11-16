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
