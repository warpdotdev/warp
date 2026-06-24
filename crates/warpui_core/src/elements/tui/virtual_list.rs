//! Virtualized variable-height lists for large TUI surfaces.
//!
//! [`TuiColumn`](super::TuiColumn) is the right tool when the parent already has
//! concrete child elements to lay out and paint. A column asks each fixed child
//! for its desired height and renders the children in order, which means the
//! caller has already materialized the children for the frame.
//!
//! [`TuiVirtualList`] is for long, variable-height content where most rows are
//! off-screen, such as terminal history. Instead of owning child elements, it
//! owns a viewport and a persistent scroll anchor, asks a
//! [`TuiVirtualListSource`] which source items are adjacent to that anchor, and
//! requests only the row slices that intersect the viewport. This keeps normal
//! rendering proportional to visible rows/items, not total history size, and
//! avoids the "render everything into a large buffer, then clip" pattern.

use std::cell::RefCell;
use std::rc::Rc;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext, TuiRect, TuiSize,
};
use crate::geometry::vector::Vector2F;
use crate::{AppContext, Event};

const WHEEL_STEP: usize = 3;

/// A visible item plus the row offset where rendering starts inside it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionedItem<I> {
    pub item: I,
    pub row_offset: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum ScrollState<I> {
    FollowBottom,
    Anchored(PositionedItem<I>),
}

/// Persistent scroll state for a [`TuiVirtualList`].
///
/// Create this once in the owning view and clone it into each rendered list.
#[derive(Clone)]
pub struct TuiVirtualListHandle<I>(Rc<RefCell<ScrollState<I>>>);

impl<I> Default for TuiVirtualListHandle<I> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(ScrollState::FollowBottom)))
    }
}

impl<I: Clone> TuiVirtualListHandle<I> {
    /// Creates a handle pinned to the bottom of the list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins the viewport to the bottom of the source.
    pub fn follow_bottom(&self) {
        *self.0.borrow_mut() = ScrollState::FollowBottom;
    }

    /// Anchors the viewport to a source item and row offset.
    pub fn scroll_to_item(&self, item: I, row_offset: usize) {
        *self.0.borrow_mut() = ScrollState::Anchored(PositionedItem { item, row_offset });
    }

    /// Returns whether the viewport is currently pinned to the source bottom.
    pub fn is_following_bottom(&self) -> bool {
        matches!(*self.0.borrow(), ScrollState::FollowBottom)
    }

    fn state(&self) -> ScrollState<I> {
        self.0.borrow().clone()
    }

    fn set_state(&self, state: ScrollState<I>) {
        *self.0.borrow_mut() = state;
    }
}

/// Source of virtual-list items and visible row slices.
pub trait TuiVirtualListSource {
    type ItemId: Clone + Eq;
    /// Returns the first item in source order, if any.
    ///
    /// This may be called during event dispatch and rendering. Keep it cheap or
    /// backed by an index when the source is large.

    fn first_item(&self) -> Option<Self::ItemId>;
    /// Returns the last item in source order, if any.
    ///
    /// Used for bottom-follow rendering. It should not require materializing all
    /// source items.
    fn last_item(&self) -> Option<Self::ItemId>;
    /// Returns the next item after `item` in source order.
    ///
    /// Called repeatedly while filling a viewport from an anchor.
    fn next_item(&self, item: Self::ItemId) -> Option<Self::ItemId>;
    /// Returns the previous item before `item` in source order.
    ///
    /// Called repeatedly while finding the bottom-follow anchor and when
    /// scrolling upward.
    fn previous_item(&self, item: Self::ItemId) -> Option<Self::ItemId>;
    /// Returns `item`'s row height at `width`.
    ///
    /// This is a hot-path method: the list may call it multiple times for the
    /// same item during one layout, render, or event pass. Source implementations
    /// should make this O(1) or otherwise cached where possible.
    fn item_height(&self, item: Self::ItemId, width: u16) -> usize;
    /// Renders up to `rows` rows from `item`, starting at `row_offset`.
    ///
    /// Implementations must confine writes to `area` and should do work
    /// proportional to `rows * area.width`, not to the item's full height.
    fn render_item_slice(
        &self,
        item: Self::ItemId,
        row_offset: usize,
        rows: u16,
        area: TuiRect,
        buffer: &mut TuiBuffer,
    );

