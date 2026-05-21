use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use itertools::Itertools;

use super::*;
use crate::{
    elements::{ClippedScrollStateHandle, ClippedScrollable, DispatchEventResult, Flex},
    platform::WindowStyle,
    TypedActionView,
};
use crate::{
    elements::{ConstrainedBox, EventHandler, ParentElement, Rect, Stack},
    presenter::DispatchedActionKind,
    App, AppContext, Entity, Event, Presenter, ViewContext, WindowInvalidation,
};

/// Since we support scrolling in both vertical and horizontal directions,
/// this macro makes it easier to define tests for both directions. Simply
/// define an axis-agnostic "test function" that is _essentially_ a test
/// but has two main differences:
/// - it isn't decorated with the #[test] macro, and
/// - it takes a `axis: Axis` argument.
///
/// Then, you can use this macro to turn that test function into two real tests
/// (one for each scrollable direction).
macro_rules! define_axis_agnostic_tests {
    ($test_function:ident) => {
        concat_idents::concat_idents!(test_name = $test_function, _, vertical {
            #[test]
            fn test_name() {
                $test_function(Axis::Vertical);
            }
        });

        concat_idents::concat_idents!(test_name = $test_function, _, horizontal {
            #[test]
            fn test_name() {
                $test_function(Axis::Horizontal);
            }
        });
    };
}

fn create_presenter_and_render<F, T>(
    app: &mut App,
    build_root_view: F,
    window_size: Vector2F,
) -> Rc<RefCell<Presenter>>
where
    T: crate::View + TypedActionView,
    F: FnOnce(&mut ViewContext<T>) -> T,
{
    let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, build_root_view);

    let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));

    let mut updated = HashSet::new();
    updated.insert(app.root_view_id(window_id).unwrap());
    let invalidation = WindowInvalidation {
        updated,
        ..Default::default()
    };

    app.update(move |ctx| {
        presenter.borrow_mut().invalidate(invalidation, ctx);
        let _ = presenter
            .borrow_mut()
            .build_scene(window_size, 1., None, ctx);
        presenter
    })
}

struct BasicScrollableView {
    /// [`Axis::Horizontal`] will create a horizontally scrollable view.
    /// [`Axis::Vertical`] will create a verticall scrollable view.
    axis: Axis,
    // maps view id to number of mouse downs
    mouse_downs: HashMap<usize, u32>,
    clipped_scroll_state: ClippedScrollStateHandle,
    scroll_area_size: f32,
    num_elements: usize,
}

pub fn init(app: &mut AppContext) {
    app.add_action("test_view:mouse_down", BasicScrollableView::mouse_down);
}

impl BasicScrollableView {
    const ITEM_SIZE: f32 = 50.;
    const SCROLLBAR_SIZE: ScrollbarWidth = ScrollbarWidth::Auto;

    fn new(axis: Axis, scroll_area_size: f32, num_elements: usize) -> Self {
        Self {
            axis,
            scroll_area_size,
            num_elements,
            clipped_scroll_state: Default::default(),
            mouse_downs: Default::default(),
        }
    }

    fn mouse_down(&mut self, view_id: &usize, _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_down on view_id {view_id}");
        let entry = self.mouse_downs.entry(*view_id).or_insert(0);
        *entry += 1;
        true
    }
}

impl Entity for BasicScrollableView {
    type Event = String;
}

