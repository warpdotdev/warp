use std::cell::Cell;
use std::rc::Rc;

use super::TuiHoverable;
use crate::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPoint, TuiRect, TuiSize,
};
use crate::elements::MouseStateHandle;
use crate::event::ModifiersState;
use crate::{App, EntityId, EntityIdMap};

fn left_mouse_down(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

fn left_mouse_up(x: u16, y: u16) -> TuiEvent {
    TuiEvent::LeftMouseUp {
        position: TuiPoint::new(x, y),
        modifiers: ModifiersState::default(),
    }
}

#[test]
fn mouse_moves_toggle_hover_state_and_notify_without_consuming_the_event() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = MouseStateHandle::default();
            let mut hoverable = TuiHoverable::new(handle.clone(), ().finish());

            let area = TuiRect::new(0, 0, 10, 1);
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            // Returns (event handled, view notified) after moving the mouse.
            let mut move_to = |x, y| {
                let mut event_ctx = TuiEventContext::default();
                event_ctx.set_origin_view(Some(EntityId::new()));
                let handled = hoverable.dispatch_event(
                    &TuiEvent::MouseMoved {
                        position: TuiPoint::new(x, y),
                        modifiers: ModifiersState::default(),
                        is_synthetic: false,
                    },
                    area,
                    &mut event_ctx,
                    &mut ctx,
                    app_ctx,
                );
                (handled, !event_ctx.take_notified().is_empty())
            };

            // Hover in: state flips and the view is notified, but the move
            // still propagates for sibling hoverables.
            assert_eq!(move_to(2, 0), (false, true));
            assert!(handle.lock().unwrap().is_hovered());

            // A move within the area is not a transition: no notification.
            assert_eq!(move_to(4, 0), (false, false));
            assert!(handle.lock().unwrap().is_hovered());

            // Hover out: state flips back and the view is notified again.
            assert_eq!(move_to(4, 3), (false, true));
            assert!(!handle.lock().unwrap().is_hovered());
        });
    });
}

#[test]
fn click_fires_on_release_after_press_inside_area() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let state = MouseStateHandle::default();
            let mut hoverable =
                TuiHoverable::new(state.clone(), ().finish()).on_click(move |_ctx, _app| {
                    counter.set(counter.get() + 1);
                });

            let area = TuiRect::new(0, 0, 4, 2);
            let mut event_ctx = TuiEventContext::default();
            event_ctx.set_origin_view(Some(EntityId::new()));
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            // The press arms the click (recorded on the shared state) and is
            // consumed, but the callback does not fire yet.
            let handled = hoverable.dispatch_event(
                &left_mouse_down(1, 1),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(handled);
            assert!(state.lock().unwrap().is_clicked());
            assert_eq!(hits.get(), 0);

            // The release inside the area fires the callback and disarms.
            let handled = hoverable.dispatch_event(
                &left_mouse_up(1, 1),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(handled);
            assert!(!state.lock().unwrap().is_clicked());
            assert_eq!(hits.get(), 1);

            // A press outside the area is left unhandled and never arms.
            let handled = hoverable.dispatch_event(
                &left_mouse_down(10, 10),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(!handled);
            assert!(!state.lock().unwrap().is_clicked());
            // ...so the following release inside the area does not fire.
            let handled = hoverable.dispatch_event(
                &left_mouse_up(1, 1),
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
fn release_outside_area_cancels_the_click() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let state = MouseStateHandle::default();
            let mut hoverable =
                TuiHoverable::new(state.clone(), ().finish()).on_click(move |_ctx, _app| {
                    counter.set(counter.get() + 1);
                });

            let area = TuiRect::new(0, 0, 4, 2);
            let mut event_ctx = TuiEventContext::default();
            event_ctx.set_origin_view(Some(EntityId::new()));
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            assert!(hoverable.dispatch_event(
                &left_mouse_down(1, 1),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            ));
            assert!(state.lock().unwrap().is_clicked());

            // Releasing outside disarms without firing, and the release is
            // left unhandled for other elements.
            let handled = hoverable.dispatch_event(
                &left_mouse_up(10, 10),
                area,
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            );
            assert!(!handled);
            assert!(!state.lock().unwrap().is_clicked());
            assert_eq!(hits.get(), 0);
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
            let mut hoverable =
                TuiHoverable::new(MouseStateHandle::default(), AlwaysHandles.finish()).on_click(
                    move |_, _| {
                        outer_counter.set(outer_counter.get() + 1);
                    },
                );

            let mut event_ctx = TuiEventContext::default();
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let handled = hoverable.dispatch_event(
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
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &crate::AppContext,
    ) -> TuiSize {
        constraint.min
    }

    fn render(
        &self,
        _area: TuiRect,
        _buffer: &mut crate::elements::tui::TuiBuffer,
        _ctx: &mut TuiPaintContext,
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
