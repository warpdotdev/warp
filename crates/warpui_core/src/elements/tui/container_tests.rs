use std::cell::Cell;
use std::rc::Rc;

use ratatui::style::Color;

use super::TuiContainer;
use crate::elements::tui::test_support::{render_to_lines, with_event_context};
use crate::elements::tui::{
    TuiChildView, TuiConstraint, TuiElement, TuiEvent, TuiEventHandler, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiPresentationContext, TuiRect, TuiScreenPoint,
    TuiScreenPosition, TuiSize, TuiText,
};
use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::presenter::tui::TuiPresenter;
use crate::{App, AppContext, EntityId, EntityIdMap};

#[test]
fn padding_offsets_the_child() {
    let container = TuiContainer::new(TuiText::new("X").finish()).with_padding(1);
    assert_eq!(
        render_to_lines(container, TuiSize::new(3, 3)),
        vec!["   ", " X ", "   "],
    );
}

#[test]
fn directional_padding_offsets_the_child() {
    let container = TuiContainer::new(TuiText::new("X").finish())
        .with_padding_left(2)
        .with_padding_top(1);
    assert_eq!(
        render_to_lines(container, TuiSize::new(3, 2)),
        vec!["   ", "  X"],
    );
}

#[test]
fn axis_padding_offsets_the_child() {
    let container = TuiContainer::new(TuiText::new("X").finish())
        .with_padding_x(1)
        .with_padding_y(1);
    assert_eq!(
        render_to_lines(container, TuiSize::new(3, 3)),
        vec!["   ", " X ", "   "],
    );
}

#[test]
fn border_frames_the_child() {
    let container = TuiContainer::new(TuiText::new("X").finish()).with_border();
    assert_eq!(
        render_to_lines(container, TuiSize::new(3, 3)),
        vec!["┌─┐", "│X│", "└─┘"],
    );
}

#[test]
fn border_and_padding_compose() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut container = TuiContainer::new(TuiText::new("X").finish())
                .with_border()
                .with_padding(1);

            // Child inset by 2 (border + padding) on each side: 1x1 child -> 5x5 total.
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = container.layout(
                TuiConstraint::loose(TuiSize::new(20, 20)),
                &mut ctx,
                app_ctx,
            );
            assert_eq!(size, TuiSize::new(5, 5));

            assert_eq!(
                render_to_lines(container, TuiSize::new(5, 5)),
                vec!["┌───┐", "│   │", "│ X │", "│   │", "└───┘"],
            );
        });
    });
}

#[test]
fn background_fills_the_padding_area() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let container = TuiContainer::new(TuiText::new("X").finish())
                .with_padding(1)
                .with_background(Color::Blue);
            let frame = TuiPresenter::new().present_element(
                container.finish(),
                TuiRect::new(0, 0, 3, 3),
                app_ctx,
            );

            assert_eq!(frame.buffer[(0, 0)].bg, Color::Blue);
            assert_eq!(frame.buffer[(1, 1)].symbol(), "X");
        });
    });
}

#[test]
fn present_recurses_into_the_child() {
    let root = EntityId::from_usize(1);
    let embedded = EntityId::from_usize(2);
    let mut parent_by_child = EntityIdMap::default();

    {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiPresentationContext::new(root, &mut rendered_views, &mut parent_by_child);
        let child_node = TuiChildView::from_rendered(embedded, Box::new(()), ctx.rendered_views);
        let mut container = TuiContainer::new(child_node.finish()).with_border();
        container.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&embedded), Some(&root));
}

#[test]
fn dispatch_event_forwards_to_the_child_inside_the_inset() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut container = TuiContainer::new(
                TuiEventHandler::new(TuiText::new("X").finish())
                    .on_key("enter", move |_, _, _| counter.set(counter.get() + 1))
                    .finish(),
            )
            .with_border()
            .with_padding(1);

            let event = TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: "enter".to_owned(),
                    ..Default::default()
                },
                chars: "enter".to_owned(),
                details: KeyEventDetails::default(),
                is_composing: false,
            };
            let handled = with_event_context(|event_ctx| {
                container.dispatch_event(&event, event_ctx, app_ctx)
            });

            assert!(handled);
            assert_eq!(hits.get(), 1);
        });
    });
}

/// A leaf element that always reports a cursor at its own top-left `(0, 0)`.
struct CursorElement {
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
        let size = constraint.clamp(TuiSize::new(1, 1));
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
        ctx.set_terminal_cursor(origin);
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }
}

#[test]
fn cursor_position_offsets_by_border_and_padding() {
    // The child reports its cursor at (0, 0); a 1-cell border + 1-cell padding
    // insets it by 2, so the container reports the cursor at (2, 2) within its
    // own area (inside the frame, not at the corner).
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let container = TuiContainer::new(
                CursorElement {
                    size: None,
                    origin: None,
                }
                .finish(),
            )
            .with_border()
            .with_padding(1);
            let frame = TuiPresenter::new().present_element(
                container.finish(),
                TuiRect::new(0, 0, 5, 5),
                app_ctx,
            );
            assert_eq!(frame.cursor, Some((2, 2)));
        });
    });
}
