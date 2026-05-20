use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;

use crate::elements::ConstrainedBox;
use crate::elements::Container;
use crate::elements::Empty;
use crate::elements::ParentElement;
use crate::elements::Rect;
use crate::platform::WindowStyle;
use crate::Entity;
use crate::View;
use crate::WindowId;
use crate::{App, TypedActionView};

use super::*;

struct TestRootView {
    parent_size: Vector2F,
    children_sizes: Vec<Vector2F>,
    axis: Axis,
    run_spacing: f32,
}

impl TestRootView {
    pub fn new(
        parent_size: Vector2F,
        children_sizes: Vec<Vector2F>,
        axis: Axis,
        run_spacing: f32,
    ) -> Self {
        Self {
            parent_size,
            children_sizes,
            axis,
            run_spacing,
        }
    }
}

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "Wrap::tests::TestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let mut wrap = Wrap::new(self.axis).with_run_spacing(self.run_spacing);
        wrap.extend(self.children_sizes.iter().map(|size| {
            ConstrainedBox::new(Rect::new().finish())
                .with_height(size.y())
                .with_width(size.x())
                .finish()
        }));

        ConstrainedBox::new(wrap.finish())
            .with_width(self.parent_size.x())
            .with_height(self.parent_size.y())
            .finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

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

impl View for TestDynamicView {
    fn ui_name() -> &'static str {
        "Wrap::tests::TestDynamicView"
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
        .flat_map(|layer| layer.rects.iter())
        .map(|rect| rect.bounds);

    itertools::assert_equal(actual_rects, rects);
}

#[test]
fn test_row_wraps_across_runs() {
    App::test((), |mut app| async move {
        let child_size = vec2f(100., 100.);
        let app = &mut app;

        // Attempt to render 4 100x100 rects into a 250x250 box. This should result in the first
        // two rects rendered in a row, followed by a 10px horizontal spacing, followed by the
        // next two rects rendered in a row.
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestRootView::new(
                vec2f(250., 250.),
                vec![child_size; 4],
                Axis::Horizontal,
                10.,
            )
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(100., 0.), child_size),
                RectF::new(vec2f(0., 110.), child_size),
                RectF::new(vec2f(100., 110.), child_size),
            ],
        );
    })
}

#[test]
fn test_column_wraps_across_runs() {
    App::test((), |mut app| async move {
        let child_size = vec2f(100., 100.);
        let app = &mut app;

        // Attempt to render 4 100x100 rects into a 250x250 box. This should result in the first
        // two rects rendered in a column, followed by a 10px vertical spacing, followed by the
        // next two rects rendered in a column.
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestRootView::new(vec2f(250., 250.), vec![child_size; 4], Axis::Vertical, 10.)
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(0., 100.), child_size),
                RectF::new(vec2f(110., 0.), child_size),
                RectF::new(vec2f(110., 100.), child_size),
            ],
        );
    })
}

/// Tests that elements within a `Wrap` are not rendered if they can't be fit within the max
/// size constraint along the cross axis.
#[test]
fn test_wrap_with_too_many_elements() {
    App::test((), |mut app| async move {
        let child_size = vec2f(100., 100.);
        let app = &mut app;

        // Attempt to render 10 100x100 rects into a 250x250 box. This should result in the
        // first two rects rendered in a row, followed by a 10px horizontal spacing, followed by
        // the next two rects rendered in a row. Only four of the ten rects can fit in the
        // parent box.
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestRootView::new(vec2f(250., 250.), vec![child_size; 10], Axis::Vertical, 10.)
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // Only 4 elements are painted--even though there were 10 initial elements passed to the
        // wrap.
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(0., 100.), child_size),
                RectF::new(vec2f(110., 0.), child_size),
                RectF::new(vec2f(110., 100.), child_size),
            ],
        );
    })
}

