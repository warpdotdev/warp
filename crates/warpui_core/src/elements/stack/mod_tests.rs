use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use itertools::Itertools;
use pathfinder_geometry::rect::RectF;

use super::*;

use crate::{
    elements::{Clipped, DispatchEventResult},
    platform::WindowStyle,
    TypedActionView,
};
use crate::{
    elements::{ConstrainedBox, EventHandler, ParentElement, Rect, ZIndex},
    App, AppContext, Entity, Event, Presenter, ViewContext, ViewHandle, WindowId,
    WindowInvalidation,
};

#[derive(Default)]
struct View {
    // maps view id to number of mouse downs
    mouse_downs: HashMap<usize, u32>,
    mouse_ups: HashMap<usize, u32>,
    mouse_dragged: HashMap<usize, u32>,
}

pub fn init(app: &mut AppContext) {
    app.add_action("test_view:mouse_down", View::mouse_down);
    app.add_action("test_view:mouse_up", View::mouse_up);
    app.add_action("test_view:mouse_dragged", View::mouse_dragged);
}

impl View {
    fn mouse_down(&mut self, view_id: &usize, _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_down on view_id {view_id}");
        let entry = self.mouse_downs.entry(*view_id).or_insert(0);
        *entry += 1;
        true
    }

    fn mouse_up(&mut self, view_id: &usize, _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_up on view_id {view_id}");
        let entry = self.mouse_ups.entry(*view_id).or_insert(0);
        *entry += 1;
        true
    }

    fn mouse_dragged(&mut self, view_id: &usize, _ctx: &mut ViewContext<Self>) -> bool {
        log::info!("Recording mouse_dragged on view_id {view_id}");
        let entry = self.mouse_dragged.entry(*view_id).or_insert(0);
        *entry += 1;
        true
    }
}

impl TypedActionView for View {
    type Action = ();
}

impl Entity for View {
    type Event = String;
}

impl crate::core::View for View {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut s = Stack::new();
        s.add_child(
            EventHandler::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(50.)
                    .with_width(50.)
                    .finish(),
            )
            .on_left_mouse_down(|evt_ctx, _ctx, _position| {
                evt_ctx.dispatch_action("test_view:mouse_down", 0usize);
                DispatchEventResult::StopPropagation
            })
            .on_left_mouse_up(|evt_ctx, _ctx, _position| {
                evt_ctx.dispatch_action("test_view:mouse_up", 0usize);
                DispatchEventResult::StopPropagation
            })
            .on_mouse_dragged(|evt_ctx, _ctx, _position| {
                evt_ctx.dispatch_action("test_view:mouse_dragged", 0usize);
                DispatchEventResult::StopPropagation
            })
            .finish(),
        );
        s.add_child(
            Positioned::new(
                EventHandler::new(
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(50.)
                        .with_width(50.)
                        .finish(),
                )
                .on_left_mouse_down(|evt_ctx, _ctx, _position| {
                    evt_ctx.dispatch_action("test_view:mouse_down", 1usize);
                    DispatchEventResult::StopPropagation
                })
                .on_left_mouse_up(|evt_ctx, _ctx, _position| {
                    evt_ctx.dispatch_action("test_view:mouse_up", 1usize);
                    DispatchEventResult::StopPropagation
                })
                .on_mouse_dragged(|evt_ctx, _ctx, _position| {
                    evt_ctx.dispatch_action("test_view:mouse_dragged", 1usize);
                    DispatchEventResult::StopPropagation
                })
                .finish(),
            )
            .with_offset(OffsetPositioning::offset_from_parent(
                vec2f(25., 25.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ))
            .finish(),
        );
        s.add_child(
            Positioned::new(
                Clipped::sized(
                    EventHandler::new(
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(50.)
                            .with_width(50.)
                            .finish(),
                    )
                    .on_left_mouse_down(|evt_ctx, _ctx, _position| {
                        evt_ctx.dispatch_action("test_view:mouse_down", 2usize);
                        DispatchEventResult::StopPropagation
                    })
                    .on_left_mouse_up(|evt_ctx, _ctx, _position| {
                        evt_ctx.dispatch_action("test_view:mouse_up", 2usize);
                        DispatchEventResult::StopPropagation
                    })
                    .on_mouse_dragged(|evt_ctx, _ctx, _position| {
                        evt_ctx.dispatch_action("test_view:mouse_dragged", 2usize);
                        DispatchEventResult::StopPropagation
                    })
                    .finish(),
                    vec2f(25., 25.),
                )
                .finish(),
            )
            .with_offset(OffsetPositioning::offset_from_parent(
                vec2f(100., 100.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ))
            .finish(),
        );
        s.finish()
    }

    fn ui_name() -> &'static str {
        "View"
    }
}

