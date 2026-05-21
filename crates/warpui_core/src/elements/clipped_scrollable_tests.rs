use std::collections::HashSet;

use pathfinder_geometry::vector::vec2f;

use crate::{
    elements::{Axis, ConstrainedBox, Empty, Flex, ParentElement, SavePosition, Stack},
    platform::WindowStyle,
    units::IntoPixels,
    App, Element, Entity, Presenter, TypedActionView, WindowInvalidation,
};

use super::{ClippedScrollStateHandle, ClippedScrollable, ScrollTarget, ScrollToPositionMode};

macro_rules! assert_float_eq {
    ($lhs:expr, $rhs:expr) => {{
        let lhs = $lhs;
        let rhs = $rhs;
        assert!(
            (lhs - rhs).abs() < f32::EPSILON,
            "{} ({}) != {} ({})",
            lhs,
            stringify!($lhs),
            rhs,
            stringify!($rhs)
        );
    }};
}

#[derive(Default)]
struct View {
    scroll_handle: ClippedScrollStateHandle,
}

impl Entity for View {
    type Event = ();
}

impl crate::core::View for View {
    fn ui_name() -> &'static str {
        "View"
    }

    fn render(&self, _: &crate::AppContext) -> Box<dyn crate::Element> {
        let mut children = vec![];
        for i in 0..10 {
            children.push(
                SavePosition::new(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_height(20.)
                        .with_width(100.)
                        .finish(),
                    &format!("child_{i}"),
                )
                .finish(),
            );
        }

        let mut stack = Stack::new();
        stack.add_child(
            ClippedScrollable::new(
                Axis::Vertical,
                Flex::column().with_children(children).finish(),
                self.scroll_handle.clone(),
            )
            .finish(),
        );
        stack.finish()
    }
}

impl TypedActionView for View {
    type Action = ();
}

#[test]
fn test_scroll_to_position() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| View::default());

        let mut presenter = Presenter::new(window_id);

        let mut updated = HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        let scroll_state = view.read(app, |view, _| view.scroll_handle.clone());
        let window_size = vec2f(100., 100.);
        let scale_factor = 1.;

        app.update(move |ctx| {
            presenter.invalidate(invalidation.clone(), ctx);
            // The `ClippedScrollable` has 10 elements in total, each with a height of 20.
            // The window height is 100 so, with a scroll top of 0, the first 5 elements should be
            // in view.
            presenter.build_scene(window_size, scale_factor, None, ctx);

            // An element fully below the scrollable area should be the last item in view
            // after we scroll to it.
            scroll_state.scroll_to_position(ScrollTarget {
                position_id: "child_6".to_string(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size, scale_factor, None, ctx);
            assert_float_eq!(scroll_state.scroll_start().as_f32(), 40.);

            // An element fully above the scrollable area should be the first item in view after
            // it's scrolled to.
            scroll_state.scroll_to_position(ScrollTarget {
                position_id: "child_1".to_string(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size, scale_factor, None, ctx);
            assert_float_eq!(scroll_state.scroll_start().as_f32(), 20.);

            // An element fully within the viewport should no-op.
            scroll_state.scroll_to_position(ScrollTarget {
                position_id: "child_3".to_string(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size, scale_factor, None, ctx);
            assert_float_eq!(scroll_state.scroll_start().as_f32(), 20.);

            // An element that is partially above the viewport should be scrolled fully within the viewport.
            // First, make the scroll top 1.0 pixels. We need to call build scene after this so the
            // position cache is updated appropriately.
            scroll_state.clipped_scroll_data.lock().scroll_start_px = (1_f32).into_pixels();
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size, scale_factor, None, ctx);

            // Now we can invoke the scroll to position API and verify the correct result.
            scroll_state.scroll_to_position(ScrollTarget {
                position_id: "child_0".to_string(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size, scale_factor, None, ctx);
            assert_float_eq!(scroll_state.scroll_start().as_f32(), 0.);

            // An element that is partially below the viewport should be scrolled fully within the viewport.
            // First, make the scroll top 1.0 pixels. We need to call build scene after this so the
            // position cache is updated appropriately.
            scroll_state.clipped_scroll_data.lock().scroll_start_px = (1_f32).into_pixels();
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size, scale_factor, None, ctx);

            // Now we can invoke the scroll to position API and verify the correct result.
            scroll_state.scroll_to_position(ScrollTarget {
                position_id: "child_5".to_string(),
                mode: ScrollToPositionMode::FullyIntoView,
            });
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(window_size, scale_factor, None, ctx);
            assert_float_eq!(scroll_state.scroll_start().as_f32(), 20.);
        });
    });
}
