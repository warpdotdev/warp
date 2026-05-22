use std::collections::HashSet;

use super::*;
use crate::core::View;
use crate::elements::{ConstrainedBox, Rect, Scrollable, ScrollbarWidth};
use crate::platform::WindowStyle;
use crate::prelude::Fill;
use crate::units::{IntoPixels, Pixels};
use crate::{
    App, AppContext, Entity, Event, Presenter, TypedActionView, ViewContext, WindowInvalidation,
};

/// Test context that captures scroll position information.
/// For this test, we use the context as a simple marker that scroll preservation is desired.
/// The adjustment function computes the actual scroll position.
#[derive(Clone, Debug)]
struct TestScrollContext;

/// Test view that uses a viewported list with scroll preservation.
struct ScrollPreservationTestView {
    list_state: ListState<TestScrollContext>,
    item_heights: Rc<RefCell<Vec<Pixels>>>,
}

impl ScrollPreservationTestView {
    fn new(item_count: usize, initial_height: Pixels) -> Self {
        let item_heights = Rc::new(RefCell::new(vec![initial_height; item_count]));
        let item_heights_clone = item_heights.clone();

        let (list_state, _scroll_rx) = ListState::new_with_scroll_preservation(
            move |index, _scroll_offset, _app| {
                let heights = item_heights_clone.borrow();
                let height = heights.get(index).copied().unwrap_or(100.0.into_pixels());
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(height.as_f32())
                    .with_width(200.)
                    .finish()
            },
            // Adjustment function: preserve scroll at current item-relative offset
            |_index, _ctx, _app| Some(Pixels::zero()),
        );

        // Add items to the list
        for _ in 0..item_count {
            list_state.add_item();
        }

        Self {
            list_state,
            item_heights,
        }
    }

    fn set_item_height(&self, index: usize, height: Pixels) {
        let mut heights = self.item_heights.borrow_mut();
        if index < heights.len() {
            heights[index] = height;
        }
    }
}

impl Entity for ScrollPreservationTestView {
    type Event = ();
}

impl crate::core::View for ScrollPreservationTestView {
    fn ui_name() -> &'static str {
        "scroll_preservation_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        ConstrainedBox::new(List::new(self.list_state.clone()).finish())
            .with_height(300.)
            .with_width(200.)
            .finish()
    }
}

impl TypedActionView for ScrollPreservationTestView {
    type Action = ();
}

#[test]
#[ignore = "Flaking on CI - KC looking into 3/31/26"]
fn test_scroll_preservation_adjusts_position_when_item_above_grows() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Create view with 20 items, each 50px tall (1000px total)
        // Viewport is 300px, so we can scroll to item 5 without hitting max
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            ScrollPreservationTestView::new(20, 50.0.into_pixels())
        });

        let root_view_id = app.root_view_id(window_id).unwrap();

        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);

            // First layout pass to measure all items
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated: updated.clone(),
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        // Scroll to item 5 (should be at scroll position 250px = 5 * 50px)
        view.update(app, |view, ctx| {
            view.list_state.scroll_to(5);
            ctx.notify();
        });

        // Layout again after scrolling and verify scroll position
        let root_view_id = app.root_view_id(window_id).unwrap();
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        // Verify scroll position before change
        let scroll_index_before = view.read(app, |view, _| view.list_state.get_scroll_index());
        assert_eq!(scroll_index_before, 5, "Should be scrolled to item 5");

        // Set scroll context (simulates what on_scroll_settled does in real usage)
        view.update(app, |view, _| {
            view.list_state.set_scroll_context(Some(TestScrollContext));
        });

        // Now change item 2's height from 50px to 100px (item above scroll position)
        view.update(app, |view, ctx| {
            view.set_item_height(2, 100.0.into_pixels());
            view.list_state.invalidate_height_for_index(2);
            ctx.notify();
        });

        // Layout again - this should trigger scroll preservation
        let root_view_id = app.root_view_id(window_id).unwrap();
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        // After the height change, the scroll position should be adjusted
        view.read(app, |view, _| {
            let scroll_index = view.list_state.get_scroll_index();
            // The scroll index should remain at 5 since we're preserving the visual position
            assert_eq!(
                scroll_index, 5,
                "Scroll index should still be 5 after adjustment"
            );
        });
    });
}

