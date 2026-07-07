use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{
    TuiViewportContent, TuiViewportPosition, TuiViewportVerticalAlignment, TuiViewportWindow,
    TuiViewportedElement, TuiViewportedList, TuiViewportedListState, TuiVisibleViewportItem,
};
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPoint, TuiRect, TuiScrollable, TuiSize, TuiText,
};
use crate::event::ModifiersState;
use crate::{App, AppContext, EntityId, EntityIdMap};

#[derive(Clone)]
struct FakeItem {
    lines: Vec<String>,
    height: usize,
}

#[derive(Clone)]
struct FakeContent {
    items: Rc<RefCell<Vec<FakeItem>>>,
    requests: Rc<RefCell<Vec<TuiViewportWindow>>>,
    widths: Rc<RefCell<Vec<u16>>>,
}

impl FakeContent {
    fn new(items: Vec<FakeItem>) -> Self {
        Self {
            items: Rc::new(RefCell::new(items)),
            requests: Rc::new(RefCell::new(Vec::new())),
            widths: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl TuiViewportedElement for FakeContent {
    fn visible_items(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiViewportContent {
        self.requests.borrow_mut().push(window);
        self.widths.borrow_mut().push(available_width);
        let viewport_bottom = window
            .scroll_top
            .saturating_add(usize::from(window.viewport_height));
        let mut origin_y = 0usize;
        let mut visible_items = Vec::new();
        for item in self.items.borrow().iter() {
            let item_top = origin_y;
            let item_bottom = item_top.saturating_add(item.height);
            if item_bottom > window.scroll_top && item_top < viewport_bottom {
                visible_items.push(TuiVisibleViewportItem {
                    origin_y: item_top,
                    element: Box::new(TuiText::new(item.lines.join("\n")).truncate()),
                });
            }
            origin_y = item_bottom;
        }
        TuiViewportContent {
            content_height: origin_y,
            items: visible_items,
        }
    }
}

fn fake_item(id: usize, height: usize) -> FakeItem {
    FakeItem {
        lines: (0..height).map(|row| format!("{id}:{row}")).collect(),
        height,
    }
}

fn viewport_with_state(
    state: TuiViewportedListState,
    content: FakeContent,
) -> TuiViewportedList<FakeContent> {
    TuiViewportedList::new(state, content)
}

fn render_viewport(app: &App, viewport: &mut impl TuiElement, size: TuiSize) -> Vec<String> {
    app.read(|app_ctx| {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        viewport.layout(TuiConstraint::tight(size), &mut ctx, app_ctx);
        let area = TuiRect::new(0, 0, size.width, size.height);
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        viewport.render(area, &mut buffer, &mut paint_ctx);
        buffer.to_lines()
    })
}

/// Dispatches a wheel event against the viewport's last layout. Positive
/// `delta_y` scrolls toward the top; negative scrolls toward the bottom
/// (matching the crossterm → warp wheel mapping). Returns whether the event was
/// handled.
fn wheel(app: &App, viewport: &mut impl TuiElement, size: TuiSize, delta_y: f32) -> bool {
    wheel_with_notify_count(app, viewport, size, delta_y).0
}

fn wheel_with_notify_count(
    app: &App,
    viewport: &mut impl TuiElement,
    size: TuiSize,
    delta_y: f32,
) -> (bool, usize) {
    app.read(|app_ctx| {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let area = TuiRect::new(0, 0, size.width, size.height);
        let mut event_ctx = TuiEventContext::default();
        event_ctx.set_origin_view(Some(EntityId::new()));
        let event = TuiEvent::ScrollWheel {
            position: TuiPoint::new(0, 0),
            delta: (0, delta_y as isize),
            precise: false,
            modifiers: ModifiersState::default(),
        };
        let handled = viewport.dispatch_event(&event, area, &mut event_ctx, &mut ctx, app_ctx);
        (handled, event_ctx.take_notified().len())
    })
}

#[test]
fn request_includes_scroll_top_and_height() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 3), fake_item(2, 3)]);
        let requests = content.requests.clone();
        let widths = content.widths.clone();
        let state = TuiViewportedListState::new_at_end();
        state.scroll_to_rows_from_top(2);
        let mut viewport = viewport_with_state(state, content);

        render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(
            requests.borrow().as_slice(),
            &[TuiViewportWindow {
                scroll_top: 2,
                viewport_height: 4,
            }],
        );
        assert_eq!(widths.borrow().as_slice(), &[8]);
    });
}

#[test]
fn end_position_renders_only_the_visible_item_rows() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 3), fake_item(2, 3), fake_item(3, 3)]);
        let mut viewport = viewport_with_state(TuiViewportedListState::new_at_end(), content);

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(&lines[0][..3], "2:2");
        assert_eq!(&lines[1][..3], "3:0");
        assert_eq!(&lines[3][..3], "3:2");
    });
}

