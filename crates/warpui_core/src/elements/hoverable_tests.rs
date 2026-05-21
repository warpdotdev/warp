use super::*;
use crate::elements::DispatchEventResult;
use crate::r#async::Timer;
use crate::{
    elements::{
        ChildAnchor, ConstrainedBox, EventHandler, OffsetPositioning, ParentAnchor, ParentElement,
        ParentOffsetBounds, Rect, Stack, Text,
    },
    fonts::FamilyId,
    platform::WindowStyle,
    App, AppContext, Entity, Event, Presenter, TypedActionView, ViewContext, WindowInvalidation,
};
use pathfinder_geometry::vector::vec2f;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
enum ElementIdentifier {
    BottomStack,
    HoverableElementBottomLeft,
    HoverableElementTopRight,
}

fn mouse_moved_event(position: Vector2F) -> Event {
    Event::MouseMoved {
        position,
        cmd: false,
        shift: false,
        is_synthetic: false,
    }
}

#[derive(Default)]
struct View {
    // Maps identifier to number of mouse down events
    mouse_downs: HashMap<ElementIdentifier, usize>,
    mouse_ups: HashMap<ElementIdentifier, usize>,
    hover_ins: HashMap<ElementIdentifier, usize>,
    hover_outs: HashMap<ElementIdentifier, usize>,
    bottom_mouse_state: MouseStateHandle,
    top_mouse_state: MouseStateHandle,

    /// If [`Some`], the top-right hoverable will
    /// have a hover-in delay with this duration.
    hover_in_delay: Option<Duration>,

    /// If [`Some`], the top-right hoverable will
    /// have a hover-out delay with this duration.
    hover_out_delay: Option<Duration>,
}

pub fn init(app: &mut AppContext) {
    app.add_action("hoverable_test:mouse_down", View::mouse_down);
    app.add_action("hoverable_test:mouse_up", View::mouse_up);
    app.add_action("hoverable_test:hover_in", View::hover_in);
    app.add_action("hoverable_test:hover_out", View::hover_out);
}

impl View {
    fn with_hover_in_delay(mut self, delay: Duration) -> Self {
        self.hover_in_delay = Some(delay);
        self
    }

    fn with_hover_out_delay(mut self, delay: Duration) -> Self {
        self.hover_out_delay = Some(delay);
        self
    }

    fn mouse_down(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_down on element {identifier:?}");
        let entry = self.mouse_downs.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }

    fn mouse_up(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_up on element {identifier:?}");
        let entry = self.mouse_ups.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }

    fn hover_in(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        log::info!("Recording hover in on element {identifier:?}");
        let entry = self.hover_ins.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }

    fn hover_out(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        log::info!("Recording hover out on element {identifier:?}");
        let entry = self.hover_outs.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }

    fn num_hover_in_events(&self, identifier: &ElementIdentifier) -> usize {
        *self.hover_ins.get(identifier).unwrap_or(&0)
    }

    fn num_hover_out_events(&self, identifier: &ElementIdentifier) -> usize {
        *self.hover_outs.get(identifier).unwrap_or(&0)
    }
}

impl Entity for View {
    type Event = ();
}

impl crate::core::View for View {
    fn ui_name() -> &'static str {
        "hoverable_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();