#[test]
#[ignore = "Flaking on CI - KC looking into 3/31/26"]
fn test_scroll_preservation_no_adjustment_when_item_below_changes() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Create view with 20 items, each 50px tall (1000px total)
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            ScrollPreservationTestView::new(20, 50.0.into_pixels())
        });

        let root_view_id = app.root_view_id(window_id).unwrap();

        // First layout pass
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        // Scroll to item 3
        view.update(app, |view, ctx| {
            view.list_state.scroll_to(3);
            ctx.notify();
        });

        // Layout after scrolling
        let root_view_id = app.root_view_id(window_id).unwrap();
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        let scroll_index_before = view.read(app, |view, _| view.list_state.get_scroll_index());
        let scroll_offset_before = view.read(app, |view, _| view.list_state.get_scroll_offset());
        assert_eq!(scroll_index_before, 3);

        // Change item 7's height (item below scroll position)
        view.update(app, |view, ctx| {
            view.set_item_height(7, 200.0.into_pixels());
            view.list_state.invalidate_height_for_index(7);
            ctx.notify();
        });

        // Layout again
        let root_view_id = app.root_view_id(window_id).unwrap();
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        // Scroll position should remain unchanged since item 7 is below scroll position
        view.read(app, |view, _| {
            let scroll_index = view.list_state.get_scroll_index();
            let scroll_offset = view.list_state.get_scroll_offset();
            assert_eq!(
                scroll_index, scroll_index_before,
                "Scroll index should not change when item below scroll position changes"
            );
            assert_eq!(
                scroll_offset, scroll_offset_before,
                "Scroll offset should not change when item below scroll position changes"
            );
        });
    });
}

#[test]
#[ignore = "Flaking on CI - KC looking into 3/31/26"]
fn test_list_state_without_scroll_preservation_backward_compatible() {
    App::test((), |mut app| async move {
        let app = &mut app;

        // Create a simple view using the non-generic ListState (backward compatibility)
        struct SimpleListView {
            list_state: ListState<()>,
        }

        impl Entity for SimpleListView {
            type Event = ();
        }

        impl crate::core::View for SimpleListView {
            fn ui_name() -> &'static str {
                "simple_list_view"
            }

            fn render(&self, _: &AppContext) -> Box<dyn Element> {
                ConstrainedBox::new(List::new(self.list_state.clone()).finish())
                    .with_height(300.)
                    .with_width(200.)
                    .finish()
            }
        }

        impl TypedActionView for SimpleListView {
            type Action = ();
        }

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            let list_state = ListState::new(|_index, _scroll_offset, _app| {
                ConstrainedBox::new(Rect::new().finish())
                    .with_height(50.)
                    .with_width(200.)
                    .finish()
            });
            for _ in 0..10 {
                list_state.add_item();
            }
            SimpleListView { list_state }
        });

        let root_view_id = app.root_view_id(window_id).unwrap();

        // Layout should work without scroll preservation
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        // Scroll to item 5
        view.update(app, |view, ctx| {
            view.list_state.scroll_to(5);
            ctx.notify();
        });

        // Invalidate an item - should work without scroll preservation callbacks
        view.update(app, |view, ctx| {
            view.list_state.invalidate_height_for_index(2);
            ctx.notify();
        });

        // Layout again - should not crash
        let root_view_id = app.root_view_id(window_id).unwrap();
        app.update(move |ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = HashSet::new();
            updated.insert(root_view_id);
            let invalidation = WindowInvalidation {
                updated,
                ..Default::default()
            };
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(200., 300.), 1., None, ctx);
        });

        // Verify basic functionality still works
        view.read(app, |view, _| {
            // Just verify we can read the scroll index without panicking
            let _scroll_index = view.list_state.get_scroll_index();
        });
    });
}

// Create a view with a list that has a scroll sender
struct ScrollSenderTestView {
    list_state: ListState<()>,
    scroll_rx: async_channel::Receiver<ScrollOffset>,
}

impl Entity for ScrollSenderTestView {
    type Event = ();
}

