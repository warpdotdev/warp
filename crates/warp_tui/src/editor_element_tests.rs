use std::cell::RefCell;
use std::rc::Rc;

use string_offset::CharOffset;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::CoreEditorModel;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    Color, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
    TuiStyle,
};
use warpui_core::event::KeyEventDetails;
use warpui_core::keymap::Keystroke;
use warpui_core::{App, AppContext, ModelHandle};

use super::{TuiEditorAction, TuiEditorElement, TuiEditorStyles};
use crate::tui_builder::TuiUiBuilder;

/// A char-cell editor model seeded with `text`.
fn model(ctx: &mut AppContext, text: &str) -> ModelHandle<CodeEditorModel> {
    ctx.add_model(|ctx| {
        let mut model = CodeEditorModel::new_tui(0, ctx);
        model.reset_content(InitialBufferState::plain_text(text), ctx);
        model
    })
}

#[test]
fn masked_editor_paints_only_mask_glyphs() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "top-secret");
            let element = TuiEditorElement::new(&model, ctx).masked();
            let rendered = render_lines(ctx, element, 20, 1).join("\n");
            assert_eq!(rendered, "••••••••••");
            assert!(!rendered.contains("top-secret"));
        });
    });
}

#[test]
fn selection_span_uses_grapheme_width() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "a\u{2328}\u{fe0f}b");
            let mut element = TuiEditorElement::new(&model, ctx);
            element.selection_ranges = vec![CharOffset::range(1..3)];
            let buffer = render_buffer(ctx, element, 10, 1);

            // The selection style uses a solid bg color (theme foreground);
            // verify the highlight covers both display columns of the wide
            // grapheme and leaves the surrounding cells untouched.
            let selection_bg = TuiUiBuilder::from_app(ctx).selection_style().bg;
            assert_ne!(Some(buffer[(0, 0)].bg), selection_bg);
            assert_eq!(Some(buffer[(1, 0)].bg), selection_bg);
            assert_eq!(Some(buffer[(2, 0)].bg), selection_bg);
            assert_ne!(Some(buffer[(3, 0)].bg), selection_bg);
        });
    });
}
#[test]
fn text_overrides_follow_soft_wrapped_character_ranges() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "/plan argument");
            let styles = TuiEditorStyles {
                text_overrides: vec![(
                    CharOffset::zero()..CharOffset::from(5),
                    TuiStyle::default().fg(Color::Blue),
                )],
                ..Default::default()
            };
            let element = TuiEditorElement::new(&model, ctx).with_styles(styles);
            let buffer = render_buffer(ctx, element, 4, 10);
            // Unicode line breaking wraps after '/', so the styled "/plan"
            // range spans "/" on row 0 and "plan" on row 1.
            assert_eq!(buffer[(0, 0)].fg, Color::Blue);
            assert_eq!(buffer[(0, 1)].fg, Color::Blue);
            assert_eq!(buffer[(3, 1)].fg, Color::Blue);
            assert_ne!(buffer[(0, 2)].fg, Color::Blue);
        });
    });
}

/// Lays out and renders `element` into a buffer.
fn render_buffer(
    ctx: &AppContext,
    mut element: TuiEditorElement,
    width: u16,
    height: u16,
) -> TuiBuffer {
    render_buffer_in_place(ctx, &mut element, width, height)
}

/// Like [`render_buffer`], but leaves the element usable so tests can lay the
/// same cached element out repeatedly (the presenter reuses elements across
/// frames).
fn render_buffer_in_place(
    ctx: &AppContext,
    element: &mut TuiEditorElement,
    width: u16,
    height: u16,
) -> TuiBuffer {
    let mut rendered_views = EntityIdMap::default();
    let mut lctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(width, height)),
        &mut lctx,
        ctx,
    );
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    {
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(
            TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
            &mut surface,
            &mut paint_ctx,
        );
    }
    buffer
}