    /// Returns the item at an absolute row offset, when the source can seek.
    ///
    /// The generic list does not require this for normal anchored scrolling.
    /// Sources can implement it later for find, jump-to-row, restoration, or
    /// scrollbar positioning.
    fn seek_row(&self, _row: usize, _width: u16) -> Option<PositionedItem<Self::ItemId>> {
        None
    }

    /// Returns the total source height, when the source tracks it cheaply.
    ///
    /// This is optional because many virtualized sources can scroll and render
    /// efficiently without knowing an exact total height.
    fn total_height(&self, _width: u16) -> Option<usize> {
        None
    }
}

/// A viewported, variable-height TUI list that renders only visible row slices.
pub struct TuiVirtualList<S: TuiVirtualListSource> {
    handle: TuiVirtualListHandle<S::ItemId>,
    source: S,
}

impl<S: TuiVirtualListSource> TuiVirtualList<S> {
    /// Builds a virtual list over `source` using persistent `handle` state.
    pub fn new(handle: TuiVirtualListHandle<S::ItemId>, source: S) -> Self {
        Self { handle, source }
    }

    fn bottom_anchor(&self, width: u16, viewport_height: u16) -> Option<PositionedItem<S::ItemId>> {
        let mut item = self.source.last_item()?;
        let mut remaining = usize::from(viewport_height);
        loop {
            let height = self.source.item_height(item.clone(), width);
            if height == 0 {
                if let Some(previous) = self.source.previous_item(item.clone()) {
                    item = previous;
                    continue;
                }
                return None;
            }
            if height >= remaining {
                return Some(PositionedItem {
                    item: item.clone(),
                    row_offset: height - remaining,
                });
            }
            remaining -= height;
            let Some(previous) = self.source.previous_item(item.clone()) else {
                return Some(PositionedItem {
                    item,
                    row_offset: 0,
                });
            };
            item = previous;
        }
    }

    fn current_anchor(
        &self,
        width: u16,
        viewport_height: u16,
    ) -> Option<PositionedItem<S::ItemId>> {
        match self.handle.state() {
            ScrollState::FollowBottom => self.bottom_anchor(width, viewport_height),
            ScrollState::Anchored(anchor) => Some(self.clamp_anchor(anchor, width)),
        }
    }

    fn clamp_anchor(
        &self,
        anchor: PositionedItem<S::ItemId>,
        width: u16,
    ) -> PositionedItem<S::ItemId> {
        let height = self.source.item_height(anchor.item.clone(), width);
        PositionedItem {
            item: anchor.item,
            row_offset: anchor.row_offset.min(height.saturating_sub(1)),
        }
    }

    fn rows_from_anchor_to_end(
        &self,
        mut anchor: PositionedItem<S::ItemId>,
        width: u16,
        cap: usize,
    ) -> usize {
        let mut rows = 0usize;
        loop {
            let height = self.source.item_height(anchor.item.clone(), width);
            rows = rows.saturating_add(height.saturating_sub(anchor.row_offset));
            if rows > cap {
                return rows;
            }
            let Some(next) = self.source.next_item(anchor.item.clone()) else {
                return rows;
            };
            anchor = PositionedItem {
                item: next,
                row_offset: 0,
            };
        }
    }

    fn normalize_anchor(
        &self,
        anchor: PositionedItem<S::ItemId>,
        width: u16,
        viewport_height: u16,
    ) -> ScrollState<S::ItemId> {
        if self.rows_from_anchor_to_end(anchor.clone(), width, usize::from(viewport_height))
            <= usize::from(viewport_height)
        {
            ScrollState::FollowBottom
        } else {
            ScrollState::Anchored(anchor)
        }
    }

    fn scroll_anchor(
        &self,
        anchor: PositionedItem<S::ItemId>,
        rows: isize,
        width: u16,
        viewport_height: u16,
    ) -> ScrollState<S::ItemId> {
        if rows < 0 {
            ScrollState::Anchored(self.scroll_toward_start(anchor, rows.unsigned_abs(), width))
        } else if rows > 0 {
            let anchor = self.scroll_toward_end(anchor, rows as usize, width);
            self.normalize_anchor(anchor, width, viewport_height)
        } else {
            self.normalize_anchor(anchor, width, viewport_height)
        }
    }

