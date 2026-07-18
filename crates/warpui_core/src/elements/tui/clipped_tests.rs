use std::cell::Cell;
use std::rc::Rc;

use super::TuiClipped;
use crate::elements::tui::test_support::{dispatch_presented_event, render_to_lines};
use crate::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiFlex, TuiHoverable, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPoint, TuiRect, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiText,
};
use crate::elements::MouseStateHandle;
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::{App, AppContext, EntityIdMap};
struct MissingRetainedSize;

impl TuiElement for MissingRetainedSize {
    /// Returns a non-empty layout without retaining it.
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 1))
    }

    /// Paints nothing.
    fn render(
        &mut self,
        _origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
    }
}

/// Rejects children that violate the retained-size contract.
#[test]
#[should_panic(expected = "TuiClipped child size must be retained after layout")]
fn render_requires_the_child_to_retain_its_layout_size() {
    render_to_lines(
        TuiClipped::new(MissingRetainedSize.finish()),
        TuiSize::new(1, 1),
    );
}

/// Composes absolute origins through nested scratch surfaces.
#[test]
fn nested_clipping_preserves_the_requested_logical_rows() {
    let inner =
        TuiClipped::new(TuiText::new("a\nb\nc\nd").truncate().finish()).with_viewport_origin_y(1);
    let outer = TuiClipped::new(inner.finish()).with_viewport_origin_y(1);

    assert_eq!(render_to_lines(outer, TuiSize::new(1, 2)), vec!["c", "d"],);
}

#[test]
fn renders_from_the_requested_logical_row() {
    let clipped =
        TuiClipped::new(TuiText::new("a\nb\nc").truncate().finish()).with_viewport_origin_y(1);

    assert_eq!(
        render_to_lines(clipped, TuiSize::new(3, 2)),
        vec!["b  ", "c  "],
    );
}

#[test]
fn layout_preserves_child_width_and_reports_visible_height() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut clipped = TuiClipped::new(TuiText::new("a\nb\nc").truncate().finish())
                .with_viewport_origin_y(1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = clipped.layout(TuiConstraint::loose(TuiSize::new(3, 10)), &mut ctx, app_ctx);

            assert_eq!(size, TuiSize::new(1, 2));
        });
    });
}

struct CursorElement {
    cursor: (u16, u16),
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiElement for CursorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        let size = constraint.clamp(TuiSize::new(1, 3));
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        position: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        let origin = ctx.scene_point(position);
        self.origin = Some(origin);
        ctx.set_terminal_cursor(TuiScreenPoint::new(
            origin.x.saturating_add(i32::from(self.cursor.0)),
            origin.y.saturating_add(i32::from(self.cursor.1)),
            origin.z_index,
        ));
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

fn clipped_cursor_frame(cursor: (u16, u16)) -> crate::presenter::tui::TuiFrame {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let clipped = TuiClipped::new(
                CursorElement {
                    cursor,
                    size: None,
                    origin: None,
                }
                .finish(),
            )
            .with_viewport_origin_y(1);
            TuiPresenter::new().present_element(clipped.finish(), TuiRect::new(0, 0, 3, 2), app_ctx)
        })
    })
}

#[test]
fn cursor_position_is_shifted_into_the_visible_window() {
    assert_eq!(clipped_cursor_frame((0, 2)).cursor, Some((0, 1)));
}

#[test]
fn cursor_position_above_the_visible_window_is_hidden() {
    assert_eq!(clipped_cursor_frame((0, 0)).cursor, None);
}

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

#[test]
fn hoverable_inside_clipped_content_uses_visible_screen_geometry() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0));
            let counter = hits.clone();
            let hoverable =
                TuiHoverable::new(MouseStateHandle::default(), TuiText::new("hit").finish())
                    .on_click(move |_, _| counter.set(counter.get() + 1));
            let child = TuiFlex::column()
                .child(TuiText::new("hidden").finish())
                .child(hoverable.finish());
            let clipped = TuiClipped::new(child.finish()).with_viewport_origin_y(1);
            let mut presenter = TuiPresenter::new();
            presenter.present_element(clipped.finish(), TuiRect::new(0, 0, 6, 1), app_ctx);

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(1, 0), app_ctx).0);
            assert_eq!(hits.get(), 0, "click fires on release");

            let released = TuiEvent::LeftMouseUp {
                position: TuiPoint::new(1, 0),
                modifiers: ModifiersState::default(),
            };
            assert!(dispatch_presented_event(&mut presenter, &released, app_ctx).0);
            assert_eq!(hits.get(), 1);

            assert!(!dispatch_presented_event(&mut presenter, &left_mouse_down(1, 1), app_ctx).0);
        });
    });
}
