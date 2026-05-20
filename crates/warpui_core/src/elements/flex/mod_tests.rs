use std::collections::HashSet;

use super::*;
use crate::elements::{Align, SavePosition, Stack};
use crate::geometry::rect::RectF;

use crate::platform::WindowStyle;
use crate::{
    elements::{ConstrainedBox, ParentElement, Rect},
    App, Entity, Presenter, TypedActionView, WindowId, WindowInvalidation,
};

type RenderFn = dyn Fn(&AppContext) -> Box<dyn Element> + 'static;

struct TestDynamicView {
    render: Box<RenderFn>,
}

impl TestDynamicView {
    fn new(render: impl Fn(&AppContext) -> Box<dyn Element> + 'static) -> Self {
        Self {
            render: Box::new(render),
        }
    }
}

impl Entity for TestDynamicView {
    type Event = ();
}

impl crate::core::View for TestDynamicView {
    fn ui_name() -> &'static str {
        "Flex::tests::TestDynamicView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        (self.render)(app)
    }
}

impl TypedActionView for TestDynamicView {
    type Action = ();
}

/// Asserts that the bounds of all the painted rects match that of `rects`.
fn assert_bounds_of_rects(
    app: &mut App,
    window_id: WindowId,
    rects: impl IntoIterator<Item = RectF>,
) {
    let presenter_ref = app
        .presenter(window_id)
        .expect("Test window should have a presenter since first frame is rendered.");
    let presenter = presenter_ref.borrow();
    let scene = presenter
        .scene()
        .expect("Presenter should have rendered a scene after the test_view was updated.");

    let actual_rects = scene
        .layers()
        .next()
        .into_iter()
        .flat_map(|layer| layer.rects.iter())
        .map(|rect| rect.bounds);

    itertools::assert_equal(actual_rects, rects);
}

struct View {
    flex_main_axis_size: MainAxisSize,
    flex_main_axis_alignment: MainAxisAlignment,
    flex_cross_axis_alignment: CrossAxisAlignment,
}

impl View {
    fn new(
        axis_size: MainAxisSize,
        main_axis_alignment: MainAxisAlignment,
        cross_axis_alignment: CrossAxisAlignment,
    ) -> Self {
        Self {
            flex_main_axis_size: axis_size,
            flex_main_axis_alignment: main_axis_alignment,
            flex_cross_axis_alignment: cross_axis_alignment,
        }
    }
}

impl Entity for View {
    type Event = String;
}

impl crate::core::View for View {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        let flex = Flex::row()
            .with_children([
                SavePosition::new(
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(20.)
                        .with_width(50.)
                        .finish(),
                    "view_1",
                )
                .finish(),
                SavePosition::new(
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(30.)
                        .with_width(50.)
                        .finish(),
                    "view_2",
                )
                .finish(),
                SavePosition::new(
                    Flex::row()
                        .with_child(
                            ConstrainedBox::new(Rect::new().finish())
                                .with_height(50.)
                                .with_width(50.)
                                .finish(),
                        )
                        .finish(),
                    "view_3",
                )
                .finish(),
            ])
            .with_cross_axis_alignment(self.flex_cross_axis_alignment)
            .with_main_axis_alignment(self.flex_main_axis_alignment)
            .with_main_axis_size(self.flex_main_axis_size);

        Stack::new()
            .with_child(
                Align::new(SavePosition::new(flex.finish(), "flex").finish())
                    .top_left()
                    .finish(),
            )
            .finish()
    }

    fn ui_name() -> &'static str {
        "View"
    }
}

impl TypedActionView for View {
    type Action = ();
}

