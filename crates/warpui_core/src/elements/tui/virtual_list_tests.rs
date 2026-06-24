use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{TuiVirtualList, TuiVirtualListHandle, TuiVirtualListSource};
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext, TuiRect,
    TuiSize, TuiStyle,
};
use crate::event::{KeyEventDetails, ModifiersState};
use crate::geometry::vector::{vec2f, Vector2F};
use crate::keymap::Keystroke;
use crate::{App, Event};

#[derive(Clone)]
struct RowItem {
    rows: Vec<&'static str>,
}

#[derive(Clone)]
struct FakeSource {
    items: Vec<RowItem>,
    rendered_slices: Rc<RefCell<Vec<(usize, usize, u16)>>>,
}

impl FakeSource {
    fn new(items: Vec<Vec<&'static str>>) -> Self {
        Self {
            items: items.into_iter().map(|rows| RowItem { rows }).collect(),
            rendered_slices: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn rendered_slices(&self) -> Vec<(usize, usize, u16)> {
        self.rendered_slices.borrow().clone()
    }
}

impl TuiVirtualListSource for FakeSource {
    type ItemId = usize;

    fn first_item(&self) -> Option<Self::ItemId> {
        (!self.items.is_empty()).then_some(0)
    }

    fn last_item(&self) -> Option<Self::ItemId> {
        self.items.len().checked_sub(1)
    }

    fn next_item(&self, item: Self::ItemId) -> Option<Self::ItemId> {
        (item + 1 < self.items.len()).then_some(item + 1)
    }

    fn previous_item(&self, item: Self::ItemId) -> Option<Self::ItemId> {
        item.checked_sub(1)
    }

    fn item_height(&self, item: Self::ItemId, _width: u16) -> usize {
        self.items[item].rows.len()
    }

    fn render_item_slice(
        &self,
        item: Self::ItemId,
        row_offset: usize,
        rows: u16,
        area: TuiRect,
        buffer: &mut TuiBuffer,
    ) {
        self.rendered_slices
            .borrow_mut()
            .push((item, row_offset, rows));

        for row in 0..rows {
            let source_row = row_offset + usize::from(row);
            let text = self.items[item].rows[source_row];
            buffer.set_string(area.x, area.y + row, text, TuiStyle::default());
        }
    }

    fn total_height(&self, _width: u16) -> Option<usize> {
        Some(self.items.iter().map(|item| item.rows.len()).sum())
    }
}

fn lines(element: &mut dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(TuiConstraint::tight(size), &mut ctx);
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
    element.render(
        TuiRect::new(0, 0, size.width, size.height),
        &mut buffer,
        &mut ctx,
    );
    buffer.to_lines()
}

/// Lays out a leaf TUI element with no embedded child views.
fn layout_element(element: &mut dyn TuiElement, size: TuiSize) {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(TuiConstraint::tight(size), &mut ctx);
}

/// Dispatches to a leaf TUI element with no embedded child views.
fn dispatch_event(
    element: &mut dyn TuiElement,
    event: &Event,
    area: TuiRect,
    event_ctx: &mut TuiEventContext,
    app_ctx: &crate::AppContext,
) -> bool {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.dispatch_event(event, area, event_ctx, &mut ctx, app_ctx)
}

fn source() -> FakeSource {
    FakeSource::new(vec![
        vec!["a0", "a1"],
        vec!["b0", "b1", "b2"],
        vec!["c0"],
        vec!["d0", "d1"],
    ])
}

fn scroll_wheel(position: Vector2F, delta_y: f32) -> Event {
    Event::ScrollWheel {
        position,
        delta: vec2f(0.0, delta_y),
        precise: false,
        modifiers: ModifiersState::default(),
    }
}

fn key(name: &str) -> Event {
    Event::KeyDown {
        keystroke: Keystroke {
            key: name.to_owned(),
            ..Default::default()
        },
        chars: String::new(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

#[test]
fn follow_bottom_renders_only_visible_tail_slices() {
    let handle = TuiVirtualListHandle::new();
    let source = source();
    let observed_source = source.clone();
    let mut list = TuiVirtualList::new(handle, source);

    assert_eq!(
        lines(&mut list, TuiSize::new(2, 4)),
        vec!["b2", "c0", "d0", "d1"],
    );
    assert_eq!(
        observed_source.rendered_slices(),
        vec![(1, 2, 1), (2, 0, 1), (3, 0, 2)]
    );
}

#[test]
fn anchored_render_starts_inside_an_item_and_stops_at_viewport() {
    let handle = TuiVirtualListHandle::new();
    handle.scroll_to_item(1, 1);
    let source = source();
    let observed_source = source.clone();
    let mut list = TuiVirtualList::new(handle, source);

    assert_eq!(lines(&mut list, TuiSize::new(2, 3)), vec!["b1", "b2", "c0"],);
    assert_eq!(
        observed_source.rendered_slices(),
        vec![(1, 1, 2), (2, 0, 1)]
    );
}

#[test]
fn scrolling_up_from_bottom_disables_follow_bottom() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = TuiVirtualListHandle::new();
            let mut list = TuiVirtualList::new(handle.clone(), source());
            let area = TuiRect::new(0, 0, 2, 4);
            layout_element(&mut list, area.as_size());

            let mut event_ctx = TuiEventContext::default();
            let handled = dispatch_event(&mut list, &key("up"), area, &mut event_ctx, app_ctx);

            assert!(handled);
            assert!(!handle.is_following_bottom());
            assert_eq!(
                lines(&mut list, area.as_size()),
                vec!["b1", "b2", "c0", "d0"]
            );
        });
    });
}

#[test]
fn scrolling_down_to_tail_restores_follow_bottom() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = TuiVirtualListHandle::new();
            handle.scroll_to_item(1, 1);
            let mut list = TuiVirtualList::new(handle.clone(), source());
            let area = TuiRect::new(0, 0, 2, 4);
            layout_element(&mut list, area.as_size());

            let mut event_ctx = TuiEventContext::default();
            let handled = dispatch_event(&mut list, &key("down"), area, &mut event_ctx, app_ctx);

            assert!(handled);
            assert!(handle.is_following_bottom());
            assert_eq!(
                lines(&mut list, area.as_size()),
                vec!["b2", "c0", "d0", "d1"]
            );
        });
    });
}

#[test]
fn mouse_wheel_outside_viewport_is_ignored() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let handle = TuiVirtualListHandle::new();
            let mut list = TuiVirtualList::new(handle.clone(), source());
            let area = TuiRect::new(0, 0, 2, 4);
            layout_element(&mut list, area.as_size());

            let mut event_ctx = TuiEventContext::default();
            let handled = dispatch_event(
                &mut list,
                &scroll_wheel(vec2f(0.0, 10.0), -1.0),
                area,
                &mut event_ctx,
                app_ctx,
            );

            assert!(!handled);
            assert!(handle.is_following_bottom());
        });
    });
}