/// Returns rendered rows with trailing spaces removed.
fn render_lines(
    ctx: &AppContext,
    element: TuiEditorElement,
    width: u16,
    height: u16,
) -> Vec<String> {
    render_buffer(ctx, element, width, height)
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_string())
        .collect()
}
fn dispatch_event(ctx: &AppContext, element: TuiEditorElement, event: &TuiEvent) -> bool {
    dispatch_event_with_view_focus(ctx, element, event, true)
}

/// Like [`dispatch_event`], but supplies the owning view's focus snapshot,
/// mirroring the GUI's `EditorView::focused` → `EditorElement` path.
fn dispatch_event_with_view_focus(
    ctx: &AppContext,
    mut element: TuiEditorElement,
    event: &TuiEvent,
    view_focused: bool,
) -> bool {
    element = element.with_view_focused(view_focused);
    let mut rendered_views = EntityIdMap::default();
    let mut layout_ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = element.layout(
        TuiConstraint::loose(TuiSize::new(80, 20)),
        &mut layout_ctx,
        ctx,
    );
    let area = TuiRect::new(0, 0, size.width, size.height);
    // Paint once so the element retains its scene geometry for hit-testing.
    let scene = {
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(
            TuiScreenPosition::new(i32::from(area.x), i32::from(area.y)),
            &mut surface,
            &mut paint_ctx,
        );
        Rc::new(paint_ctx.scene.clone())
    };
    let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
    element.dispatch_event(event, &mut event_ctx, ctx)
}

#[test]
fn editable_paste_emits_one_complete_text_action() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "");
            let actions = Rc::new(RefCell::new(Vec::new()));
            let actions_for_handler = actions.clone();
            let element = TuiEditorElement::new(&model, ctx)
                .editable()
                .on_action(move |action, _| actions_for_handler.borrow_mut().push(action));
            let payload = "first\n\nsecond\n";

            assert!(dispatch_event(
                ctx,
                element,
                &TuiEvent::Paste {
                    text: payload.to_owned(),
                },
            ));
            let actions = actions.borrow();
            assert_eq!(actions.len(), 1);
            let TuiEditorAction::PasteText(text) = &actions[0] else {
                panic!("expected PasteText");
            };
            assert_eq!(text, payload);
        });
    });
}

#[test]
fn editable_editor_ignores_text_when_another_view_is_focused() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let actions = Rc::new(RefCell::new(Vec::new()));

            // Focus elsewhere: the editable editor declines typed text.
            let actions_for_handler = actions.clone();
            let model_unfocused = model(ctx, "");
            let element = TuiEditorElement::new(&model_unfocused, ctx)
                .editable()
                .on_action(move |action, _| actions_for_handler.borrow_mut().push(action));
            let key = TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: "a".to_owned(),
                    ..Default::default()
                },
                chars: "a".to_owned(),
                details: KeyEventDetails::default(),
                is_composing: false,
            };
            assert!(!dispatch_event_with_view_focus(ctx, element, &key, false));
            assert!(actions.borrow().is_empty());

            // Focus on the owning view: typed text is consumed.
            let actions_for_handler = actions.clone();
            let model_focused = model(ctx, "");
            let element = TuiEditorElement::new(&model_focused, ctx)
                .editable()
                .on_action(move |action, _| actions_for_handler.borrow_mut().push(action));
            assert!(dispatch_event_with_view_focus(ctx, element, &key, true));
            assert!(matches!(
                actions.borrow().as_slice(),
                [TuiEditorAction::InsertChar('a')]
            ));
        });
    });
}

#[test]
fn read_only_editor_ignores_paste() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "unchanged");
            let actions = Rc::new(RefCell::new(Vec::new()));
            let actions_for_handler = actions.clone();
            let element = TuiEditorElement::new(&model, ctx)
                .on_action(move |action, _| actions_for_handler.borrow_mut().push(action));

            assert!(!dispatch_event(
                ctx,
                element,
                &TuiEvent::Paste {
                    text: "ignored".to_owned(),
                },
            ));
            assert!(actions.borrow().is_empty());
        });
    });
}

#[test]
fn plain_rows_paint_with_wrapping() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "abcdef\ngh");
            let element = TuiEditorElement::new(&model, ctx);
            assert_eq!(render_lines(ctx, element, 4, 10), vec!["abcd", "ef", "gh"]);
        });
    });
}

