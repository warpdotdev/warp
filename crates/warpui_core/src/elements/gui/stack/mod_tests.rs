use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use itertools::Itertools;
use pathfinder_geometry::rect::RectF;

use super::*;
use crate::r#async::Timer;
use crate::elements::{
    Clipped, ConstrainedBox, DispatchEventResult, EventHandler, Hoverable, MouseState,
    MouseStateHandle, ParentElement, Rect, TooltipState, ZIndex,
};
use crate::platform::WindowStyle;
use crate::{
    App, AppContext, Entity, EntityIdSet, Event, Presenter, Scene, TypedActionView, ViewContext,
    ViewHandle, WindowId, WindowInvalidation,
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

        let mut updated = EntityIdSet::default();
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
            updated: EntityIdSet::from_iter([app
                .root_view_id(window_id)
                .expect("Root view must exist")]),
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

/// Size of the base (in-flow) child in [`HoverOverlayTooltipView`].
fn tooltip_base_size() -> Vector2F {
    vec2f(80., 60.)
}
/// Size of the floating tooltip child in [`HoverOverlayTooltipView`].
fn tooltip_overlay_size() -> Vector2F {
    vec2f(120., 20.)
}
/// Vertical gap between the base and the tooltip in [`HoverOverlayTooltipView`].
const TOOLTIP_GAP: f32 = 4.;

/// A view mirroring the `overlay_tool_tip_on_element` composition used by the
/// image alt-text tooltip: a [`Hoverable`] wrapping a [`Stack`] whose only
/// in-flow child is a sized base element, plus — when hovered — a tooltip added
/// via [`Stack::add_positioned_overlay_child`] anchored below the base.
///
/// This is the exact shape produced by
/// `UiBuilder::overlay_tool_tip_on_element` (overlay = true), reduced to plain
/// [`Rect`] children so it needs no theme/appearance setup.
///
/// The whole thing is wrapped in a [`Clipped`] sized to the base, mirroring the
/// editor's viewport clip layer (`RichTextElement::paint` starts a bounded clip
/// layer around all blocks). Because the tooltip is anchored *below* the base —
/// outside this clip — a tooltip painted in-flow would be clipped away, while a
/// true floating overlay escapes the clip. This is what makes the test a real
/// analogue of the editor scenario rather than a bare-Stack check.
struct HoverOverlayTooltipView {
    mouse_state: MouseStateHandle,
}

impl HoverOverlayTooltipView {
    fn new() -> Self {
        Self {
            mouse_state: Default::default(),
        }
    }
}

impl Entity for HoverOverlayTooltipView {
    type Event = String;
}

impl crate::core::View for HoverOverlayTooltipView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let hoverable = Hoverable::new(self.mouse_state.clone(), |state: &MouseState| {
            let base = ConstrainedBox::new(Rect::new().finish())
                .with_width(tooltip_base_size().x())
                .with_height(tooltip_base_size().y())
                .finish();

            let mut stack = Stack::new().with_child(base);

            if state.is_hovered() {
                let tooltip = ConstrainedBox::new(Rect::new().finish())
                    .with_width(tooltip_overlay_size().x())
                    .with_height(tooltip_overlay_size().y())
                    .finish();
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., TOOLTIP_GAP),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }

            stack.finish()
        })
        .finish();

        Clipped::sized(hoverable, tooltip_base_size()).finish()
    }

    fn ui_name() -> &'static str {
        "HoverOverlayTooltipView"
    }
}

impl TypedActionView for HoverOverlayTooltipView {
    type Action = ();
}

/// Collect the bounds of every rect drawn in the scene's normal (in-flow)
/// layers — i.e. everything that establishes document layout, excluding
/// floating overlay layers.
fn normal_layer_rect_bounds(scene: &Scene) -> Vec<RectF> {
    scene
        .normal_layers()
        .flat_map(|layer| layer.rects.iter().map(|r| r.bounds))
        .collect()
}

