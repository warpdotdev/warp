use super::*;
use crate::{
    elements::{ConstrainedBox, DispatchEventResult, EventHandler, Rect, ZIndex},
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
    BottomContainer,
}

#[derive(Default)]
struct View {
    // Maps identifier to number of mouse down events
    mouse_downs: HashMap<ElementIdentifier, usize>,
}

fn init(app: &mut AppContext) {
    app.add_action("container_test:mouse_down", View::mouse_down);
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
        "container_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Container::new(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(100.)
                    .with_width(100.)
                    .finish(),
            )
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_action(
                    "container_test:mouse_down",
                    ElementIdentifier::BottomContainer,
                );
                DispatchEventResult::StopPropagation
            })
            .finish(),
        )
        .with_foreground_overlay(Fill::Solid(ColorU::white()))
        .finish()
    }
}

impl TypedActionView for View {
    type Action = ();
}

#[test]
fn test_container_element_overlay_click_handling() {
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
            assert_eq!(scene.layer_count(), 2);
            let presenter = Rc::new(RefCell::new(presenter));

            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(50., 50.),
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
                1,
                *view
                    .mouse_downs
                    .get(&ElementIdentifier::BottomContainer)
                    .unwrap()
            );
        });
    });
}