impl View for ScrollSenderTestView {
    fn ui_name() -> &'static str {
        "scroll_sender_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        // Wrap List in Scrollable so that ScrollWheel events are routed through
        // to scroll_vertically, which broadcasts on the scroll channel.
        Scrollable::vertical(
            Default::default(),
            ScrollableElement::finish_scrollable(List::new(self.list_state.clone())),
            ScrollbarWidth::None,
            Fill::None,
            Fill::None,
            Fill::None,
        )
        .finish()
    }
}

impl TypedActionView for ScrollSenderTestView {
    type Action = ();
}

#[test]
#[ignore = "Flaking on CI - KC looking into 3/31/26"]
fn test_scroll_sender_receives_events_on_scroll() {
    App::test((), |mut app| async move {
        let app = &mut app;

        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_| {
            let (list_state, scroll_rx) = ListState::new_with_scroll_preservation(
                |_index, _scroll_offset, _app| {
                    ConstrainedBox::new(Rect::new().finish())
                        .with_height(100.)
                        .with_width(200.)
                        .finish()
                },
                |_index, _ctx, _app| None,
            );
            // Add enough items to enable scrolling
            for _ in 0..20 {
                list_state.add_item();
            }
            ScrollSenderTestView {
                list_state,
                scroll_rx,
            }
        });

        let scroll_rx = _view.read(app, |view, _| view.scroll_rx.clone());

        let root_view_id = app.root_view_id(window_id).unwrap();
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));

        app.update({
            let presenter = presenter.clone();
            move |ctx| {
                // Layout the list so the element has a size and can handle events.
                let mut updated = HashSet::new();
                updated.insert(root_view_id);
                let invalidation = WindowInvalidation {
                    updated,
                    ..Default::default()
                };
                presenter.borrow_mut().invalidate(invalidation, ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(200., 300.), 1., None, ctx);

                // Simulate a scroll wheel event inside the list bounds.
                // delta.y is negative to scroll down (increases scroll position).
                ctx.simulate_window_event(
                    Event::ScrollWheel {
                        position: vec2f(100., 150.),
                        delta: vec2f(0., -50.),
                        precise: true,
                        modifiers: Default::default(),
                    },
                    window_id,
                    presenter.clone(),
                );
            }
        });

        assert!(
            !scroll_rx.is_empty(),
            "scroll_vertically should send a scroll event over the channel"
        );
    });
}

struct TestView {
    list_state: ListState<()>,
    heights: Vec<f32>,
}

impl TestView {
    fn new(item_heights: Vec<f32>, ctx: &mut ViewContext<Self>) -> Self {
        let handle = ctx.handle();
        let list_state = ListState::new(move |index, _, app| {
            let height = handle
                .upgrade(app)
                .and_then(|handle| handle.as_ref(app).heights.get(index).copied())
                .unwrap_or(50.);
            ConstrainedBox::new(Rect::new().finish())
                .with_height(height)
                .with_width(100.)
                .finish()
        });

        for _ in 0..item_heights.len() {
            list_state.add_item();
        }

        Self {
            list_state,
            heights: item_heights,
        }
    }

    fn set_height(&mut self, index: usize, height: f32) {
        self.heights[index] = height;
    }
}

impl Entity for TestView {
    type Event = ();
}

impl crate::core::View for TestView {
    fn ui_name() -> &'static str {
        "viewported_list_tests::TestView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        List::new(self.list_state.clone()).finish()
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

#[test]
fn test_scroll_position_invalidation_scrolled_to() {
    // Test: Create items, scroll to a position within an item, invalidate that item, layout.
    // Scroll position should not change.
    App::test((), |mut app| async move {
        let app = &mut app;
        let item_heights = vec![100., 1200.];

        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(item_heights, ctx)
        });

        let scroll_index = 1;
        let scroll_offset = 50.0.into_pixels();
        view.update(app, |view, ctx| {
            view.list_state
                .scroll_to_with_offset(scroll_index, scroll_offset);
            ctx.notify();

            assert_eq!(view.list_state.get_scroll_index(), scroll_index);
            assert_eq!(view.list_state.get_scroll_offset(), scroll_offset);
        });

        // Verify scroll position is preserved after layout
        view.read(app, |view, _| {
            assert_eq!(
                view.list_state.get_scroll_index(),
                scroll_index,
                "Scroll index should be preserved after layout"
            );
            assert_eq!(
                view.list_state.get_scroll_offset(),
                scroll_offset,
                "Scroll offset should be preserved after layout"
            );
        });

        view.update(app, |view, ctx| {
            view.list_state.invalidate_height_for_index(scroll_index);
            ctx.notify();
        });

        // Verify scroll position is preserved after invalidation
        view.read(app, |view, _| {
            assert_eq!(
                view.list_state.get_scroll_index(),
                scroll_index,
                "Scroll index should be preserved after invalidation"
            );
            assert_eq!(
                view.list_state.get_scroll_offset(),
                scroll_offset,
                "Scroll offset should be preserved after invalidation"
            );
        });
    });
}