#[test]
fn rows_from_top_position_starts_at_the_requested_absolute_row() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 3), fake_item(2, 3), fake_item(3, 3)]);
        let state = TuiViewportedListState::new_at_end();
        state.scroll_to_rows_from_top(1);
        let mut viewport = viewport_with_state(state, content);

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(&lines[0][..3], "1:1");
        assert_eq!(&lines[1][..3], "1:2");
        assert_eq!(&lines[2][..3], "2:0");
        assert_eq!(&lines[3][..3], "2:1");
    });
}

#[test]
fn rows_from_top_past_content_clamps_to_end() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let state = TuiViewportedListState::new_at_end();
        state.scroll_to_rows_from_top(99);
        let mut viewport = viewport_with_state(state.clone(), content);

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 1));

        assert!(state.is_at_end());
        assert_eq!(&lines[0][..3], "2:0");
    });
}

#[test]
fn scrolling_up_clamps_to_the_top_without_snapping_to_bottom() {
    App::test((), |app| async move {
        let content = FakeContent::new((1..=5).map(|id| fake_item(id, 3)).collect());
        let state = TuiViewportedListState::new_at_end();
        let mut viewport =
            TuiScrollable::new(Box::new(viewport_with_state(state.clone(), content)));
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        // Scroll up well past the top; it must clamp, not snap back to bottom.
        for _ in 0..10 {
            wheel(&app, &mut viewport, size, 1.0);
            render_viewport(&app, &mut viewport, size);
        }
        let lines = render_viewport(&app, &mut viewport, size);

        assert!(!state.is_at_end());
        assert_eq!(state.position(), TuiViewportPosition::RowsFromTop(0));
        assert_eq!(&lines[0][..3], "1:0");
        assert_eq!(&lines[1][..3], "1:1");
        // A further up-scroll at the top is a no-op, but is consumed by default.
        assert!(wheel(&app, &mut viewport, size, 1.0));
    });
}

#[test]
fn scrolling_down_pins_to_bottom_without_overscrolling() {
    App::test((), |app| async move {
        let content = FakeContent::new((1..=5).map(|id| fake_item(id, 3)).collect());
        let state = TuiViewportedListState::new_at_end();
        let mut viewport =
            TuiScrollable::new(Box::new(viewport_with_state(state.clone(), content)));
        let size = TuiSize::new(8, 4);

        // Scroll up to the top, then back down past the end.
        render_viewport(&app, &mut viewport, size);
        for _ in 0..10 {
            wheel(&app, &mut viewport, size, 1.0);
            render_viewport(&app, &mut viewport, size);
        }
        for _ in 0..10 {
            wheel(&app, &mut viewport, size, -1.0);
            render_viewport(&app, &mut viewport, size);
        }
        let lines = render_viewport(&app, &mut viewport, size);

        // Pinned to the end: the last four rows, no blank rows below.
        assert!(state.is_at_end());
        assert_eq!(&lines[0][..3], "4:2");
        assert_eq!(&lines[3][..3], "5:2");
        // A further down-scroll at the bottom is a no-op, but is consumed by default.
        assert!(wheel(&app, &mut viewport, size, -1.0));
    });
}

#[test]
fn scrolling_is_a_noop_when_all_content_fits() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let state = TuiViewportedListState::new_at_end();
        let mut viewport =
            TuiScrollable::new(Box::new(viewport_with_state(state.clone(), content)));
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        assert!(wheel(&app, &mut viewport, size, -1.0));
        render_viewport(&app, &mut viewport, size);
        assert!(wheel(&app, &mut viewport, size, 1.0));
        assert!(state.is_at_end());
    });
}

#[test]
fn default_alignment_starts_short_content_at_the_top() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let mut viewport = viewport_with_state(TuiViewportedListState::new_at_end(), content);

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(&lines[0][..3], "1:0");
        assert_eq!(&lines[1][..3], "2:0");
        assert_eq!(lines[2].trim(), "");
        assert_eq!(lines[3].trim(), "");
    });
}

#[test]
fn grow_from_bottom_docks_short_content_at_the_bottom() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let state = TuiViewportedListState::new_at_end();
        let mut viewport = viewport_with_state(state, content)
            .with_vertical_alignment(TuiViewportVerticalAlignment::GrowFromBottom);

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(lines[0].trim(), "");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(&lines[2][..3], "1:0");
        assert_eq!(&lines[3][..3], "2:0");
    });
}

