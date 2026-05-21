use super::*;
use crate::{
    elements::{
        ChildAnchor, ConstrainedBox, DispatchEventResult, EventHandler, Hoverable,
        MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Rect,
        Stack, ZIndex,
    },
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
    TopStackBase,
    TopStackOverlay,
    Hoverable,
}

#[derive(Default)]
struct View {
    // Maps identifier to number of mouse down events
    mouse_downs: HashMap<ElementIdentifier, usize>,
    mouse_state: MouseStateHandle,
}

pub fn init(app: &mut AppContext) {
    app.add_action("clipped_test:mouse_down", View::mouse_down);
}

impl View {
    fn mouse_down(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_down on element {identifier:?}");
        let entry = self.mouse_downs.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }
}

impl Entity for View {
    type Event = ();
}

impl crate::core::View for View {
    fn ui_name() -> &'static str {
        "clipped_test_view"
    }

    // The element tree looks like the following:
    // - Scene
    //   - Stack
    //      - Bottom Stack
    //         - Base (This acts as the base layer for the scene)
    //         - BottomStack
    //      - Top Stack
    //         - TopStackBase (Bottom)
    //         - TopStackOverlay (Top)
    //
    // --------------------------------
    // | TopStackBase  | Hoverable |  |
    // | (Clipped)     | |         |  |
    // |               --|----------  |
    // |                 |            |
    // |      -----------|-------     |
    // |     | Overlap   |       |    |
    // |-----------------        |    |
    // |     |                   |    |
    // |     |                   |    |
    // |     |TopStackOverlay    |    |
    // |      -------------------     |
    // |                              |
    // |                              |
    // |--------------                |
    // |              |               |
    // |              |               |
    // |              |               |
    // |              |               |
    // |BottomStack   |               |
    // --------------------------------
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let mut bottom_stack = Stack::new();

        bottom_stack.add_child(
            ConstrainedBox::new(Rect::new().finish())
                .with_height(100.)
                .with_width(50.)
                .finish(),
        );
        bottom_stack.add_positioned_child(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(25.)
                    .with_width(25.)
                    .finish(),
            )
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_action("clipped_test:mouse_down", ElementIdentifier::BottomStack);
                DispatchEventResult::StopPropagation
            })
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 75.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        let mut top_stack = Stack::new();

        top_stack.add_positioned_child(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(25.)
                    .with_width(25.)
                    .finish(),
            )
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_action("clipped_test:mouse_down", ElementIdentifier::TopStackBase);
                DispatchEventResult::StopPropagation
            })
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        top_stack.add_positioned_child(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(25.)
                    .with_width(25.)
                    .finish(),
            )
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_action(
                    "clipped_test:mouse_down",
                    ElementIdentifier::TopStackOverlay,
                );
                DispatchEventResult::StopPropagation
            })
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(15., 15.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        top_stack.add_positioned_child(
            Hoverable::new(self.mouse_state.clone(), |_| {
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(20.)
                    .with_height(8.)
                    .finish()
            })
            .on_click(|evt, _, _| {
                evt.dispatch_action("clipped_test:mouse_down", ElementIdentifier::Hoverable);
            })
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(15., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        let mut stack = Stack::new();
        stack.add_child(bottom_stack.finish());
        stack.add_child(Clipped::sized(top_stack.finish(), Vector2F::new(25., 25.)).finish());

        // Force the Stack to take up the full size of the window by pulling
        // the minimum size constraint up to the size of the window.
        ConstrainedBox::new(stack.finish())
            .with_min_width(f32::MAX)
            .with_min_height(f32::MAX)
            .finish()
    }
}

impl TypedActionView for View {
    type Action = ();
}

#[test]
fn test_clipped_element_click_handling() {
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
            let scene = presenter.build_scene(vec2f(100., 100.), 1., None, ctx);
            assert_eq!(scene.z_index(), ZIndex::new(0));
            assert_eq!(scene.layer_count(), 9);
            let presenter = Rc::new(RefCell::new(presenter));

            // Click on the bottom stack. This should work because the bottom
            // stack is not clipped.
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

            // Click on the top stack base. This should work because the
            // click is within the clipped range of the top stack.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(10., 10.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Click on the overlap between top stack base and top stack overlay.
            // This should work because the click is still within the clipped
            // range of the top stack.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(20., 20.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Click on the part of top stack overlay not overlapping with base.
            // This should not work because it is outside of the clip bound of the
            // base stack.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(30., 30.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Click on the overlap between hoverable and top stack overlay.
            // This should work because the click is still within the clipped
            // range of the top stack.
            // Note Hoverable needs both mouse down and mouse up to fire the
            // on click event.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(20., 5.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(20., 5.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Click on the part of hoverable not overlapping with base.
            // This should not work because it is outside of the clip bound of the
            // base stack.
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(30., 5.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(30., 5.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _| {
            assert_eq!(
                1,
                *view
                    .mouse_downs
                    .get(&ElementIdentifier::BottomStack)
                    .unwrap()
            );
            assert_eq!(
                1,
                *view
                    .mouse_downs
                    .get(&ElementIdentifier::TopStackBase)
                    .unwrap()
            );
            assert_eq!(
                1,
                *view
                    .mouse_downs
                    .get(&ElementIdentifier::TopStackOverlay)
                    .unwrap()
            );
            assert_eq!(
                1,
                *view.mouse_downs.get(&ElementIdentifier::Hoverable).unwrap()
            );
        });
    });
}
