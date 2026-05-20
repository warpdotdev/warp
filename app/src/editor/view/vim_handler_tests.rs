use super::*;
use crate::editor::EditorView;
use itertools::Itertools;
use std::collections::HashSet;
use unindent::Unindent;
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

/// Helper function for testing vim mode commands.
/// This creates an editor with the given content and enters Vim Normal mode,
/// as opposed to the default Insert mode.
fn add_editor_vim_normal_mode(buffer_content: &str, app: &mut App) -> ViewHandle<EditorView> {
    let (_, editor) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        let options = EditorOptions {
            supports_vim_mode: true,
            ..Default::default()
        };
        let base_text = buffer_content.unindent();
        let mut editor = EditorView::new_with_base_text(base_text.as_str(), options, ctx);
        editor
            .select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
            .unwrap();
        editor.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        assert_eq!(editor.vim_mode(ctx), Some(VimMode::Normal));
        editor
    });
    editor
}

#[test]
fn test_vim_ctrl_c_normal() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("some text", &mut app);

        // Ctrl-c in normal mode should clear the buffer
        // and stay in normal mode.
        editor.update(&mut app, |view, view_ctx| {
            view.handle_action(&EditorAction::CtrlC, view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_ctrl_c_insert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("", &mut app);

        // Insert some text.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("i", view_ctx);
            view.vim_user_insert("add text", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "add text");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert))
        });

        // Ctrl-c in insert mode should clear the buffer
        // and stay in insert mode.
        //
        // Why not switch to normal mode?
        // Terminal vi mode implementations typically use insert mode as the default mode,
        // because inserting text is generally more common in the terminal than editing text.
        // To be consistent with those, we're not changing to normal mode on ctrl-c.
        editor.update(&mut app, |view, view_ctx| {
            view.handle_action(&EditorAction::CtrlC, view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert))
        });
    });
}

#[test]
fn test_vim_ctrl_c_replace() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("some text", &mut app);

        // Ctrl-c in replace mode should cancel the replacement.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("r", view_ctx);
            view.handle_action(&EditorAction::CtrlC, view_ctx);
            view.vim_user_insert("~", view_ctx);
        });

        // Instead of replacing "s" with "~", the "~" should be interpreted
        // on its own.
        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "Some text");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_ctrl_c_visual() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("some text", &mut app);

        // Ctrl-c in replace mode should exit visual mode.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("vw", view_ctx);
            view.handle_action(&EditorAction::CtrlC, view_ctx);
            view.vim_user_insert("d", view_ctx);
        });

        // Instead of deleting the word, the "d" should be a pending operator.
        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "some text");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
            assert_eq!(view.vim_model.as_ref(app_ctx).state().showcmd, "d");
        });
    });
}

#[test]
fn test_vim_enter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("letters", &mut app);

        // Go into insert mode, then enter.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("A", view_ctx);
            view.enter(view_ctx);
        });

        // Vim FSA should stay in insert mode.
        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert))
        });

        // Go into normal mode, then enter.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            view.enter(view_ctx);
        });

        // Vim FSA should switch back to insert mode.
        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert))
        });
    });
}

#[test]
fn test_vim_number_repeat_action() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "line 1
            line 2
            line 3
            line 4",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("3dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), "line 4");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_number_repeat_word_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "abc de f
            gh ijkl m nop"
                .unindent()
                .as_str(),
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("3w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        // Try to go past the end of the buffer.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("4w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 12)..DisplayPoint::new(1, 12)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("2b", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 8)..DisplayPoint::new(1, 8)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("2b2e", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
        });
    });
}

#[test]
fn test_vim_number_repeat_line_motion() {
    // TODO: make sure to test line motions where number-repeat does nothing

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aa
            bbbbb
            ccc
            dddd"
                .unindent()
                .as_str(),
            &mut app,
        );

        // Lay out the window so that up and down motions work.
        // Layout code starts here.
        let window_id = app.read(|ctx| editor.window_id(ctx));
        let mut presenter = warpui::presenter::Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = warpui::WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(
                pathfinder_geometry::vector::vec2f(1000., 1000.),
                1.,
                None,
                ctx,
            );
        });
        // Layout code ends here.

        // #$
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("2$", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("3$", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(3, 3)..DisplayPoint::new(3, 3)]
            );
        });

        // #^ ignores the repetition count.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("3^", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)]
            );
        });
    });
}

#[test]
fn test_vim_number_repeat_character_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aa
            bbbbb
            ccc
            dddd"
                .unindent()
                .as_str(),
            &mut app,
        );

        // Lay out the window so that up and down motions work.
        // Layout code starts here.
        let window_id = app.read(|ctx| editor.window_id(ctx));
        let mut presenter = warpui::presenter::Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = warpui::WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(
                pathfinder_geometry::vector::vec2f(1000., 1000.),
                1.,
                None,
                ctx,
            );
        });
        // Layout code ends here.

        // #j
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("3j", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)]
            );
        });

        // #l
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("3l", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(3, 3)..DisplayPoint::new(3, 3)]
            );
        });

        // #k
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("2k", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
        });

        // #h (and attempt to go past the start of the line)
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("4h", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });
    });
}

#[test]
fn test_vim_number_repeat_op_word_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "line 1
            line 2
            line 3
            line 4",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("d3w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "line 1
                3
                line 4"
                    .unindent()
                    .as_str()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c2w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), "\n3\nline 4");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert))
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("replacement", view_ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx)
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "replacement
                 3
                 line 4"
                    .unindent()
                    .as_str()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });

        // attempt to delete more words than there are after the cursor
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("d3w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "replacement\n3\n");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_number_repeat_op_line_object() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "line 1
            line 2
            line 3 is longer
            line 4 is long
            line 5
            line 6
            line 7 too",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("d2d", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "line 1
                line 4 is long
                line 5
                line 6
                line 7 too"
                    .unindent()
                    .as_str()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("c3c", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "line 1

                line 7 too"
                    .unindent()
                    .as_str()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert))
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("replacement", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 11)..DisplayPoint::new(1, 11)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "line 1
                replacement
                line 7 too"
                    .unindent()
                    .as_str()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert))
        });
    });
}

#[test]
fn test_vim_number_repeat_op_line_motions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "The quick brown fox
            jumped over
            the lazy dog.",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c2$", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)]
            );
            assert_eq!(view.buffer_text(app_ctx), "The quick \nthe lazy dog.");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("bear\nhigh-fived", view_ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 8)..DisplayPoint::new(1, 8)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "The quick bear
                high-fived
                the lazy dog."
                    .unindent()
                    .as_str()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("d2$", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "The quick
                the lazy dog."
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_number_repeat_character_motions_right() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "The quick brown fox
            jumped",
            &mut app,
        );

        // Replace right
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c5l", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)]
            );
            assert_eq!(view.buffer_text(app_ctx), "The quick  fox\njumped");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("purple", view_ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            view.vim_user_insert("l", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 16)..DisplayPoint::new(0, 16)]
            );
            assert_eq!(view.buffer_text(app_ctx), "The quick purple fox\njumped");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });

        // Delete right, attempting to delete past the end of the line
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("d5l", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The quick purple\njumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_number_repeat_character_motions_left() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "The quick
            brown fox jumped",
            &mut app,
        );

        // Replace left
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 9)..DisplayPoint::new(1, 9)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c3h", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The quick\nbrown  jumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("bear", view_ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The quick\nbrown bear jumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 9)..DisplayPoint::new(1, 9)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });

        // Delete left, attempting to delete past the start of the line
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("d12h", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The quick\nr jumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_number_repeat_character_motions_down() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "The
            quick
            brown
            fox
            jumped",
            &mut app,
        );

        // Replace down
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c2j", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), "The\n\njumped");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("cat", view_ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The\ncat\njumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });

        // Delete down, attempting to delete further than the last line
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("d3j", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_number_repeat_character_motions_up() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "The
            quick
            brown
            fox
            jumped",
            &mut app,
        );

        // Replace up
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(3, 1)..DisplayPoint::new(3, 1)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c2k", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The\n\njumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("squirrel", view_ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "The\nsquirrel\njumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });

        // Delete up, attempting to delete further than the first line
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("d4k", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "jumped");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_number_repeat_op_combination() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "line 1
            line 2
            line 3
            line 4
            line 5",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("2d3w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "line 1
                line 5"
                    .unindent()
                    .as_str(),
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });

        editor.update(&mut app, |view, view_ctx| {
            view.set_buffer_text(
                "line 1
                line 2
                line 3
                line 4
                line 5"
                    .unindent()
                    .as_str(),
                view_ctx,
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("3d2w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "line 1
                line 5"
                    .unindent()
                    .as_str(),
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_number_repeat_yank_paste_linewise() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aaa
            b b b
            cc"
            .unindent()
            .as_str(),
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("y2y", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "aaa
                b b b
                cc"
                .unindent()
                .as_str()
            );
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("3P", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "aaa
                b b b
                cc
                b b b
                cc
                b b b
                cc
                b b b
                cc"
                .unindent()
                .as_str()
            );
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });
    });
}

