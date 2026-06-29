use super::TuiClipped;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiRect, TuiSize, TuiText,
};
use crate::{App, AppContext, EntityIdMap};

fn render_to_lines(element: &mut dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    element.render(area, &mut buffer, &mut ctx);
    buffer.to_lines()
}

#[test]
fn renders_from_the_requested_logical_row() {
    let mut clipped = TuiClipped::new(TuiText::new("a\nb\nc").truncate()).with_vertical_offset(1);

    assert_eq!(
        render_to_lines(&mut clipped, TuiSize::new(3, 2)),
        vec!["b  ", "c  "],
    );
}

#[test]
fn layout_reports_the_visible_size_after_the_offset() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut clipped =
                TuiClipped::new(TuiText::new("a\nb\nc").truncate()).with_vertical_offset(1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = clipped.layout(TuiConstraint::loose(TuiSize::new(3, 10)), &mut ctx, app_ctx);

            assert_eq!(size, TuiSize::new(3, 2));
        });
    });
}

struct CursorElement {
    cursor: (u16, u16),
}

impl TuiElement for CursorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 3))
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {}

    fn cursor_position(&self, _area: TuiRect, _ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        Some(self.cursor)
    }
}

#[test]
fn cursor_position_is_shifted_into_the_visible_window() {
    let clipped = TuiClipped::new(CursorElement { cursor: (0, 2) }).with_vertical_offset(1);
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };

    assert_eq!(
        clipped.cursor_position(TuiRect::new(0, 0, 3, 2), &mut ctx),
        Some((0, 1)),
    );
}

#[test]
fn cursor_position_above_the_visible_window_is_hidden() {
    let clipped = TuiClipped::new(CursorElement { cursor: (0, 0) }).with_vertical_offset(1);
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };

    assert_eq!(
        clipped.cursor_position(TuiRect::new(0, 0, 3, 2), &mut ctx),
        None,
    );
}
