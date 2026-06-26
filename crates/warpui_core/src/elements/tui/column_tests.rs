use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use super::TuiColumn;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiChildView, TuiConstraint, TuiElement, TuiEventContext,
    TuiEventHandler, TuiLayoutContext, TuiParentElement, TuiPresentationContext, TuiRect, TuiSize,
    TuiText,
};
use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::{App, EntityId, Event};

fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
    let mut rendered_views = HashMap::new();
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
fn stacks_two_children_top_to_bottom() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut column = TuiColumn::new()
                .with_child(Box::new(TuiText::new("AA")))
                .with_child(Box::new(TuiText::new("BB")));

            let mut rendered_views = HashMap::new();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = column.layout(TuiConstraint::loose(TuiSize::new(2, 10)), &mut ctx, app_ctx);
            assert_eq!(size, TuiSize::new(2, 2));

            assert_eq!(
                render_to_lines(&column, TuiSize::new(2, 2)),
                vec!["AA", "BB"]
            );
        });
    });
}

#[test]
fn sums_multi_row_children_at_the_correct_offsets() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            // The middle child spans two rows, so the trailing child must land on row 3.
            let mut column = TuiColumn::new()
                .with_child(Box::new(TuiText::new("A")))
                .with_child(Box::new(TuiText::new("BB\nCC").truncate()))
                .with_child(Box::new(TuiText::new("D")));

            // Layout must be called before render so TuiColumn.child_sizes is populated.
            let mut rendered_views = HashMap::new();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = column.layout(TuiConstraint::loose(TuiSize::new(2, 4)), &mut ctx, app_ctx);
            assert_eq!(size, TuiSize::new(2, 4));
            assert_eq!(
                render_to_lines(&column, TuiSize::new(2, 4)),
                vec!["A ", "BB", "CC", "D "],
            );
        });
    });
}

#[test]
fn clamps_total_height_to_the_constraint_and_clips_overflow() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut column = TuiColumn::new()
                .with_child(Box::new(TuiText::new("A")))
                .with_child(Box::new(TuiText::new("BB\nCC").truncate()))
                .with_child(Box::new(TuiText::new("D")));

            // Layout populates child_sizes; render and dispatch rely on them.
            let mut rendered_views = HashMap::new();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = column.layout(
                TuiConstraint::new(TuiSize::ZERO, TuiSize::new(2, 3)),
                &mut ctx,
                app_ctx,
            );
            assert_eq!(size, TuiSize::new(2, 3));

            // Only the first three rows fit; the final child is clipped away.
            assert_eq!(
                render_to_lines(&column, TuiSize::new(2, 3)),
                vec!["A ", "BB", "CC"],
            );
        });
    });
}

#[test]
fn present_recurses_into_children() {
    let root = EntityId::from_usize(1);
    let embedded = EntityId::from_usize(2);
    let mut parent_by_child = HashMap::new();

    {
        let mut rendered_views_for_child = HashMap::new();
        let mut ctx =
            TuiPresentationContext::new(root, &mut rendered_views_for_child, &mut parent_by_child);
        let child_node = TuiChildView::from_rendered(
            embedded,
            Box::new(TuiText::new("body")),
            ctx.rendered_views,
        );
        let mut column = TuiColumn::new()
            .with_child(Box::new(TuiText::new("header")))
            .with_child(Box::new(child_node));
        column.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&embedded), Some(&root));
}

fn key_event(key: &str) -> Event {
    Event::KeyDown {
        keystroke: Keystroke {
            key: key.to_owned(),
            ..Default::default()
        },
        chars: key.to_owned(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

#[test]
fn dispatch_event_offers_children_in_order_and_stops_when_handled() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let first_hits = Rc::new(Cell::new(0u32));
            let second_hits = Rc::new(Cell::new(0u32));
            let first_counter = first_hits.clone();
            let second_counter = second_hits.clone();

            let mut column = TuiColumn::new()
                .with_child(Box::new(TuiText::new("header")))
                .with_child(Box::new(
                    TuiEventHandler::new(TuiText::new("first")).on_key("x", move |_, _, _| {
                        first_counter.set(first_counter.get() + 1)
                    }),
                ))
                .with_child(Box::new(
                    TuiEventHandler::new(TuiText::new("second")).on_key("x", move |_, _, _| {
                        second_counter.set(second_counter.get() + 1)
                    }),
                ));

            // Layout must run before dispatch so TuiColumn.child_sizes is populated.
            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = HashMap::new();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            column.layout(TuiConstraint::loose(TuiSize::new(10, 5)), &mut ctx, app_ctx);
            let handled = column.dispatch_event(
                &key_event("x"),
                TuiRect::new(0, 0, 10, 5),
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );

            assert!(handled);
            assert_eq!(first_hits.get(), 1);
            assert_eq!(
                second_hits.get(),
                0,
                "dispatch must stop at the first child that handles the event"
            );
        });
    });
}
