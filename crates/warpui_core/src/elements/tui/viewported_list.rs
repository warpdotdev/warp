//! A generalized, source-driven viewport for ordered variable-height TUI items.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiScrollableElement, TuiSize,
};
use crate::{AppContext, Event};
/// A stable item-relative scroll anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiViewportAnchor<ItemId> {
    pub item_id: ItemId,
    pub row_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiViewportPosition<ItemId> {
    End,
    Anchored(TuiViewportAnchor<ItemId>),
}

/// Shared storage for a caller-owned viewport position.
#[derive(Clone)]
pub struct TuiViewportHandle<ItemId>(Rc<RefCell<TuiViewportPosition<ItemId>>>);

impl<ItemId> Default for TuiViewportHandle<ItemId> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(TuiViewportPosition::End)))
    }
}

impl<ItemId: Clone> TuiViewportHandle<ItemId> {
    /// Creates viewport position storage initially set to the index end.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current caller-owned viewport position.
    pub fn position(&self) -> TuiViewportPosition<ItemId> {
        self.0.borrow().clone()
    }

    /// Stores a new caller-owned viewport position.
    pub fn set_position(&self, position: TuiViewportPosition<ItemId>) {
        *self.0.borrow_mut() = position;
    }

    /// Requests rendering from the end of the ordered index.
    pub fn scroll_to_end(&self) {
        self.set_position(TuiViewportPosition::End);
    }

    /// Anchors the viewport to `item_id` at `row_offset`.
    pub fn scroll_to_item(&self, item_id: ItemId, row_offset: usize) {
        self.set_position(TuiViewportPosition::Anchored(TuiViewportAnchor {
            item_id,
            row_offset,
        }));
    }