#[test]
fn test_scroll_position_invalidation_on_screen() {
    // Test: Create items, scroll to a position within an item, invalidate that item, layout.
    // Scroll position should not change.
    App::test((), |mut app| async move {
        let app = &mut app;

        let item_heights = vec![100., 1200.];

        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(item_heights, ctx)
        });

        // Scroll to item 0 (first item) with offset 50
        // This puts us at pixel position 50, well within max scroll range
        let scroll_index = 0;
        let scroll_offset = 50.0.into_pixels();
        view.update(app, |view, ctx| {
            view.list_state
                .scroll_to_with_offset(scroll_index, scroll_offset);
            ctx.notify();

            assert_eq!(view.list_state.get_scroll_index(), scroll_index);
            assert_eq!(view.list_state.get_scroll_offset(), scroll_offset);
        });

        // Verify scroll position is preserved after layout
        view.read(app, |view, _| {
            assert_eq!(
                view.list_state.get_scroll_index(),
                scroll_index,
                "Scroll index should be preserved after layout"
            );
            assert_eq!(
                view.list_state.get_scroll_offset(),
                scroll_offset,
                "Scroll offset should be preserved after layout"
            );
        });

        // Invalidate item 1 (index 0) - the item we're scrolled into - and re-layout
        view.update(app, |view, ctx| {
            view.list_state.invalidate_height_for_index(scroll_index);
            ctx.notify();
        });

        // Verify scroll position is preserved after invalidation
        view.read(app, |view, _| {
            assert_eq!(
                view.list_state.get_scroll_index(),
                scroll_index,
                "Scroll index should be preserved after invalidation"
            );
            assert_eq!(
                view.list_state.get_scroll_offset(),
                scroll_offset,
                "Scroll offset should be preserved after invalidation"
            );
        });
    });
}

#[test]
fn test_scroll_position_clamp() {
    // Test: Create items, scroll to a position within an item, invalidate that item, layout.
    // Scroll position should not change.
    App::test((), |mut app| async move {
        let app = &mut app;

        let item_heights = vec![100., 1200.];

        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TestView::new(item_heights, ctx)
        });

        let scroll_index = 1;
        let scroll_offset = 300.0.into_pixels();
        view.update(app, |view, ctx| {
            view.list_state
                .scroll_to_with_offset(scroll_index, scroll_offset);
            ctx.notify();

            assert_eq!(view.list_state.get_scroll_index(), scroll_index);
            assert_eq!(view.list_state.get_scroll_offset(), scroll_offset);
        });

        // Verify scroll position is preserved after layout
        view.read(app, |view, _| {
            assert_eq!(
                view.list_state.get_scroll_index(),
                scroll_index,
                "Scroll index should be preserved after layout"
            );
            assert_eq!(
                view.list_state.get_scroll_offset(),
                scroll_offset,
                "Scroll offset should be preserved after layout"
            );
        });

        view.update(app, |view, ctx| {
            view.set_height(scroll_index, 500.);
            view.list_state.invalidate_height_for_index(scroll_index);
            ctx.notify();
        });

        // Verify scroll position is preserved after invalidation
        view.read(app, |view, _| {
            assert_eq!(
                view.list_state.get_scroll_index(),
                0,
                "Scroll index should be preserved after invalidation"
            );
            assert_eq!(
                view.list_state.get_scroll_offset(),
                0.0.into_pixels(),
                "Scroll offset should be preserved after invalidation"
            );
        });
    });
}
