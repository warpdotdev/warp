use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pathfinder_geometry::vector::{vec2f, Vector2F};

use super::*;
use crate::elements::{
    ChildAnchor, ConstrainedBox, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Rect, Stack,
};
use crate::platform::WindowStyle;
use crate::{
    App, AppContext, Entity, EntityId, Presenter, TypedActionView, ViewContext, WindowInvalidation,
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
enum ElementIdentifier {
    Base,
    Inset,
    Overlay,
}

#[derive(Default)]
struct View {
    // Maps identifier to number of mouse down events
    mouse_downs: HashMap<ElementIdentifier, usize>,
    mouse_ins: HashMap<ElementIdentifier, usize>,
    mouse_in_behavior: MouseInBehavior,
}

pub fn init(app: &mut AppContext) {
    app.add_action("event_handler_test:mouse_down", View::mouse_down);
    app.add_action("event_handler_test:mouse_in", View::mouse_in);
}

impl View {
    fn mouse_down(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        let entry = self.mouse_downs.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }

    fn mouse_in(&mut self, identifier: &ElementIdentifier, _: &mut ViewContext<Self>) -> bool {
        let entry = self.mouse_ins.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }
}

impl Entity for View {
    type Event = ();
}

impl View {
    fn new(mouse_in_behavior: MouseInBehavior) -> Self {
        Self {
            mouse_in_behavior,
            ..Default::default()
        }
    }
}

impl crate::core::View for View {
    fn ui_name() -> &'static str {
        "event_handler_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let mut inner_stack = Stack::new();
        inner_stack.add_child(
            ConstrainedBox::new(Rect::new().finish())
                .with_height(100.)
                .with_width(100.)
                .finish(),
        );
        inner_stack.add_positioned_child(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(25.)
                    .with_width(25.)
                    .finish(),
            )
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_action("event_handler_test:mouse_down", ElementIdentifier::Inset);
                DispatchEventResult::StopPropagation
            })
            .on_mouse_in(
                |evt, _, _| {
                    evt.dispatch_action("event_handler_test:mouse_in", ElementIdentifier::Inset);
                    DispatchEventResult::StopPropagation
                },
                Some(self.mouse_in_behavior),
            )
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 75.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        let mut stack = Stack::new();
        stack.add_child(
            EventHandler::new(inner_stack.finish())
                .on_left_mouse_down(|evt, _, _| {
                    evt.dispatch_action("event_handler_test:mouse_down", ElementIdentifier::Base);
                    DispatchEventResult::StopPropagation
                })
                .on_mouse_in(
                    |evt, _, _| {
                        evt.dispatch_action("event_handler_test:mouse_in", ElementIdentifier::Base);
                        DispatchEventResult::StopPropagation
                    },
                    Some(self.mouse_in_behavior),
                )
                .finish(),
        );
        stack.add_positioned_child(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(25.)
                    .with_width(25.)
                    .finish(),
            )
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_action("event_handler_test:mouse_down", ElementIdentifier::Overlay);
                DispatchEventResult::StopPropagation
            })
            .on_mouse_in(
                |evt, _, _| {
                    evt.dispatch_action("event_handler_test:mouse_in", ElementIdentifier::Overlay);
                    DispatchEventResult::StopPropagation
                },
                Some(self.mouse_in_behavior),
            )
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(75., 0.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );

        stack.finish()
    }
}

impl TypedActionView for View {
    type Action = ();
}

#[test]
fn test_layered_click_handling() {
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
            assert_eq!(scene.layer_count(), 5);
            let presenter = Rc::new(RefCell::new(presenter));

            // Click on the overlay
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

            // Click on the inset
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

            // Click on the top-left area of the base
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

            // Click on the bottom-right area of the base
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(90., 90.),
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
                *view.mouse_downs.get(&ElementIdentifier::Overlay).unwrap()
            );
            assert_eq!(1, *view.mouse_downs.get(&ElementIdentifier::Inset).unwrap());
            assert_eq!(2, *view.mouse_downs.get(&ElementIdentifier::Base).unwrap());
        });
    });
}