impl crate::core::View for BasicScrollableView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut flex = Flex::new(self.axis);
        for i in 0..self.num_elements {
            let id = i + 1;
            flex.add_child(
                EventHandler::new(
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(50.)
                        .with_width(50.)
                        .finish(),
                )
                .on_left_mouse_down(move |evt_ctx, _ctx, _position| {
                    evt_ctx.dispatch_action("test_view:mouse_down", id);
                    DispatchEventResult::StopPropagation
                })
                .finish(),
            );
        }

        if matches!(self.axis, Axis::Vertical) {
            ConstrainedBox::new(
                ClippedScrollable::vertical(
                    self.clipped_scroll_state.clone(),
                    ConstrainedBox::new(flex.finish())
                        .with_height(self.num_elements as f32 * 50.)
                        .finish(),
                    ScrollbarWidth::Auto,
                    Fill::None,
                    Fill::None,
                    Fill::None,
                )
                .finish(),
            )
            .with_height(self.scroll_area_size)
            .finish()
        } else {
            ConstrainedBox::new(
                ClippedScrollable::horizontal(
                    self.clipped_scroll_state.clone(),
                    ConstrainedBox::new(flex.finish())
                        .with_width(self.num_elements as f32 * 50.)
                        .finish(),
                    ScrollbarWidth::Auto,
                    Fill::None,
                    Fill::None,
                    Fill::None,
                )
                .finish(),
            )
            .with_width(self.scroll_area_size)
            .finish()
        }
    }

    fn ui_name() -> &'static str {
        "View"
    }
}

impl TypedActionView for BasicScrollableView {
    type Action = ();
}

const STACKED_VIEW_LENGTH: f32 = 100.;

/// Similar to [`BasicScrollableView`] except the scrollable
/// view is an element on a [`Stack`].
struct StackedScrollableView {
    axis: Axis,
    clipped_scroll_state: ClippedScrollStateHandle,
}

impl StackedScrollableView {
    fn new(axis: Axis) -> Self {
        Self {
            axis,
            clipped_scroll_state: Default::default(),
        }
    }
}

impl Entity for StackedScrollableView {
    type Event = String;
}

impl crate::core::View for StackedScrollableView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut inner_stack = Stack::new();
        inner_stack.add_child(
            ConstrainedBox::new(Rect::new().finish())
                .with_height(STACKED_VIEW_LENGTH)
                .with_width(STACKED_VIEW_LENGTH)
                .finish(),
        );

        if matches!(self.axis, Axis::Vertical) {
            ConstrainedBox::new(
                ClippedScrollable::vertical(
                    self.clipped_scroll_state.clone(),
                    inner_stack.finish(),
                    ScrollbarWidth::Auto,
                    Fill::None,
                    Fill::None,
                    Fill::None,
                )
                .finish(),
            )
            // Make the scrollable element half as large as the child so that
            // there is something to scroll.
            .with_height(STACKED_VIEW_LENGTH / 2.)
            .finish()
        } else {
            ConstrainedBox::new(
                ClippedScrollable::horizontal(
                    self.clipped_scroll_state.clone(),
                    inner_stack.finish(),
                    ScrollbarWidth::Auto,
                    Fill::None,
                    Fill::None,
                    Fill::None,
                )
                .finish(),
            )
            // Make the scrollable element half as large as the child so that
            // there is something to scroll.
            .with_width(STACKED_VIEW_LENGTH / 2.)
            .finish()
        }
    }

    fn ui_name() -> &'static str {
        "StackedView"
    }
}

impl TypedActionView for StackedScrollableView {
    type Action = ();
}