#[test]
fn test_flex_main_axis_alignment() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            View::new(
                MainAxisSize::Min,
                MainAxisAlignment::Start,
                CrossAxisAlignment::Start,
            )
        });

        let mut presenter = Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).expect("root view should exist"));
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation.clone(), ctx);
            let window_size = RectF::new(Vector2F::zero(), vec2f(300., 300.));
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            let view_1_size = vec2f(50., 20.);
            let view_2_size = vec2f(50., 30.);
            let view_3_size = vec2f(50., 50.);

            // The view has a min axis size, so each element should be rendered right next to
            // each other and the flex should take up the total size of the elements.
            assert_eq!(view_1, RectF::new(Vector2F::zero(), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(50., 0.), view_2_size));
            assert_eq!(view_3, RectF::new(vec2f(100., 0.), view_3_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(150., 50.)));

            view.update(ctx, |view, _ctx| {
                view.flex_main_axis_alignment = MainAxisAlignment::Start;
                view.flex_main_axis_size = MainAxisSize::Max;
            });

            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            // The view has a flex axis alignment of start, so ensure that each child element
            // is rendered next to each other, but that the flex expands out to the max size of
            // the window.
            assert_eq!(view_1, RectF::new(Vector2F::zero(), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(50., 0.), view_2_size));
            assert_eq!(view_3, RectF::new(vec2f(100., 0.), view_3_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(300., 50.)));

            view.update(ctx, |view, _ctx| {
                view.flex_main_axis_alignment = MainAxisAlignment::SpaceBetween;
                view.flex_main_axis_size = MainAxisSize::Max;
            });

            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            // Ensure that the elements are evenly spaced (with no extra space at the
            // beginning or end) and that the flex expands out to the max size of the window.
            assert_eq!(view_1, RectF::new(Vector2F::zero(), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(125., 0.), view_2_size));
            assert_eq!(view_3, RectF::new(vec2f(250., 0.), view_3_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(300., 50.)));

            view.update(ctx, |view, _ctx| {
                view.flex_main_axis_alignment = MainAxisAlignment::SpaceEvenly;
                view.flex_main_axis_size = MainAxisSize::Max;
            });

            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            // Ensure that the elements are evenly spaced, including space before and after
            // the child elements, and that the flex expands out to the max size of the window.
            assert_eq!(view_1, RectF::new(vec2f(37.5, 0.), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(125., 0.), view_2_size));
            assert_eq!(view_3, RectF::new(vec2f(212.5, 0.), view_3_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(300., 50.)));

            view.update(ctx, |view, _ctx| {
                view.flex_main_axis_alignment = MainAxisAlignment::End;
                view.flex_main_axis_size = MainAxisSize::Max;
            });

            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            // The view has a flex axis alignment of end, so ensure that each child element
            // is rendered next to each other, but that the flex expands out to the max size of
            // the window.
            assert_eq!(view_3, RectF::new(vec2f(250., 0.), view_3_size));
            assert_eq!(view_2, RectF::new(vec2f(200., 0.), view_2_size));
            assert_eq!(view_1, RectF::new(vec2f(150., 0.), view_1_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(300., 50.)));
        });
    })
}

#[test]
fn test_flex_row_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test basic row spacing
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                Flex::row()
                    .with_spacing(10.)
                    .with_children([
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(30.)
                            .with_width(50.)
                            .finish(),
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(30.)
                            .with_width(60.)
                            .finish(),
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(30.)
                            .with_width(40.)
                            .finish(),
                    ])
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // Children should have 10px spacing between them
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), vec2f(50., 30.)),
                RectF::new(vec2f(60., 0.), vec2f(60., 30.)), // 50 + 10
                RectF::new(vec2f(130., 0.), vec2f(40., 30.)), // 50 + 10 + 60 + 10
            ],
        );
    })
}

#[test]
fn test_flex_column_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test column spacing
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                Flex::column()
                    .with_spacing(15.)
                    .with_children([
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(30.)
                            .with_width(60.)
                            .finish(),
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(40.)
                            .with_width(80.)
                            .finish(),
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(25.)
                            .with_width(70.)
                            .finish(),
                    ])
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // Children should have 15px vertical spacing between them
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), vec2f(60., 30.)),
                RectF::new(vec2f(0., 45.), vec2f(80., 40.)), // 30 + 15
                RectF::new(vec2f(0., 100.), vec2f(70., 25.)), // 30 + 15 + 40 + 15
            ],
        );
    })
}