/// Regression test for the image alt-text tooltip reserving layout space.
///
/// The tooltip must float in an overlay layer and leave the in-flow content
/// (the base element) untouched. Before the fix this was structurally
/// guaranteed by the [`Stack`] excluding positioned children from its size
/// computation; this test pins that guarantee against the exact composition the
/// image block uses, so a future refactor of the tooltip helper can't
/// regress the "zero layout impact on hover" contract.
#[test]
fn test_overlay_tooltip_does_not_reserve_layout_space_on_hover() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            HoverOverlayTooltipView::new()
        });

        // Frame 1: not hovered. Capture the in-flow layout and confirm no overlay
        // layer exists yet.
        app.update(|ctx| ctx.simulate_render_frame(window_id));
        let (unhovered_bounds, unhovered_overlay_count) = read_scene(app, window_id, |scene| {
            (normal_layer_rect_bounds(scene), scene.overlay_layer_count())
        });
        assert_eq!(
            unhovered_overlay_count, 0,
            "No overlay layer should exist before hover"
        );
        assert!(
            unhovered_bounds.contains(&RectF::new(Vector2F::zero(), tooltip_base_size())),
            "Base element should be laid out at its natural size, got {unhovered_bounds:?}"
        );

        // Move the mouse over the base element to trigger hover, then render the
        // next frame so the hovered element tree is laid out and painted.
        let presenter = app
            .presenter(window_id)
            .expect("Test window should have a presenter after the first frame.");
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: vec2f(10., 10.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter,
            );
        });
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        // Frame 2: hovered. The tooltip must appear in an overlay layer, and the
        // normal (in-flow) layout must be byte-for-byte identical to the
        // unhovered frame — the tooltip reserves no document-flow space.
        let (hovered_bounds, hovered_overlay_count, overlay_bounds) =
            read_scene(app, window_id, |scene| {
                let overlay_bounds: Vec<RectF> = scene
                    .layers()
                    .skip(scene.layer_count() - scene.overlay_layer_count())
                    .flat_map(|layer| layer.rects.iter().map(|r| r.bounds))
                    .collect();
                (
                    normal_layer_rect_bounds(scene),
                    scene.overlay_layer_count(),
                    overlay_bounds,
                )
            });

        assert_eq!(
            hovered_overlay_count, 1,
            "Hovering should add exactly one floating overlay layer for the tooltip"
        );
        assert_eq!(
            hovered_bounds, unhovered_bounds,
            "In-flow layout must be unchanged by hover: the tooltip must not \
             reserve document-flow space"
        );

        // The tooltip itself must live in the overlay layer, anchored below the
        // base (base bottom = 60, plus the 4px gap = y 64), never in the normal
        // layers.
        let expected_tooltip = RectF::new(
            vec2f(0., tooltip_base_size().y() + TOOLTIP_GAP),
            tooltip_overlay_size(),
        );
        assert!(
            overlay_bounds.contains(&expected_tooltip),
            "Tooltip should float in the overlay layer at {expected_tooltip:?}, \
             got overlay rects {overlay_bounds:?}"
        );
    });
}

/// Below-right nudge applied to the pointer in [`HoverPointerTooltipView`],
/// mirroring the image alt-text tooltip's `TOOLTIP_POINTER_OFFSET`.
fn tooltip_pointer_offset() -> Vector2F {
    vec2f(12., 16.)
}

/// A view mirroring the `overlay_tool_tip_at_pointer` composition used by the
/// image alt-text tooltip: a [`Hoverable`] wrapping a [`Stack`] whose in-flow
/// child is a sized base element, plus — when hovered — a tooltip added via
/// [`Stack::add_positioned_overlay_child`] anchored at the *pointer* rather than
/// the base's rect. The pointer-relative offset comes from
/// [`MouseState::hover_position`], reduced to plain [`Rect`] children so it needs
/// no theme/appearance setup.
struct HoverPointerTooltipView {
    mouse_state: MouseStateHandle,
}

impl HoverPointerTooltipView {
    fn new() -> Self {
        Self {
            mouse_state: Default::default(),
        }
    }
}

impl Entity for HoverPointerTooltipView {
    type Event = String;
}

impl crate::core::View for HoverPointerTooltipView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        Hoverable::new(self.mouse_state.clone(), |state: &MouseState| {
            let base = ConstrainedBox::new(Rect::new().finish())
                .with_width(tooltip_base_size().x())
                .with_height(tooltip_base_size().y())
                .finish();

            let mut stack = Stack::new().with_child(base);

            if state.is_hovered() {
                let tooltip = ConstrainedBox::new(Rect::new().finish())
                    .with_width(tooltip_overlay_size().x())
                    .with_height(tooltip_overlay_size().y())
                    .finish();
                // Position at the captured pointer offset (plus the nudge),
                // exactly as `overlay_tool_tip_at_pointer` does.
                let offset = state
                    .hover_position()
                    .map(|pointer| pointer + tooltip_pointer_offset())
                    .unwrap_or_default();
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        offset,
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }

            stack.finish()
        })
        .finish()
    }

    fn ui_name() -> &'static str {
        "HoverPointerTooltipView"
    }
}