const FIRST_CHILD_POSITION_ID: &str = "RelativePositionedView::first_child_position_id";

/// A view for testing that renders the second child in a stack based on what's specified in
/// `second_child_positioning`.
#[derive(Default)]
struct RelativePositionedView {
    second_child_positioning: Option<OffsetPositioning>,
    second_child_size: Option<Vector2F>,
}

impl RelativePositionedView {
    fn new() -> Self {
        Self {
            second_child_positioning: None,
            second_child_size: None,
        }
    }

    fn first_child_position_id() -> &'static str {
        FIRST_CHILD_POSITION_ID
    }
}

impl Entity for RelativePositionedView {
    type Event = String;
}

impl crate::core::View for RelativePositionedView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let mut s = Stack::new();
        s.add_child(
            SavePosition::new(
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(50.)
                    .with_width(50.)
                    .finish(),
                FIRST_CHILD_POSITION_ID,
            )
            .finish(),
        );

        if let Some(second_child_positioning) = &self.second_child_positioning {
            s.add_child(
                Positioned::new(if let Some(second_child_size) = &self.second_child_size {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_width(second_child_size.x())
                        .with_height(second_child_size.y())
                        .finish()
                } else {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(50.)
                        .with_width(50.)
                        .finish()
                })
                .with_offset(second_child_positioning.clone())
                .finish(),
            );
        }

        // Force the Stack to take up the full size of the window by pulling
        // the minimum size constraint up to the size of the window.
        ConstrainedBox::new(s.finish())
            .with_min_width(f32::MAX)
            .with_min_height(f32::MAX)
            .finish()
    }

    fn ui_name() -> &'static str {
        "View"
    }
}

impl TypedActionView for RelativePositionedView {
    type Action = ();
}

