use std::collections::HashSet;

use string_offset::CharOffset;
use warp::tui_export::Appearance;
use warp_editor::model::CoreEditorModel;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, EntityIdMap};
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiRect, TuiScreenPosition, TuiSize,
};
use warpui_core::keymap::Trigger;
use warpui_core::{App, TuiView as _, TypedActionView as _};

use super::{TuiEditorView, TuiEditorViewAction};
use crate::editor_element::TuiEditorAction;
use crate::editor_interaction::TuiEditorCommand;
use crate::test_fixtures::TestHostView;

/// Renders an editor view to trimmed lines.
fn render_lines(app: &App, editor: &warpui_core::ViewHandle<TuiEditorView>) -> Vec<String> {
    render_lines_at_width(app, editor, 30)
}

#[test]
fn layout_clamps_stale_scroll_after_resize_and_text_replacement() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        render_lines_at_width(&app, &editor, 3);
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::PasteText("abcdef".to_string())),
                ctx,
            );
        });
        assert_eq!(scroll_offset(&app, &editor), 2);

        assert_eq!(render_lines_at_width(&app, &editor, 30)[0], "abcdef");
        assert_eq!(scroll_offset(&app, &editor), 0);

        render_lines_at_width(&app, &editor, 3);
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::MoveToLineEnd),
                ctx,
            );
        });
        assert_eq!(scroll_offset(&app, &editor), 2);

        editor.update(&mut app, |editor, ctx| editor.set_text("x", ctx));
        assert_eq!(scroll_offset(&app, &editor), 2);
        assert_eq!(render_lines_at_width(&app, &editor, 3)[0], "x");
        assert_eq!(scroll_offset(&app, &editor), 0);
    });
}

/// Renders an editor view at a fixed width.
fn render_lines_at_width(
    app: &App,
    editor: &warpui_core::ViewHandle<TuiEditorView>,
    width: u16,
) -> Vec<String> {
    app.read(|ctx| {
        let mut element = editor.as_ref(ctx).render(ctx);
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let size = element.layout(
            TuiConstraint::loose(TuiSize::new(width, 4)),
            &mut layout_ctx,
            ctx,
        );
        let area = TuiRect::new(0, 0, size.width, size.height);
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
        buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect()
    })
}

/// Returns the editor's first visible char-cell row.
fn scroll_offset(app: &App, editor: &warpui_core::ViewHandle<TuiEditorView>) -> u32 {
    editor.read(app, |editor, ctx| {
        editor
            .model
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .char_cell()
            .expect("TUI editor model is char-cell")
            .scroll_offset()
    })
}

#[test]
fn single_line_paste_discards_later_lines() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::PasteText(
                    "first\nsecond".to_string(),
                )),
                ctx,
            );
        });

        assert_eq!(editor.read(&app, |editor, ctx| editor.text(ctx)), "first");
        editor.update(&mut app, |editor, ctx| {
            editor.set_text("third\nfourth", ctx);
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::InsertNewline),
                ctx,
            );
        });
        assert_eq!(editor.read(&app, |editor, ctx| editor.text(ctx)), "third");
    });
}

#[test]
fn kill_and_yank_are_shared_with_the_generic_editor() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        render_lines(&app, &editor);

        editor.update(&mut app, |editor, ctx| {
            editor.set_text("abcd", ctx);
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::MoveLeft),
                ctx,
            );
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::MoveLeft),
                ctx,
            );
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::KillToLineEnd),
                ctx,
            );
        });
        assert_eq!(editor.read(&app, |editor, ctx| editor.text(ctx)), "ab");

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&TuiEditorViewAction::Command(TuiEditorCommand::Yank), ctx);
        });
        assert_eq!(editor.read(&app, |editor, ctx| editor.text(ctx)), "abcd");
    });
}

#[test]
fn editor_follows_cursor_within_its_one_row_viewport() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        render_lines_at_width(&app, &editor, 3);

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::PasteText("abcd".to_string())),
                ctx,
            );
        });
        assert_eq!(scroll_offset(&app, &editor), 1);

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::MoveLeft),
                ctx,
            );
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::MoveLeft),
                ctx,
            );
        });
        assert_eq!(scroll_offset(&app, &editor), 0);
    });
}

#[test]
fn focus_hooks_update_editor_focus_without_changing_text() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (window_id, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        editor.update(&mut app, |editor, ctx| {
            editor.set_text("gen", ctx);
            ctx.focus_self();
        });
        assert!(editor.read(&app, |editor, _| editor.is_focused()));
        assert_eq!(render_lines(&app, &editor)[0], "gen");

        let other = app.update(|ctx| ctx.add_tui_view(window_id, |_| TestHostView));
        other.update(&mut app, |_, ctx| ctx.focus_self());
        assert!(!editor.read(&app, |editor, _| editor.is_focused()));
        assert_eq!(render_lines(&app, &editor)[0], "gen");
    });
}