impl TypedActionView for HoverPointerTooltipView {
    type Action = ();
}

/// Pins that the pointer-anchored tooltip lands at the mouse coordinate, not at
/// the element's rect.
///
/// The base sits at the window origin, so `MouseState::hover_position` (the
/// cursor relative to the element's origin) equals the raw cursor position. A
/// tooltip anchored at that position must appear at `cursor + nudge` — distinct
/// from where an element-rect-anchored tooltip (below the base's bottom edge)
/// would sit. This locks the "derives from mouse coords" contract against a
/// regression back to element-anchoring.
#[test]
fn test_overlay_tooltip_positions_at_mouse_pointer() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            HoverPointerTooltipView::new()
        });

        // Frame 1: lay the view out so the Hoverable records a paint origin.
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        // Hover at a point that is inside the base but not at its corner, so the
        // pointer coordinate is unambiguously distinct from any element-rect
        // anchor (top/bottom-left corners).
        let cursor = vec2f(30., 25.);
        let presenter = app
            .presenter(window_id)
            .expect("Test window should have a presenter after the first frame.");
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::MouseMoved {
                    position: cursor,
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                },
                window_id,
                presenter,
            );
        });
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        // The tooltip must float in the overlay layer at the cursor plus the
        // below-right nudge — not at the base's bottom-left.
        let overlay_bounds = read_scene(app, window_id, |scene| {
            scene
                .layers()
                .skip(scene.layer_count() - scene.overlay_layer_count())
                .flat_map(|layer| layer.rects.iter().map(|r| r.bounds))
                .collect::<Vec<RectF>>()
        });

        let expected_tooltip =
            RectF::new(cursor + tooltip_pointer_offset(), tooltip_overlay_size());
        assert!(
            overlay_bounds.contains(&expected_tooltip),
            "Tooltip should float at the pointer position {expected_tooltip:?}, \
             got overlay rects {overlay_bounds:?}"
        );

        // Guard against silent regression to element-rect anchoring: the
        // element-anchored position (base bottom-left + gap) must NOT be where
        // the tooltip landed.
        let element_anchored = RectF::new(
            vec2f(0., tooltip_base_size().y() + TOOLTIP_GAP),
            tooltip_overlay_size(),
        );
        assert!(
            !overlay_bounds.contains(&element_anchored),
            "Tooltip must not be anchored to the element rect at {element_anchored:?}; \
             it should track the pointer"
        );
    });
}

/// Delay `D` used by the hysteresis scene tests below. Short enough to keep the
/// real-time waits brief, long enough to be reliably observable across a frame.
const HYSTERESIS_DELAY: std::time::Duration = std::time::Duration::from_millis(120);
/// Jitter tolerance (px) for the hysteresis scene tests.
const HYSTERESIS_JITTER: f32 = 3.0;

/// A view mirroring the real `overlay_tool_tip_at_pointer` composition end to
/// end: a [`Hoverable`] opted into browser-`title` tooltip hysteresis via
/// [`Hoverable::with_pointer_hysteresis`], whose build closure shows a
/// pointer-anchored overlay tooltip iff the machine reports
/// [`TooltipState::Visible`] — exactly as the image alt-text builder does,
/// reduced to plain [`Rect`] children so it needs no theme/appearance setup.
///
/// Unlike [`HoverPointerTooltipView`] (which reads `hover_position` directly and
/// so only exercises the Stack overlay primitive), this drives the actual
/// hysteresis wiring: intra-element moves, timer-armed show/dismiss, and the
/// redraw re-dispatch that resolves matured deadlines.
struct HysteresisTooltipView {
    mouse_state: MouseStateHandle,
}

impl HysteresisTooltipView {
    fn new() -> Self {
        Self {
            mouse_state: Default::default(),
        }
    }
}

impl Entity for HysteresisTooltipView {
    type Event = String;
}