#[test]
fn test_paint_sets_z_index() {
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
            let scene = presenter.build_scene(vec2f(300., 300.), 1., None, ctx);
            assert_eq!(scene.z_index(), ZIndex::new(0));
            assert_eq!(scene.layer_count(), 5);
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
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(15., 15.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Fire event on second child
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
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(30., 30.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(30., 30.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Fire event on third child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(120., 120.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(120., 120.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(120., 120.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );

            // Fire event on clipped part of third child
            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: vec2f(140., 140.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseUp {
                    position: vec2f(140., 140.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: vec2f(140., 140.),
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        view.read(app, |view, _ctx| {
            assert_eq!(1, *view.mouse_downs.get(&0).unwrap());
            assert_eq!(1, *view.mouse_downs.get(&1).unwrap());
            assert_eq!(1, *view.mouse_downs.get(&2).unwrap());
            assert_eq!(1, *view.mouse_ups.get(&0).unwrap());
            assert_eq!(1, *view.mouse_ups.get(&1).unwrap());
            assert_eq!(1, *view.mouse_ups.get(&2).unwrap());
            assert_eq!(1, *view.mouse_dragged.get(&0).unwrap());
            assert_eq!(1, *view.mouse_dragged.get(&1).unwrap());
            assert_eq!(1, *view.mouse_dragged.get(&2).unwrap());
        });
    })
}

#[test]
fn test_relative_positioning() {
    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            RelativePositionedView::new()
        });

        position_child_and_assert_location(
            OffsetPositioning::offset_from_save_position_element(
                RelativePositionedView::first_child_position_id(),
                vec2f(25., 25.),
                PositionedElementOffsetBounds::Unbounded,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
            RectF::new(vec2f(25., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Update the view to position the top right of the child offset from the top right of
        // the parent (this should mean part of the child is clipped offscreen on the left).
        position_child_and_assert_location(
            OffsetPositioning::offset_from_save_position_element(
                RelativePositionedView::first_child_position_id(),
                vec2f(25., 25.),
                PositionedElementOffsetBounds::Unbounded,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::TopRight,
            ),
            RectF::new(vec2f(-25., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Offset with the same position, but bound horizontally to the parent so the element is
        // no longer clipped past the left side of the screen.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::ParentByPosition,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(0., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Now just bound vertically to the parent. This should not change the positioning since
        // the element is already bound vertically within the parent.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::ParentByPosition,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(-25., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Update the view to position the top left of the child offset from the top right of the
        // parent.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(25.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(75., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Now, bound vertically with the parent--this should have no effect here since the
        // child is fully contained within its parent.
        let new_positioning = OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                RelativePositionedView::first_child_position_id(),
                PositionedElementOffsetBounds::Unbounded,
                OffsetType::Pixel(25.),
                AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
            ),
            PositioningAxis::relative_to_stack_child(
                RelativePositionedView::first_child_position_id(),
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(25.),
                AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
            ),
        );

        position_child_and_assert_location(
            new_positioning,
            RectF::new(vec2f(75., 25.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Position the child's bottom right corner on the parent's bottom right corner. With
        // no offset this means they should be stacked directly on top of each other.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Right),
                ),
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Bottom),
                ),
            ),
            RectF::new(vec2f(0., 0.), vec2f(50., 50.)),
            app,
            window_id,
            view.clone(),
        );

        // Align the child vertically from the parent and horizontally from the child.
        position_child_and_assert_location(
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    RelativePositionedView::first_child_position_id(),
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(5.),
                    AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    OffsetType::Pixel(5.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
            RectF::new(vec2f(55., 5.), vec2f(50., 50.)),
            app,
            window_id,
            view,
        );
    })
}

#[test]
fn test_relative_positioning_bound_to_window_by_size() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            RelativePositionedView::new()
        });
        let window_size = view.update(app, |_, ctx| {
            ctx.notify();
            ctx.windows()
                .platform_window(window_id)
                .expect("Window should exist for platform.")
                .size()
        });

        let offset = vec2f(25., 25.);
        let positioning = OffsetPositioning::offset_from_save_position_element(
            RelativePositionedView::first_child_position_id(),
            offset,
            PositionedElementOffsetBounds::WindowBySize,
            PositionedElementAnchor::BottomRight,
            ChildAnchor::TopLeft,
        );
        view.update(app, |view, ctx| {
            view.second_child_positioning = Some(positioning);

            // Set the offset-positioned child's size to the window size so the bounding
            // behavior is actually tested.
            view.second_child_size = Some(window_size);
            ctx.notify();
        });

        // Simulate a render frame to ensure the scene is built.
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        let presenter_ref = app
            .presenter(window_id)
            .expect("Test window should have a presenter since first frame is rendered.");
        let presenter = presenter_ref.borrow();
        let scene = presenter
            .scene()
            .expect("Presenter should have rendered a scene after the view was updated.");

        // The expected bounds should go from the anchor position with offset to the edge of
        // the window bounds. Note the usage of `RectF::from_points`, which specifies top-left
        // and bottom-right coordinates, rather than the default `RectF::new()` constructor.
        let expected_bounds = RectF::from_points(vec2f(75., 75.), window_size);
        assert_eq!(
            scene
                .layers()
                .collect_vec()
                .get(2)
                .unwrap()
                .rects
                .iter()
                .map(|r| { r.bounds })
                .collect::<Vec<_>>(),
            vec![expected_bounds]
        );
    })
}

#[test]
fn test_relative_positioning_bound_to_window_by_position() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            RelativePositionedView::new()
        });
        let window_size = view.update(app, |_, ctx| {
            ctx.notify();
            ctx.windows()
                .platform_window(window_id)
                .expect("Window should exist for platform.")
                .size()
        });

        let offset = vec2f(25., 25.);
        let positioning = OffsetPositioning::offset_from_save_position_element(
            RelativePositionedView::first_child_position_id(),
            offset,
            PositionedElementOffsetBounds::WindowByPosition,
            PositionedElementAnchor::BottomRight,
            ChildAnchor::TopLeft,
        );
        view.update(app, |view, ctx| {
            view.second_child_positioning = Some(positioning);

            // Set the offset-positioned child's size to the window size so the bounding
            // behavior is actually tested.
            view.second_child_size = Some(window_size);
            ctx.notify();
        });

        let presenter_ref = app
            .presenter(window_id)
            .expect("Test window should have a presenter since first frame is rendered.");
        let presenter = presenter_ref.borrow();
        let scene = presenter
            .scene()
            .expect("Presenter should have rendered a scene after the view was updated.");

        // The expected bounds should have a modified position to accommodate the size of the
        // positioned child (it should be moved back to (0,0) from it's 'default' (75, 75).
        //
        // Note the usage of `RectF::from_points`, which specifies top-left
        // and bottom-right coordinates, rather than the default `RectF::new()` constructor.
        let expected_bounds = RectF::from_points(vec2f(0., 0.), window_size);
        assert_eq!(
            scene
                .layers()
                .collect_vec()
                .get(2)
                .unwrap()
                .rects
                .iter()
                .map(|r| { r.bounds })
                .collect::<Vec<_>>(),
            vec![expected_bounds]
        );
    })
}