#[test]
fn keybinding_initializer_registers_line_start_for_input_and_editor() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);

        let triggers_for = |name: &str| {
            app.read(|ctx| {
                ctx.get_key_bindings()
                    .filter(|binding| binding.name == name)
                    .filter_map(|binding| match binding.trigger {
                        Trigger::Keystrokes(keys) => keys.first().map(|key| key.normalized()),
                        Trigger::Empty | Trigger::Standard(_) | Trigger::Custom(_) => None,
                    })
                    .collect::<HashSet<_>>()
            })
        };
        let expected = HashSet::from(["home".to_string(), "ctrl-a".to_string()]);
        assert_eq!(triggers_for("tui:input:move_to_line_start"), expected);
        assert_eq!(triggers_for("tui:editor:move_to_line_start"), expected);
        let kill_to_line_end = HashSet::from(["ctrl-k".to_string(), "cmd-delete".to_string()]);
        assert_eq!(triggers_for("tui:input:kill_to_line_end"), kill_to_line_end);
        assert_eq!(
            triggers_for("tui:editor:kill_to_line_end"),
            kill_to_line_end
        );
        assert_eq!(
            triggers_for("tui:input:insert_newline"),
            HashSet::from([
                "shift-enter".to_string(),
                "ctrl-j".to_string(),
                "alt-enter".to_string(),
            ])
        );
        assert_eq!(
            triggers_for("tui:editor:insert_newline"),
            HashSet::from([
                "shift-enter".to_string(),
                "ctrl-j".to_string(),
                "alt-enter".to_string(),
            ])
        );
        assert!(app.read(|ctx| ctx.get_binding_by_name("tui:editor:move_up").is_none()));
    });
}

/// Regression for APP-4915: `cmd-delete` must bind to `KillToLineEnd` (delete to
/// end of line) in both the TUI input editor and the standalone editor view,
/// matching the GUI's `cmd-delete` / delete-all-right convention. `alt-delete`
/// must remain bound to `DeleteWordForward` (option+delete deletes a word).
#[test]
fn cmd_delete_binds_kill_to_line_end_and_alt_delete_binds_delete_word_forward() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);

        let triggers_for = |name: &str| {
            app.read(|ctx| {
                ctx.get_key_bindings()
                    .filter(|binding| binding.name == name)
                    .filter_map(|binding| match binding.trigger {
                        Trigger::Keystrokes(keys) => keys.first().map(|key| key.normalized()),
                        Trigger::Empty | Trigger::Standard(_) | Trigger::Custom(_) => None,
                    })
                    .collect::<HashSet<_>>()
            })
        };

        // `cmd-delete` deletes to end of line in both editor surfaces.
        assert!(
            triggers_for("tui:input:kill_to_line_end").contains("cmd-delete"),
            "cmd-delete must bind tui:input:kill_to_line_end"
        );
        assert!(
            triggers_for("tui:editor:kill_to_line_end").contains("cmd-delete"),
            "cmd-delete must bind tui:editor:kill_to_line_end"
        );

        // `alt-delete` keeps deleting the next word (option+delete -> DeleteWordForward).
        assert!(
            triggers_for("tui:input:delete_word_forward").contains("alt-delete"),
            "alt-delete must remain bound to tui:input:delete_word_forward"
        );
        assert!(
            triggers_for("tui:editor:delete_word_forward").contains("alt-delete"),
            "alt-delete must remain bound to tui:editor:delete_word_forward"
        );
    });
}

#[test]
fn mouse_selection_action_focuses_the_editor() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (window_id, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        let other = app.update(|ctx| ctx.add_tui_view(window_id, |_| TestHostView));
        other.update(&mut app, |_, ctx| ctx.focus_self());
        assert!(!editor.read(&app, |editor, _| editor.is_focused()));

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::SelectionStartAt {
                    offset: CharOffset::from(1),
                }),
                ctx,
            );
        });
        assert!(editor.read(&app, |editor, _| editor.is_focused()));
    });
}

#[test]
fn actions_edit_the_single_line_buffer() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::PasteText("gen".to_string())),
                ctx,
            );
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::Backspace),
                ctx,
            );
        });
        // Line navigation is visual-row-aware; layout establishes the real
        // terminal width before the command runs.
        render_lines(&app, &editor);
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::MoveToLineStart),
                ctx,
            );
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::InsertChar('X')),
                ctx,
            );
        });
        assert_eq!(editor.read(&app, |editor, ctx| editor.text(ctx)), "Xge");
    });
}
