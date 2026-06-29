use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use super::{
    RenderedViewportItem, TuiViewportCursor, TuiViewportHandle, TuiViewportIndex,
    TuiViewportIndexItem, TuiViewportIndexPosition, TuiViewportedList, ViewportRenderRequest,
};
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext, TuiRect,
    TuiScrollable, TuiSize, TuiText,
};
use crate::event::ModifiersState;
use crate::geometry::vector::Vector2F;
use crate::{App, EntityId, Event};

#[derive(Clone)]
struct FakeItem {
    id: usize,
    lines: Vec<String>,
    height: usize,
    needs_measurement: bool,
}

#[test]
fn width_change_remeasures_view_measured_items() {
    App::test((), |app| async move {
        let mut item = fake_item(1, 1);
        item.needs_measurement = false;
        let index = FakeIndex::new(vec![item]);
        let updates = index.height_updates.clone();
        let mut viewport =
            TuiViewportedList::new(TuiViewportHandle::new(), index, move |request, _| {
                RenderedViewportItem {
                    element: Box::new(TuiText::new("row")),
                    measured_full_height: Some(usize::from(request.width / 4).max(1)),
                }
            });

        render_viewport(&app, &mut viewport, TuiSize::new(4, 4));
        render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(updates.borrow().as_slice(), &[vec![(1, 2)]]);
    });
}

#[test]
fn missing_anchor_falls_back_to_follow_bottom() {
    App::test((), |app| async move {
        let index = FakeIndex::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let handle = TuiViewportHandle::new();
        handle.scroll_to_item(99, 0);
        let mut viewport =
            TuiViewportedList::new(handle.clone(), index, |request, _| slice_element(request));

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 1));

        assert!(handle.is_following_bottom());
        assert_eq!(&lines[0][..3], "2:0");
    });
}

#[derive(Clone)]
struct FakeIndex {
    items: Rc<RefCell<Vec<FakeItem>>>,
    cursor_open: Rc<Cell<bool>>,
    height_updates: HeightUpdates,
}

type HeightUpdates = Rc<RefCell<Vec<Vec<(usize, usize)>>>>;
impl FakeIndex {
    fn new(items: Vec<FakeItem>) -> Self {
        Self {
            items: Rc::new(RefCell::new(items)),
            cursor_open: Rc::new(Cell::new(false)),
            height_updates: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

struct FakeCursor<'a> {
    items: Ref<'a, Vec<FakeItem>>,
    position: Option<usize>,
}

impl TuiViewportCursor for FakeCursor<'_> {
    type ItemId = usize;
    type Item = FakeItem;

    fn item(&self) -> Option<TuiViewportIndexItem<Self::ItemId, Self::Item>> {
        let item = self.items.get(self.position?)?.clone();
        Some(TuiViewportIndexItem {
            id: item.id,
            height: item.height,
            needs_measurement: item.needs_measurement,
            item,
        })
    }

    fn next(&mut self) {
        self.position = self
            .position
            .and_then(|position| (position + 1 < self.items.len()).then_some(position + 1));
    }

    fn prev(&mut self) {
        self.position = match self.position {
            Some(0) | None => None,
            Some(position) => Some(position - 1),
        };
    }
}

impl TuiViewportIndex for FakeIndex {
    type ItemId = usize;
    type Item = FakeItem;

    fn with_cursor<R>(
        &self,
        position: TuiViewportIndexPosition<'_, Self::ItemId>,
        f: impl FnOnce(&mut dyn TuiViewportCursor<ItemId = Self::ItemId, Item = Self::Item>) -> R,
    ) -> R {
        let items = self.items.borrow();
        let position = match position {
            TuiViewportIndexPosition::Start => (!items.is_empty()).then_some(0),
            TuiViewportIndexPosition::End => items.len().checked_sub(1),
            TuiViewportIndexPosition::Item(id) => items.iter().position(|item| item.id == *id),
        };
        self.cursor_open.set(true);
        let result = f(&mut FakeCursor { items, position });
        self.cursor_open.set(false);
        result
    }