        stack.add_child(
            ConstrainedBox::new(Rect::new().finish())
                .with_height(100.)
                .with_width(100.)
                .finish(),
        );
        stack.add_positioned_child(
            Hoverable::new(self.bottom_mouse_state.clone(), |_| {
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(25.)
                    .with_width(25.)
                    .finish()
            })
            .on_click(|evt, _, _| {
                evt.dispatch_action(
                    "hoverable_test:mouse_down",
                    ElementIdentifier::HoverableElementBottomLeft,
                );
            })
            .on_hover(|hovered, evt, _, _| {
                let action_name = if hovered {
                    "hoverable_test:hover_in"
                } else {
                    "hoverable_test:hover_out"
                };
                evt.dispatch_action(action_name, ElementIdentifier::HoverableElementBottomLeft);
            })
            .with_cursor(Cursor::Crosshair)
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 75.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        let mut hoverable = Hoverable::new(self.top_mouse_state.clone(), |_| {
            ConstrainedBox::new(Rect::new().finish())
                .with_height(25.)
                .with_width(25.)
                .finish()
        })
        .on_hover(|hovered, evt, _, _| {
            let action_name = if hovered {
                "hoverable_test:hover_in"
            } else {
                "hoverable_test:hover_out"
            };
            evt.dispatch_action(action_name, ElementIdentifier::HoverableElementTopRight);
        })
        .with_cursor(Cursor::PointingHand);

        if let Some(delay) = self.hover_in_delay {
            hoverable = hoverable.with_hover_in_delay(delay);
        }

        if let Some(delay) = self.hover_out_delay {
            hoverable = hoverable.with_hover_out_delay(delay);
        }

        stack.add_positioned_child(
            hoverable.finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(75., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );
        stack.add_positioned_child(
            ConstrainedBox::new(Rect::new().finish())
                .with_height(70.)
                .with_width(70.)
                .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(15., 15.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        let mut scene = Stack::new();
        scene.add_child(
            EventHandler::new(stack.finish())
                .on_left_mouse_down(|evt, _, _| {
                    evt.dispatch_action(
                        "hoverable_test:mouse_down",
                        ElementIdentifier::BottomStack,
                    );
                    DispatchEventResult::StopPropagation
                })
                .on_left_mouse_up(|evt, _, _| {
                    evt.dispatch_action("hoverable_test:mouse_up", ElementIdentifier::BottomStack);
                    DispatchEventResult::StopPropagation
                })
                .finish(),
        );
        scene.finish()
    }
}

impl TypedActionView for View {
    type Action = ();
}

#[test]
fn test_hoverable_element_click_handling() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let mut presenter = Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(100., 100.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            // Click on the hoverable element on the bottom left corner.
            // This event should be handled by the Hoverable as it has a
            // click_handler.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(10., 90.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Mouse up on the hoverable element on the bottom left corner.
            // This should trigger the click_handler and increment the mouse_down
            // count on HoverableElement by 1.
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(10., 90.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Click on the hoverable element on the upper right corner.
            // This event should be handled by the BottomStack instead of Hoverable
            // as the element does not have a click_handler.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(90., 10.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Mouse up on the hoverable element on the top right corner.
            // Since the element does not have a click_handler, this should
            // be captured by the base stack.
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(90., 10.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Click on the base stack.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(10., 10.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _| {
            assert_eq!(
                2,
                *view
                    .mouse_downs
                    .get(&ElementIdentifier::BottomStack)
                    .unwrap()
            );
            assert_eq!(
                1,
                *view
                    .mouse_downs
                    .get(&ElementIdentifier::HoverableElementBottomLeft)
                    .unwrap()
            );
            assert_eq!(
                1,
                *view.mouse_ups.get(&ElementIdentifier::BottomStack).unwrap()
            );
        });
    });
}

#[test]
fn test_hoverable_element_hover_handling_no_delay() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let mut presenter = Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        // Make sure there are no hover events to start.
        view.read(app, |view, _| {
            assert_eq!(
                0,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementBottomLeft)
            );
            assert_eq!(
                0,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementBottomLeft)
            );
        });

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(100., 100.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            // Move the mouse over the hoverable element in the bottom left corner.
            // This event should be handled immediately by the hover handler
            // without delay.
            let event = mouse_moved_event(vec2f(10., 90.));

            // Before the event, the cursor should have it's default shape.
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);
            ctx.simulate_window_event(event.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, event);

            // After the event, the cursor should have the set cursor shape.
            assert_eq!(ctx.get_cursor_shape(), Cursor::Crosshair);