#[test]
fn test_vim_number_repeat_yank_paste_charwise() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("a bc d efg h", &mut app);

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("y2w", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "a bc d efg h");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("3P", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "a bc d bc d bc d bc d efg h");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 16)..DisplayPoint::new(0, 16)]
            );
        });
    });
}

#[test]
fn test_vim_delete_lines() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "abcd
                efg
                hijk

                lmnop
                qrs",
            &mut app,
        );

        // delete the first line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "efg\nhijk\n\nlmnop\nqrs");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        // delete the last line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(4, 1)..DisplayPoint::new(4, 1)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "efg\nhijk\n\nlmnop");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)]
            );
        });

        // delete an empty line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "efg\nhijk\nlmnop");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)]
            );
        });

        // delete the remaining lines
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "efg\nhijk");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "efg");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        // attempt to delete in an empty buffer
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dd", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        Ok(())
    })
}

#[test]
fn test_vim_delete_lines_d0() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcd\n  efgh\nijkl", &mut app);

        // delete the first half of the middle line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("d0", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), "abcd\ngh\nijkl");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_delete_lines_d_caret() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcd\n  efgh\nijkl", &mut app);

        // delete the first half of the middle line, but not whitespace
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("d^", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2)]
            );
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  gh\nijkl");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_delete_lines_d_dollar() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcd\n  efgh\nijkl", &mut app);

        // delete the second half of the middle line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("d$", view_ctx)
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  ef\nijkl");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });

        // delete the second half of the bottom line using the D alias
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("D", view_ctx)
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  ef\nij");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 1)..DisplayPoint::new(2, 1)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal))
        });
    });
}

#[test]
fn test_vim_replace_simple() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcdef", &mut app);

        // Replace first character
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("r", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Replace));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), "abcdef");
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("g", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), "gbcdef");
        });

        // Replace last character
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("r", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Replace));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
            assert_eq!(view.buffer_text(app_ctx), "gbcdef");
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("h", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
            assert_eq!(view.buffer_text(app_ctx), "gbcdeh");
        });
    });
}

#[test]
fn test_vim_replace_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcdef\nghijk", &mut app);

        // Replace first three characters.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("3r", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcdef\nghijk");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Replace));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("l", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "llldef\nghijk");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
        });
    });
}

/// Replace last characters in the line.
/// Note that if count > number of characters left in the line,
/// the whole operation is cancelled, and no characters are replaced.
#[test]
fn test_vim_replace_number_repeat_end_of_line() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcdef\nghijk", &mut app);

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("3r", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcdef\nghijk");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Replace));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("m", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcdef\nghijk");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });
    });
}

#[test]
fn test_vim_delete_char_x() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcde", &mut app);

        // delete first character in the buffer
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("x", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "bcde");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
            );
        });

        // delete last character in the buffer
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("x", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "bcd");
            // In vim mode, the cursor should move back so that it's covering
            // the last character in the buffer
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)],
            );
        });

        // delete last character in the buffer again
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("x", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "bc");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)],
            );
        });
    });
}

#[test]
fn test_vim_delete_char_motion_sideways() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo hello", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w", ctx);
            view.vim_user_insert("dl", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo ello");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)],
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dh", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echoello");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("$", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)],
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dl", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echoell");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)],
            );
        });
    });
}

#[test]
fn test_vim_delete_char_motion_up() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo 1", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dk", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();
            view.vim_user_insert("dk", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 2");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 3)..Point::new(1, 3)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dk", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 5)..Point::new(1, 5)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dk", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(2, 2)..Point::new(2, 2)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dk", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2\necho 3\necho 4", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(2, 0)..Point::new(2, 0)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dk", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\necho 4");
        });
    });
}

#[test]
fn test_vim_delete_char_motion_down() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo 1", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();
            view.vim_user_insert("dj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 1)..Point::new(1, 1)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 5)..Point::new(1, 5)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx).trim(), "echo 1");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(0, 2)..Point::new(0, 2)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo 1\necho 2\necho 3\necho 4", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 0)..Point::new(1, 0)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("dj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\necho 4");
        });
    });
}

#[test]
fn test_vim_delete_char_space() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo hi
            echo there",
            &mut app,
        );

        // Deleting forward at the start of the buffer works.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d ", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "cho hi\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
            );
        });

        // Move to the end of the line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("$", ctx);
        });

        // Deleting forward at the end of the line deletes the last character.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d ", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "cho h\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
            );
        });

        // Deleting forward wraps around to the next line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3d ", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "cho cho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
            );
        });
    });
}

#[test]
fn test_vim_delete_char_backspace() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo hi
            echo there",
            &mut app,
        );

        // Deleting backward at the start of the buffer does nothing.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo hi\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
            );
        });

        // Move to the end of the line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("$", ctx);
        });

        // Deleting backward at the end of the line removes the second-to-last character.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo i\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)],
            );
        });

        // Move to the next line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w", ctx);
        });

        // Deleting backward wraps around to the previous line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3d", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echoecho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
            );
        });
    });
}

#[test]
fn test_vim_delete_word_dw() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "abc de.f g-hi-j
            kL",
            &mut app,
        );

        // dw
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dw", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "de.f g-hi-j
                    kL"
                .unindent()
            );
        });

        // dW
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dW", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                "g-hi-j
                    kL"
                .unindent()
            );
        });

        // dW again does not cross a line break
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dW", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "\nkL");
        });
    });
}

#[test]
fn test_vim_delete_word_de() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "g-hi-j kL
            mNop.qr st u v wX/yZ",
            &mut app,
        );

        // dE
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dE", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.buffer_text(app_ctx),
                " kL
            mNop.qr st u v wX/yZ"
                    .unindent()
            );
        });

        // de
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("de", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "\nmNop.qr st u v wX/yZ");
        });

        // dE across a line break
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dE", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), " st u v wX/yZ");
        });
    });
}

#[test]
fn test_vim_delete_word_db() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("mNop.qr st u v wX/yZ", &mut app);

        // move the cursor to the end of the line
        editor.update(&mut app, |view, view_ctx| {
            view.move_to_line_end(view_ctx);
            // account for vim cursor position quirk
            view.move_left(/* stop at line start */ true, view_ctx);
        });
        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 19)..DisplayPoint::new(0, 19)],
            );
        });

        // dB
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dB", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "mNop.qr st u v Z");
        });

        // db
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("db", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "mNop.qr st u Z");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });

        // dw, to get rid of the last "Z"
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dw", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "mNop.qr st u ");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
        });

        // db, twice more
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("db", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "mNop.qr st  ");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 11)..DisplayPoint::new(0, 11)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("db", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "mNop.qr  ");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
        });

        // dB to remove symbol
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dB", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), " ");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });
    });
}

#[test]
fn test_vim_delete_word_dge() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo hello-hi warp-dev", &mut app);

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 20)..DisplayPoint::new(0, 20)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("dge", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo hello-hi warpev");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 18)..DisplayPoint::new(0, 18)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dge", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo hello-hev");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dgE", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echev");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });
    });
}

#[test]
fn test_vim_delete_word_empty() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("", &mut app);

        // try each operation on an empty buffer
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dw", view_ctx);
            view.vim_user_insert("dW", view_ctx);
            view.vim_user_insert("de", view_ctx);
            view.vim_user_insert("dE", view_ctx);
            view.vim_user_insert("db", view_ctx);
            view.vim_user_insert("dB", view_ctx);
            view.vim_user_insert("dge", view_ctx);
            view.vim_user_insert("dgE", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "");
        });
    });
}

