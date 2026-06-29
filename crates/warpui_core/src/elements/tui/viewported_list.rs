//! A generalized, source-driven viewport for ordered variable-height TUI items.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiScrollableElement, TuiSize,
};
use crate::{AppContext, Event};

const MAX_STABILIZATION_PASSES: usize = 4;

/// A stable item-relative scroll anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiViewportAnchor<ItemId> {
    pub item_id: ItemId,
    pub row_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiViewportScrollState<ItemId> {
    FollowBottom,
    Anchored(TuiViewportAnchor<ItemId>),
}

/// Persistent scroll state for a [`TuiViewportedList`].
#[derive(Clone)]
pub struct TuiViewportHandle<ItemId>(Rc<RefCell<TuiViewportScrollState<ItemId>>>);

impl<ItemId> Default for TuiViewportHandle<ItemId> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(TuiViewportScrollState::FollowBottom)))
    }
}

impl<ItemId: Clone> TuiViewportHandle<ItemId> {
    /// Creates viewport state that initially follows the end of the index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins the viewport to the end of the index.
    pub fn follow_bottom(&self) {
        *self.0.borrow_mut() = TuiViewportScrollState::FollowBottom;
    }

    /// Anchors the viewport to `item_id` at `row_offset`.
    pub fn scroll_to_item(&self, item_id: ItemId, row_offset: usize) {
        *self.0.borrow_mut() = TuiViewportScrollState::Anchored(TuiViewportAnchor {
            item_id,
            row_offset,
        });
    }

    /// Returns whether the viewport is following the end of the index.
    pub fn is_following_bottom(&self) -> bool {
        matches!(*self.0.borrow(), TuiViewportScrollState::FollowBottom)
    }

    fn state(&self) -> TuiViewportScrollState<ItemId> {
        self.0.borrow().clone()
    }

    fn set_state(&self, state: TuiViewportScrollState<ItemId>) {
        *self.0.borrow_mut() = state;
    }
}

/// The position at which an ordered-index cursor should start.
#[derive(Clone, Copy)]
pub enum TuiViewportIndexPosition<'a, ItemId> {
    Start,
    End,
    Item(&'a ItemId),
}

/// An owned ordered-index item descriptor.
pub struct TuiViewportIndexItem<ItemId, Item> {
    pub id: ItemId,
    pub item: Item,
    pub height: usize,
    pub needs_measurement: bool,
}

/// A scoped cursor over a caller-owned ordered index.
pub trait TuiViewportCursor {
    type ItemId: Clone + Eq;
    type Item;

    /// Returns an owned descriptor for the current item.
    fn item(&self) -> Option<TuiViewportIndexItem<Self::ItemId, Self::Item>>;

    /// Moves to the next item.
    fn next(&mut self);

    /// Moves to the previous item.
    fn prev(&mut self);
}

/// Adapts a caller-owned ordered-height index for [`TuiViewportedList`].
pub trait TuiViewportIndex {
    type ItemId: Clone + Eq;
    type Item;

    /// Opens a scoped cursor and releases all borrowed backing state when `f` returns.
    fn with_cursor<R>(
        &self,
        position: TuiViewportIndexPosition<'_, Self::ItemId>,
        f: impl FnOnce(&mut dyn TuiViewportCursor<ItemId = Self::ItemId, Item = Self::Item>) -> R,
    ) -> R;

    /// Applies view-measured item heights to the backing index.
    fn update_heights(&self, _updates: &[(Self::ItemId, usize)]) {}
}

/// The request passed to a viewport's injected item renderer.
pub struct ViewportRenderRequest<Item> {
    pub item: Item,
    pub visible_rows: Range<usize>,
    pub width: u16,
}

/// A visible item element and optional full logical height measurement.
pub struct RenderedViewportItem {
    pub element: Box<dyn TuiElement>,
    pub measured_full_height: Option<usize>,
}

struct CollectedItem<ItemId, Item> {
    id: ItemId,
    item: Item,
    indexed_height: usize,
    needs_measurement: bool,
    visible_rows: Range<usize>,
}

struct VisibleElement<ItemId> {
    _item_id: ItemId,
    element: Box<dyn TuiElement>,
    height: u16,
}

