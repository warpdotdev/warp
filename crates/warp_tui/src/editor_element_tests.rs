use std::cell::RefCell;
use std::rc::Rc;

use string_offset::CharOffset;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::CoreEditorModel;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    Color, Modifier, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
    TuiStyle,
};
use warpui_core::event::KeyEventDetails;
use warpui_core::keymap::Keystroke;
use warpui_core::{App, AppContext, ModelHandle};

use super::{TuiEditorAction, TuiEditorElement, TuiEditorStyles};

/// A char-cell editor model seeded with `text`.
fn model(ctx: &mut AppContext, text: &str) -> ModelHandle<CodeEditorModel> {
    ctx.add_model(|ctx| {
        let mut model = CodeEditorModel::new_tui(0, ctx);
        model.reset_content(InitialBufferState::plain_text(text), ctx);
        model
    })
}
#[test]
fn selection_span_uses_grapheme_width() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let model = model(ctx, "a\u{2328}\u{fe0f}b");
            let mut element = TuiEditorElement::new(&model, ctx);
            element.sel_char_range = Some(CharOffset::range(1..3));
            let buffer = render_buffer(ctx, element, 10, 1);

            assert!(!buffer[(0, 0)].modifier.contains(Modifier::REVERSED));
            assert!(buffer[(1, 0)].modifier.contains(Modifier::REVERSED));
            assert!(buffer[(2, 0)].modifier.contains(Modifier::REVERSED));
            assert!(!buffer[(3, 0)].modifier.contains(Modifier::REVERSED));
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
            let TuiEditorAction::InsertText(text) = &actions[0] else {
                panic!("expected InsertText");
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
