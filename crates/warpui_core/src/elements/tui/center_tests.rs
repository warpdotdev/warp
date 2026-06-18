use std::cell::Cell;
use std::rc::Rc;

use super::TuiCenter;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEventContext, TuiEventHandler, TuiRect,
    TuiSize, TuiText,
};
use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::{App, Event};

fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
    element.render(TuiRect::new(0, 0, size.width, size.height), &mut buffer);
    buffer.to_lines()
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
fn centers_child_within_a_larger_area() {
    let mut center = TuiCenter::new(TuiText::new("hi"));
    let size = TuiSize::new(6, 3);
    center.layout(TuiConstraint::tight(size));
    // "hi" measures 2x1, so it lands at column 2, row 1 of the 6x3 area.
    assert_eq!(
        render_to_lines(&center, size),
        vec!["      ", "  hi  ", "      "],
    );
}

#[test]
fn claims_the_whole_area() {
    let mut center = TuiCenter::new(TuiText::new("x"));
    assert_eq!(
        center.layout(TuiConstraint::loose(TuiSize::new(8, 4))),
        TuiSize::new(8, 4),
    );
}

#[test]
fn clamps_a_child_larger_than_the_area() {
    let mut center = TuiCenter::new(TuiText::new("hello").truncate());
    let size = TuiSize::new(3, 1);
    center.layout(TuiConstraint::tight(size));
    // The child's natural width (5) is clamped to the area width (3) with a zero
    // offset, and the row is truncated to fit.
    assert_eq!(render_to_lines(&center, size), vec!["hel"]);
}

#[test]
fn dispatch_event_reaches_the_centered_child() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut center = TuiCenter::new(
                TuiEventHandler::new(TuiText::new("hi"))
                    .on_key("x", move |_, _, _| counter.set(counter.get() + 1)),
            );
            center.layout(TuiConstraint::tight(TuiSize::new(10, 5)));

            let mut event_ctx = TuiEventContext::default();
            let handled = center.dispatch_event(
                &key_event("x"),
                TuiRect::new(0, 0, 10, 5),
                &mut event_ctx,
                app_ctx,
            );

            assert!(handled);
            assert_eq!(hits.get(), 1);
        });
    });
}