impl crate::core::View for HysteresisTooltipView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        Hoverable::new(self.mouse_state.clone(), |state: &MouseState| {
            let base = ConstrainedBox::new(Rect::new().finish())
                .with_width(tooltip_base_size().x())
                .with_height(tooltip_base_size().y())
                .finish();

            let mut stack = Stack::new().with_child(base);

            // The fade machine reports `Visible` at any opacity > 0. This
            // reduced view ignores the opacity (its plain `Rect` child carries no
            // color to fade); the real builder scales the tooltip's colors by it.
            // What this scene test pins is presence/position/relocation of the
            // overlay, which are governed by `at`.
            if let Some(TooltipState::Visible { at, .. }) = state.tooltip_hysteresis_state() {
                let tooltip = ConstrainedBox::new(Rect::new().finish())
                    .with_width(tooltip_overlay_size().x())
                    .with_height(tooltip_overlay_size().y())
                    .finish();
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        at + tooltip_pointer_offset(),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }

            stack.finish()
        })
        .with_pointer_hysteresis(HYSTERESIS_DELAY, HYSTERESIS_JITTER)
        .finish()
    }

    fn ui_name() -> &'static str {
        "HysteresisTooltipView"
    }
}

impl TypedActionView for HysteresisTooltipView {
    type Action = ();
}

/// Bounds of every rect in the scene's floating overlay layers.
fn overlay_layer_rect_bounds(scene: &Scene) -> Vec<RectF> {
    scene
        .layers()
        .skip(scene.layer_count() - scene.overlay_layer_count())
        .flat_map(|layer| layer.rects.iter().map(|r| r.bounds))
        .collect()
}

/// Dispatch a real (non-synthetic) mouse move and record it as the window's last
/// move, so the timer-driven redraw re-dispatches it as a synthetic move — the
/// mechanism that lets a matured show/dismiss deadline resolve on the next frame.
fn move_mouse(app: &mut App, window_id: WindowId, position: Vector2F) {
    let presenter = app
        .presenter(window_id)
        .expect("Test window should have a presenter after the first frame.");
    let event = Event::MouseMoved {
        position,
        cmd: false,
        shift: false,
        is_synthetic: false,
    };
    app.update(|ctx| {
        ctx.simulate_window_event(event.clone(), window_id, presenter);
        ctx.set_last_mouse_move_event(window_id, event);
    });
}

/// Fade spec point 1–2: the tooltip is fully faded out before the pointer is
/// ever seen (no rest sample), and fully faded in at the pointer position once
/// the pointer has rested for `D`. Drives the real fade wiring (Hoverable arms a
/// per-frame re-sample timer → redraw re-dispatches the held move → the machine
/// reads it as rest and advances the fade), not just the pure state machine.
///
/// This reduced view carries no color on its `Rect` child, so it cannot observe
/// *opacity* — it treats any opacity > 0 as "overlay present". It therefore pins
/// what it can see at the scene level: no overlay before any pointer sample, and
/// the overlay present at the settled position after resting for `D`. The
/// intermediate fade opacities (fade-in curve, `p·D` fade-out, reversal) are
/// covered precisely by the `tooltip_hysteresis` state-machine tests, which
/// control the clock and can read the opacity directly. Note that because the
/// render frame re-dispatches the last move, the machine sees the resting
/// pointer within a frame and the fade-in begins right away — so this scene test
/// asserts the *end* of the fade-in, not a "still hidden mid-delay" checkpoint.
#[test]
fn test_hysteresis_tooltip_shows_at_pointer_after_rest_delay() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, _view) =
            app.add_window(WindowStyle::NotStealFocus, |_| HysteresisTooltipView::new());
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        // Before any pointer sample, the tooltip is fully faded out.
        let overlay = read_scene(app, window_id, overlay_layer_rect_bounds);
        assert!(
            overlay.is_empty(),
            "tooltip must be fully faded out before the pointer is seen; got {overlay:?}"
        );

        // Rest the pointer, then wait out the fade-in; the per-frame re-sample
        // timer re-dispatches the held move (read as rest), driving the opacity up
        // to full over `D`.
        let rest = vec2f(30., 25.);
        move_mouse(app, window_id, rest);
        Timer::after(HYSTERESIS_DELAY * 3).await;
        app.update(|ctx| ctx.simulate_render_frame(window_id));
        let overlay = read_scene(app, window_id, overlay_layer_rect_bounds);
        let expected = RectF::new(rest + tooltip_pointer_offset(), tooltip_overlay_size());
        assert!(
            overlay.contains(&expected),
            "tooltip should be faded in at the resting pointer {expected:?}; got {overlay:?}"
        );
    });
}

