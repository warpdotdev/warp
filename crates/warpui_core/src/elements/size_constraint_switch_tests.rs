use itertools::Itertools;
use lazy_static::lazy_static;
use pathfinder_geometry::vector::vec2f;

use crate::{
    elements::{ChildView, ConstrainedBox, Rect},
    platform::WindowStyle,
    App, Entity, TypedActionView, View, ViewContext, ViewHandle, WindowId,
};

use super::*;

lazy_static! {
    static ref PARENT_SIZE_FOR_DEFAULT_CHILD: Vector2F = vec2f(600., 600.);
    static ref PARENT_SIZE_FOR_CONDITIONAL_CHILD: Vector2F = vec2f(400., 400.);
    static ref DEFAULT_CHILD_SIZE: Vector2F = vec2f(300., 300.);
    static ref CONDITIONAL_CHILD_SIZE: Vector2F = vec2f(200., 200.);
}

const CONDIITIONAL_CHILD_THRESHOLD: f32 = 500.;

struct TestChildView {
    condition: SizeConstraintCondition,
}

impl TestChildView {
    fn new(condition: SizeConstraintCondition) -> Self {
        Self { condition }
    }
}

impl Entity for TestChildView {
    type Event = ();
}

impl View for TestChildView {
    fn ui_name() -> &'static str {
        "SizeConstraintSwitch::tests::TestChildView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        SizeConstraintSwitch::new(
            ConstrainedBox::new(Rect::new().finish())
                .with_width(DEFAULT_CHILD_SIZE.x())
                .with_height(DEFAULT_CHILD_SIZE.y())
                .finish(),
            vec![(
                self.condition,
                ConstrainedBox::new(Rect::new().finish())
                    .with_width(CONDITIONAL_CHILD_SIZE.x())
                    .with_height(CONDITIONAL_CHILD_SIZE.y())
                    .finish(),
            )],
        )
        .finish()
    }
}

struct TestRootView {
    parent_size: Vector2F,
    child_handle: ViewHandle<TestChildView>,
}

impl TestRootView {
    pub fn new(
        parent_size: Vector2F,
        condition: SizeConstraintCondition,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let child_handle = ctx.add_view(|_ctx| TestChildView::new(condition));
        Self {
            parent_size,
            child_handle,
        }
    }
}

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "SizeConstraintSwitch::tests::TestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        ConstrainedBox::new(ChildView::new(&self.child_handle).finish())
            .with_max_width(self.parent_size.x())
            .with_max_height(self.parent_size.y())
            .finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

fn assert_rendered_rect_with_size(app: &mut App, window_id: WindowId, size: Vector2F) {
    let presenter_ref = app
        .presenter(window_id)
        .expect("Test window should have a presenter since first frame is rendered.");
    let presenter = presenter_ref.borrow();
    let scene = presenter
        .scene()
        .expect("Presenter should have rendered a scene after the test_view was updated.");

    assert_eq!(
        scene
            .layers()
            .collect_vec()
            .first()
            .unwrap()
            .rects
            .iter()
            .map(|r| { r.bounds.size() })
            .collect::<Vec<_>>(),
        vec![size]
    );
}

#[test]
fn renders_default_child_when_no_conditions_match() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestRootView::new(
                *PARENT_SIZE_FOR_DEFAULT_CHILD,
                SizeConstraintCondition::WidthLessThan(CONDIITIONAL_CHILD_THRESHOLD),
                ctx,
            )
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });

        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);
    })
}

#[test]
fn renders_element_with_max_width_condition() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestRootView::new(
                *PARENT_SIZE_FOR_DEFAULT_CHILD,
                SizeConstraintCondition::WidthLessThan(CONDIITIONAL_CHILD_THRESHOLD),
                ctx,
            )
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);

        test_view.update(app, |test_view, ctx| {
            test_view.parent_size = *PARENT_SIZE_FOR_CONDITIONAL_CHILD;
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *CONDITIONAL_CHILD_SIZE);
    })
}

#[test]
fn renders_element_with_max_height_condition() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestRootView::new(
                *PARENT_SIZE_FOR_DEFAULT_CHILD,
                SizeConstraintCondition::HeightLessThan(CONDIITIONAL_CHILD_THRESHOLD),
                ctx,
            )
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);

        test_view.update(app, |test_view, ctx| {
            test_view.parent_size = *PARENT_SIZE_FOR_CONDITIONAL_CHILD;
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *CONDITIONAL_CHILD_SIZE);
    })
}

#[test]
fn renders_element_with_max_size_condition() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestRootView::new(
                *PARENT_SIZE_FOR_DEFAULT_CHILD,
                SizeConstraintCondition::SizeSmallerThan(Size {
                    width: CONDIITIONAL_CHILD_THRESHOLD,
                    height: CONDIITIONAL_CHILD_THRESHOLD,
                }),
                ctx,
            )
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);

        test_view.update(app, |test_view, ctx| {
            test_view.parent_size = *PARENT_SIZE_FOR_CONDITIONAL_CHILD;
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *CONDITIONAL_CHILD_SIZE);
    })
}

#[test]
fn size_condition_doesnt_match_with_valid_width_only() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestRootView::new(
                *PARENT_SIZE_FOR_DEFAULT_CHILD,
                SizeConstraintCondition::SizeSmallerThan(Size {
                    width: CONDIITIONAL_CHILD_THRESHOLD,
                    height: CONDIITIONAL_CHILD_THRESHOLD,
                }),
                ctx,
            )
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);

        test_view.update(app, |test_view, ctx| {
            test_view.parent_size = vec2f(
                PARENT_SIZE_FOR_CONDITIONAL_CHILD.x(),
                test_view.parent_size.y(),
            );
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);
    })
}

#[test]
fn size_condition_doesnt_match_with_valid_height_only() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestRootView::new(
                *PARENT_SIZE_FOR_DEFAULT_CHILD,
                SizeConstraintCondition::SizeSmallerThan(Size {
                    width: CONDIITIONAL_CHILD_THRESHOLD,
                    height: CONDIITIONAL_CHILD_THRESHOLD,
                }),
                ctx,
            )
        });
        test_view.update(app, |_, ctx| {
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);

        test_view.update(app, |test_view, ctx| {
            test_view.parent_size = vec2f(
                test_view.parent_size.x(),
                PARENT_SIZE_FOR_CONDITIONAL_CHILD.y(),
            );
            ctx.notify();
        });
        assert_rendered_rect_with_size(app, window_id, *DEFAULT_CHILD_SIZE);
    })
}
