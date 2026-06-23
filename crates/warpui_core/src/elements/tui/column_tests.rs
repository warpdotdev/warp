use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use super::TuiColumn;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiChildView, TuiConstraint, TuiElement, TuiEventContext,
    TuiEventHandler, TuiInputLine, TuiLayoutContext, TuiParentElement, TuiPresentationContext,
    TuiRect, TuiSize, TuiText,
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
    let mut column = TuiColumn::new()
        .child(TuiText::new("AA"))
        .child(TuiText::new("BB"));

    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = column.layout(TuiConstraint::loose(TuiSize::new(2, 10)), &mut ctx);
    assert_eq!(size, TuiSize::new(2, 2));

    assert_eq!(
        render_to_lines(&column, TuiSize::new(2, 2)),
        vec!["AA", "BB"]
    );
}

#[test]
fn sums_multi_row_children_at_the_correct_offsets() {
    // The middle child spans two rows, so the trailing child must land on row 3.
    let mut column = TuiColumn::new()
        .child(TuiText::new("A"))
        .child(TuiText::new("BB\nCC").truncate())
        .child(TuiText::new("D"));

    // Layout must be called before render so TuiColumn.child_sizes is populated.
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = column.layout(TuiConstraint::loose(TuiSize::new(2, 4)), &mut ctx);
    assert_eq!(size, TuiSize::new(2, 4));
    assert_eq!(
        render_to_lines(&column, TuiSize::new(2, 4)),
        vec!["A ", "BB", "CC", "D "],
    );
}

#[test]
fn clamps_total_height_to_the_constraint_and_clips_overflow() {
    let mut column = TuiColumn::new()
        .child(TuiText::new("A"))
        .child(TuiText::new("BB\nCC").truncate())
        .child(TuiText::new("D"));

    // Layout populates child_sizes; render and dispatch rely on them.
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    let size = column.layout(
        TuiConstraint::new(TuiSize::ZERO, TuiSize::new(2, 3)),
        &mut ctx,
    );
    assert_eq!(size, TuiSize::new(2, 3));

    // Only the first three rows fit; the final child is clipped away.
    assert_eq!(
        render_to_lines(&column, TuiSize::new(2, 3)),
        vec!["A ", "BB", "CC"],
    );
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
            .child(TuiText::new("header"))
            .child(child_node);
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
                .child(TuiText::new("header"))
                .child(
                    TuiEventHandler::new(TuiText::new("first")).on_key("x", move |_, _, _| {
                        first_counter.set(first_counter.get() + 1)
                    }),
                )
                .child(
                    TuiEventHandler::new(TuiText::new("second")).on_key("x", move |_, _, _| {
                        second_counter.set(second_counter.get() + 1)
                    }),
                );

            // Layout must run before dispatch so TuiColumn.child_sizes is populated.
            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = HashMap::new();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            column.layout(TuiConstraint::loose(TuiSize::new(10, 5)), &mut ctx);
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

#[test]
fn forwards_cursor_position_offset_by_slot() {
    // A two-row header sits above an input line; the column lifts the input's
    // cursor down by the header's height.
    let mut column = TuiColumn::new()
        .child(TuiText::new("a\nb").truncate())
        .child(TuiInputLine::new("hi", 2));
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    column.layout(TuiConstraint::tight(TuiSize::new(10, 5)), &mut ctx);
    assert_eq!(
        column.cursor_position(TuiRect::new(0, 0, 10, 5), &mut ctx),
        Some((2, 2)),
    );
}

#[test]
fn flex_child_fills_height_left_after_fixed_children() {
    let mut column = TuiColumn::new()
        .child(TuiText::new("H").truncate())
        .flex_child(TuiText::new("B\nB\nB\nB\nB\nB").truncate());

    // With a flex child the column fills the height it is offered (5), not the
    // content total (7).
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    let size = column.layout(TuiConstraint::new(TuiSize::ZERO, TuiSize::new(1, 5)), &mut ctx);
    assert_eq!(size, TuiSize::new(1, 5));

    // The header takes row 0; the flex body fills the remaining four rows, and
    // its extra rows are clipped to that slot.
    assert_eq!(
        render_to_lines(&column, TuiSize::new(1, 5)),
        vec!["H", "B", "B", "B", "B"],
    );
}

#[test]
fn multiple_flex_children_split_remaining_height_evenly() {
    let mut column = TuiColumn::new()
        .child(TuiText::new("H").truncate())
        .flex_child(TuiText::new("A\nA\nA\nA").truncate())
        .flex_child(TuiText::new("B\nB\nB\nB").truncate());

    // Seven rows: one for the header, the remaining six split evenly (3 + 3).
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    column.layout(TuiConstraint::tight(TuiSize::new(1, 7)), &mut ctx);
    assert_eq!(
        render_to_lines(&column, TuiSize::new(1, 7)),
        vec!["H", "A", "A", "A", "B", "B", "B"],
    );
}

#[test]
fn flex_remainder_favors_earlier_children() {
    let mut column = TuiColumn::new()
        .flex_child(TuiText::new("A\nA\nA\nA\nA").truncate())
        .flex_child(TuiText::new("B\nB\nB\nB\nB").truncate());

    // Five rows across two flex children: base 2 each, the odd remainder going
    // to the first => 3 + 2.
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    column.layout(TuiConstraint::tight(TuiSize::new(1, 5)), &mut ctx);
    assert_eq!(
        render_to_lines(&column, TuiSize::new(1, 5)),
        vec!["A", "A", "A", "B", "B"],
    );
}