#[test]
fn test_vim_dw_newline_quirks() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo foo  \necho bar", &mut app);

        // A "w" motion can traverse the newline.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("eelw", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        // An "e" motion can also traverse the newline.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("ggeee", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
        });

        // An "e" motion with an operator can traverse the newline.
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("gelde", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo foo bar");
        });

        // However, a "w" motion with an operator *cannot* traverse the newline.
        editor.update(&mut app, |view, view_ctx| {
            view.set_buffer_text("echo foo  \necho bar", view_ctx);
            view.vim_user_insert("ggeeldw", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo foo\necho bar");
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("dw", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo fo\necho bar");
        });

        // Including a count allows it to traverse the newline
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("2dw", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo fbar");
        });
    });
}

#[test]
fn test_vim_db_newline_quirks() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo foo\necho bar", &mut app);

        // A "b" motion can traverse the newline.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Gb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        // "db" will traverse but not delete the newline.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Gdb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo \necho bar");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        // It will delete the newline if the cursor didn't start on column 0.
        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo foo\n echo bar", ctx);
            view.vim_user_insert("Gl", ctx);
            view.vim_user_insert("db", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo echo bar");
        });

        // It will also delete the newlines if the line above is empty
        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo foo\n\necho bar", ctx);
            view.vim_user_insert("Gdb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo echo bar");
        });

        // It will also delete the newline if there is a count > 1.
        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("echo foo\necho bar", ctx);
            view.vim_user_insert("G2db", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo bar");
        });
    });
}

#[test]
fn test_vim_jump_to_end_and_beginning() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "abc
            def
            ghi",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("G", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("gg", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });
    });
}

#[test]
fn test_vim_begin_line_below() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "abc
            def
            ghi",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("o", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "abc
                def

                ghi"
                .unindent()
            );
        });
    });
}

#[test]
fn test_vim_begin_line_above() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "abcdef
                    ghijkl
                    mnopqr",
            &mut app,
        );

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)],
                view_ctx,
            )
            .unwrap();
        });

        // Lay out the window so that up and down motions work.
        // Layout code starts here.
        let window_id = app.read(|ctx| editor.window_id(ctx));
        let mut presenter = warpui::presenter::Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = warpui::WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(
                pathfinder_geometry::vector::vec2f(1000., 1000.),
                1.,
                None,
                ctx,
            );
        });
        // Layout code ends here.

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("O", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "abcdef

                ghijkl
                mnopqr"
                    .unindent()
            );
        });
    });
}

#[test]
fn test_vim_substitute_char() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcdef", &mut app);

        // Substitute at start of line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("s", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(view.buffer_text(app_ctx), "bcdef");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("xyz", view_ctx)
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "xyzbcdef");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
        });

        // Substitute at end of line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("s", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(view.buffer_text(app_ctx), "xyzbcde");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("jkl", view_ctx)
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "xyzbcdejkl");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)]
            );
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });
    });
}

#[test]
fn test_vim_substitute_line() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aaa
            bbb
            ccc",
            &mut app,
        );

        // substitute first line
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("S", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "\n
                bbb
                ccc"
                .unindent()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("new text", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "new text
                bbb
                ccc"
                .unindent()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });

        // substitute last line
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("S", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "new text
                bbb
                \n"
                .unindent()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("replacement", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 11)..DisplayPoint::new(2, 11)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "new text
                bbb
                replacement"
                    .unindent()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 10)..DisplayPoint::new(2, 10)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_change_line_cc() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aaa
            bbb
            ccc",
            &mut app,
        );
        //
        // start typing out the first "c"
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("c", view_ctx);
        });

        // no changes should have happened at this point
        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "aaa
                bbb
                ccc"
                .unindent()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });

        // finish typing out "cc"
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("c", view_ctx);
        });

        // now the mode and content should change
        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "\n
                bbb
                ccc"
                .unindent()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("new text", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
            assert_eq!(
                view.buffer_text(app_ctx),
                "new text
                bbb
                ccc"
                .unindent()
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_change_line_c0() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcd\n  efgh\nijkl", &mut app);

        // c0
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c0", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), "abcd\nfgh\nijkl");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });
    });
}

#[test]
fn test_vim_change_line_c_caret() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcd\n  efgh\nijkl", &mut app);

        // c^
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c^", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2)]
            );
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  gh\nijkl");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });
    });
}

#[test]
fn test_vim_change_line_c_dollar() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("abcd\n  efgh\nijkl", &mut app);

        // c$
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("c$", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  e\nijkl");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
        });

        // add some text to replace it
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("replacement", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  ereplacement\nijkl");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(1, 14)..DisplayPoint::new(1, 14)]
            );
        });

        // test the C alias that does the same thing
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 3)..DisplayPoint::new(2, 3)],
                view_ctx,
            )
            .unwrap();
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            view.vim_user_insert("C", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  ereplacement\nij");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)]
            );
        });

        // add some text to replace that too
        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("another", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "abcd\n  ereplacement\nijanother");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(2, 9)..DisplayPoint::new(2, 9)]
            );
        });
    });
}

#[test]
fn test_vim_change_word_cw() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aaa b-B_b ccc", &mut app);

        // cw
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("cw", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), " b-B_b ccc");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("d", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
            assert_eq!(view.buffer_text(app_ctx), "d b-B_b ccc");
        });

        // cW
        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            assert_eq!(view.vim_mode(view_ctx), Some(VimMode::Normal));
            view.vim_user_insert("w", view_ctx);
            view.vim_user_insert("cW", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
            assert_eq!(view.buffer_text(app_ctx), "d  ccc");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("e", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
            assert_eq!(view.buffer_text(app_ctx), "d e ccc");
        });
    });
}

#[test]
fn test_vim_change_word_ce() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aaa b-B_b", &mut app);

        // ce
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("ce", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.buffer_text(app_ctx), " b-B_b");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("d", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
            assert_eq!(view.buffer_text(app_ctx), "d b-B_b");
        });

        // cE
        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            assert_eq!(view.vim_mode(view_ctx), Some(VimMode::Normal));
            view.vim_user_insert("w", view_ctx);
            view.vim_user_insert("cE", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
            assert_eq!(view.buffer_text(app_ctx), "d ");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("e", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
            assert_eq!(view.buffer_text(app_ctx), "d e");
        });
    });
}

#[test]
fn test_vim_change_word_cb() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aaa b-B_b ccc", &mut app);

        // cb
        editor.update(&mut app, |view, view_ctx| {
            // move to end of buffer
            view.select_ranges(
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("cb", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)]
            );
            assert_eq!(view.buffer_text(app_ctx), "aaa b-B_b c");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("d", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 11)..DisplayPoint::new(0, 11)]
            );
            assert_eq!(view.buffer_text(app_ctx), "aaa b-B_b dc");
        });

        // cB
        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            assert_eq!(view.vim_mode(view_ctx), Some(VimMode::Normal));
            view.vim_user_insert("cB", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
            assert_eq!(view.buffer_text(app_ctx), "aaa dc");
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_user_insert("e", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
            assert_eq!(view.buffer_text(app_ctx), "aaa edc");
        });
    });
}

#[test]
fn test_vim_change_word_cge() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo hello-hi warp-dev", &mut app);

        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 20)..DisplayPoint::new(0, 20)],
                view_ctx,
            )
            .unwrap();
            view.vim_user_insert("cge", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo hello-hi warpev");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 18)..DisplayPoint::new(0, 18)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            view.vim_user_insert("cge", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echo hello-hpev");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });

        editor.update(&mut app, |view, view_ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), view_ctx);
            view.vim_user_insert("cgE", view_ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "echhpev");
            assert_eq!(
                view.selected_ranges(app_ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
            assert_eq!(view.vim_mode(app_ctx), Some(VimMode::Insert));
        });
    });
}

#[test]
fn test_vim_change_char_motion_sideways() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo hello", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w", ctx);
            view.vim_user_insert("cl", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo ello");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("y", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo yello");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.vim_user_insert("ch", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echoyello");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(".", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo.yello");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.vim_user_insert("$", ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("cl", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo.yell");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)],
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ing", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo.yelling");
        });
    });
}