#[test]
fn test_default_mouse_in_behavior() {
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
            assert_eq!(scene.layer_count(), 5);
            let presenter = Rc::new(RefCell::new(presenter));

            // Non-synthetic move over the overlay
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(90., 10.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );

            // Non-synthetic move over the inset
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(10., 90.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );

            // Non-synthetic move over top left the base
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(10., 10.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );

            // Non-synthetic move over the bottom right of base
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(90., 90.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );
        });

        view.read(app, |view, _| {
            assert_eq!(1, *view.mouse_ins.get(&ElementIdentifier::Overlay).unwrap());
            assert_eq!(1, *view.mouse_ins.get(&ElementIdentifier::Inset).unwrap());
            // Only 2 events should be fired because 1) the inset is a child of the base
            // and doesn't propagate events to its parent 2) the overlay event is not propagated
            // to the base.
            assert_eq!(2, *view.mouse_ins.get(&ElementIdentifier::Base).unwrap());
        });
    });
}

#[test]
fn test_mouse_in_behavior_dont_fire_on_synthetic_events() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            View::new(MouseInBehavior {
                fire_on_synthetic_events: false,
                fire_when_covered: true,
            })
        });

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
            assert_eq!(scene.layer_count(), 5);
            let presenter = Rc::new(RefCell::new(presenter));

            // Non-synthetic move over the overlay
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(90., 10.),
                    cmd: false,
                    shift: false,
                    is_synthetic: true,
                },
                window_id,
                presenter.clone(),
            );
        });

        view.read(app, |view, _| {
            assert_eq!(
                0,
                *view
                    .mouse_ins
                    .get(&ElementIdentifier::Overlay)
                    .unwrap_or(&0)
            );
            assert_eq!(
                0,
                *view.mouse_ins.get(&ElementIdentifier::Inset).unwrap_or(&0)
            );
            assert_eq!(
                0,
                *view.mouse_ins.get(&ElementIdentifier::Base).unwrap_or(&0)
            );
        });
    });
}

#[test]
fn test_mouse_in_behavior_dont_fire_when_covered() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            View::new(MouseInBehavior {
                fire_on_synthetic_events: true,
                fire_when_covered: false,
            })
        });

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
            assert_eq!(scene.layer_count(), 5);
            let presenter = Rc::new(RefCell::new(presenter));

            // Non-synthetic move over the overlay
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(90., 10.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );

            // Non-synthetic move over the inset
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(10., 90.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );

            // Non-synthetic move over top left the base
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(10., 10.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );

            // Non-synthetic move over the bottom right of base
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(90., 90.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter.clone(),
            );
        });

        view.read(app, |view, _| {
            assert_eq!(1, *view.mouse_ins.get(&ElementIdentifier::Overlay).unwrap());
            assert_eq!(1, *view.mouse_ins.get(&ElementIdentifier::Inset).unwrap());
            assert_eq!(2, *view.mouse_ins.get(&ElementIdentifier::Base).unwrap());
        });
    });
}

/// For testing event propagation
#[derive(Debug)]
enum PropagationViewAction {
    MouseDown(ElementIdentifier),
}

#[derive(Default)]
struct PropagationView {
    // Maps identifier to number of mouse down events
    mouse_downs: HashMap<ElementIdentifier, usize>,
    allow_propagation: bool,
}

impl PropagationView {
    fn mouse_down(&mut self, identifier: &ElementIdentifier) -> bool {
        let entry = self.mouse_downs.entry(*identifier).or_insert(0);
        *entry += 1;
        true
    }

    fn set_propagation(&mut self, allow_propagation: bool, ctx: &mut ViewContext<Self>) {
        self.allow_propagation = allow_propagation;
        ctx.notify();
    }
}

impl Entity for PropagationView {
    type Event = ();
}

impl crate::core::View for PropagationView {
    fn ui_name() -> &'static str {
        "event_handler_test_propagation_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let allow_propagation = self.allow_propagation;

