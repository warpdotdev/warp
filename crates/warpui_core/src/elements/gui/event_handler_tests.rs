use std::collections::HashMap;
use std::rc::Rc;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;

use super::*;
use crate::elements::{
    ChildAnchor, ConstrainedBox, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Rect, SelectableArea, Selection, SelectionFragment, SelectionHandle,
    SmartSelectFn, Stack,
};
use crate::platform::WindowStyle;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{IsRect, SelectionDirection, SelectionType};
use crate::{
    App, AppContext, Entity, EntityId, EntityIdSet, Presenter, TypedActionView, ViewContext,
    WindowInvalidation,
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
enum ElementIdentifier {
    Base,
    Inset,
    Overlay,
}

#[test]
fn test_right_mouse_down_with_shift_reports_modifier() {
    App::test((), |mut app| async move {
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let mut presenter = Presenter::new(window_id);
        let mut updated = EntityIdSet::default();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(100., 100.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            for shift in [false, true] {
                ctx.simulate_window_event(
                    Event::RightMouseDown {
                        position: vec2f(10., 10.),
                        cmd: false,
                        shift,
                        click_count: 1,
                    },
                    window_id,
                    presenter.clone(),
                );
            }
        });

        view.read(&app, |view, _| {
            assert_eq!(view.right_click_shifts, vec![false, true]);
        });
    });
}

#[derive(Default)]
struct View {
    // Maps identifier to number of mouse down events
    mouse_downs: HashMap<ElementIdentifier, usize>,
    mouse_ins: HashMap<ElementIdentifier, usize>,
    right_click_shifts: Vec<bool>,
    mouse_in_behavior: MouseInBehavior,
}

pub fn init(app: &mut AppContext) {
    app.add_action("event_handler_test:mouse_down", View::mouse_down);
    app.add_action("event_handler_test:mouse_in", View::mouse_in);
    app.add_action(
        "event_handler_test:right_click_shift",
        View::record_right_click_shift,
    );
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

    fn record_right_click_shift(&mut self, shift: &bool, _: &mut ViewContext<Self>) -> bool {
        self.right_click_shifts.push(*shift);
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
                .on_right_mouse_down(|evt, _, _, modifiers| {
                    evt.dispatch_action("event_handler_test:right_click_shift", modifiers.shift);
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

        let mut updated = EntityIdSet::default();
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

        let mut updated = EntityIdSet::default();
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

        let mut updated = EntityIdSet::default();
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

        let mut updated = EntityIdSet::default();
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
    let mut updated = EntityIdSet::default();
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

/// A fixed-size leaf that implements [`SelectableElement`] by reporting the
/// absolute x-coordinates it was asked to select, so a drag-selection test can
/// assert on them without depending on real font/glyph layout.
#[derive(Default)]
struct RealPathSelectableLeaf {
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl Element for RealPathSelectableLeaf {
    fn layout(
        &mut self,
        _constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = vec2f(200., 50.);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
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

impl SelectableElement for RealPathSelectableLeaf {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        _is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        Some(vec![SelectionFragment {
            text: format!("{:.0}..{:.0}", selection_start.x(), selection_end.x()),
            origin: self
                .origin
                .expect("origin should be set by paint before selection"),
        }])
    }

    fn expand_selection(
        &self,
        _point: Vector2F,
        _direction: SelectionDirection,
        _unit: SelectionType,
        _word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        None
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        Some(absolute_point.x() < absolute_point_other.x())
    }

    fn smart_select(
        &self,
        _absolute_point: Vector2F,
        _smart_select_fn: SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        None
    }

    fn calculate_clickable_bounds(&self, _current_selection: Option<Selection>) -> Vec<RectF> {
        Vec::new()
    }
}

#[derive(Default)]
struct SelectionResultView {
    last_selection: Option<String>,
}

fn init_selection_result(app: &mut AppContext) {
    app.add_action(
        "event_handler_test:selection_updated",
        SelectionResultView::store_selection,
    );
}

impl SelectionResultView {
    fn store_selection(&mut self, selection: &Option<String>, _: &mut ViewContext<Self>) -> bool {
        self.last_selection = selection.clone();
        true
    }
}

impl Entity for SelectionResultView {
    type Event = ();
}

impl crate::core::View for SelectionResultView {
    fn ui_name() -> &'static str {
        "event_handler_test_selection_result_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        SelectableArea::new(
            SelectionHandle::default(),
            |args, evt, _| {
                evt.dispatch_action(
                    "event_handler_test:selection_updated",
                    args.selection.clone(),
                );
            },
            EventHandler::new(Box::new(RealPathSelectableLeaf::default())).finish(),
        )
        .finish()
    }
}

impl TypedActionView for SelectionResultView {
    type Action = ();
}

/// Regression test for APP-5361. Drives the real `SelectableArea -> EventHandler ->
/// <leaf>` tree through actual mouse-event dispatch. Fails on the parent revision,
/// since `SelectableArea::on_mouse_down` bails out as soon as
/// `self.child.as_selectable_element()` returns `None`.
#[test]
fn test_drag_selection_through_event_handler_selects_leaf_text() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init_selection_result);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            SelectionResultView::default()
        });

        let mut presenter = Presenter::new(window_id);
        let mut updated = EntityIdSet::default();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(300., 300.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

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
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(150., 10.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(150., 10.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _| {
            assert_eq!(view.last_selection.as_deref(), Some("10..150"));
        });
    });
}

#[derive(Default)]
struct ScrollWheelView {
    scroll_wheel_fired: usize,
}

fn init_scroll_wheel_view(app: &mut AppContext) {
    app.add_action(
        "event_handler_test:scroll_wheel",
        ScrollWheelView::on_scroll_wheel,
    );
}

impl ScrollWheelView {
    fn on_scroll_wheel(&mut self, _: &(), _: &mut ViewContext<Self>) -> bool {
        self.scroll_wheel_fired += 1;
        true
    }
}

impl Entity for ScrollWheelView {
    type Event = ();
}

impl crate::core::View for ScrollWheelView {
    fn ui_name() -> &'static str {
        "event_handler_test_scroll_wheel_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        EventHandler::new(
            ConstrainedBox::new(Rect::new().finish())
                .with_height(100.)
                .with_width(100.)
                .finish(),
        )
        .on_scroll_wheel(|evt, _, _, _| {
            evt.dispatch_action("event_handler_test:scroll_wheel", ());
            DispatchEventResult::PropagateToParent
        })
        .finish()
    }
}

impl TypedActionView for ScrollWheelView {
    type Action = ();
}

/// Regression test for the `render_scrollable_collapsible_content` auto-scroll-unpin
/// behavior: `EventHandler::on_scroll_wheel` must still fire on a `ScrollWheel` event,
/// unaffected by adding the `SelectableElement` forwarding in this PR.
#[test]
fn test_scroll_wheel_handler_still_dispatches() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init_scroll_wheel_view);
        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, |_| ScrollWheelView::default());

        let mut presenter = Presenter::new(window_id);
        let mut updated = EntityIdSet::default();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(100., 100.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: vec2f(10., 10.),
                    delta: vec2f(0., -5.),
                    precise: false,
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _| {
            assert_eq!(view.scroll_wheel_fired, 1);
        });
    });
}