#[test]
fn test_vim_change_char_motion_up() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo 1", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ck", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 1", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "insert 1");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();
            view.vim_user_insert("ck", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "\necho 2");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 2", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "insert 2\necho 2");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 1)..Point::new(1, 1)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("ck", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 3", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "insert 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 5)..Point::new(1, 5)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("ck", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "\necho 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 4", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "insert 4\necho 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(2, 2)..Point::new(2, 2)], ctx)
                    .unwrap();
            });
        });

        editor.update(&mut app, |view, ctx| {
            // Separate update to ensure the DisplayMap is up-to-date
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)]
            );
            view.vim_user_insert("ck", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\n");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 5", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\ninsert 5");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2\necho 3\necho 4", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(2, 0)..Point::new(2, 0)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("ck", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\n\necho 4");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 6", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\ninsert 6\necho 4");
        });
    });
}

#[test]
fn test_vim_change_char_motion_down() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo 1", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("cj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 1", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "insert 1");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();
            view.vim_user_insert("cj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 2", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "insert 2");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 1)..Point::new(1, 1)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("cj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\n");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 3", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\ninsert 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 5)..Point::new(1, 5)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("cj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\n");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 4", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\ninsert 4");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2\necho 3", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(0, 2)..Point::new(0, 2)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("cj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "\necho 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 5", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "insert 5\necho 3");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.set_buffer_text("echo 1\necho 2\necho 3\necho 4", ctx);
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model
                    .select_ranges_by_offset(vec![Point::new(1, 0)..Point::new(1, 0)], ctx)
                    .unwrap();
            });
            view.vim_user_insert("cj", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\n\necho 4");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("insert 6", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo 1\ninsert 6\necho 4");
        });
    });
}

#[test]
fn test_vim_change_char_space() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo hello
            echo there",
            &mut app,
        );

        // Changing forword a single character works.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w", ctx);
            view.vim_user_insert("c ", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo ello\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("y", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo yello\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        // Move to the end of the line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("e", ctx);
        });

        // Changing forward should wrap around to the next line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3c ", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo yellcho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
        });
    });
}

#[test]
fn test_vim_change_char_backspace() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo hello
            echo there",
            &mut app,
        );

        // Changing backward at the start of a line simply enters insert mode.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("c", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo hello\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            assert_eq!(view.vim_mode(ctx).unwrap(), VimMode::Insert);
        });

        // Move to the end of the line.
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.vim_user_insert("$", ctx);
        });

        // Changing backward at the end of a line works.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("c", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo helo\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("i", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo helio\necho there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
        });

        // Move to the next line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ee", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
        });

        // Changing backward should wrap around to the previous line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("6c", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo helo there");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
        });
    });
}

#[test]
fn test_vim_char_append() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let view = add_editor_vim_normal_mode(
            "aaa bbb ccc
            ddd eee fff",
            &mut app,
        );

        // Append after first character
        view.update(&mut app, |view, ctx| {
            // 'a' into insert mode
            view.vim_user_insert("a", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
            // insert a character after the first "a"
            assert_eq!(
                view.buffer_text(ctx),
                "aaa bbb ccc
                ddd eee fff"
                    .unindent()
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_user_insert("h", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "ahaa bbb ccc
                ddd eee fff"
                    .unindent()
            );
        });

        view.update(&mut app, |view, ctx| {
            // esc into normal mode
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        view.read(&app, |view, ctx| {
            // cursor should be over the added letter "h"
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
        });

        // Append after last character in the line
        view.update(&mut app, |view, ctx| {
            // place the cursor at the end of the line
            view.select_ranges(
                vec![DisplayPoint::new(0, 11)..DisplayPoint::new(0, 11)],
                ctx,
            )
            .unwrap();

            // 'a' into insert mode
            view.vim_user_insert("a", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
            // insert a character after the last "c"
            assert_eq!(
                view.buffer_text(ctx),
                "ahaa bbb ccc
                ddd eee fff"
                    .unindent()
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_user_insert("i", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "ahaa bbb ccci
                ddd eee fff"
                    .unindent()
            );
        });

        view.update(&mut app, |view, ctx| {
            // esc into normal mode
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        view.read(&app, |view, ctx| {
            // cursor should be over the added letter "i"
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
        });

        // Append after last character in the buffer
        view.update(&mut app, |view, ctx| {
            // place the cursor at the end of the buffer
            view.select_ranges(
                vec![DisplayPoint::new(1, 10)..DisplayPoint::new(1, 10)],
                ctx,
            )
            .unwrap();

            // 'a' into insert mode
            view.vim_user_insert("a", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 11)..DisplayPoint::new(1, 11)]
            );
            // insert a character after the last "g"
            assert_eq!(
                view.buffer_text(ctx),
                "ahaa bbb ccci
                ddd eee fff"
                    .unindent()
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_user_insert("j", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "ahaa bbb ccci
                ddd eee fffj"
                    .unindent()
            );
        });

        view.update(&mut app, |view, ctx| {
            // esc into normal mode
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        view.read(&app, |view, ctx| {
            // cursor should be over the added letter "h"
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 11)..DisplayPoint::new(1, 11)]
            );
        });

        Ok(())
    })
}

#[test]
fn test_vim_line_prepend() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let view = add_editor_vim_normal_mode(
            "   abcd
            efg",
            &mut app,
        );

        // Prepend to first line
        view.update(&mut app, |view, ctx| {
            // 'I' into insert mode
            view.vim_user_insert("I", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_user_insert("sample text", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "   sample textabcd
                efg"
                .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx)
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });

        // Prepend to last line
        view.update(&mut app, |view, ctx| {
            // place the cursor at the start of the last line
            view.select_ranges(vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)], ctx)
                .unwrap();

            // 'I' into insert mode
            view.vim_user_insert("I", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_user_insert("another", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "   sample textabcd
                anotherefg"
                    .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx)
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
        });
    });
}

#[test]
fn test_vim_line_append() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let view = add_editor_vim_normal_mode(
            "abcd
            efg",
            &mut app,
        );

        // Append to first line
        view.update(&mut app, |view, ctx| {
            // 'A' into insert mode
            view.vim_user_insert("A", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_user_insert("sample text", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "abcdsample text
                efg"
                .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx)
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        // Append to last line
        view.update(&mut app, |view, ctx| {
            // place the cursor at the start of the last line
            view.select_ranges(vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)], ctx)
                .unwrap();

            // 'A' into insert mode
            view.vim_user_insert("A", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_user_insert("another", ctx);
        });

        view.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "abcdsample text
                efganother"
                    .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 10)..DisplayPoint::new(1, 10)]
            );
        });

        view.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx)
        });

        view.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 9)..DisplayPoint::new(1, 9)]
            );
        });

        Ok(())
    })
}

#[test]
fn test_vim_line_navigation() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor_view = add_editor_vim_normal_mode("   echo hello", &mut app);

        editor_view.update(&mut app, |editor, ctx| {
            editor.navigate_line(1, &LineMotion::End, ctx);
        });

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| {
            editor.navigate_line(1, &LineMotion::FirstNonWhitespace, ctx);
        });

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| {
            editor.navigate_line(1, &LineMotion::Start, ctx);
        });

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });
    });
}

#[test]
fn test_vim_cursor_line_cap_on_forward_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor_view = add_editor_vim_normal_mode("echo hello", &mut app);

        editor_view.update(&mut app, |editor, ctx| {
            editor.vim_user_insert("ww", ctx);
        });

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| {
            editor.vim_user_insert("ll", ctx);
        });

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });
    });
}