    fn update_heights(&self, updates: &[(Self::ItemId, usize)]) {
        self.height_updates.borrow_mut().push(updates.to_vec());
        let mut items = self.items.borrow_mut();
        for (id, height) in updates {
            if let Some(item) = items.iter_mut().find(|item| item.id == *id) {
                item.height = *height;
                item.needs_measurement = false;
            }
        }
    }
}

fn fake_item(id: usize, height: usize) -> FakeItem {
    FakeItem {
        id,
        lines: (0..height).map(|row| format!("{id}:{row}")).collect(),
        height,
        needs_measurement: false,
    }
}

fn slice_element(request: ViewportRenderRequest<FakeItem>) -> RenderedViewportItem {
    let lines = request.item.lines[request.visible_rows].join("\n");
    RenderedViewportItem {
        element: Box::new(TuiText::new(lines).truncate()),
        measured_full_height: None,
    }
}

fn render_viewport(app: &App, viewport: &mut impl TuiElement, size: TuiSize) -> Vec<String> {
    app.read(|app_ctx| {
        let mut rendered_views = HashMap::new();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        viewport.layout(TuiConstraint::tight(size), &mut ctx, app_ctx);
        let area = TuiRect::new(0, 0, size.width, size.height);
        let mut buffer = TuiBuffer::empty(area);
        viewport.render(area, &mut buffer, &mut ctx);
        buffer.to_lines()
    })
}

/// Dispatches a wheel event against the viewport's last layout. Positive
/// `delta_y` scrolls toward the top; negative scrolls toward the bottom
/// (matching the crossterm → warp wheel mapping). Returns whether the event was
/// handled.
fn wheel(app: &App, viewport: &mut impl TuiElement, size: TuiSize, delta_y: f32) -> bool {
    wheel_with_update_count(app, viewport, size, delta_y).0
}

fn wheel_with_update_count(
    app: &App,
    viewport: &mut impl TuiElement,
    size: TuiSize,
    delta_y: f32,
) -> (bool, usize) {
    app.read(|app_ctx| {
        let mut rendered_views = HashMap::new();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let area = TuiRect::new(0, 0, size.width, size.height);
        let mut event_ctx = TuiEventContext::default();
        event_ctx.set_origin_view(Some(EntityId::new()));
        let event = Event::ScrollWheel {
            position: Vector2F::new(0.0, 0.0),
            delta: Vector2F::new(0.0, delta_y),
            precise: false,
            modifiers: ModifiersState::default(),
        };
        let handled = viewport.dispatch_event(&event, area, &mut event_ctx, &mut ctx, app_ctx);
        (handled, event_ctx.take_updates().len())
    })
}

#[test]
fn scrolling_up_clamps_to_the_top_without_snapping_to_bottom() {
    App::test((), |app| async move {
        let index = FakeIndex::new((1..=5).map(|id| fake_item(id, 3)).collect());
        let handle = TuiViewportHandle::new();
        let mut viewport = TuiScrollable::new(TuiViewportedList::new(
            handle.clone(),
            index,
            |request, _| slice_element(request),
        ));
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        // Scroll up well past the top; it must clamp, not snap back to bottom.
        for _ in 0..10 {
            wheel(&app, &mut viewport, size, 1.0);
            render_viewport(&app, &mut viewport, size);
        }
        let lines = render_viewport(&app, &mut viewport, size);

        assert!(!handle.is_following_bottom());
        assert_eq!(&lines[0][..3], "1:0");
        assert_eq!(&lines[1][..3], "1:1");
        // A further up-scroll at the top is a no-op, but is consumed by default.
        assert!(wheel(&app, &mut viewport, size, 1.0));
    });
}

#[test]
fn scrolling_down_pins_to_bottom_without_overscrolling() {
    App::test((), |app| async move {
        let index = FakeIndex::new((1..=5).map(|id| fake_item(id, 3)).collect());
        let handle = TuiViewportHandle::new();
        let mut viewport = TuiScrollable::new(TuiViewportedList::new(
            handle.clone(),
            index,
            |request, _| slice_element(request),
        ));
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
        assert!(handle.is_following_bottom());
        assert_eq!(&lines[0][..3], "4:2");
        assert_eq!(&lines[3][..3], "5:2");
        // A further down-scroll at the bottom is a no-op, but is consumed by default.
        assert!(wheel(&app, &mut viewport, size, -1.0));
    });
}

#[test]
fn scrolling_is_a_noop_when_all_content_fits() {
    App::test((), |app| async move {
        let index = FakeIndex::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let handle = TuiViewportHandle::new();
        let mut viewport = TuiScrollable::new(TuiViewportedList::new(
            handle.clone(),
            index,
            |request, _| slice_element(request),
        ));
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        assert!(wheel(&app, &mut viewport, size, -1.0));
        render_viewport(&app, &mut viewport, size);
        assert!(wheel(&app, &mut viewport, size, 1.0));
        assert!(handle.is_following_bottom());
    });
}

#[test]
fn scrolling_queues_a_view_update_when_scroll_state_changes() {
    App::test((), |app| async move {
        let index = FakeIndex::new((1..=5).map(|id| fake_item(id, 3)).collect());
        let handle = TuiViewportHandle::new();
        let mut viewport =
            TuiScrollable::new(TuiViewportedList::new(handle, index, |request, _| {
                slice_element(request)
            }));
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        assert_eq!(
            wheel_with_update_count(&app, &mut viewport, size, 1.0),
            (true, 1),
        );
    });
}

#[test]
fn propagating_scrollable_returns_unhandled_when_scroll_state_does_not_change() {
    App::test((), |app| async move {
        let index = FakeIndex::new(vec![fake_item(1, 1), fake_item(2, 1)]);
        let handle = TuiViewportHandle::new();
        let mut viewport = TuiScrollable::new(TuiViewportedList::new(
            handle.clone(),
            index,
            |request, _| slice_element(request),
        ))
        .with_propagate_mousewheel_if_not_handled(true);
        let size = TuiSize::new(8, 4);

        render_viewport(&app, &mut viewport, size);
        assert_eq!(
            wheel_with_update_count(&app, &mut viewport, size, -1.0),
            (false, 0),
        );
        assert_eq!(
            wheel_with_update_count(&app, &mut viewport, size, 1.0),
            (false, 0),
        );
        assert!(handle.is_following_bottom());
    });
}

#[test]
fn follow_bottom_renders_only_the_visible_item_rows() {
    App::test((), |app| async move {
        let index = FakeIndex::new(vec![fake_item(1, 3), fake_item(2, 3), fake_item(3, 3)]);
        let requests = Rc::new(RefCell::new(Vec::<(usize, Range<usize>)>::new()));
        let requests_for_render = requests.clone();
        let mut viewport =
            TuiViewportedList::new(TuiViewportHandle::new(), index, move |request, _| {
                requests_for_render
                    .borrow_mut()
                    .push((request.item.id, request.visible_rows.clone()));
                slice_element(request)
            });

        let lines = render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(requests.borrow().as_slice(), &[(2, 2..3), (3, 0..3)]);
        assert_eq!(&lines[0][..3], "2:2");
        assert_eq!(&lines[1][..3], "3:0");
        assert_eq!(&lines[3][..3], "3:2");
    });
}

#[test]
fn anchored_viewport_starts_at_the_requested_item_row() {
    App::test((), |app| async move {
        let index = FakeIndex::new(vec![fake_item(1, 3), fake_item(2, 3), fake_item(3, 3)]);
        let handle = TuiViewportHandle::new();
        handle.scroll_to_item(1, 1);
        let requests = Rc::new(RefCell::new(Vec::<(usize, Range<usize>)>::new()));
        let requests_for_render = requests.clone();
        let mut viewport = TuiViewportedList::new(handle, index, move |request, _| {
            requests_for_render
                .borrow_mut()
                .push((request.item.id, request.visible_rows.clone()));
            slice_element(request)
        });

        render_viewport(&app, &mut viewport, TuiSize::new(8, 4));

        assert_eq!(requests.borrow().as_slice(), &[(1, 1..3), (2, 0..2)]);
    });
}

#[test]
fn traversal_scope_ends_before_item_rendering() {
    App::test((), |app| async move {
        let index = FakeIndex::new(vec![fake_item(1, 1)]);
        let cursor_open = index.cursor_open.clone();
        let mut viewport =
            TuiViewportedList::new(TuiViewportHandle::new(), index, move |request, _| {
                assert!(!cursor_open.get(), "cursor scope must end before rendering");
                slice_element(request)
            });

        render_viewport(&app, &mut viewport, TuiSize::new(8, 1));
    });
}

#[test]
fn measured_height_updates_stabilize_in_the_same_layout() {
    App::test((), |app| async move {
        let mut item = fake_item(1, 1);
        item.lines = vec!["1:0".to_owned(), "1:1".to_owned(), "1:2".to_owned()];
        item.needs_measurement = true;
        let index = FakeIndex::new(vec![item]);
        let updates = index.height_updates.clone();
        let requests = Rc::new(RefCell::new(Vec::<Range<usize>>::new()));
        let requests_for_render = requests.clone();
        let mut viewport =
            TuiViewportedList::new(TuiViewportHandle::new(), index, move |request, _| {
                requests_for_render
                    .borrow_mut()
                    .push(request.visible_rows.clone());
                RenderedViewportItem {
                    element: Box::new(
                        TuiText::new(request.item.lines[request.visible_rows].join("\n"))
                            .truncate(),
                    ),
                    measured_full_height: Some(3),
                }
            });

        render_viewport(&app, &mut viewport, TuiSize::new(8, 3));

        assert_eq!(updates.borrow().as_slice(), &[vec![(1, 3)]]);
        assert_eq!(requests.borrow().as_slice(), &[0..1, 0..3]);
    });
}