#[test]
fn test_flex_spacing_with_center_alignment() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test spacing with center alignment
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                ConstrainedBox::new(
                    Flex::row()
                        .with_spacing(20.)
                        .with_children([
                            ConstrainedBox::new(Rect::new().finish())
                                .with_height(30.)
                                .with_width(40.)
                                .finish(),
                            ConstrainedBox::new(Rect::new().finish())
                                .with_height(30.)
                                .with_width(40.)
                                .finish(),
                        ])
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .finish(),
                )
                .with_width(300.)
                .with_height(100.)
                .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // Total content width: 40 + 20 + 40 = 100
        // Remaining space: 300 - 100 = 200
        // Leading space: 200 / 2 = 100
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(100., 0.), vec2f(40., 30.)),
                RectF::new(vec2f(160., 0.), vec2f(40., 30.)), // 100 + 40 + 20
            ],
        );
    })
}

#[test]
fn test_flex_spacing_empty() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test empty flex with spacing
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| Flex::row().with_spacing(15.).finish())
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // Empty flex should render no children
        assert_bounds_of_rects(app, window_id, []);
    })
}

#[test]
fn test_flex_spacing_single_child() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test single child with spacing (should have no effect)
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                Flex::row()
                    .with_spacing(20.)
                    .with_child(
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(30.)
                            .with_width(50.)
                            .finish(),
                    )
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // Single child should be positioned at origin regardless of spacing
        assert_bounds_of_rects(app, window_id, [RectF::new(vec2f(0., 0.), vec2f(50., 30.))]);
    })
}

#[test]
fn test_flex_cross_axis_alignment() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            View::new(
                MainAxisSize::Min,
                MainAxisAlignment::Start,
                CrossAxisAlignment::Start,
            )
        });

        let mut presenter = Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).expect("root view should exist"));
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation.clone(), ctx);
            let window_size = RectF::new(Vector2F::zero(), vec2f(300., 300.));
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1_size = vec2f(50., 20.);
            let view_2_size = vec2f(50., 30.);
            let view_3_size = vec2f(50., 50.);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            assert_eq!(view_1, RectF::new(Vector2F::zero(), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(50., 0.), view_2_size));
            assert_eq!(view_3, RectF::new(vec2f(100., 0.), view_3_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(150., 50.)));

            view.update(ctx, |view, _ctx| {
                view.flex_cross_axis_alignment = CrossAxisAlignment::Center;
            });

            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            assert_eq!(view_1, RectF::new(vec2f(0., 15.), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(50., 10.), view_2_size));
            assert_eq!(view_3, RectF::new(vec2f(100., 0.), view_3_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(150., 50.)));

            view.update(ctx, |view, _ctx| {
                view.flex_cross_axis_alignment = CrossAxisAlignment::End;
            });

            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            assert_eq!(view_1, RectF::new(vec2f(0., 30.), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(50., 20.), view_2_size));
            assert_eq!(view_3, RectF::new(vec2f(100., 0.), view_3_size));
            assert_eq!(flex, RectF::new(Vector2F::zero(), vec2f(150., 50.)));

            view.update(ctx, |view, _ctx| {
                view.flex_cross_axis_alignment = CrossAxisAlignment::Stretch;
            });

            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size.size(), 1., None, ctx);

            let view_1 = presenter
                .position_cache()
                .get_position("view_1")
                .expect("position should exist");
            let view_2 = presenter
                .position_cache()
                .get_position("view_2")
                .expect("position should exist");
            let view_3 = presenter
                .position_cache()
                .get_position("view_3")
                .expect("position should exist");
            let flex = presenter
                .position_cache()
                .get_position("flex")
                .expect("position should exist");

            assert_eq!(view_1, RectF::new(vec2f(0., 0.), view_1_size));
            assert_eq!(view_2, RectF::new(vec2f(50., 0.), view_2_size));
            // view 3 is a Flex::row(), so applying cross-axis stretch to its
            // parent should cause the child flex height to fill the parent's
            // maximum height (which, in this case, is the height of the window).
            assert_eq!(
                view_3,
                RectF::new(
                    vec2f(100., 0.),
                    vec2f(view_3_size.x(), window_size.height())
                )
            );
            assert_eq!(
                flex,
                RectF::new(Vector2F::zero(), vec2f(150., window_size.height()))
            );
        });
    })
}