#[test]
fn test_vim_cursor_line_cap_on_up_down_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor_view = add_editor_vim_normal_mode(
            "echo hello
            echo foo bar baz
            echo
            echo hello everybody",
            &mut app,
        );

        editor_view.update(&mut app, |editor, ctx| editor.vim_user_insert("j$", ctx));

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(1, 15)..DisplayPoint::new(1, 15)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| editor.vim_user_insert("k", ctx));

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| editor.vim_user_insert("j", ctx));

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(1, 15)..DisplayPoint::new(1, 15)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| editor.vim_user_insert("j", ctx));

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(2, 3)..DisplayPoint::new(2, 3)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| editor.vim_user_insert("k", ctx));

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(1, 15)..DisplayPoint::new(1, 15)]
            );
        });

        editor_view.update(&mut app, |editor, ctx| editor.vim_user_insert("2j", ctx));

        editor_view.read(&app, |editor, app| {
            assert_eq!(
                editor.selected_ranges(app),
                [DisplayPoint::new(3, 15)..DisplayPoint::new(3, 15)]
            );
        });
    });
}

#[test]
fn test_vim_toggle_case_simple() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aBéÓ.f", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ABéÓ.f");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbéÓ.f");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉÓ.f");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉó.f");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉó.f");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉó.F");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });
    });
}

#[test]
fn test_vim_toggle_case_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aBéÓ.f\nIü", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉÓ.f\nIü");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉó.f\nIü");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        // Attempt to toggle case past the end of the line.
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)], ctx)
                .unwrap();
            view.vim_user_insert("3~", ctx);
        });

        // Only characters before the end of the line should be toggled.
        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉó.F\nIü");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });
    });
}

#[test]
fn test_vim_toggle_case_visual() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aBéÓ.f", &mut app);

        editor.update(&mut app, |view, ctx| {
            // Enter visual mode.
            view.vim_user_insert("v", ctx);
            // Select the first three characters.
            view.vim_user_insert("ll", ctx);
            // Switch case.
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉÓ.f");
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });

        editor.update(&mut app, |view, ctx| {
            // Enter visual mode at the end of the line.
            view.vim_user_insert("$v", ctx);
            // Select the last three characters.
            view.vim_user_insert("hhh", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.snapshot(ctx).vim_visual_tails(ctx).collect_vec(),
                vec![DisplayPoint::new(0, 6)],
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)],
            );
            assert_eq!(
                view.vim_mode(ctx),
                Some(VimMode::Visual(MotionType::Charwise))
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Switch case.
            view.vim_user_insert("~", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "AbÉó.F");
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_toggle_case_normal_operation() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aBéÓ.f
            ç!Yr
            d",
            &mut app,
        );

        // Toggle case for one word.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("g~w", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "AbÉó.f
                ç!Yr
                d"
                .unindent()
            );
        });

        // Toggle case for that word again, moving in the opposite direction.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("g~b", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "aBéÓ.f
                ç!Yr
                d"
                .unindent()
            );
        });

        // Toggle case for the current line and the one below it.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("wg~j", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "AbÉó.F
                Ç!yR
                d"
                .unindent()
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });
    })
}

#[test]
fn test_vim_uppercase_normal() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aBéÓ.f", &mut app);

        // Uppercase one word.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("gUw", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ABÉÓ.f");
        });
    });
}

#[test]
fn test_vim_uppercase_visual() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aBéÓ.f", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ve", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
            assert_eq!(
                view.snapshot(ctx).vim_visual_tails(ctx).collect_vec(),
                vec![DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("U", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
            assert_eq!(view.buffer_text(ctx), "ABÉÓ.f");
        });
    });
}

#[test]
fn test_vim_lowercase_normal() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aBéÓ.F", &mut app);

        // Lowercase one word.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("guw", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "abéó.F");
        });
    });
}

#[test]
fn test_vim_lowercase_visual() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("aBéÓ.F", &mut app);

        // Toggle case for one word.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("veu", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "abéó.F");
        });
    });
}

#[test]
fn test_vim_accept_full_autosuggestions_char() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo ", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.set_autosuggestion(
                "foo bar-baz",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
            view.vim_user_insert("$", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        // Moving right one char accepts full autosuggestion
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("l", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foo bar-baz");
            assert_eq!(view.current_autosuggestion_text(), None);
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });
    });
}

#[test]
fn test_vim_accept_full_autosuggestions_line_end() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo ", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.set_autosuggestion(
                "foo bar-baz",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
            view.vim_user_insert("$", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        // Moving to end-of-line accepts full autosuggestion
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("$", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foo bar-baz");
            assert_eq!(view.current_autosuggestion_text(), None);
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });
    });
}

#[test]
fn test_vim_accept_partial_autosuggestions_word() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo ", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.set_autosuggestion(
                "foo bar-baz",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
            view.vim_user_insert("e", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foo");
            assert_eq!(view.current_autosuggestion_text(), Some(" bar-baz"));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("e", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foo bar");
            assert_eq!(view.current_autosuggestion_text(), Some("-baz"));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 11)..DisplayPoint::new(0, 11)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("W", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foo bar-baz");
            assert_eq!(view.current_autosuggestion_text(), None);
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });
    });
}

#[test]
fn test_vim_operators_on_word_text_objects() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo foo-bar baz", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("daw", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "foo-bar baz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ciw", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(view.buffer_text(ctx), "-bar baz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(
                &Keystroke::parse("escape").expect("invalid keystroke string"),
                ctx,
            );
            view.vim_user_insert("diw", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "bar baz");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("diw", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), " baz");
        });
    });
}

#[test]
fn test_vim_operators_on_quote_text_objects() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "echo `echo foo-bar baz` hi\necho 'hello world' 'stuff'",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("di`", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo `` hi\necho 'hello world' 'stuff'"
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)]
            );
        });

        // Deleting inner quote again should have no effect
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("di`", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo `` hi\necho 'hello world' 'stuff'"
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ca`", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo  hi\necho 'hello world' 'stuff'"
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        // There are no longer quotes on this line, so the following
        // should have no effect.
        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.vim_user_insert("ca`", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo  hi\necho 'hello world' 'stuff'"
            );
            // TODO: The mode shouldn't actually change if a valid range isn't found, but there is
            // some complication with the design that makes this diffcult right now.
            // assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.vim_user_insert("jEEEdi'", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo  hi\necho 'hello world' ''");
        });
    });
}

#[test]
fn test_vim_find_char_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo foo bar baz", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("tz", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Tf", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Fe", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Fk", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        // Make sure the editor doesn't panic on empty buffer.
        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("", ctx);
            view.vim_user_insert("fa", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });
    });
}

#[test]
fn test_vim_operators_with_find_char_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo foo bar baz", &mut app);

        editor.update(&mut app, |view, ctx| {
            // Move into the midde of the buffer.
            view.vim_user_insert("we", ctx);
            // Peform a delete
            view.vim_user_insert("dfb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foar baz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dtz", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dTf", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo fz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dFo", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });
    });
}

#[test]
fn test_vim_jump_to_matching_bracket() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "echo $(( 2 + 2 )) = 4
            foo[1]
            function hi() {
                echo hello
            }",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 16)..DisplayPoint::new(0, 16)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("l%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move to the next line
            view.vim_user_insert("$w", ctx);
            view.vim_user_insert("%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 5)..DisplayPoint::new(1, 5)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move to the next line
            view.vim_user_insert("W$", ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(4, 0)..DisplayPoint::new(4, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(2, 14)..DisplayPoint::new(2, 14)]
            );
        });
    });
}

#[test]
fn test_vim_operators_on_jump_to_matching_bracket() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "echo $(( 2 + 2 )) = 4
            foo[1]",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                " = 4
                foo[1]"
                    .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("c%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                " = 4
                foo"
                .unindent()
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.vim_user_insert("0d%", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                " = 4
                foo"
                .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });
    });
}

#[test]
fn test_vim_block_text_objects() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "function hello(name) {
                 if (name.startsWith('a')) {
                     console.log(name)
                 }
             }",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("EEdib", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function hello() {
                     if (name.startsWith('a')) {
                         console.log(name)
                     }
                 }"
                .unindent(),
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("WWWWdiB", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function hello() {
                     if (name.startsWith('a')) {
                     }
                 }"
                .unindent(),
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.move_to_buffer_start(ctx);
            view.vim_user_insert("WWci{", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function hello() {

                 }"
                .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });
    });
}