#[test]
fn gutter_numbers_first_rows_and_blanks_continuations() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            // Width 8 with a 1-digit gutter (+2 gap) leaves 5 content columns.
            let model = model(ctx, "abcdef\ngh");
            let element = TuiEditorElement::new(&model, ctx).with_line_number_gutter();
            assert_eq!(
                render_lines(ctx, element, 8, 10),
                vec!["1  abcde", "   f", "2  gh"]
            );
        });
    });
}

#[test]
fn hide_trailing_empty_line_elides_the_final_blank_row() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "a\nb\n");
            let with_flag = TuiEditorElement::new(&model, ctx)
                .with_line_number_gutter()
                .hide_trailing_empty_line();
            assert_eq!(render_lines(ctx, with_flag, 8, 10), vec!["1  a", "2  b"]);

            // Without the flag the trailing empty line keeps its row (the
            // input's cursor legitimately sits there).
            let without_flag = TuiEditorElement::new(&model, ctx);
            assert_eq!(render_lines(ctx, without_flag, 8, 10), vec!["a", "b", ""]);
        });
    });
}

#[test]
fn scroll_windows_the_visible_rows() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "l0\nl1\nl2\nl3\nl4");
            // Scroll state lives on the char-cell render state; push the wrap
            // width first so the row math matches the layout below.
            {
                let render = model.as_ref(ctx).render_state().as_ref(ctx);
                let char_cell = render.char_cell().expect("char-cell model");
                char_cell.set_terminal_width(10);
                char_cell.scroll_by(2, 2, CharOffset::zero(), &[]);
                assert_eq!(char_cell.scroll_offset(), 2);
            }
            let element = TuiEditorElement::new(&model, ctx).with_viewport_rows(2);
            assert_eq!(render_lines(ctx, element, 10, 10), vec!["l2", "l3"]);
        });
    });
}

#[test]
fn width_change_follows_cursor_after_reflow() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "abcde");
            model.update(ctx, |model, ctx| {
                model.select_at(CharOffset::from(6), false, ctx);
                model.end_selection(ctx);
            });

            let wide = TuiEditorElement::new(&model, ctx).with_viewport_rows(1);
            assert_eq!(render_lines(ctx, wide, 10, 10), vec!["abcde"]);

            let narrow = TuiEditorElement::new(&model, ctx).with_viewport_rows(1);
            assert_eq!(render_lines(ctx, narrow, 3, 10), vec!["de"]);
            let render = model.as_ref(ctx).render_state().as_ref(ctx);
            let char_cell = render.char_cell().expect("char-cell model");
            assert_eq!(char_cell.scroll_offset(), 1);
        });
    });
}

/// An editable, view-focused element over `model` with fixed `placeholder`
/// ghost text in the given `style`.
fn placeholder_element(
    ctx: &AppContext,
    model: &ModelHandle<CodeEditorModel>,
    placeholder: &str,
    style: TuiStyle,
) -> TuiEditorElement {
    let placeholder = placeholder.to_owned();
    TuiEditorElement::new(model, ctx)
        .editable()
        .with_view_focused(true)
        .with_placeholder_ghost_text(move |_| Some((placeholder.clone(), style)))
}

#[test]
fn placeholder_ghost_text_renders_only_while_buffer_empty() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let empty = model(ctx, "");
            let element = placeholder_element(ctx, &empty, "type here", TuiStyle::default());
            // One pad cell separates the cursor from the hint.
            assert_eq!(render_lines(ctx, element, 20, 5), vec![" type here"]);

            let populated = model(ctx, "draft");
            let element = placeholder_element(ctx, &populated, "type here", TuiStyle::default());
            assert_eq!(render_lines(ctx, element, 20, 5), vec!["draft"]);
        });
    });
}

#[test]
fn placeholder_ghost_text_requires_view_focus() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let empty = model(ctx, "");
            let element = TuiEditorElement::new(&empty, ctx)
                .editable()
                .with_view_focused(false)
                .with_placeholder_ghost_text(|_| {
                    Some(("type here".to_owned(), TuiStyle::default()))
                });
            assert_eq!(render_lines(ctx, element, 20, 5), vec![""]);
        });
    });
}

