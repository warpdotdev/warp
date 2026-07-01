use std::cell::Cell;
use std::rc::Rc;

use super::TuiEventHandler;
use crate::elements::tui::{
    TuiChildView, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPoint,
    TuiPresentationContext, TuiRect,
};
use crate::event::{KeyEventDetails, ModifiersState};
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

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

#[test]
fn invokes_callback_on_matching_key_and_reports_handled() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut handler =
                TuiEventHandler::new(()).on_key("enter", move |_event, _ctx, _app| {
                    counter.set(counter.get() + 1);
                });

            let area = TuiRect::new(0, 0, 4, 1);
            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let handled = handler.dispatch_event(
                &key_event("enter"),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(handled);
            assert_eq!(hits.get(), 1);

            // A non-matching key is left unhandled for ancestors, runs no callback.
            let handled =
                handler.dispatch_event(&key_event("esc"), area, &mut event_ctx, &mut ctx, app_ctx);
            assert!(!handled);
            assert_eq!(hits.get(), 1);
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

            let inner = TuiEventHandler::new(()).on_key("enter", move |_, _, _| {
                inner_counter.set(inner_counter.get() + 1)
            });
            let mut outer = TuiEventHandler::new(inner).on_key("enter", move |_, _, _| {
                outer_counter.set(outer_counter.get() + 1)
            });

            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let handled = outer.dispatch_event(
                &key_event("enter"),
                TuiRect::new(0, 0, 1, 1),
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );

            assert!(handled);
            assert_eq!(inner_hits.get(), 1);
            assert_eq!(outer_hits.get(), 0);
        });
    });
}

#[test]
fn click_inside_area_runs_callback_and_reports_handled() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut handler = TuiEventHandler::new(()).on_click(move |_ctx, _app| {
                counter.set(counter.get() + 1);
            });

            let area = TuiRect::new(0, 0, 4, 2);
            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let handled = handler.dispatch_event(
                &left_mouse_down(1, 1),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(handled);
            assert_eq!(hits.get(), 1);

            // A click outside the area is left unhandled and runs no callback.
            let handled = handler.dispatch_event(
                &left_mouse_down(10, 10),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(!handled);
            assert_eq!(hits.get(), 1);
        });
    });
}

#[test]
fn child_consumes_the_event_before_the_click_handler() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let outer_hits = Rc::new(Cell::new(0u32));
            let outer_counter = outer_hits.clone();

            // A child that always handles the event pre-empts the wrapper's click.
            let mut handler = TuiEventHandler::new(AlwaysHandles).on_click(move |_, _| {
                outer_counter.set(outer_counter.get() + 1);
            });

            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let handled = handler.dispatch_event(
                &left_mouse_down(0, 0),
                TuiRect::new(0, 0, 1, 1),
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );

            assert!(handled);
            assert_eq!(outer_hits.get(), 0);
        });
    });
}

/// A leaf element that reports every event as handled, used to verify the
/// wrapper defers to its child.
struct AlwaysHandles;

impl TuiElement for AlwaysHandles {
    fn layout(
        &mut self,
        constraint: crate::elements::tui::TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &crate::AppContext,
    ) -> crate::elements::tui::TuiSize {
        constraint.min
    }

    fn render(
        &self,
        _area: TuiRect,
        _buffer: &mut crate::elements::tui::TuiBuffer,
        _ctx: &mut TuiLayoutContext,
    ) {
    }

    fn dispatch_event(
        &mut self,
        _event: &TuiEvent,
        _area: TuiRect,
        _event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &crate::AppContext,
    ) -> bool {
        true
    }
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
        let mut handler = TuiEventHandler::new(child_node);
        handler.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&embedded), Some(&root));
}