#[test]
fn test_vim_jump_to_unmatched_bracket() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "function hello(name) {
                 if (name.startsWith('a')) {
                     console.log(name)
                 }
             }",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("www])", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 19)..DisplayPoint::new(0, 19)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("[(", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("WWWf(di(", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function hello(name) {
                     if (name.startsWith()) {
                         console.log(name)
                     }
                 }"
                .unindent(),
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 24)..DisplayPoint::new(1, 24)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("bbdab", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function hello(name) {
                     if  {
                         console.log(name)
                     }
                 }"
                .unindent(),
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ca}", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "function hello(name) ".unindent(),);
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 21)..DisplayPoint::new(0, 21)]
            );
        });
    });
}

#[test]
fn test_vim_delete_and_paste_charwise() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo hello", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("xp", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ceho hello");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dawP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ceho hello");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("0D", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("p", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ceho hello");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });
    });
}

#[test]
fn test_vim_delete_and_paste_linewise() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "echo 1
             echo 2
             echo 3",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ddp", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 2
                 echo 1
                 echo 3"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ggP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 1
                 echo 2
                 echo 1
                 echo 3"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ggwwddp", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 1
                 echo 1
                 echo 2
                 echo 3"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("GdkP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 1
                 echo 2
                 echo 3
                 echo 1"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("GddP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 1
                 echo 2
                 echo 1
                 echo 3"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)]
            );
        });
    });
}

#[test]
fn test_vim_yank_and_paste_charwise() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo '(foo bar) baz'", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Wyi'", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("P", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo '(foo bar) baz(foo bar) baz'");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 18)..DisplayPoint::new(0, 18)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("lya(P", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo '(foo bar) baz(foo bar)(foo bar) baz'"
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 27)..DisplayPoint::new(0, 27)]
            );
        });
    });
}

#[test]
fn test_vim_yank_and_paste_space() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo hello\necho hi", &mut app);

        // Yank and paste "e".
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("y ", ctx);
            view.vim_user_insert("3p", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "eeeecho hello\necho hi");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        // Move to the end of the first line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("$", ctx);
        });

        // Yank and paste "o\nec" forward should wrap around to the next line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("y4 ", ctx);
            view.vim_user_insert("p", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "eeeecho helloo\nec\necho hi");
        });
    })
}

#[test]
fn test_vim_yank_and_paste_backspace() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo hello\necho hi", &mut app);

        // Move to the end of the line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("$", ctx);
        });

        // Yank and paste "l".
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("y", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
            view.vim_user_insert("3p", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo helllllo\necho hi");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 11)..DisplayPoint::new(0, 11)]
            );
        });

        // Move to the next line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ee", ctx);
        });

        // Yank and paste "o\nech" backward should wrap around to the previous line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("y6", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
            view.vim_user_insert("p", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo hellllllo\necho\necho hi");
        });
    })
}

#[test]
fn test_vim_yank_and_paste_linewise() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "echo 1
             echo 2
             echo 3",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("yylp", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 1
                 echo 1
                 echo 2
                 echo 3"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("GykwP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 1
                 echo 1
                 echo 2
                 echo 2
                 echo 3
                 echo 3"
                    .unindent()
                    .as_str()
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Make the top line unique so that we can perform a more meaningful assertion.
            view.vim_user_insert("ggwr4", ctx);
            view.vim_user_insert("yyP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo 4
                 echo 4
                 echo 1
                 echo 2
                 echo 2
                 echo 3
                 echo 3"
                    .unindent()
                    .as_str()
            );
        });
    });
}

#[test]
fn test_vim_semicolon_and_comma() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode("echo foof bar food", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ff;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo fooood",);
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dFo", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foood");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(";;;;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echood");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });
    });
}

#[test]
fn test_vim_semicolon_forward_at_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Search for the first occurrence of "b".
            view.vim_user_insert("fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip ahead 2 occurrences.
            view.vim_user_insert("2;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip ahead again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("2;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the start of the line.
            view.vim_user_insert("0", ctx);

            // Move to the first occurrence again.
            view.vim_user_insert(";", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should NOT match the same occurrence.
            view.vim_user_insert("fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });
    });
}

#[test]
fn test_vim_semicolon_forward_before_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Search for the first occurrence of "b".
            view.vim_user_insert("tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip ahead 2 occurrences.
            view.vim_user_insert("2;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip ahead again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("2;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the start of the line.
            view.vim_user_insert("0", ctx);

            // Move to the first occurrence again.
            view.vim_user_insert(";", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should match the same occurrence.
            view.vim_user_insert("tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });
    });
}

#[test]
fn test_vim_semicolon_backward_at_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            // Search for the last occurrence of "b".
            view.vim_user_insert("Fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 14)..DisplayPoint::new(1, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip back 2 occurrences.
            view.vim_user_insert("2;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip back again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("3;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the end of the line.
            view.vim_user_insert("$", ctx);

            // Move to the last occurrence again.
            view.vim_user_insert(";", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 14)..DisplayPoint::new(1, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should NOT match the same occurrence.
            view.vim_user_insert("Fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 5)..DisplayPoint::new(1, 5)]
            );
        });
    });
}

#[test]
fn test_vim_semicolon_backward_before_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        // Starting from the last character on the 2nd line,
        // search for the last occurrence of "b".
        editor.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            view.vim_user_insert("Tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 15)..DisplayPoint::new(1, 15)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(";", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip back 2 occurrences.
            view.vim_user_insert("2;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip back again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("3;", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the end of the line.
            view.vim_user_insert("$", ctx);

            // Move to the last occurrence again.
            view.vim_user_insert(";", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 15)..DisplayPoint::new(1, 15)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should match the same occurrence.
            view.vim_user_insert("Tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 15)..DisplayPoint::new(1, 15)]
            );
        });
    });
}

#[test]
fn test_vim_comma_forward_at_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Move to the middle of the first line.
            view.select_ranges(
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)],
                ctx,
            )
            .unwrap();
            // Search for the next occurrence of "b".
            view.vim_user_insert("fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip back 2 occurrences.
            view.vim_user_insert("2,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip back again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("5,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the end of the line.
            view.move_to_line_end(ctx);

            // Move to the last occurrence.
            view.vim_user_insert(",", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 16)..DisplayPoint::new(0, 16)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move one occurrence back again.
            view.vim_user_insert(",", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should match the same occurrence.
            view.vim_user_insert("fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 16)..DisplayPoint::new(0, 16)]
            );
        });
    });
}

#[test]
fn test_vim_comma_forward_before_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Move to the middle of the first line.
            view.select_ranges(
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)],
                ctx,
            )
            .unwrap();
            // Search for the next occurrence of "b".
            view.vim_user_insert("tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip back 2 occurrences.
            view.vim_user_insert("2,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip back again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("5,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the end of the line.
            view.move_to_line_end(ctx);

            // Move to the 3rd-to-last occurrence.
            view.vim_user_insert("3,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should NOT match the same occurrence.
            view.vim_user_insert("tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });
    });
}

#[test]
fn test_vim_comma_backward_at_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Move to the middle of the first line
            view.select_ranges(vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)], ctx)
                .unwrap();
            // Search for the last occurrence of "b".
            view.vim_user_insert("Fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip ahead 2 occurrences.
            view.vim_user_insert("2,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip ahead again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("5,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 14)..DisplayPoint::new(0, 14)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the start of the line.
            view.move_to_line_start(ctx);

            // Move to the first occurrence.
            view.vim_user_insert(",", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move to the next occurrence.
            view.vim_user_insert(",", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should NOT match the same occurrence.
            view.vim_user_insert("Fb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
        });
    });
}

