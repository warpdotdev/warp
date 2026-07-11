use string_offset::CharOffset;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::CoreEditorModel;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiRect,
    TuiSize,
};
use warpui_core::{App, AppContext, ModelHandle};

use super::TuiEditorElement;

/// A char-cell editor model seeded with `text`.
fn model(ctx: &mut AppContext, text: &str) -> ModelHandle<CodeEditorModel> {
    ctx.add_model(|ctx| {
        let mut model = CodeEditorModel::new_tui(0, ctx);
        model.reset_content(InitialBufferState::plain_text(text), ctx);
        model
    })
}

/// Lays out and renders `element` into an `area`-sized buffer, returning its
/// rows trimmed of trailing spaces (blank rows become empty strings).
fn render_lines(
    ctx: &AppContext,
    mut element: TuiEditorElement,
    width: u16,
    height: u16,
) -> Vec<String> {
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
    element.render(area, &mut buffer, &mut paint_ctx);
    buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_string())
        .collect()
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