    /// Returns whether the requested viewport position is the index end.
    pub fn is_at_end(&self) -> bool {
        matches!(*self.0.borrow(), TuiViewportPosition::End)
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
type VisibleElements<ItemId> = Vec<VisibleElement<ItemId>>;
type HeightUpdates<ItemId> = Vec<(ItemId, usize)>;

/// A variable-height viewport that delegates ordered storage and item rendering.
pub struct TuiViewportedList<Index, RenderItem, OnPositionChange>
where
    Index: TuiViewportIndex,
{
    index: Index,
    render_item: RenderItem,
    on_position_change: OnPositionChange,
    position: TuiViewportPosition<Index::ItemId>,
    visible_elements: Vec<VisibleElement<Index::ItemId>>,
    first_anchor: Option<TuiViewportAnchor<Index::ItemId>>,
    last_width: Option<u16>,
    size: TuiSize,
}

impl<Index, RenderItem, OnPositionChange> TuiViewportedList<Index, RenderItem, OnPositionChange>
where
    Index: TuiViewportIndex,
    RenderItem: Fn(ViewportRenderRequest<Index::Item>, &AppContext) -> RenderedViewportItem,
    OnPositionChange: FnMut(TuiViewportPosition<Index::ItemId>),
{
    /// Creates a generalized viewport over `index` with an injected item renderer.
    pub fn new(
        position: TuiViewportPosition<Index::ItemId>,
        index: Index,
        render_item: RenderItem,
        on_position_change: OnPositionChange,
    ) -> Self {
        Self {
            index,
            render_item,
            on_position_change,
            position,
            visible_elements: Vec::new(),
            first_anchor: None,
            last_width: None,
            size: TuiSize::ZERO,
        }
    }

    fn collect_visible(
        &mut self,
        viewport_height: usize,
    ) -> Vec<CollectedItem<Index::ItemId, Index::Item>> {
        let collected = match &self.position {
            TuiViewportPosition::End => self
                .index
                .with_cursor(TuiViewportIndexPosition::End, |cursor| {
                    collect_from_end(cursor, viewport_height)
                }),
            TuiViewportPosition::Anchored(anchor) => self
                .index
                .with_cursor(TuiViewportIndexPosition::Item(&anchor.item_id), |cursor| {
                    collect_from_anchor(cursor, anchor.row_offset, viewport_height)
                }),
        };
        if collected.is_empty() && matches!(self.position, TuiViewportPosition::Anchored(_)) {
            self.set_position(TuiViewportPosition::End);
            self.index
                .with_cursor(TuiViewportIndexPosition::End, |cursor| {
                    collect_from_end(cursor, viewport_height)
                })
        } else {
            collected
        }
    }

    fn set_position(&mut self, position: TuiViewportPosition<Index::ItemId>) {
        if self.position != position {
            self.position = position.clone();
            (self.on_position_change)(position);
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
        let collected = self.collect_visible(viewport_height);
        let (rendered, height_updates) =
            self.render_collected(collected, width, width_changed, app);

        if height_updates.is_empty() {
            self.visible_elements = rendered;
        } else {
            // The GUI terminal block list uses a fixed measure/apply/final-pass
            // shape for dynamic rich content heights: measure visible dynamic
            // content, write updated heights into the model-owned sum tree, then
            // rebuild the viewport once for the frame's final visible range.
            // TUI item height feedback similarly arrives after scoped index
            // traversal, so one recollect is the equivalent of the GUI's final
            // viewport pass without an arbitrary stabilization loop.
            self.index.update_heights(&height_updates);
            let collected = self.collect_visible(viewport_height);
            let (rendered, remaining_updates) =
                self.render_collected(collected, width, width_changed, app);
            if !remaining_updates.is_empty() {
                log::warn!(
                    "TUI viewport item heights changed during the final layout pass; \
                     the next invalidation will apply the updated visible range"
                );
                self.index.update_heights(&remaining_updates);
            }
            self.visible_elements = rendered;
        }

        for visible in &mut self.visible_elements {
            visible.element.layout(
                TuiConstraint::tight(TuiSize::new(width, visible.height)),
                ctx,
                app,
            );
        }
    }

    fn render_collected(
        &mut self,
        collected: Vec<CollectedItem<Index::ItemId, Index::Item>>,
        width: u16,
        width_changed: bool,
        app: &AppContext,
    ) -> (VisibleElements<Index::ItemId>, HeightUpdates<Index::ItemId>) {
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
        (rendered, height_updates)
    }

    /// Scrolls the viewport by `rows` (negative = toward the top), clamping at
    /// both ends. `viewport_height` is needed for the bottom clamp: once a
    /// screenful or less remains below the new top, the viewport pins to the
    /// end instead of scrolling content off the bottom into blank space.
    fn scroll_by(&mut self, rows: isize, viewport_height: usize) -> bool {
        if rows == 0 || viewport_height == 0 {
            return false;
        }
        // Resolve the current top anchor. `End` only yields a top anchor for
        // upward scrolls; downward is already pinned to the index end.
        let current = match self.position.clone() {
            TuiViewportPosition::End if rows < 0 => self.first_anchor.clone(),
            TuiViewportPosition::End => return false,
            TuiViewportPosition::Anchored(anchor) => Some(anchor),
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

        // Clamp at the end: if a screenful or less remains below the new top,
        // pin to the end rather than leaving blank rows below the last item.
        let candidate_item_id = candidate.item_id.clone();
        let fills_viewport = self.index.with_cursor(
            TuiViewportIndexPosition::Item(&candidate_item_id),
            |cursor| rows_below_exceed(cursor, candidate.row_offset, viewport_height),
        );
        let next = if fills_viewport {
            TuiViewportPosition::Anchored(candidate)
        } else {
            TuiViewportPosition::End
        };
        if self.position == next {
            return false;
        }
        self.set_position(next);
        true
    }
}

impl<Index, RenderItem, OnPositionChange> TuiElement
    for TuiViewportedList<Index, RenderItem, OnPositionChange>
where
    Index: TuiViewportIndex,
    RenderItem: Fn(ViewportRenderRequest<Index::Item>, &AppContext) -> RenderedViewportItem,
    OnPositionChange: FnMut(TuiViewportPosition<Index::ItemId>),
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

impl<Index, RenderItem, OnPositionChange> TuiScrollableElement
    for TuiViewportedList<Index, RenderItem, OnPositionChange>
where
    Index: TuiViewportIndex,
    RenderItem: Fn(ViewportRenderRequest<Index::Item>, &AppContext) -> RenderedViewportItem,
    OnPositionChange: FnMut(TuiViewportPosition<Index::ItemId>),
{
    fn scroll_by_rows(&mut self, rows: isize, viewport_height: usize) -> bool {
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

fn collect_from_end<ItemId: Clone + Eq, Item>(
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
                // caller's end clamp turns this into `End`.
                anchor.row_offset = item.height.saturating_sub(1);
                break;
            }
        }
    }
    Some(anchor)
}

/// Returns whether strictly more than `viewport_height` rows lie at or below
/// the top `row_offset` of the cursor's current item through the end of the
/// index. When this is false, the top is at or past the end-most position
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