#[test]
fn test_vim_comma_backward_before_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "aaa baa aba aab baa
            ba bbba aaaaa b aa",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Move to the middle of the first line
            view.select_ranges(vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)], ctx)
                .unwrap();
            // Search for the last occurrence of "b".
            view.vim_user_insert("Tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Skip ahead 2 occurrences.
            view.vim_user_insert("2,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Trying to skip ahead again should do nothing because
            // there aren't enough occurrences in the current line.
            view.vim_user_insert("5,", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move back to the start of the line.
            view.move_to_line_start(ctx);

            // Move to the first occurrence.
            view.vim_user_insert(",", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Move to the next occurrence.
            view.vim_user_insert(",", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Restart the search.
            // The cursor should NOT match the same occurrence.
            view.vim_user_insert("Tb", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });
    });
}

/// This does not include registers for the system clipboard.
#[test]
fn test_vim_general_registers() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let editor = add_editor_vim_normal_mode(
            "echo hello
             printf world
             echo hi",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Yank a word.
            view.vim_user_insert("yaw", ctx);
            // Delete a word into register "a".
            view.vim_user_insert("ww\"adaw", ctx);
            // Paste them next to one another.
            view.vim_user_insert("P\"ap", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo hello
                 echo printf world
                 echo hi"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 11)..DisplayPoint::new(1, 11)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("gg\"\"yy", ctx);
            view.vim_user_insert("G\"bdd", ctx);
            view.vim_user_insert("p\"bP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo hello
                 echo printf world
                 echo hi
                 echo hello"
                    .unindent()
                    .as_str()
            );
        });
    });
}

#[test]
fn test_vim_registers_system_clipboard() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.update(|ctx| {
            ctx.clipboard()
                .write(ClipboardContent::plain_text("foobar".to_owned()))
        });

        let editor = add_editor_vim_normal_mode(
            "echo hello
             printf world
             echo hi",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w\"+P", ctx);
            view.vim_user_insert("dd", ctx);
            view.vim_user_insert("ww\"*dd", ctx);
        });

        let system_clipboard = app.update(|ctx| ctx.clipboard().read().plain_text);
        assert_eq!(system_clipboard, "echo hi\n");

        editor.update(&mut app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "printf world");
            view.vim_user_insert("0w", ctx);
            view.vim_user_insert("p", ctx);
            view.vim_user_insert("gg\"+P", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo hi
                 printf world
                 echo foobarhello"
                    .unindent()
                    .as_str()
            );
        });

        app.update(|ctx| {
            ctx.clipboard()
                .write(ClipboardContent::plain_text("sushi".to_owned()))
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("gg\"*P", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "sushiecho hi
                 printf world
                 echo foobarhello"
                    .unindent()
                    .as_str()
            );
        });
    });
}

#[test]
fn test_vim_buffer_beginning_with_operator() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo hello
             printf world
             echo hi",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("yggep", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo hello
                 echo hello
                 printf world
                 echo hi"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("wwdgg", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo hi");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });
    });
}

#[test]
fn test_vim_buffer_end_with_operator() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo hello
             printf world
             echo hi",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("wwyGwp", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo hello
                 printf world
                 printf world
                 echo hi
                 echo hi"
                    .unindent()
                    .as_str()
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 11)..DisplayPoint::new(2, 11)],
                ctx,
            )
            .unwrap();
            view.vim_user_insert("dG", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo hello
                 printf world"
                    .unindent()
                    .as_str()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
        });
    });
}

#[test]
fn test_vim_basic_dot_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo foo bar baz", &mut app);

        // The first "." should do nothing.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(".", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foo bar baz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("x.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ho foo bar baz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("dw*.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "bar baz");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("rww.fa.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "war wwz");
        });

        editor.update(&mut app, |view, ctx| {
            // Go back to the first word and change it to "foobar"
            view.vim_user_insert("0cefoobar", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            // Go to the second word and repeat
            view.vim_user_insert("w.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "foobar foobar");
        });
    });
}

#[test]
fn test_vim_dot_repeat_into_insert_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("iecho", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("0.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echoecho");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("A foo", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(".", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echoecho foo foo");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("obar", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(".", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echoecho foo foo\nbar\nbar");
        });

        editor.update(&mut app, |view, ctx| {
            // Go to the last word in the buffer and `c` it.
            view.vim_user_insert("Gciw", ctx);
            // Enter "hello"
            view.vim_user_insert("hello", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            // Repeat it on the first word in the buffer
            view.vim_user_insert("ggll.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "hello foo foo\nbar\nhello");
        });
    });
}

#[test]
fn test_vim_dot_repeat_with_number_repeat() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23",
            &mut app,
        );

        // "." with no count should repeat the last event's count.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2d3w.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "12 13 14 15 16 17 18 19 20 21 22 23");
        });

        // Dot with its own count overrides.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "14 15 16 17 18 19 20 21 22 23");
        });

        // It should remember the last override.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(".", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "16 17 18 19 20 21 22 23");
        });

        // An explicit "1" is an override.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("1.", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "17 18 19 20 21 22 23");
        });
    });
}

#[test]
fn test_vim_number_repeat_insert_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo  hi", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("wh", ctx);
            view.vim_user_insert("2ifoo", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foofoo hi");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 10)..DisplayPoint::new(0, 10)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2ad", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foofoodd hi");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2A bar", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foofoodd hi bar bar");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 23)..DisplayPoint::new(0, 23)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert(".", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo foofoodd hi bar bar bar bar");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 31)..DisplayPoint::new(0, 31)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3oprintf", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "echo foofoodd hi bar bar bar bar\nprintf\nprintf\nprintf"
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(3, 5)..DisplayPoint::new(3, 5)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("gg3Oexit", ctx);
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "exit\nexit\nexit\necho foofoodd hi bar bar bar bar\nprintf\nprintf\nprintf"
            );
        });
    });
}

#[test]
fn test_vim_charwise_visual_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "echo foo
             printf quick
             echo bar",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Select the first word.
            view.vim_user_insert("vel", ctx);
            view.vim_user_insert("d", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "foo
                 printf quick
                 echo bar"
                    .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            // Select the first word again, but this time from end to front.
            view.vim_user_insert("evb", ctx);
            view.vim_user_insert("yP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "foofoo
                 printf quick
                 echo bar"
                    .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("vj", ctx);
            view.vim_user_insert("d", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "fontf quick
                 echo bar"
                    .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("0v3e", ctx);
            view.vim_user_insert("c", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), " bar");
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });
    });
}

#[test]
fn test_vim_linewise_visual_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "function say_hi(name, thing) {
                 echo -n hello
             }",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("$", ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("V%yP", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function say_hi(name, thing) {
                     echo -n hello
                 }
                 function say_hi(name, thing) {
                     echo -n hello
                 }"
                .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Vj", ctx);
            view.vim_user_insert("d", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "}
                 function say_hi(name, thing) {
                     echo -n hello
                 }"
                .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Vj$", ctx);
            view.vim_user_insert("d", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "    echo -n hello
                 }"
                .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_visual_mode_text_objects() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "function say_hi(name, thing) {
                 echo -n hello
             }",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            // Select inside the function params parentheses.
            view.vim_user_insert("f,vib", ctx);
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("c", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function say_hi() {
                     echo -n hello
                 }"
                .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 16)..DisplayPoint::new(0, 16)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_keystroke(&Keystroke::parse("escape").unwrap(), ctx);
            view.vim_user_insert("vaW", ctx);
            view.vim_user_insert("d", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "function {
                     echo -n hello
                 }"
                .unindent(),
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
        });
    });
}

#[test]
fn test_vim_visual_mode_paste() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo foo bar", &mut app);

        editor.update(&mut app, |view, ctx| {
            // Select "foo" and yank it.
            view.vim_user_insert("wve", ctx);
            view.vim_user_insert("y", ctx);
            // Select "bar" and paste "foo" over it.
            view.vim_user_insert("wve", ctx);
            view.vim_user_insert("p", ctx);
        });

        editor.update(&mut app, |view, ctx| {
            // Go back and paste "bar" over "foo".
            view.vim_user_insert("gevb", ctx);
            view.vim_user_insert("p", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo bar foo");
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_unnamed_system_clipboard() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("echo food bar", &mut app);

        app.update(|ctx| {
            AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
                let _ = editor_settings
                    .vim_unnamed_system_clipboard
                    .set_value(true, ctx);
            });
            ctx.clipboard()
                .write(ClipboardContent::plain_text("sushi".to_owned()))
        });

        editor.update(&mut app, |view, ctx| view.vim_user_insert("wP", ctx));

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo sushifood bar");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("w\"ayaw", ctx);
        });

        editor.update(&mut app, |view, ctx| {
            assert_eq!(ctx.clipboard().read().plain_text, "sushi");
            view.vim_user_insert("bge\"ap", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "echo bar sushifood bar");
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("0daw", ctx);
        });

        app.update(|ctx| {
            assert_eq!(ctx.clipboard().read().plain_text, "echo ");
        });
    });
}