#[test]
fn grow_from_bottom_does_not_offset_rows_from_top() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let state = TuiViewportedListState::new_at_end();
        state.scroll_to_rows_from_top(0);
        let mut viewport = viewport_with_state(state, content)
            .with_vertical_alignment(TuiViewportVerticalAlignment::GrowFromBottom);

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(&lines[0][..3], "1:0");
        assert_eq!(&lines[1][..3], "2:0");
        assert_eq!(lines[2].trim(), "");
        assert_eq!(lines[3].trim(), "");
    });
}

#[test]
fn scrolling_notifies_the_view_when_scroll_state_changes() {
    App::test((), |app| async move {
        let content = FakeContent::new((1..=5).map(|id| fake_item(id, 3)).collect());
        let state = TuiViewportedListState::new_at_end();
        let mut viewport = TuiScrollable::new(Box::new(viewport_with_state(state, content)));
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        assert_eq!(
            wheel_with_notify_count(&app, &mut viewport, size, 1.0),
            (true, 1),
        );
    });
}

#[test]
fn propagating_scrollable_returns_unhandled_when_scroll_state_does_not_change() {
    App::test((), |app| async move {
        let content = FakeContent::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let state = TuiViewportedListState::new_at_end();
        let mut viewport =
            TuiScrollable::new(Box::new(viewport_with_state(state.clone(), content)))
                .with_propagate_mousewheel_if_not_handled(true);
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        assert_eq!(
            wheel_with_notify_count(&app, &mut viewport, size, -1.0),
            (false, 0),
        );
        assert_eq!(
            wheel_with_notify_count(&app, &mut viewport, size, 1.0),
            (false, 0),
        );
        assert!(state.is_at_end());
    });
}

struct CursorElement {
    cursor: (u16, u16),
}

impl TuiElement for CursorElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 3))
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {}

    fn cursor_position(&self, _area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        Some(self.cursor)
    }
}

struct SingleElementContent {
    element: RefCell<Option<Box<dyn TuiElement>>>,
}

impl TuiViewportedElement for SingleElementContent {
    fn visible_items(
        &self,
        _window: TuiViewportWindow,
        _available_width: u16,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiViewportContent {
        TuiViewportContent {
            content_height: 3,
            items: vec![TuiVisibleViewportItem {
                origin_y: 0,
                element: self
                    .element
                    .borrow_mut()
                    .take()
                    .expect("element is rendered once"),
            }],
        }
    }
}

fn single_element_viewport(
    position: TuiViewportPosition,
    element: Box<dyn TuiElement>,
) -> TuiViewportedList<SingleElementContent> {
    let state = TuiViewportedListState::new_at_end();
    state.set_position(position);
    TuiViewportedList::new(
        state,
        SingleElementContent {
            element: RefCell::new(Some(element)),
        },
    )
}

#[test]
fn cursor_position_is_shifted_into_the_visible_window() {
    App::test((), |app| async move {
        let mut viewport = single_element_viewport(
            TuiViewportPosition::RowsFromTop(1),
            CursorElement { cursor: (0, 2) }.finish(),
        );

        app.read(|app_ctx| {
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            viewport.layout(TuiConstraint::tight(TuiSize::new(3, 2)), &mut ctx, app_ctx);

            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            assert_eq!(
                viewport.cursor_position(TuiRect::new(0, 0, 3, 2), &mut paint_ctx),
                Some((0, 1)),
            );
        });
    });
}

struct DispatchRecorder {
    called: Rc<Cell<bool>>,
}

impl TuiElement for DispatchRecorder {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        constraint.clamp(TuiSize::new(1, 3))
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {}

    fn dispatch_event(
        &mut self,
        _event: &TuiEvent,
        _area: TuiRect,
        _event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        self.called.set(true);
        true
    }
}

#[test]
fn dispatch_filters_mouse_events_outside_visible_window() {
    App::test((), |app| async move {
        let called = Rc::new(Cell::new(false));
        let mut viewport = single_element_viewport(
            TuiViewportPosition::RowsFromTop(1),
            DispatchRecorder {
                called: called.clone(),
            }
            .finish(),
        );

        app.read(|app_ctx| {
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let mut event_ctx = TuiEventContext::default();
            viewport.layout(TuiConstraint::tight(TuiSize::new(3, 2)), &mut ctx, app_ctx);

            let event = TuiEvent::LeftMouseDown {
                position: TuiPoint::new(0, 2),
                modifiers: ModifiersState::default(),
                click_count: 1,
                is_first_mouse: false,
            };
            assert!(!viewport.dispatch_event(
                &event,
                TuiRect::new(0, 0, 3, 2),
                &mut event_ctx,
                &mut ctx,
                app_ctx,
            ));
            assert!(!called.get());
        });
    });
}