/// A variable-height viewport that delegates ordered storage and item rendering.
pub struct TuiViewportedList<Index, RenderItem>
where
    Index: TuiViewportIndex,
{
    index: Index,
    render_item: RenderItem,
    handle: TuiViewportHandle<Index::ItemId>,
    visible_elements: Vec<VisibleElement<Index::ItemId>>,
    first_anchor: Option<TuiViewportAnchor<Index::ItemId>>,
    last_width: Option<u16>,
    size: TuiSize,
}

impl<Index, RenderItem> TuiViewportedList<Index, RenderItem>
where
    Index: TuiViewportIndex,
    RenderItem: Fn(ViewportRenderRequest<Index::Item>, &AppContext) -> RenderedViewportItem,
{
    /// Creates a generalized viewport over `index` with an injected item renderer.
    pub fn new(
        handle: TuiViewportHandle<Index::ItemId>,
        index: Index,
        render_item: RenderItem,
    ) -> Self {
        Self {
            index,
            render_item,
            handle,
            visible_elements: Vec::new(),
            first_anchor: None,
            last_width: None,
            size: TuiSize::ZERO,
        }
    }

    fn collect_visible(
        &self,
        viewport_height: usize,
    ) -> Vec<CollectedItem<Index::ItemId, Index::Item>> {
        let state = self.handle.state();
        let collected = match &state {
            TuiViewportScrollState::FollowBottom => self
                .index
                .with_cursor(TuiViewportIndexPosition::End, |cursor| {
                    collect_from_bottom(cursor, viewport_height)
                }),
            TuiViewportScrollState::Anchored(anchor) => self
                .index
                .with_cursor(TuiViewportIndexPosition::Item(&anchor.item_id), |cursor| {
                    collect_from_anchor(cursor, anchor.row_offset, viewport_height)
                }),
        };
        if collected.is_empty() && matches!(state, TuiViewportScrollState::Anchored(_)) {
            self.handle.follow_bottom();
            self.index
                .with_cursor(TuiViewportIndexPosition::End, |cursor| {
                    collect_from_bottom(cursor, viewport_height)
                })
        } else {
            collected
        }
    }

    fn layout_visible_elements(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) {
        self.visible_elements.clear();
        self.first_anchor = None;
        let viewport_height = usize::from(constraint.max.height);
        let width = constraint.max.width;
        let width_changed = self.last_width.replace(width) != Some(width);

        for pass in 0..MAX_STABILIZATION_PASSES {
            let collected = self.collect_visible(viewport_height);
            self.first_anchor = collected.first().map(|item| TuiViewportAnchor {
                item_id: item.id.clone(),
                row_offset: item.visible_rows.start,
            });

            let mut rendered = Vec::with_capacity(collected.len());
            let mut height_updates = Vec::new();
            for item in collected {
                let visible_height = item.visible_rows.len().min(usize::from(u16::MAX)) as u16;
                let result = (self.render_item)(
                    ViewportRenderRequest {
                        item: item.item,
                        visible_rows: item.visible_rows,
                        width,
                    },
                    app,
                );
                if item.needs_measurement || width_changed {
                    if let Some(height) = result.measured_full_height {
                        if height != item.indexed_height {
                            height_updates.push((item.id.clone(), height));
                        }
                    }
                }
                rendered.push(VisibleElement {
                    _item_id: item.id,
                    element: result.element,
                    height: visible_height,
                });
            }

            if !height_updates.is_empty() {
                self.index.update_heights(&height_updates);
                if pass + 1 < MAX_STABILIZATION_PASSES {
                    continue;
                }
                log::warn!("TUI viewport item heights did not stabilize during layout");
            }

            self.visible_elements = rendered;
            break;
        }

        for visible in &mut self.visible_elements {
            visible.element.layout(
                TuiConstraint::tight(TuiSize::new(width, visible.height)),
                ctx,
                app,
            );
        }
    }

    /// Scrolls the viewport by `rows` (negative = toward the top), clamping at
    /// both ends. `viewport_height` is needed for the bottom clamp: once a
    /// screenful or less remains below the new top, the viewport pins to the
    /// end instead of scrolling content off the bottom into blank space.
    fn scroll_by(&self, rows: isize, viewport_height: usize) -> bool {
        if rows == 0 || viewport_height == 0 {
            return false;
        }
        // Resolve the current top anchor. `FollowBottom` only yields a top
        // anchor for upward scrolls; downward is already pinned to the end.
        let current = match self.handle.state() {
            TuiViewportScrollState::FollowBottom if rows < 0 => self.first_anchor.clone(),
            TuiViewportScrollState::FollowBottom => return false,
            TuiViewportScrollState::Anchored(anchor) => Some(anchor),
        };
        let Some(current) = current else {
            return false;
        };

        // Walk the top anchor by `rows`, clamping at the very top.
        let current_item_id = current.item_id.clone();
        let candidate = self
            .index
            .with_cursor(TuiViewportIndexPosition::Item(&current_item_id), |cursor| {
                walk_anchor(cursor, current, rows)
            });
        let Some(candidate) = candidate else {
            return false;
        };

        // Clamp at the bottom: if a screenful or less remains below the new top,
        // pin to the end rather than leaving blank rows below the last item.
        let candidate_item_id = candidate.item_id.clone();
        let fills_viewport = self.index.with_cursor(
            TuiViewportIndexPosition::Item(&candidate_item_id),
            |cursor| rows_below_exceed(cursor, candidate.row_offset, viewport_height),
        );
        let next = if fills_viewport {
            TuiViewportScrollState::Anchored(candidate)
        } else {
            TuiViewportScrollState::FollowBottom
        };
        if self.handle.state() == next {
            return false;
        }
        self.handle.set_state(next);
        true
    }
}