/// The presenter caches elements across frames while the state a hint depends
/// on changes without the owning view being invalidated; the provider must
/// therefore be re-resolved on every layout pass, not snapshotted once.
#[test]
fn placeholder_ghost_text_provider_resolves_on_every_layout() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let empty = model(ctx, "");
            let hint = Rc::new(RefCell::new("first".to_owned()));
            let hint_for_provider = hint.clone();
            let mut element = TuiEditorElement::new(&empty, ctx)
                .editable()
                .with_view_focused(true)
                .with_placeholder_ghost_text(move |_| {
                    Some((hint_for_provider.borrow().clone(), TuiStyle::default()))
                });
            let lines = |buffer: TuiBuffer| {
                buffer
                    .to_lines()
                    .into_iter()
                    .map(|line| line.trim_end().to_string())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                lines(render_buffer_in_place(ctx, &mut element, 20, 5)),
                vec![" first"]
            );
            *hint.borrow_mut() = "second".to_owned();
            assert_eq!(
                lines(render_buffer_in_place(ctx, &mut element, 20, 5)),
                vec![" second"]
            );
        });
    });
}

#[test]
fn trailing_ghost_text_outranks_placeholder_ghost_text() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let empty = model(ctx, "");
            let element = placeholder_element(ctx, &empty, "placeholder", TuiStyle::default())
                .with_trailing_ghost_text("<argument>", TuiStyle::default());
            assert_eq!(render_lines(ctx, element, 20, 5), vec!["<argument>"]);
        });
    });
}

#[test]
fn placeholder_ghost_text_paints_with_configured_style() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let empty = model(ctx, "");
            let style = TuiStyle::default().fg(Color::Blue);
            let element = placeholder_element(ctx, &empty, "hint", style);
            let buffer = render_buffer(ctx, element, 20, 5);
            assert_eq!(buffer[(1, 0)].symbol(), "h");
            assert_eq!(buffer[(1, 0)].fg, Color::Blue);
            assert_eq!(buffer[(4, 0)].fg, Color::Blue);
        });
    });
}

#[test]
fn placeholder_ghost_text_truncates_to_element_width() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let empty = model(ctx, "");
            let element = placeholder_element(ctx, &empty, "a very long hint", TuiStyle::default());
            assert_eq!(render_lines(ctx, element, 6, 5), vec![" a ver"]);
        });
    });
}

/// Tab-indented diff rows must paint with the correct number of leading blank
/// columns. A single leading tab at tab-stop 4 should produce four leading
/// spaces, so the first non-whitespace glyph is at column 4 (0-based).
/// This is the headless regression test for APP-5014.
#[test]
fn tab_indented_buffer_rows_preserve_leading_indent() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            // One tab before "fn"; with tab size 4, that is 4 leading spaces.
            let m = model(ctx, "\tfn foo() {}\n");
            let element = TuiEditorElement::new(&m, ctx);
            let lines = render_lines(ctx, element, 40, 5);
            // The rendered line must start with 4 spaces, not zero.
            let first = &lines[0];
            assert!(
                first.starts_with("    fn"),
                "expected 4 leading spaces before 'fn', got: {first:?}"
            );
            // Confirm the 'f' is at column 4, not column 0.
            assert_eq!(&first[..4], "    ");
            assert_eq!(&first[4..6], "fn");
        });
    });
}

/// Space-indented diff rows must not be altered: spaces already paint
/// correctly, so the fix must not change them.
#[test]
fn space_indented_buffer_rows_are_unchanged() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let m = model(ctx, "    fn foo() {}\n");
            let element = TuiEditorElement::new(&m, ctx);
            let lines = render_lines(ctx, element, 40, 5);
            let first = &lines[0];
            assert!(
                first.starts_with("    fn"),
                "expected 4 leading spaces before 'fn', got: {first:?}"
            );
        });
    });
}