#[test]
fn test_relative_positioning_bound_to_missing_anchor() {
    App::test((), |mut app| async move {
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| {
            let mut view = RelativePositionedView::new();

            view.second_child_positioning = Some(OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    "nonexistent_anchor",
                    PositionedElementOffsetBounds::WindowBySize,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Middle, XAxisAnchor::Middle),
                )
                .with_conditional_anchor(),
                PositioningAxis::relative_to_stack_child(
                    "nonexistent_anchor",
                    PositionedElementOffsetBounds::WindowBySize,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Middle),
                )
                .with_conditional_anchor(),
            ));

            view
        });

        let mut presenter = Presenter::new(window_id);

        let invalidation = WindowInvalidation {
            updated: HashSet::from([app.root_view_id(window_id).expect("Root view must exist")]),
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);

            let window_size = RectF::new(Vector2F::zero(), vec2f(300., 300.));
            let scene = presenter.build_scene(window_size.size(), 1., None, ctx);

            assert_eq!(scene.z_index(), ZIndex::new(0));
            assert_eq!(scene.layer_count(), 3);

            let stack_layer = scene.layers().nth(2).expect("Should be 3 layers");
            assert!(
                stack_layer.rects.is_empty(),
                "Relative-positioned element should not have been laid out"
            );
            // In addition to the assertion that there's no rect for the second
            // child, this implicitly tests that we don't panic during layout.
        });
    });
}

/// Positions the second child using the positioning and asserts the child is at bounds
/// indicated within `expected_child_bounds`.
fn position_child_and_assert_location(
    positioning: OffsetPositioning,
    expected_child_bounds: RectF,
    app: &mut App,
    window_id: WindowId,
    view: ViewHandle<RelativePositionedView>,
) {
    view.update(app, |view, _| {
        view.second_child_positioning = Some(positioning);
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
        let window_size = RectF::new(Vector2F::zero(), vec2f(300., 300.));
        let scene = presenter.build_scene(window_size.size(), 1., None, ctx);

        assert_eq!(scene.z_index(), ZIndex::new(0));
        assert_eq!(scene.layer_count(), 3);

        assert_eq!(
            scene
                .layers()
                .nth(1)
                .unwrap()
                .rects
                .iter()
                .map(|r| r.bounds)
                .collect::<Vec<_>>(),
            vec![RectF::new(Vector2F::zero(), vec2f(50., 50.))]
        );
        assert_eq!(
            scene
                .layers()
                .nth(2)
                .unwrap()
                .rects
                .iter()
                .map(|r| { r.bounds })
                .collect::<Vec<_>>(),
            vec![expected_child_bounds]
        );
    });
}