impl<Index, RenderItem> TuiElement for TuiViewportedList<Index, RenderItem>
where
    Index: TuiViewportIndex,
    RenderItem: Fn(ViewportRenderRequest<Index::Item>, &AppContext) -> RenderedViewportItem,
{
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.layout_visible_elements(constraint, ctx, app);
        self.size = constraint.max;
        self.size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        let mut y = area.y;
        for visible in &self.visible_elements {
            if y >= area.bottom() {
                break;
            }
            let height = visible.height.min(area.bottom() - y);
            let slot = TuiRect::new(area.x, y, area.width, height);
            visible.element.render(slot, buffer, ctx);
            y = y.saturating_add(height);
        }
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        let mut y = area.y;
        for visible in &self.visible_elements {
            if y >= area.bottom() {
                break;
            }
            let height = visible.height.min(area.bottom() - y);
            let slot = TuiRect::new(area.x, y, area.width, height);
            if let Some((x, child_y)) = visible.element.cursor_position(slot, ctx) {
                return Some((x, y.saturating_sub(area.y).saturating_add(child_y)));
            }
            y = y.saturating_add(height);
        }
        None
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for visible in &mut self.visible_elements {
            visible.element.present(ctx);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        // Offer the event to the visible items only. Wheel scrolling is owned by
        // the [`TuiScrollable`](super::TuiScrollable) wrapper, which drives this
        // element's scroll position via [`TuiScrollableElement::scroll_by_rows`].
        let mut y = area.y;
        for visible in &mut self.visible_elements {
            if y >= area.bottom() {
                break;
            }
            let height = visible.height.min(area.bottom() - y);
            let slot = TuiRect::new(area.x, y, area.width, height);
            if visible
                .element
                .dispatch_event(event, slot, event_ctx, ctx, app)
            {
                return true;
            }
            y = y.saturating_add(height);
        }
        false
    }
}

impl<Index, RenderItem> TuiScrollableElement for TuiViewportedList<Index, RenderItem>
where
    Index: TuiViewportIndex,
    RenderItem: Fn(ViewportRenderRequest<Index::Item>, &AppContext) -> RenderedViewportItem,
{
    fn scroll_by_rows(&self, rows: isize, viewport_height: usize) -> bool {
        self.scroll_by(rows, viewport_height)
    }
}