/// Tests that when the first item exceeds the max size constraint, it is clamped for layout
/// purposes and clipped during paint. Subsequent items that fit are still laid out.
#[test]
fn test_wrap_first_element_exceeds_size_constraint() {
    App::test((), |mut app| async move {
        let child_size = vec2f(100., 100.);
        let mut children = vec![vec2f(500., 500.)];
        children.extend(vec![child_size; 2]);

        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestRootView::new(vec2f(250., 250.), children, Axis::Vertical, 10.)
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // The first oversized element is clamped to 250x250 for layout, filling the
        // entire space. Subsequent items can't fit on the cross axis.
        // The child is laid out at 250x500 (cross axis is clamped by ConstrainedBox
        // to 250, main axis stays 500), then clamped to 250x250 for run placement.
        assert_bounds_of_rects(
            app,
            window_id,
            [RectF::new(vec2f(0., 0.), vec2f(250., 500.))],
        );
    })
}

/// Tests that when the second element exceeds the size constraint, it is clamped and
/// subsequent items continue to be laid out if they fit.
#[test]
fn test_second_element_exceeds_size_constraint() {
    App::test((), |mut app| async move {
        let child_size = vec2f(100., 100.);
        let children = vec![child_size, vec2f(300., 300.), child_size, child_size];

        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestRootView::new(vec2f(250., 250.), children, Axis::Vertical, 10.)
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // The first element (100x100) is in column 1. The oversized second element
        // (250x300 after ConstrainedBox clamp, then 250x250 for layout) doesn't fit
        // in the current run and creates a new one. Its clamped cross-axis size (250)
        // plus existing runs (100 + 10) exceeds 250, so only the first child fits.
        assert_bounds_of_rects(app, window_id, [RectF::new(vec2f(0., 0.), child_size)]);
    })
}

#[test]
fn test_min_size_along_row() {
    App::test((), |mut app| async move {
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                // Use a Container so there's a rect for the Wrap element's bounds.
                ConstrainedBox::new(
                    Container::new(
                        Wrap::row()
                            .with_main_axis_size(MainAxisSize::Min)
                            .with_children([
                                ConstrainedBox::new(Empty::new().finish())
                                    .with_width(100.)
                                    .with_height(100.)
                                    .finish(),
                                ConstrainedBox::new(Empty::new().finish())
                                    .with_width(100.)
                                    .with_height(100.)
                                    .finish(),
                            ])
                            .finish(),
                    )
                    .finish(),
                )
                // Ensure the Wrap has _more_ than enough space for the two children.
                .with_max_width(250.)
                .with_max_height(250.)
                .finish()
            })
        });

        test_view.update(&mut app, |_, ctx| ctx.notify());

        // The Wrap element is painted using the minimum space needed for the two children, even
        // though it could expand further.
        assert_bounds_of_rects(
            &mut app,
            window_id,
            [RectF::new(vec2f(0., 0.), vec2f(200., 100.))],
        );
    });
}

#[test]
fn test_fill_element_within_size_constraint() {
    App::test((), |mut app| async move {
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                ConstrainedBox::new(
                    Wrap::row()
                        .with_children([
                            ConstrainedBox::new(Rect::new().finish())
                                .with_width(100.)
                                .with_height(100.)
                                .finish(),
                            WrapFill::new(200., Rect::new().finish()).finish(),
                        ])
                        .finish(),
                )
                .with_width(400.)
                .with_height(100.)
                .finish()
            })
        });

        test_view.update(&mut app, |_, ctx| ctx.notify());

        // Both children are painted, and the second expands to fill available space.
        assert_bounds_of_rects(
            &mut app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), vec2f(100., 100.)),
                RectF::new(vec2f(100., 0.), vec2f(300., 100.)),
            ],
        );
    });
}

#[test]
fn test_wrap_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test with spacing between children in the same run
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(100., 50.);
                // Create a Wrap that can fit 3 children horizontally with spacing
                let mut wrap = Wrap::row().with_spacing(10.);
                wrap.extend((0..3).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(350.) // 100 + 10 + 100 + 10 + 100 = 320, so they fit in one run
                    .with_height(200.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(100., 50.);
        // All 3 children should be in the same run with 10px spacing between them
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(110., 0.), child_size), // 100 + 10
                RectF::new(vec2f(220., 0.), child_size), // 100 + 10 + 100 + 10
            ],
        );
    })
}