        let handler = EventHandler::new(
            ConstrainedBox::new(Rect::new().finish())
                .with_height(100.)
                .with_width(100.)
                .finish(),
        )
        .on_left_mouse_down(move |evt, _, _| {
            evt.dispatch_typed_action(PropagationViewAction::MouseDown(ElementIdentifier::Inset));
            if allow_propagation {
                DispatchEventResult::PropagateToParent
            } else {
                DispatchEventResult::StopPropagation
            }
        })
        .finish();

        EventHandler::new(handler)
            .on_left_mouse_down(|evt, _, _| {
                evt.dispatch_typed_action(PropagationViewAction::MouseDown(
                    ElementIdentifier::Base,
                ));
                DispatchEventResult::StopPropagation
            })
            .finish()
    }
}

impl TypedActionView for PropagationView {
    type Action = PropagationViewAction;

    fn handle_action(&mut self, action: &Self::Action, _: &mut ViewContext<Self>) {
        match action {
            PropagationViewAction::MouseDown(identifier) => {
                self.mouse_down(identifier);
            }
        }
    }
}

fn invalidate_and_rebuild_scene(
    presenter: &Rc<RefCell<Presenter>>,
    root_view_id: EntityId,
    ctx: &mut AppContext,
) {
    let mut updated = HashSet::new();
    updated.insert(root_view_id);
    let invalidation = WindowInvalidation {
        updated,
        ..Default::default()
    };
    presenter.borrow_mut().invalidate(invalidation, ctx);
    presenter
        .borrow_mut()
        .build_scene(vec2f(100., 100.), 1., None, ctx);
}

#[test]
fn test_event_propagation() {
    App::test((), |mut app| async move {
        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, |_| PropagationView::default());

        let root_view_id = view.id();
        app.update(move |ctx| {
            invalidate_and_rebuild_scene(
                &ctx.presenter(window_id).expect("Window should exist"),
                root_view_id,
                ctx,
            );

            // Click on the inset with propagation disabled
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(90., 10.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                ctx.presenter(window_id)
                    .expect("window should exist")
                    .clone(),
            );
        });

        view.read(&app, |view, _| {
            assert_eq!(1, *view.mouse_downs.get(&ElementIdentifier::Inset).unwrap());
            assert_eq!(view.mouse_downs.get(&ElementIdentifier::Base), None);
        });

        // Allow propagation
        view.update(&mut app, |view, ctx| {
            view.set_propagation(true, ctx);
        });

        app.update(move |ctx| {
            // Click on the inset with propagation enabled
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(90., 10.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                ctx.presenter(window_id)
                    .expect("window should exist")
                    .clone(),
            );
        });

        // Both the inset and the base should have received the even
        view.read(&app, |view, _| {
            assert_eq!(2, *view.mouse_downs.get(&ElementIdentifier::Inset).unwrap());
            assert_eq!(1, *view.mouse_downs.get(&ElementIdentifier::Base).unwrap());
        });
    })
}

fn select_entire_probe_text(
    _content: &str,
    _click_offset: string_offset::ByteOffset,
) -> Option<std::ops::Range<string_offset::ByteOffset>> {
    Some(string_offset::ByteOffset::zero()..string_offset::ByteOffset::from(1))
}

/// Records the arguments forwarded to each `SelectableElement` method so the
/// test can assert that `EventHandler` forwards selection to its child.
#[derive(Clone, Default)]
struct SelectableProbeState {
    get_selection_args: Rc<RefCell<Vec<(Vector2F, Vector2F, IsRect)>>>,
    expand_selection_args: Rc<RefCell<Vec<(Vector2F, SelectionDirection, SelectionType)>>>,
    semantic_order_args: Rc<RefCell<Vec<(Vector2F, Vector2F)>>>,
    smart_select_args: Rc<RefCell<Vec<Vector2F>>>,
    clickable_bounds_args: Rc<RefCell<Vec<Option<Selection>>>>,
}

struct SelectableProbeElement {
    state: SelectableProbeState,
    size: Vector2F,
}