/// Fade spec point 4 (exit): leaving the element fades the tooltip out (at the
/// faster exit rate) until it is fully gone, driven through the real wiring.
/// Unlike an instant snap, the fade takes `D / EXIT_FADE_RATE_MULTIPLIER`; this
/// test waits that fade out and confirms the overlay is gone once it settles.
#[test]
fn test_hysteresis_tooltip_fades_out_on_exit() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, _view) =
            app.add_window(WindowStyle::NotStealFocus, |_| HysteresisTooltipView::new());
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        // Rest and fade in.
        let rest = vec2f(30., 25.);
        move_mouse(app, window_id, rest);
        Timer::after(HYSTERESIS_DELAY * 3).await;
        app.update(|ctx| ctx.simulate_render_frame(window_id));
        assert!(
            !read_scene(app, window_id, overlay_layer_rect_bounds).is_empty(),
            "precondition: tooltip visible after resting"
        );

        // Move well outside the base (base is 80x60 at the origin) and wait out
        // the exit fade. The per-frame re-sample timer re-dispatches the held
        // outside position (read as "still gone"), driving opacity down to 0.
        move_mouse(app, window_id, vec2f(500., 500.));
        Timer::after(HYSTERESIS_DELAY * 3).await;
        app.update(|ctx| ctx.simulate_render_frame(window_id));
        let overlay = read_scene(app, window_id, overlay_layer_rect_bounds);
        assert!(
            overlay.is_empty(),
            "tooltip must fully fade out after leaving the element; got {overlay:?}"
        );
    });
}

/// Single-instance relocation (fade spec point 3): moving to a new rest spot
/// does not slide or crossfade a second instance — the tooltip at the old spot
/// fades out first, then fades in at the new spot. Driven through the real
/// wiring, this test pins the *endpoint*: after resting at a new spot long
/// enough for the old to fade out and the new to fade in, the overlay is at the
/// new spot and not the old.
#[test]
fn test_hysteresis_tooltip_relocates_to_new_rest_spot() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.update(init);
        let (window_id, _view) =
            app.add_window(WindowStyle::NotStealFocus, |_| HysteresisTooltipView::new());
        app.update(|ctx| ctx.simulate_render_frame(window_id));

        // Rest and fade in at the first spot. Both spots are kept in the top strip
        // of the base (y small) so the pointer never lands *under* a visible
        // tooltip (which floats below-right at +16 in y) — a pointer covered by
        // its own tooltip reads as "left the element" and would dismiss, an
        // orthogonal hit-testing concern noted in the deliverable.
        let first = vec2f(15., 8.);
        move_mouse(app, window_id, first);
        Timer::after(HYSTERESIS_DELAY * 3).await;
        app.update(|ctx| ctx.simulate_render_frame(window_id));
        let overlay = read_scene(app, window_id, overlay_layer_rect_bounds);
        assert!(
            overlay.contains(&RectF::new(
                first + tooltip_pointer_offset(),
                tooltip_overlay_size()
            )),
            "precondition: tooltip visible at first rest spot; got {overlay:?}"
        );

        // Move to a new spot and hold still there. Under the single-instance
        // model the old tooltip finishes fading out at `first`, then a fresh
        // fade-in captures `second`. Wait out both fades (each ≤ D), holding the
        // pointer still at `second` so the re-dispatch keeps it "at rest".
        let second = vec2f(55., 8.);
        move_mouse(app, window_id, second);
        // Several settle waits, re-asserting the held position each time so the
        // machine sees continuous rest at `second` across the fade-out→fade-in.
        for _ in 0..3 {
            Timer::after(HYSTERESIS_DELAY).await;
            move_mouse(app, window_id, second);
            app.update(|ctx| ctx.simulate_render_frame(window_id));
        }
        let overlay = read_scene(app, window_id, overlay_layer_rect_bounds);
        assert!(
            overlay.contains(&RectF::new(
                second + tooltip_pointer_offset(),
                tooltip_overlay_size()
            )),
            "tooltip should end up faded in at the new rest spot; got {overlay:?}"
        );
        assert!(
            !overlay.contains(&RectF::new(
                first + tooltip_pointer_offset(),
                tooltip_overlay_size()
            )),
            "tooltip should no longer be at the old spot; got {overlay:?}"
        );
    });
}

/// Read the last-rendered scene for `window_id` and project it with `f`.
fn read_scene<T>(app: &mut App, window_id: WindowId, f: impl FnOnce(&Scene) -> T) -> T {
    let presenter_ref = app
        .presenter(window_id)
        .expect("Test window should have a presenter since a frame was rendered.");
    let presenter = presenter_ref.borrow();
    let scene = presenter
        .scene()
        .expect("Presenter should have rendered a scene.");
    f(scene)
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

    let mut updated = EntityIdSet::default();
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