    fn scroll_toward_start(
        &self,
        mut anchor: PositionedItem<S::ItemId>,
        mut rows: usize,
        width: u16,
    ) -> PositionedItem<S::ItemId> {
        while rows > 0 {
            if anchor.row_offset >= rows {
                anchor.row_offset -= rows;
                return anchor;
            }
            rows -= anchor.row_offset;
            let Some(previous) = self.source.previous_item(anchor.item.clone()) else {
                anchor.row_offset = 0;
                return anchor;
            };
            anchor.item = previous.clone();
            anchor.row_offset = self.source.item_height(previous, width);
        }
        anchor
    }

    fn scroll_toward_end(
        &self,
        mut anchor: PositionedItem<S::ItemId>,
        mut rows: usize,
        width: u16,
    ) -> PositionedItem<S::ItemId> {
        while rows > 0 {
            let height = self.source.item_height(anchor.item.clone(), width);
            let remaining_in_item = height.saturating_sub(anchor.row_offset);
            if rows < remaining_in_item {
                anchor.row_offset += rows;
                return anchor;
            }
            rows -= remaining_in_item;
            let Some(next) = self.source.next_item(anchor.item.clone()) else {
                anchor.row_offset = height;
                return anchor;
            };
            anchor.item = next;
            anchor.row_offset = 0;
        }
        anchor
    }
}

impl<S: TuiVirtualListSource> TuiElement for TuiVirtualList<S> {
    fn layout(&mut self, constraint: TuiConstraint, _ctx: &mut TuiLayoutContext) -> TuiSize {
        constraint.max
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {
        if area.is_empty() {
            return;
        }

        let Some(mut anchor) = self.current_anchor(area.width, area.height) else {
            return;
        };

        let mut y = area.y;
        let mut remaining = area.height;
        while remaining > 0 {
            let height = self.source.item_height(anchor.item.clone(), area.width);
            if anchor.row_offset < height {
                let rows = (height - anchor.row_offset).min(usize::from(remaining)) as u16;
                let slice_area = TuiRect::new(area.x, y, area.width, rows);
                self.source.render_item_slice(
                    anchor.item.clone(),
                    anchor.row_offset,
                    rows,
                    slice_area,
                    buffer,
                );
                y = y.saturating_add(rows);
                remaining -= rows;
            }
            let Some(next) = self.source.next_item(anchor.item.clone()) else {
                break;
            };
            anchor = PositionedItem {
                item: next,
                row_offset: 0,
            };
        }
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        _event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        if area.is_empty() {
            return false;
        }

        let rows = match event {
            Event::ScrollWheel {
                position, delta, ..
            } => {
                if !contains(area, *position) {
                    return false;
                }
                -((delta.y() as isize) * WHEEL_STEP as isize)
            }
            Event::KeyDown { keystroke, .. } => {
                let page = area.height.saturating_sub(1).max(1) as isize;
                match keystroke.key.as_str() {
                    "down" => 1,
                    "up" => -1,
                    "pagedown" => page,
                    "pageup" => -page,
                    "home" => {
                        if let Some(first) = self.source.first_item() {
                            let next = ScrollState::Anchored(PositionedItem {
                                item: first,
                                row_offset: 0,
                            });
                            if self.handle.state() != next {
                                self.handle.set_state(next);
                                return true;
                            }
                        }
                        return false;
                    }
                    "end" => {
                        if !self.handle.is_following_bottom() {
                            self.handle.follow_bottom();
                            return true;
                        }
                        return false;
                    }
                    _ => return false,
                }
            }
            _ => return false,
        };

        let Some(anchor) = self.current_anchor(area.width, area.height) else {
            return false;
        };
        let next = self.scroll_anchor(anchor, rows, area.width, area.height);
        if self.handle.state() == next {
            return false;
        }
        self.handle.set_state(next);
        true
    }
}

fn contains(area: TuiRect, position: Vector2F) -> bool {
    let x = position.x();
    let y = position.y();
    x >= f32::from(area.x)
        && x < f32::from(area.right())
        && y >= f32::from(area.y)
        && y < f32::from(area.bottom())
}

#[cfg(test)]
#[path = "virtual_list_tests.rs"]
mod tests;