/// Multiple tabs produce correct cumulative expansion (tab stops at 0, 4, 8, …).
#[test]
fn multiple_leading_tabs_expand_to_successive_tab_stops() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            // Two leading tabs → 8 leading spaces.
            let m = model(ctx, "\t\tbody\n");
            let element = TuiEditorElement::new(&m, ctx);
            let lines = render_lines(ctx, element, 40, 5);
            let first = &lines[0];
            assert!(
                first.starts_with("        body"),
                "expected 8 leading spaces before 'body', got: {first:?}"
            );
        });
    });
}

/// Three leading tabs in a narrow viewport (12 columns, tab size 4) must wrap
/// at the tab boundary — not lose the trailing content to a truncation bug.
///
/// Pre-fix repro: the char-cell lattice charged each \t as 0 columns, so all
/// three tabs fit on one row and `.truncate()` cut the 12-column expanded
/// string to 12 cells of blanks, hiding "abcdefgh" entirely.
/// Post-fix: each tab is charged its expanded width (4 cols), so the 3 tabs
/// consume all 12 columns of row 0 and the content wraps to row 1.
#[test]
fn tab_narrow_viewport_wraps_tabs_content_is_not_truncated() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let m = model(ctx, "\t\t\tabcdefgh");
            let element = TuiEditorElement::new(&m, ctx);
            let lines = render_lines(ctx, element, 12, 5);
            // Row 0: three tabs = 12 leading spaces; trailing-space trim yields "".
            // Row 1: the eight content glyphs that were previously lost.
            assert!(
                lines.len() >= 2,
                "expected at least 2 rows (tab row + content row), got {lines:?}"
            );
            assert_eq!(
                lines[1], "abcdefgh",
                "content after leading tabs must not be truncated; rows: {lines:?}"
            );
        });
    });
}

/// Selecting characters in a tab-indented line must produce a selection
/// highlight that starts *after* the expanded tab columns, not at the raw
/// character offset.  Pre-fix, the lattice saw the tab as 0 wide, so the
/// selection for "foo" in "\tfoo" was placed at columns 1-3 while the glyphs
/// painted at columns 4-6.
#[test]
fn tab_indent_selection_columns_follow_expanded_tab_width() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            // "\tfoo": tab at char 0 (4 cols), then 'f' at col 4, 'o' at 5, 'o' at 6.
            let m = model(ctx, "\tfoo");
            let mut element = TuiEditorElement::new(&m, ctx);
            // Select chars 1..3 — that is 'f' and 'o' at display columns 4-5.
            element.selection_ranges = vec![CharOffset::range(1..3)];
            let buffer = render_buffer(ctx, element, 10, 1);
            let selection_bg = TuiUiBuilder::from_app(ctx).selection_style().bg;
            // Columns 0-3 are the expanded tab — must NOT be highlighted.
            assert_ne!(
                Some(buffer[(0, 0)].bg),
                selection_bg,
                "column 0 (tab space) must not be selected"
            );
            assert_ne!(
                Some(buffer[(3, 0)].bg),
                selection_bg,
                "column 3 (last tab space) must not be selected"
            );
            // Columns 4-5 are 'f' and first 'o' — must be highlighted.
            assert_eq!(
                Some(buffer[(4, 0)].bg),
                selection_bg,
                "column 4 ('f') must be selected"
            );
            assert_eq!(
                Some(buffer[(5, 0)].bg),
                selection_bg,
                "column 5 ('o') must be selected"
            );
            // Column 6 is the second 'o' (outside selection) — must NOT be highlighted.
            assert_ne!(
                Some(buffer[(6, 0)].bg),
                selection_bg,
                "column 6 (outside selection) must not be selected"
            );
        });
    });
}