#[test]
fn test_vim_normal_mode_space() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aaa
            bbb",
            &mut app,
        );

        // Going many " " forward wraps around to the next line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("4 ", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 1)..DisplayPoint::new(1, 1)]
            );
        });

        // Going many " " forward doesn't go past the end of the buffer.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3 ", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2)]
            );

            // Ensure the contents of the buffer haven't changed at all.
            assert_eq!(
                view.buffer_text(ctx),
                "aaa
                bbb"
                .unindent(),
            );
        });
    })
}

#[test]
fn test_vim_normal_mode_backspace() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aaa
            bbb",
            &mut app,
        );

        // Move to the end of the second line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("ee", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2)]
            );
        });

        // Backspacing many times wraps around to the previous line.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("4", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
        });

        // Backspacing many times stops at the start of the buffer.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3", ctx);
            view.vim_keystroke(&Keystroke::parse("backspace").unwrap(), ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );

            // Ensure the contents of the buffer haven't changed at all.
            assert_eq!(
                view.buffer_text(ctx),
                "aaa
                bbb"
                .unindent(),
            );
        });
    })
}

#[test]
fn test_vim_join_line() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aaa
            bbb
            ccc",
            &mut app,
        );

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "aaa bbb\nccc");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "aaa bbb ccc");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
        });

        // Attempting to join again shouldn't do anything.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "aaa bbb ccc");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
        });
    });
}

#[test]
fn test_vim_join_line_empty() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("", &mut app);

        // Attempting to join in an empty buffer should do nothing.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "".unindent());
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "".unindent());
        });
    });
}

#[test]
fn test_vim_number_repeat_join_line() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "aaa
            bbb
            ccc
            ddd
            eee",
            &mut app,
        );

        // 2J joins 2 lines (same as J).
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "aaa bbb
                ccc
                ddd
                eee"
                .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
        });

        // 3J joins 3 lines.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "aaa bbb ccc ddd
                eee"
                .unindent()
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 11)..DisplayPoint::new(0, 11)]
            );
        });

        // 3J will attempt to join 3 lines.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("3J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "aaa bbb ccc ddd eee");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });

        // Attempting to join again shouldn't do anything.
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("J", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "aaa bbb ccc ddd eee");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)]
            );
        });
    });
}

#[test]
fn test_vim_underscore_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text(
                "hello, world 1\n      hello, world 2\n  hello, world 3\nhello, world 4\n   hello, world 5",
                ctx
            );
            view.move_to_buffer_start(ctx);
            view.vim_user_insert("W_", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("j_", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2_", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("Wd_", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)]
            );
            assert_eq!(
                view.buffer_text(ctx),
                "hello, world 1\n      hello, world 2\nhello, world 4\n   hello, world 5"
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("k0d_", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            assert_eq!(
                view.buffer_text(ctx),
                "hello, world 1\nhello, world 4\n   hello, world 5"
            );
        });
    });
}

#[test]
fn test_vim_plus_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text(
                "hello, world 1\n      hello, world 2\n  hello, world 3\nhello, world 4\n   hello, world 5",
                ctx
            );
            view.move_to_buffer_start(ctx);
            view.vim_user_insert("+", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2+", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("c+", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)]
            );
            assert_eq!(
                view.buffer_text(ctx),
                "hello, world 1\n      hello, world 2\n  hello, world 3\n"
            );
        });
    });
}

#[test]
fn test_vim_minus_motion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("", &mut app);

        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text(
                "hello, world 1\n      hello, world 2\n  hello, world 3\nhello, world 4\n   hello, world 5",
                ctx
            );
            view.vim_user_insert("-", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("2-", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("y-", ctx);
        });

        VimRegisters::handle(&app).update(&mut app, |registers, ctx| {
            let clipboard = registers.read_from_register('"', ctx).unwrap();
            assert_eq!(clipboard.text, "hello, world 1\n      hello, world 2\n");
        });
    });
}

#[test]
fn test_vim_visual_selection_with_newlines() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode(
            "ddd
            aaa
            bbb

            ccc"
            .unindent()
            .as_str(),
            &mut app,
        );

        // Ensure layout so vertical motions (j/k) use real geometry for goal columns.
        let window_id = app.read(|ctx| editor.window_id(ctx));
        let mut presenter = warpui::presenter::Presenter::new(window_id);
        let mut updated = std::collections::HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = warpui::WindowInvalidation {
            updated,
            ..Default::default()
        };
        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(
                pathfinder_geometry::vector::vec2f(1000., 1000.),
                1.,
                None,
                ctx,
            );
        });

        // Select aaa, bbb, and the newline
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)], ctx)
                .unwrap();
            view.vim_user_insert("V", ctx);
            view.vim_user_insert("2j", ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(
                view.vim_mode(app_ctx),
                Some(VimMode::Visual(MotionType::Linewise))
            );
        });

        // Delete selection
        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("d", ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "ddd\nccc".unindent());
        });

        // Select onto a smaller line above
        editor.update(&mut app, |view, ctx| {
            view.set_buffer_text("aaa\nbbbbb\nc\nddd".unindent().as_str(), ctx);
        });

        // Re-layout for new content
        let window_id = app.read(|ctx| editor.window_id(ctx));
        let mut presenter = warpui::presenter::Presenter::new(window_id);
        let mut updated = std::collections::HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = warpui::WindowInvalidation {
            updated,
            ..Default::default()
        };
        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(
                pathfinder_geometry::vector::vec2f(1000., 1000.),
                1.,
                None,
                ctx,
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)], ctx)
                .unwrap();
            view.vim_user_insert("V", ctx);
            view.vim_user_insert("j", ctx);
            view.vim_user_insert("d", ctx);
        });

        editor.read(&app, |view, app_ctx| {
            assert_eq!(view.buffer_text(app_ctx), "aaa\nddd".unindent());
        });
    });
}

#[test]
fn test_vim_paragraph_delete_inner_consecutive_blank_lines() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("first\n\n\nsecond\n", &mut app);

        editor.update(&mut app, |view, ctx| {
            // Move cursor onto the block of blank lines and delete inner paragraph.
            view.vim_user_insert("jdip", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "first\nsecond");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_paragraph_change_inner_consecutive_blank_lines() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor = add_editor_vim_normal_mode("first\n\n\nsecond\n", &mut app);

        editor.update(&mut app, |view, ctx| {
            // Change the inner paragraph in the blank-line block.
            view.vim_user_insert("jcip", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "first\n\nsecond");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Insert));
        });
    });
}

#[test]
fn test_vim_paragraph_delete_a_paragraph_two_paragraphs() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let editor =
            add_editor_vim_normal_mode("first line\nof first para\n\nsecond para line\n", &mut app);

        editor.update(&mut app, |view, ctx| {
            // Delete "a paragraph" from within the first paragraph.
            view.vim_user_insert("dap", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "second para line");
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_paragraph_yank_inner_single_paragraph() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let text = "foo bar\nnext line\n";
        let editor = add_editor_vim_normal_mode(text, &mut app);

        editor.update(&mut app, |view, ctx| {
            // Yank inner paragraph, then paste to verify correct paragraph object range.
            view.vim_user_insert("yip", ctx);
            view.vim_user_insert("p", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.buffer_text(ctx),
                "foo bar\nfoo bar\nnext line\nnext line"
            );
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)],
            );
            assert_eq!(view.vim_mode(ctx), Some(VimMode::Normal));
        });
    });
}

#[test]
fn test_vim_paragraph_visual_inner_single_paragraph() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let text = "foo bar\nnext line\n\nhello world";
        let editor = add_editor_vim_normal_mode(text, &mut app);

        editor.update(&mut app, |view, ctx| {
            view.vim_user_insert("wvip", ctx);
        });

        editor.read(&app, |view, ctx| {
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)],
            );
            assert_eq!(
                view.snapshot(ctx).vim_visual_tails(ctx).collect_vec(),
                vec![DisplayPoint::new(0, 0)],
            );
            assert_eq!(
                view.vim_mode(ctx),
                Some(VimMode::Visual(MotionType::Linewise))
            );
        });
    });
}
