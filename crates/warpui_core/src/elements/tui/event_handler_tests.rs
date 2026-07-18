use std::cell::Cell;
use std::rc::Rc;

use super::TuiEventHandler;
use crate::elements::tui::test_support::with_event_context;
use crate::elements::tui::{TuiChildView, TuiElement, TuiEvent, TuiPresentationContext};
use crate::event::KeyEventDetails;
use crate::keymap::Keystroke;
use crate::{App, EntityId, EntityIdMap};

fn key_event(key: &str) -> TuiEvent {
    TuiEvent::KeyDown {
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
fn invokes_callback_on_matching_key_and_reports_handled() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut handler =
                TuiEventHandler::new(().finish()).on_key("enter", move |_event, _ctx, _app| {
                    counter.set(counter.get() + 1);
                });

            with_event_context(|event_ctx| {
                let handled = handler.dispatch_event(&key_event("enter"), event_ctx, app_ctx);
                assert!(handled);
                assert_eq!(hits.get(), 1);

                // A non-matching key is left unhandled for ancestors, runs no callback.
                let handled = handler.dispatch_event(&key_event("esc"), event_ctx, app_ctx);
                assert!(!handled);
                assert_eq!(hits.get(), 1);
            });
        });
    });
}

#[test]
fn child_consumes_the_event_before_the_wrapper() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let inner_hits = Rc::new(Cell::new(0u32));
            let outer_hits = Rc::new(Cell::new(0u32));
            let inner_counter = inner_hits.clone();
            let outer_counter = outer_hits.clone();

            let inner = TuiEventHandler::new(().finish()).on_key("enter", move |_, _, _| {
                inner_counter.set(inner_counter.get() + 1)
            });
            let mut outer = TuiEventHandler::new(inner.finish()).on_key("enter", move |_, _, _| {
                outer_counter.set(outer_counter.get() + 1)
            });

            let handled = with_event_context(|event_ctx| {
                outer.dispatch_event(&key_event("enter"), event_ctx, app_ctx)
            });

            assert!(handled);
            assert_eq!(inner_hits.get(), 1);
            assert_eq!(outer_hits.get(), 0);
        });
    });
}

#[test]
fn present_recurses_into_the_wrapped_child() {
    let root = EntityId::from_usize(1);
    let embedded = EntityId::from_usize(2);
    let mut parent_by_child = EntityIdMap::default();

    {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiPresentationContext::new(root, &mut rendered_views, &mut parent_by_child);
        let child_node = TuiChildView::from_rendered(embedded, Box::new(()), ctx.rendered_views);
        let mut handler = TuiEventHandler::new(child_node.finish());
        handler.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&embedded), Some(&root));
}
