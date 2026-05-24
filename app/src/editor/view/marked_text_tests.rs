use vim::vim::VimMode;
use warp_core::features::FeatureFlag;
use warpui::keymap::Keystroke;
use warpui::platform::WindowStyle;
use warpui::App;

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

#[test]
fn test_korean_ime_backspace_clears_marked_syllable() {
    // Regression test for the Korean IME Backspace flow:
    //
    //   buffer: "가나" + marked "ㄷ"   (i.e. the user just deleted the vowel
    //   from a previously-marked "다")
    //   → pressing Backspace should remove the remaining "ㄷ", leaving "가나".
    //
    // The macOS Korean IME asks for this via the following NSTextInputClient
    // callback sequence inside a single -interpretKeyEvents: pass:
    //   1. setMarkedText("ㄷ")                          // transient redraw
    //   2. insertText("ㄷ")                             // queue the syllable as a commit
    //   3. setMarkedText("")                            // clear marked selection
    //   4. doCommandBySelector(deleteBackward:)         // "cancel that commit"
    //
    // host_view.m's keyDownImpl interprets (4) as "drop the queued commit"
    // and emits only the marked-text changes plus a final unmarkText. Without
    // that suppression the queued "ㄷ" would land as plain text after the
    // marked-text clear, leaving a stray jamo in the buffer that the user
    // would have to delete with an extra Backspace press.
    //
    // This test simulates the resulting editor-view event sequence (steps 1
    // and 3, then an explicit clear, with the commit suppressed) and asserts
    // that the marked jamo disappears with no stray plain-text leftover.
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _guard = FeatureFlag::ImeMarkedText.override_enabled(true);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("가나", Default::default(), ctx);

            editor
                .select_ranges(vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)], ctx)
                .unwrap();

            // Start a marked composition for the trailing "ㄷ".
            editor.set_marked_text("ㄷ", &(1..1), ctx);
            assert_eq!(editor.buffer_text(ctx), "가나ㄷ");
            assert_eq!(editor.selected_text(ctx), "ㄷ");

            // host_view.m emits SetMarkedText("") (step 3) and then unmarkText
            // (translates to ClearMarkedText) after suppressing the IME's
            // queued insertText commit on deleteBackward:.
            editor.set_marked_text("", &(0..0), ctx);
            editor.clear_marked_text(ctx);

            // The marked jamo is gone — and no stray plain-text "ㄷ" was left
            // behind by an unsuppressed commit.
            assert_eq!(editor.buffer_text(ctx), "가나");

            editor
        });
    });
}

#[test]
fn test_split_commit_with_new_marked_text() {
    // Regression test for the Korean (and other CJK) IME split-commit
    // scenario: a single keystroke can both commit the previously-composed
    // syllable AND start a new composition. Typing 'ㅏ' after '간', for
    // example, commits '가' and starts new marked '나' in the same keyDown.
    //
    // Before the host_view.m fix this scenario emitted SetMarkedText('나')
    // first, placing '나' as a buffer selection, and then dispatched
    // TypedCharacters('가') — whose insert path replaces the current
    // selection and so overwrote '나', losing the next character.
    //
    // The fix changes host_view.m to dispatch the workaround sequence
    //   1. ClearMarkedText  (drop the stale marked selection from the buffer)
    //   2. TypedCharacters  (insert the committed text into the now-empty selection)
    //   3. SetMarkedText    (re-apply the new composition)
    // so the buffer ends up as "<commit><new marked>". This test simulates
    // that sequence at the editor view layer and asserts the resulting
    // buffer and selection state.
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _guard = FeatureFlag::ImeMarkedText.override_enabled(true);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("", Default::default(), ctx);

            // Existing composition state: marked '간' held as a buffer selection.
            editor.set_marked_text("간", &(1..1), ctx);
            assert_eq!(editor.selected_text(ctx), "간");

            // User types 'ㅏ'. host_view.m emits the workaround sequence.
            editor.clear_marked_text(ctx);
            editor.user_insert("가", ctx);
            editor.set_marked_text("나", &(1..1), ctx);

            // Buffer is "가" (committed) followed by "나" (marked, selected).
            assert_eq!(editor.buffer_text(ctx), "가나");
            assert_eq!(editor.selected_text(ctx), "나");

            editor
        });
    });
}