#[test]
fn test_wrap_spacing_with_wrapping() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test with spacing that forces wrapping
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(100., 50.);
                let mut wrap = Wrap::row().with_spacing(20.);
                wrap.extend((0..4).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(250.) // Can only fit 2 children per run: 100 + 20 + 100 = 220
                    .with_height(200.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(100., 50.);
        // First 2 children in first run, next 2 in second run
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(120., 0.), child_size), // 100 + 20
                RectF::new(vec2f(0., 50.), child_size),  // New run
                RectF::new(vec2f(120., 50.), child_size), // 100 + 20 in second run
            ],
        );
    })
}

#[test]
fn test_wrap_run_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test run_spacing (vertical spacing between runs)
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(80., 40.);
                let mut wrap = Wrap::row().with_run_spacing(30.);
                wrap.extend((0..4).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(170.) // Can fit 2 children per run: 80 + 80 = 160
                    .with_height(200.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(80., 40.);
        // Two runs with 30px spacing between them
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(80., 0.), child_size),
                RectF::new(vec2f(0., 70.), child_size), // 40 + 30
                RectF::new(vec2f(80., 70.), child_size), // 40 + 30
            ],
        );
    })
}

#[test]
fn test_wrap_both_spacings() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test both spacing and run_spacing together
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(60., 30.);
                let mut wrap = Wrap::row()
                    .with_spacing(15.) // 15px between children in same run
                    .with_run_spacing(25.); // 25px between runs
                wrap.extend((0..6).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(200.) // Can fit 2 children per run: 60 + 15 + 60 = 135
                    .with_height(300.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(60., 30.);
        // 3 runs with 2 children each
        assert_bounds_of_rects(
            app,
            window_id,
            [
                // First run
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(75., 0.), child_size), // 60 + 15
                // Second run (30 + 25 = 55)
                RectF::new(vec2f(0., 55.), child_size),
                RectF::new(vec2f(75., 55.), child_size),
                // Third run (30 + 25 + 30 + 25 = 110)
                RectF::new(vec2f(0., 110.), child_size),
                RectF::new(vec2f(75., 110.), child_size),
            ],
        );
    })
}

#[test]
fn test_wrap_column_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test spacing in column wrap
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(50., 80.);
                let mut wrap = Wrap::column().with_spacing(12.);
                wrap.extend((0..4).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(200.)
                    .with_height(185.) // Can fit 2 children per column: 80 + 12 + 80 = 172
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(50., 80.);
        // Two columns with 2 children each
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(0., 92.), child_size), // 80 + 12
                RectF::new(vec2f(50., 0.), child_size), // New column
                RectF::new(vec2f(50., 92.), child_size), // 80 + 12 in second column
            ],
        );
    })
}

#[test]
fn test_wrap_column_run_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test run_spacing in column wrap (horizontal spacing between columns)
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(40., 70.);
                let mut wrap = Wrap::column().with_run_spacing(20.);
                wrap.extend((0..4).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(200.)
                    .with_height(145.) // Can fit 2 children per column: 70 + 70 = 140
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(40., 70.);
        // Two columns with 20px horizontal spacing between them
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(0., 70.), child_size),
                RectF::new(vec2f(60., 0.), child_size), // 40 + 20
                RectF::new(vec2f(60., 70.), child_size), // 40 + 20
            ],
        );
    })
}

#[test]
fn test_wrap_spacing_edge_cases() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test single child with spacing (should have no effect)
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(80., 40.);
                let mut wrap = Wrap::row().with_spacing(20.);
                wrap.extend((0..1).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(300.)
                    .with_height(200.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(80., 40.);
        // Single child should be positioned at origin regardless of spacing
        assert_bounds_of_rects(app, window_id, [RectF::new(vec2f(0., 0.), child_size)]);
    })
}