/// Tests if clipped scrolling works along `axis`.
fn test_clipped_scrolling(axis: Axis) {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(axis, 200., 10)
        });

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        let presenter_clone = presenter.clone();
        app.update(move |ctx| {
            presenter_clone.borrow_mut().invalidate(invalidation, ctx);
            let _ = presenter_clone
                .borrow_mut()
                .build_scene(vec2f(1000., 1000.), 1., None, ctx);

            // Fire event on first child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter_clone.clone(),
            );

            // Trigger a scroll to make the second child be at the start
            // of the visible area
            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: vec2f(15., 15.),
                    delta: -(50_f32.along(axis)),
                    precise: true,
                    modifiers: Default::default(),
                },
                window_id,
                presenter_clone.clone(),
            );
        });

        view.read(app, |view, _ctx| {
            assert_eq!(1, *view.mouse_downs.get(&1).unwrap());
            assert_eq!(None, view.mouse_downs.get(&2));
            assert!(view.clipped_scroll_state.scroll_start() > Pixels::zero());
        });

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        let presenter_clone = presenter.clone();
        app.update(move |ctx| {
            presenter_clone.borrow_mut().invalidate(invalidation, ctx);
            let _ = presenter_clone
                .borrow_mut()
                .build_scene(vec2f(1000., 1000.), 1., None, ctx);

            // Fire event on second child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter_clone.clone(),
            );
        });

        view.read(app, |view, _ctx| {
            assert_eq!(1, *view.mouse_downs.get(&1).unwrap());
            assert_eq!(1, *view.mouse_downs.get(&2).unwrap());
        });

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        let presenter_clone = presenter;
        app.update(move |ctx| {
            presenter_clone.borrow_mut().invalidate(invalidation, ctx);
            let _ = presenter_clone
                .borrow_mut()
                .build_scene(vec2f(1000., 1000.), 1., None, ctx);

            // Trigger a scroll back to the start
            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: vec2f(15., 15.),
                    delta: 50_f32.along(axis),
                    precise: true,
                    modifiers: Default::default(),
                },
                window_id,
                presenter_clone.clone(),
            );
        });

        view.read(app, |view, _ctx| {
            // Make sure scroll start is reset to zero
            assert!(view.clipped_scroll_state.scroll_start().as_f32().abs() < f32::EPSILON);
        });
    })
}

fn test_clipped_scrolling_no_scrollbars(axis: Axis) {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(axis, 500., 10)
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
            let _ = presenter.build_scene(vec2f(1000., 1000.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            // Fire event on first child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Try to trigger a scroll (but there shouldn't actually be one)
            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: vec2f(15., 15.),
                    delta: -(25_f32.along(axis)),
                    precise: true,
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _ctx| {
            assert_eq!(1, *view.mouse_downs.get(&1).unwrap());
            assert!(view.clipped_scroll_state.scroll_start().as_f32().abs() < f32::EPSILON);
        });

        presenter = Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            let _ = presenter.build_scene(vec2f(1000., 1000.), 1., None, ctx);
            let presenter = Rc::new(RefCell::new(presenter));

            // Fire another event on the first child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _ctx| {
            assert_eq!(2, *view.mouse_downs.get(&1).unwrap());
            assert_eq!(None, view.mouse_downs.get(&2));
            assert!(view.clipped_scroll_state.scroll_start().as_f32().abs() < f32::EPSILON);
        });
    })
}

fn test_stacked_view_scroll_handling(axis: Axis) {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            StackedScrollableView::new(axis)
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
            let scene = presenter.build_scene(
                vec2f(STACKED_VIEW_LENGTH, STACKED_VIEW_LENGTH),
                1.,
                None,
                ctx,
            );
            let presenter = Rc::new(RefCell::new(presenter));

            assert_eq!(scene.z_index(), ZIndex::new(0));
            assert_eq!(scene.layer_count(), 3);

            // Try to scroll the stacked element
            ctx.simulate_window_event(
                Event::ScrollWheel {
                    position: vec2f(25., 25.),
                    delta: -(25_f32.along(axis)),
                    precise: true,
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _ctx| {
            assert!(view.clipped_scroll_state.scroll_start() > Pixels::zero());
        });
    })
}

