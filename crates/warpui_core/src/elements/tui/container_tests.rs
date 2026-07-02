use std::cell::Cell;
use std::rc::Rc;

use ratatui::style::Color;

use super::TuiContainer;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiChildView, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiEventHandler, TuiLayoutContext, TuiPresentationContext, TuiRect, TuiSize, TuiText,
};
use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::{App, AppContext, EntityId, EntityIdMap};

fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.render(
        TuiRect::new(0, 0, size.width, size.height),
        &mut buffer,
        &mut ctx,
    );
    buffer.to_lines()
}

#[test]
fn padding_offsets_the_child() {
    let container = TuiContainer::new(TuiText::new("X")).with_padding(1);
    assert_eq!(
        render_to_lines(&container, TuiSize::new(3, 3)),
        vec!["   ", " X ", "   "],
    );
}

#[test]
fn directional_padding_offsets_the_child() {
    let container = TuiContainer::new(TuiText::new("X"))
        .with_padding_left(2)
        .with_padding_top(1);
    assert_eq!(
        render_to_lines(&container, TuiSize::new(3, 2)),
        vec!["   ", "  X"],
    );
}

#[test]
fn axis_padding_offsets_the_child() {
    let container = TuiContainer::new(TuiText::new("X"))
        .with_padding_x(1)
        .with_padding_y(1);
    assert_eq!(
        render_to_lines(&container, TuiSize::new(3, 3)),
        vec!["   ", " X ", "   "],
    );
}

#[test]
fn border_frames_the_child() {
    let container = TuiContainer::new(TuiText::new("X")).with_border();
    assert_eq!(
        render_to_lines(&container, TuiSize::new(3, 3)),
        vec!["┌─┐", "│X│", "└─┘"],
    );
}

#[test]
fn border_and_padding_compose() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut container = TuiContainer::new(TuiText::new("X"))
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
                render_to_lines(&container, TuiSize::new(5, 5)),
                vec!["┌───┐", "│   │", "│ X │", "│   │", "└───┘"],
            );
        });
    });
}

#[test]
fn background_fills_the_padding_area() {
    let container = TuiContainer::new(TuiText::new("X"))
        .with_padding(1)
        .with_background(Color::Blue);

    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 3, 3));
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    container.render(TuiRect::new(0, 0, 3, 3), &mut buffer, &mut ctx);

    // A padding cell carries the background fill...
    assert_eq!(buffer[(0, 0)].bg, Color::Blue);
    // ...and the child glyph lands in the center.
    assert_eq!(buffer[(1, 1)].symbol(), "X");
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
        let mut container = TuiContainer::new(child_node).with_border();
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
                    .on_key("enter", move |_, _, _| counter.set(counter.get() + 1)),
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
            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let handled = container.dispatch_event(
                &event,
                TuiRect::new(0, 0, 9, 9),
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );

            assert!(handled);
            assert_eq!(hits.get(), 1);
        });
    });
}

/// A leaf element that always reports a cursor at its own top-left `(0, 0)`.
struct CursorElement;

impl TuiElement for CursorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 1))
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {}

    fn cursor_position(&self, _area: TuiRect, _ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        Some((0, 0))
    }
}

#[test]
fn cursor_position_offsets_by_border_and_padding() {
    // The child reports its cursor at (0, 0); a 1-cell border + 1-cell padding
    // insets it by 2, so the container reports the cursor at (2, 2) within its
    // own area (inside the frame, not at the corner).
    let container = TuiContainer::new(CursorElement)
        .with_border()
        .with_padding(1);
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let cursor = container.cursor_position(TuiRect::new(0, 0, 5, 5), &mut ctx);
    assert_eq!(cursor, Some((2, 2)));
}