#[test]
fn test_wrap_zero_spacing() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test zero spacing and zero run_spacing
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let child_size = vec2f(60., 30.);
                let mut wrap = Wrap::row()
                    .with_spacing(0.) // No spacing between children
                    .with_run_spacing(0.); // No spacing between runs
                wrap.extend((0..4).map(|_| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(child_size.y())
                        .with_width(child_size.x())
                        .finish()
                }));

                ConstrainedBox::new(wrap.finish())
                    .with_width(130.) // Can fit 2 children per run: 60 + 60 = 120
                    .with_height(200.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        let child_size = vec2f(60., 30.);
        // Children should be tightly packed with no gaps
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), child_size),
                RectF::new(vec2f(60., 0.), child_size), // No spacing
                RectF::new(vec2f(0., 30.), child_size), // New run, no run_spacing
                RectF::new(vec2f(60., 30.), child_size), // No spacing in second run
            ],
        );
    })
}

#[test]
fn test_wrap_empty() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Test empty wrap with spacing settings
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let wrap = Wrap::row().with_spacing(15.).with_run_spacing(25.);
                // No children added

                ConstrainedBox::new(wrap.finish())
                    .with_width(300.)
                    .with_height(200.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        // Empty wrap should render no children
        assert_bounds_of_rects(app, window_id, []);
    })
}

/// Tests the core fix: an oversized item in a row wrap is clamped so that
/// subsequent items continue to be laid out on later runs.
#[test]
fn test_oversized_item_does_not_block_subsequent_items() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Container is 300x200 (row wrap). Children:
        //   1) 100x50  (fits)
        //   2) 500x50  (exceeds 300 on main axis, will be clamped to 300x50)
        //   3) 100x50  (fits on a new run after the oversized item)
        //   4) 100x50  (fits on the same run as child 3)
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                let mut wrap = Wrap::row();
                wrap.extend([
                    ConstrainedBox::new(Rect::new().finish())
                        .with_width(100.)
                        .with_height(50.)
                        .finish(),
                    ConstrainedBox::new(Rect::new().finish())
                        .with_width(500.)
                        .with_height(50.)
                        .finish(),
                    ConstrainedBox::new(Rect::new().finish())
                        .with_width(100.)
                        .with_height(50.)
                        .finish(),
                    ConstrainedBox::new(Rect::new().finish())
                        .with_width(100.)
                        .with_height(50.)
                        .finish(),
                ]);

                ConstrainedBox::new(wrap.finish())
                    .with_width(300.)
                    .with_height(200.)
                    .finish()
            })
        });
        test_view.update(app, |_, ctx| ctx.notify());

        // Row 1: child 1 (100x50)
        // Row 2: child 2, laid out at 300x50 (ConstrainedBox cross-clamp) then clamped
        //        to 300x50 for layout — it takes the full main axis.
        // Row 3: child 3 + child 4 side by side.
        // Previously, the break at line 190 would have stopped all layout after child 2.
        assert_bounds_of_rects(
            app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), vec2f(100., 50.)),
                // The oversized child paints at its full 500px width; the clip
                // layer on the Wrap visually clips it to 300px.
                RectF::new(vec2f(0., 50.), vec2f(500., 50.)),
                RectF::new(vec2f(0., 100.), vec2f(100., 50.)),
                RectF::new(vec2f(100., 100.), vec2f(100., 50.)),
            ],
        );
    })
}

#[test]
fn test_fill_element_exceeds_size_constraint() {
    App::test((), |mut app| async move {
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TestDynamicView::new(|_| {
                ConstrainedBox::new(
                    Wrap::row()
                        .with_children([
                            ConstrainedBox::new(Rect::new().finish())
                                .with_width(100.)
                                .with_height(100.)
                                .finish(),
                            WrapFill::new(200., Rect::new().finish()).finish(),
                        ])
                        .finish(),
                )
                .with_width(250.)
                .with_height(250.)
                .finish()
            })
        });

        test_view.update(&mut app, |_, ctx| ctx.notify());

        // Both children are painted, and the second wraps to a new row while filling the remaining
        // height.
        assert_bounds_of_rects(
            &mut app,
            window_id,
            [
                RectF::new(vec2f(0., 0.), vec2f(100., 100.)),
                RectF::new(vec2f(0., 100.), vec2f(250., 150.)),
            ],
        );
    });
}