            // Move the mouse to over the covering element. Still over the bottom-left
            // hoverable, but since it's covered, it should be treated as not hovering
            let event = mouse_moved_event(vec2f(20., 80.));
            ctx.simulate_window_event(event.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, event);
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);

            // Move the mouse back to the bottom-left hoverable (not over the covering element)
            // This should trigger another hover event
            let event = mouse_moved_event(vec2f(10., 90.));
            ctx.simulate_window_event(event.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, event);
            assert_eq!(ctx.get_cursor_shape(), Cursor::Crosshair);
        });

        view.read(app, |view, _| {
            assert_eq!(
                2,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementBottomLeft)
            );
            assert_eq!(
                1,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementBottomLeft)
            );
        });
    });
}

#[test]
fn test_hoverable_element_hover_handling_with_hover_in_delay() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            View::default().with_hover_in_delay(Duration::from_millis(500))
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let presenter_clone = presenter.clone();

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.borrow_mut().invalidate(invalidation, ctx);
            presenter
                .borrow_mut()
                .build_scene(vec2f(100., 100.), 1., None, ctx);

            // Move the mouse over the hoverable in the top-left corner.
            // This should not immmediately trigger hover events because this hoverable
            // has a 0.5s hover-in delay.
            let event = mouse_moved_event(vec2f(90., 10.));
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);
            ctx.simulate_window_event(event.clone(), window_id, presenter);
            ctx.set_last_mouse_move_event(window_id, event);

            // The cursor, however, should be updated immediately.
            assert_eq!(ctx.get_cursor_shape(), Cursor::PointingHand);
        });

        view.read(app, |view, _| {
            assert_eq!(
                0,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                0,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });

        // Wait 1s for the delay to complete, then verify that we got a hover event from the
        // top-right Hoverable
        Timer::after(Duration::from_secs(1)).await;
        view.read(app, |view, _| {
            assert_eq!(
                1,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                0,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });

        app.update(move |ctx| {
            // Move the mouse away from the hoverable.
            // There's no hover-out delay so there shouldn't
            // be any delay in registering the hover-out event.
            let event = mouse_moved_event(vec2f(100., 100.));
            ctx.simulate_window_event(event.clone(), window_id, presenter_clone);
            ctx.set_last_mouse_move_event(window_id, event);
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);
        });

        view.read(app, |view, _| {
            assert_eq!(
                1,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                1,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });
    });
}

#[test]
fn test_hoverable_element_hover_handling_with_hover_out_delay() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            View::default().with_hover_out_delay(Duration::from_millis(500))
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let presenter_clone = presenter.clone();

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.borrow_mut().invalidate(invalidation, ctx);
            presenter
                .borrow_mut()
                .build_scene(vec2f(100., 100.), 1., None, ctx);

            // Move the mouse over the hoverable in the top-left corner.
            // This should immmediately trigger a hover event because there is no
            // hover-in delay.
            let event = mouse_moved_event(vec2f(90., 10.));
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);
            ctx.simulate_window_event(event.clone(), window_id, presenter);
            ctx.set_last_mouse_move_event(window_id, event);
            assert_eq!(ctx.get_cursor_shape(), Cursor::PointingHand);
        });

        view.read(app, |view, _| {
            assert_eq!(
                1,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                0,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });

        app.update(move |ctx| {
            // Move the mouse away from the hoverable.
            // This should not immmediately trigger hover events because
            // this hoverable has a 0.5s hover-out delay.
            let event = mouse_moved_event(vec2f(100., 100.));
            ctx.simulate_window_event(event.clone(), window_id, presenter_clone);
            ctx.set_last_mouse_move_event(window_id, event);

            // The cursor, however, should be updated immediately.
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);
        });

        view.read(app, |view, _| {
            assert_eq!(
                1,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                0,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });

        Timer::after(Duration::from_millis(1000)).await;
        view.read(app, |view, _| {
            assert_eq!(
                1,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                1,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });
    });
}