impl SelectableProbeElement {
    fn new(state: SelectableProbeState) -> Self {
        Self {
            state,
            size: vec2f(400.0, 120.0),
        }
    }
}

impl Element for SelectableProbeElement {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        self.size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, _origin: Vector2F, _ctx: &mut PaintContext, _app: &AppContext) {}

    fn size(&self) -> Option<Vector2F> {
        Some(self.size)
    }

    fn origin(&self) -> Option<Point> {
        Some(Point::new(0.0, 0.0, ZIndex::new(0)))
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self)
    }
}

impl SelectableElement for SelectableProbeElement {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        self.state
            .get_selection_args
            .borrow_mut()
            .push((selection_start, selection_end, is_rect));
        Some(vec![SelectionFragment {
            text: "probe".to_string(),
            origin: Point::new(0.0, 0.0, ZIndex::new(0)),
        }])
    }

    fn expand_selection(
        &self,
        absolute_point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        _word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        self.state
            .expand_selection_args
            .borrow_mut()
            .push((absolute_point, direction, unit));
        Some(absolute_point + vec2f(5.0, 0.0))
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        self.state
            .semantic_order_args
            .borrow_mut()
            .push((absolute_point, absolute_point_other));
        Some(absolute_point.x() < absolute_point_other.x())
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        _smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        self.state
            .smart_select_args
            .borrow_mut()
            .push(absolute_point);
        Some((absolute_point, absolute_point + vec2f(12.0, 0.0)))
    }

    fn calculate_clickable_bounds(
        &self,
        current_selection: Option<Selection>,
    ) -> Vec<pathfinder_geometry::rect::RectF> {
        self.state
            .clickable_bounds_args
            .borrow_mut()
            .push(current_selection);
        Vec::new()
    }
}

/// Regression test for subagent/collapsible output text being unselectable:
/// `EventHandler` must forward the `SelectableElement` chain to its child so a
/// `SelectableArea` can select content wrapped in an `EventHandler`. Before the
/// fix, `EventHandler::as_selectable_element` returned `None`, severing the
/// chain.
#[test]
fn event_handler_forwards_selectable_element_to_child() {
    let probe = SelectableProbeState::default();
    let event_handler = EventHandler::new(Box::new(SelectableProbeElement::new(probe.clone())))
        .on_scroll_wheel(|_, _, _, _| DispatchEventResult::PropagateToParent);

    let selectable = event_handler
        .as_selectable_element()
        .expect("EventHandler should expose its child as selectable");

    let start = vec2f(10.0, 20.0);
    let end = vec2f(120.0, 20.0);

    let fragments = selectable
        .get_selection(start, end, IsRect::False)
        .expect("selection should be forwarded to the child");
    assert_eq!(fragments[0].text, "probe");
    assert_eq!(
        probe.get_selection_args.borrow().as_slice(),
        &[(start, end, IsRect::False)]
    );

    let expanded = selectable
        .expand_selection(
            start,
            SelectionDirection::Forward,
            SelectionType::Semantic,
            &WordBoundariesPolicy::Default,
        )
        .expect("expand_selection should be forwarded to the child");
    assert_eq!(expanded, start + vec2f(5.0, 0.0));

    let smart = selectable
        .smart_select(start, select_entire_probe_text)
        .expect("smart_select should be forwarded to the child");
    assert_eq!(smart, (start, start + vec2f(12.0, 0.0)));
    assert_eq!(probe.smart_select_args.borrow().as_slice(), &[start]);
}

/// An `EventHandler` wrapping a non-selectable child must not pretend to have a
/// selection.
#[test]
fn event_handler_returns_no_selection_for_non_selectable_child() {
    let event_handler = EventHandler::new(Rect::new().finish());
    let selectable = event_handler
        .as_selectable_element()
        .expect("EventHandler always exposes itself as selectable");
    assert!(selectable
        .get_selection(vec2f(0.0, 0.0), vec2f(10.0, 0.0), IsRect::False)
        .is_none());
}