/// Ghost (removed) rows with tab-indented content must preserve their
/// leading horizontal indent — the same fix that applies to buffer rows.
#[test]
fn tab_indented_ghost_rows_preserve_leading_indent() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            // Buffer has one line; a ghost (deleted) line is inserted before it.
            let m = model(ctx, "context\n");
            {
                let render = m.as_ref(ctx).render_state().as_ref(ctx);
                let char_cell = render.char_cell().expect("char-cell model");
                // Ghost line: one tab + "removed" — the tab must expand to 4 spaces.
                char_cell.set_test_temporary_blocks(vec![("\tremoved\n".to_string(), 0)]);
            }
            let element = TuiEditorElement::new(&m, ctx);
            let lines = render_lines(ctx, element, 40, 10);
            // The first row is the ghost; it should start with 4 spaces before "removed".
            assert!(
                lines[0].starts_with("    removed"),
                "ghost row must have 4 leading spaces before 'removed', got: {:?}",
                lines[0]
            );
        });
    });
}

/// Tab-indented rows paint correctly even when a line-number gutter is active.
/// The gutter columns are added before the content, and tab expansion inside
/// the content must use the same tab size as the layout layer.
#[test]
fn tab_indented_rows_with_line_number_gutter() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            // Single tab before "fn"; gutter is 1 digit + 2 gap = 3 cols.
            // Content width = 40 - 3 = 37 cols.
            let m = model(ctx, "\tfn foo() {}\n");
            let element = TuiEditorElement::new(&m, ctx)
                .with_line_number_gutter()
                .hide_trailing_empty_line();
            let lines = render_lines(ctx, element, 40, 5);
            // Gutter "1  " (3 chars) then 4 leading spaces from the tab, then "fn".
            assert!(
                lines[0].starts_with("1      fn"),
                "expected gutter '1  ' + 4 tab spaces before 'fn', got: {:?}",
                lines[0]
            );
        });
    });
}

/// A tab that falls on a soft-wrapped continuation row must paint with the
/// same column width that the layout layer charged it, keeping selection
/// highlight and painted glyphs aligned.
///
/// Scenario: "abcde\tXY" at terminal width 6, tab size 4.
/// - Row 0: "abcde" (logical cols 0-4, total width 5 — wraps before the tab).
/// - Row 1: "\tXY"  (logical col starts at 5; tab at col 5 → next stop 8,
///   width 3 → rendered as "   XY").
///
/// The layout lattice charges the tab 3 columns.  Paint must write 3 spaces
/// so 'X' is at display column 3 on row 1, matching the selection highlight.
/// Paint consumes the retained width for the tab rather than recomputing a
/// tab stop from the continuation row's local column.
#[test]
fn tab_on_continuation_row_paint_and_selection_agree() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let m = model(ctx, "abcde\tXY");

            // Verify the rendered text on the continuation row.
            let element = TuiEditorElement::new(&m, ctx);
            let lines = render_lines(ctx, element, 6, 5);
            assert_eq!(lines[0], "abcde", "first row must be 'abcde'");
            assert_eq!(
                lines[1], "   XY",
                "tab at logical col 5 with tab_size 4 must expand to 3 spaces \
                 (next stop at col 8, width 8-5=3), not 4"
            );

            // Verify selection highlight agrees with paint position.
            let mut element = TuiEditorElement::new(&m, ctx);
            // Select 'X' (char offset 6, one past the tab at offset 5).
            element.selection_ranges = vec![CharOffset::range(6..7)];
            let buffer = render_buffer(ctx, element, 6, 2);
            let selection_bg = TuiUiBuilder::from_app(ctx).selection_style().bg;
            // Tab columns 0-2 on row 1 must NOT be selected.
            assert_ne!(
                Some(buffer[(0, 1)].bg),
                selection_bg,
                "col 0 on row 1 (tab space) must not be selected"
            );
            assert_ne!(
                Some(buffer[(2, 1)].bg),
                selection_bg,
                "col 2 on row 1 (tab space) must not be selected"
            );
            // Column 3 on row 1 is 'X' — must be selected.
            assert_eq!(
                Some(buffer[(3, 1)].bg),
                selection_bg,
                "col 3 on row 1 ('X') must be selected; tab expands to 3 spaces"
            );
            // Column 4 on row 1 is 'Y' — outside the selection.
            assert_ne!(
                Some(buffer[(4, 1)].bg),
                selection_bg,
                "col 4 on row 1 ('Y') must not be selected"
            );
        });
    });
}