fn collect_from_anchor<ItemId: Clone + Eq, Item>(
    cursor: &mut dyn TuiViewportCursor<ItemId = ItemId, Item = Item>,
    row_offset: usize,
    viewport_height: usize,
) -> Vec<CollectedItem<ItemId, Item>> {
    let mut remaining = viewport_height;
    let mut result = Vec::new();
    let mut first = true;
    while remaining > 0 {
        let Some(item) = cursor.item() else {
            break;
        };
        if item.height == 0 {
            cursor.next();
            continue;
        }
        let start = if first {
            row_offset.min(item.height.saturating_sub(1))
        } else {
            0
        };
        first = false;
        let end = item.height.min(start.saturating_add(remaining));
        remaining = remaining.saturating_sub(end - start);
        result.push(CollectedItem {
            id: item.id,
            item: item.item,
            indexed_height: item.height,
            needs_measurement: item.needs_measurement,
            visible_rows: start..end,
        });
        cursor.next();
    }
    result
}

fn collect_from_bottom<ItemId: Clone + Eq, Item>(
    cursor: &mut dyn TuiViewportCursor<ItemId = ItemId, Item = Item>,
    viewport_height: usize,
) -> Vec<CollectedItem<ItemId, Item>> {
    let mut remaining = viewport_height;
    let mut result = Vec::new();
    while remaining > 0 {
        let Some(item) = cursor.item() else {
            break;
        };
        if item.height == 0 {
            cursor.prev();
            continue;
        }
        let start = item.height.saturating_sub(remaining);
        remaining = remaining.saturating_sub(item.height);
        result.push(CollectedItem {
            id: item.id,
            item: item.item,
            indexed_height: item.height,
            needs_measurement: item.needs_measurement,
            visible_rows: start..item.height,
        });
        cursor.prev();
    }
    result.reverse();
    result
}

/// Walks the top `anchor` by `rows` (negative = toward the top), clamping at
/// the very top. The cursor must start positioned at `anchor.item_id`. Returns
/// `None` only if that anchor item no longer exists. The bottom is not clamped
/// here; the caller does that with [`rows_below_exceed`] once the destination
/// is known.
fn walk_anchor<ItemId: Clone + Eq, Item>(
    cursor: &mut dyn TuiViewportCursor<ItemId = ItemId, Item = Item>,
    mut anchor: TuiViewportAnchor<ItemId>,
    rows: isize,
) -> Option<TuiViewportAnchor<ItemId>> {
    cursor.item()?;
    if rows < 0 {
        let mut remaining = rows.unsigned_abs();
        while remaining > 0 {
            if anchor.row_offset >= remaining {
                anchor.row_offset -= remaining;
                break;
            }
            remaining -= anchor.row_offset;
            cursor.prev();
            match cursor.item() {
                Some(item) => {
                    anchor.item_id = item.id;
                    anchor.row_offset = item.height;
                }
                None => {
                    // No earlier item: clamp to the top of the first item.
                    anchor.row_offset = 0;
                    break;
                }
            }
        }
        return Some(anchor);
    }

    let mut remaining = rows as usize;
    while remaining > 0 {
        let Some(item) = cursor.item() else {
            break;
        };
        let rows_in_item = item.height.saturating_sub(anchor.row_offset);
        if remaining < rows_in_item {
            anchor.row_offset += remaining;
            break;
        }
        remaining -= rows_in_item;
        cursor.next();
        match cursor.item() {
            Some(next) => {
                anchor.item_id = next.id;
                anchor.row_offset = 0;
            }
            None => {
                // No later item: stop at the last row of the last item. The
                // caller's bottom clamp turns this into `FollowBottom`.
                anchor.row_offset = item.height.saturating_sub(1);
                break;
            }
        }
    }
    Some(anchor)
}

/// Returns whether strictly more than `viewport_height` rows lie at or below
/// the top `row_offset` of the cursor's current item through the end of the
/// index. When this is false, the top is at or past the bottom-most position
/// that still fills the viewport, so the caller pins to the end. Walks at most
/// a viewport's worth of rows.
fn rows_below_exceed<ItemId: Clone + Eq, Item>(
    cursor: &mut dyn TuiViewportCursor<ItemId = ItemId, Item = Item>,
    row_offset: usize,
    viewport_height: usize,
) -> bool {
    let mut total = 0usize;
    let mut first = true;
    while let Some(item) = cursor.item() {
        let rows = if first {
            item.height.saturating_sub(row_offset)
        } else {
            item.height
        };
        first = false;
        total = total.saturating_add(rows);
        if total > viewport_height {
            return true;
        }
        cursor.next();
    }
    false
}

#[cfg(test)]
#[path = "viewported_list_tests.rs"]
mod tests;