fn test_clicks_in_scrollbar_gutter_change_scroll_position(axis: Axis) {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let scroll_area_size = 200.;
        // This should make the scrollbar thumb half the size of the scrollbar.
        let num_elements =
            (scroll_area_size / BasicScrollableView::ITEM_SIZE * 2.).round() as usize;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            BasicScrollableView::new(axis, scroll_area_size, num_elements)
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
            let _ = presenter
                .borrow_mut()
                .build_scene(vec2f(1000., 1000.), 1., None, ctx);

            // Assert that the scrollable view is scrolled to the start.
            view.read(ctx, |view, _ctx| {
                assert_eq!(view.clipped_scroll_state.scroll_start(), Pixels::zero());
            });

            // Click on the scrollbar gutter (somewhere before the scrollbar thumb).
            let click_position = axis.to_point(
                scroll_area_size - 5.,
                1000. - (BasicScrollableView::SCROLLBAR_SIZE.as_f32() / 2.),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: click_position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Assert that the scrollable view is no longer scrolled to the
            // top.
            view.read(ctx, |view, _ctx| {
                assert_ne!(view.clipped_scroll_state.scroll_start(), Pixels::zero());
            });
        });
    })
}

fn test_ignores_clicks_outside_scrollbar_bounds(axis: Axis) {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);

        let scroll_area_size = 100.;
        // This should make the scrollbar thumb half the size of the scrollbar.
        let num_elements =
            (scroll_area_size / BasicScrollableView::ITEM_SIZE * 2.).round() as usize;

        let window_length = 1000.;
        let window_size = vec2f(window_length, window_length);
        let presenter = create_presenter_and_render(
            app,
            |_| BasicScrollableView::new(axis, scroll_area_size, num_elements),
            window_size,
        );

        app.update(move |ctx| {
                // Define a macro to help us determine which actions would be
                // produced if we dispatched the given event.
                macro_rules! actions_for_dispatched_event {
                    ($event:expr) => {{
                        let result = presenter
                            .borrow_mut()
                            .dispatch_event($event, ctx);
                        result.actions.iter().flat_map(|action| {
                            match action.kind {
                                DispatchedActionKind::Legacy { name, .. } => Some(name),
                                _ => None
                            }
                        }).collect_vec()
                    }};
                }

                let click_point_along_inverse_axis = window_length - (BasicScrollableView::SCROLLBAR_SIZE.as_f32() / 2.);

                // Ensure that a click on the scrollbar thumb is handled.
                let click_position = axis.to_point(5., click_point_along_inverse_axis);
                let actions = actions_for_dispatched_event!(Event::LeftMouseDown {
                    position: click_position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                });
                assert_eq!(actions, vec!["scrollable_click::on_thumb"], "Should handle clicks on the scrollbar thumb");

                // Ensure that a click on the scrollbar gutter (i.e. outside of the thumb) is handled.
                let click_position = axis.to_point(scroll_area_size - 5., click_point_along_inverse_axis);
                let actions = actions_for_dispatched_event!(Event::LeftMouseDown {
                    position: click_position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                });
                assert_eq!(actions, vec!["scrollable_click::on_gutter"], "Should handle clicks on the scrollbar gutter");

                // Ensure that a click event below the scrollbar isn't handled.
                let click_position = axis.to_point(scroll_area_size + 5., click_point_along_inverse_axis);
                let actions = actions_for_dispatched_event!(Event::LeftMouseDown {
                    position: click_position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                });
                assert!(actions.is_empty(), "Should not handle click events that are outside the vertical bounds of the scrollbar");

                // Ensure that a click event to the left of the scrollbar isn't handled.
                let click_position = axis.to_point(scroll_area_size - 5., 100.);
                let actions = actions_for_dispatched_event!(Event::LeftMouseDown {
                    position: click_position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                });
                assert!(actions.is_empty(), "Should not handle click events that are outside the horizontal bounds of the scrollbar");
            });
    })
}

define_axis_agnostic_tests!(test_clipped_scrolling);
define_axis_agnostic_tests!(test_clipped_scrolling_no_scrollbars);
define_axis_agnostic_tests!(test_stacked_view_scroll_handling);
define_axis_agnostic_tests!(test_clicks_in_scrollbar_gutter_change_scroll_position);
define_axis_agnostic_tests!(test_ignores_clicks_outside_scrollbar_bounds);