#[test]
fn test_hoverable_element_hover_handling_with_hover_in_out_delay() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            View::default()
                .with_hover_out_delay(Duration::from_millis(500))
                .with_hover_in_delay(Duration::from_millis(500))
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.borrow_mut().invalidate(invalidation, ctx);
            presenter
                .borrow_mut()
                .build_scene(vec2f(100., 100.), 1., None, ctx);

            // Move the mouse over the hoverable in the top-left corner.
            // This should not immmediately trigger hover events because
            // this hoverable has a 0.5s hover-out delay.
            let event = mouse_moved_event(vec2f(90., 10.));
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);
            ctx.simulate_window_event(event.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, event);

            // The cursor should still update immediately.
            assert_eq!(ctx.get_cursor_shape(), Cursor::PointingHand);

            // Move the mouse away from the hoverable in the top-left corner.
            // Again, there's a hover-out delay so no hover events should be
            // fired still.
            let event = mouse_moved_event(vec2f(100., 100.));
            ctx.simulate_window_event(event.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, event);

            // The cursor should still update immediately.
            assert_eq!(ctx.get_cursor_shape(), Cursor::Arrow);

            // Move it back over the hoverable and wait.
            let event = mouse_moved_event(vec2f(90., 10.));
            ctx.simulate_window_event(event.clone(), window_id, presenter);
            ctx.set_last_mouse_move_event(window_id, event);

            // The cursor should still update immediately.
            assert_eq!(ctx.get_cursor_shape(), Cursor::PointingHand);
        });

        view.read(app, |view, _| {
            assert_eq!(
                0,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                0,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });

        // After waiting, there should ultimately be just one hover-in event
        // for the final mouse movement.
        //
        // The other hover-in and hover-out events should have been dropped
        // due to the mouse moving in and out of the hoverable during the
        // delay period.
        Timer::after(Duration::from_millis(1000)).await;
        view.read(app, |view, _| {
            assert_eq!(
                1,
                view.num_hover_in_events(&ElementIdentifier::HoverableElementTopRight)
            );
            assert_eq!(
                0,
                view.num_hover_out_events(&ElementIdentifier::HoverableElementTopRight)
            );
        });
    });
}

// Why would Elements that haven't been painted need to receive any mouse events?
// Shouldn't paint happen BEFORE any user interaction? Yes, but remember that Elements are
// disposable, and may get discarded and created anew between invalidations. Most of the time,
// Elements do get painted again immediately after creation. However, sometimes Elements can
// get scrolled out of view, e.g. outside a ClippedScrollable, and may not get painted. If the
// element, like an Editor, is still focused while out of view, it needs to respond to events
// without panicking. This test just never paints the Hoverable, and dispatches events on it.
#[test]
fn test_unpainted_hoverable_receives_click_events_without_panic() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let mut presenter = Presenter::new(window_id);

        let hover_state = MouseStateHandle::default();
        let mut hoverable = Hoverable::new(hover_state, |_state| {
            Text::new_inline("foobar", FamilyId(0), 10.0).finish()
        });

        app.update(move |ctx| {
            // We can't use ctx.simulate_window_event for click events here because that would
            // require us to paint the scene by calling presenter.build_scene. That's because
            // that code path uses the sizes and origins of the elements to determine which
            // elements to dispatch the click event on. Instead, we need to call the dispatch_event
            // method on the Element directly. That requires us to mock the EventContext, which we
            // can pluck from the Presenter like so:
            let mut event_ctx = presenter.mock_event_context(ctx.font_cache());

            let mouse_down = Event::LeftMouseDown {
                position: vec2f(10., 90.),
                modifiers: Default::default(),
                click_count: 1,
                is_first_mouse: false,
            };
            hoverable.dispatch_event(&DispatchedEvent::from(mouse_down), &mut event_ctx, ctx);

            let mouse_up = Event::LeftMouseUp {
                position: vec2f(10., 90.),
                modifiers: Default::default(),
            };
            hoverable.dispatch_event(&DispatchedEvent::from(mouse_up), &mut event_ctx, ctx);
        });
    });
}
