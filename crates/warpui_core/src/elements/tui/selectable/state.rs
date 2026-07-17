use std::cell::RefCell;
use std::cmp::{max, min};
use std::ops::Range;
use std::rc::Rc;

use super::{TuiGridPoint, TuiRowResize, TuiSelectionSpan};
use crate::text::SelectionType;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TuiSelectionState {
    anchor_span: TuiSelectionSpan,
    focus_span: Option<TuiSelectionSpan>,
    selection_type: SelectionType,
    is_selecting: bool,
    width: u16,
}

impl TuiSelectionState {
    /// Returns the ordered non-empty selection range.
    fn range(&self) -> Option<TuiSelectionSpan> {
        let focus_span = self.focus_span?;
        let start = min(self.anchor_span.start, focus_span.start);
        let end = max(self.anchor_span.end, focus_span.end);
        (start < end).then_some(TuiSelectionSpan { start, end })
    }
}

/// Selection data needed while extending an active gesture.
pub(crate) struct TuiSelectionInteraction {
    pub(crate) selection_type: SelectionType,
    pub(crate) anchor_span: TuiSelectionSpan,
    pub(crate) has_focus: bool,
}

/// Persistent state shared across selectable element rebuilds.
#[derive(Clone, Default)]
pub struct TuiSelectionHandle(Rc<RefCell<Option<TuiSelectionState>>>);

impl TuiSelectionHandle {
    /// Clears the selection, returning whether state existed.
    pub fn clear(&self) -> bool {
        self.0.borrow_mut().take().is_some()
    }

    /// Returns whether a mouse selection gesture is active.
    pub(crate) fn is_selecting(&self) -> bool {
        self.0
            .borrow()
            .as_ref()
            .is_some_and(|selection| selection.is_selecting)
    }

    /// Starts a new selection.
    pub(crate) fn start(
        &self,
        anchor_span: TuiSelectionSpan,
        focus_span: Option<TuiSelectionSpan>,
        selection_type: SelectionType,
        width: u16,
    ) {
        *self.0.borrow_mut() = Some(TuiSelectionState {
            anchor_span,
            focus_span,
            selection_type,
            is_selecting: true,
            width,
        });
    }

    /// Updates the selection focus while preserving existing glyph baselines.
    pub(crate) fn update_focus(&self, focus_span: TuiSelectionSpan) {
        let mut slot = self.0.borrow_mut();
        let Some(selection) = slot.as_mut() else {
            return;
        };
        selection.focus_span = Some(focus_span);
    }

    /// Ends the active gesture without clearing its range.
    pub(crate) fn finish(&self) {
        if let Some(selection) = self.0.borrow_mut().as_mut() {
            selection.is_selecting = false;
        }
    }

    /// Returns data needed to extend the current gesture.
    pub(crate) fn interaction(&self) -> Option<TuiSelectionInteraction> {
        self.0
            .borrow()
            .as_ref()
            .map(|selection| TuiSelectionInteraction {
                selection_type: selection.selection_type,
                anchor_span: selection.anchor_span,
                has_focus: selection.focus_span.is_some(),
            })
    }

    /// Returns the ordered non-empty selection range.
    pub(crate) fn range(&self) -> Option<TuiSelectionSpan> {
        self.0.borrow().as_ref().and_then(TuiSelectionState::range)
    }

    /// Clears selection when its rendered width changes.
    pub(crate) fn validate_width(&self, width: u16) -> bool {
        let mut slot = self.0.borrow_mut();
        let Some(selection) = slot.as_ref() else {
            return false;
        };
        if selection.width == width {
            return true;
        }
        *slot = None;
        false
    }

    /// Rebases selection rows around one resized content range.
    pub fn rebase_for_row_resize(&self, resize: TuiRowResize) -> bool {
        let TuiRowResize {
            old_rows,
            new_height,
        } = resize;
        let old_height = old_rows.len();
        if old_height == new_height {
            return false;
        }
        let old_end = old_rows.end;
        let new_end = old_rows.start.saturating_add(new_height);
        let mut slot = self.0.borrow_mut();
        let Some(selection) = slot.as_mut() else {
            return false;
        };

        let selected = selected_rows(selection);

        // Content below the selected rows cannot shift or invalidate the selection.
        if old_rows.start >= selected.end {
            return false;
        }

        if new_height < old_height {
            let removed = new_end..old_end;
            if ranges_intersect(&selected, &removed) {
                *slot = None;
                return true;
            }
        } else if let Some(range) = selection.range() {
            let boundary = TuiGridPoint {
                row: old_end,
                col: 0,
            };
            if range.start < boundary && boundary < range.end {
                *slot = None;
                return true;
            }
        }

        selection.anchor_span = rebase_span(
            selection.anchor_span,
            old_end,
            new_end,
            old_height,
            new_height,
        );
        selection.focus_span = selection
            .focus_span
            .map(|span| rebase_span(span, old_end, new_end, old_height, new_height));
        true
    }

    /// Applies an ordered batch of content row resizes.
    pub(crate) fn rebase_for_row_resizes(&self, mut changes: Vec<TuiRowResize>) -> bool {
        // Viewport layout can report resizes even when no selection exists; avoid
        // sorting or processing those records when there is nothing to rebase.
        if self.0.borrow().is_none() {
            return false;
        }

        changes.sort_by_key(|resize| resize.old_rows.start);
        let mut changed = false;
        let mut cumulative_delta = 0isize;
        for resize in changes {
            let old_height = resize.old_rows.len();
            let start = add_signed(resize.old_rows.start, cumulative_delta);
            changed |= self.rebase_for_row_resize(TuiRowResize {
                old_rows: start..start.saturating_add(old_height),
                new_height: resize.new_height,
            });
            cumulative_delta = cumulative_delta
                .saturating_add(resize.new_height as isize)
                .saturating_sub(old_height as isize);
        }
        changed
    }
}

/// Applies a signed row delta without underflow.
fn add_signed(value: usize, delta: isize) -> usize {
    if delta < 0 {
        value.saturating_sub(delta.unsigned_abs())
    } else {
        value.saturating_add(delta as usize)
    }
}

/// Returns the selected row range for invalidation.
fn selected_rows(selection: &TuiSelectionState) -> Range<usize> {
    let Some(range) = selection.range() else {
        return selection.anchor_span.start.row..selection.anchor_span.start.row.saturating_add(1);
    };
    let end = if range.end.col == 0 {
        range.end.row
    } else {
        range.end.row.saturating_add(1)
    };
    range.start.row..end
}

/// Returns whether two half-open row ranges intersect.
fn ranges_intersect(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

/// Rebases one selection span around a resized row range.
fn rebase_span(
    span: TuiSelectionSpan,
    old_end: usize,
    new_end: usize,
    old_height: usize,
    new_height: usize,
) -> TuiSelectionSpan {
    if span.start.row >= old_end {
        return TuiSelectionSpan {
            start: rebase_point(span.start, old_end, old_height, new_height),
            end: rebase_point(span.end, old_end, old_height, new_height),
        };
    }
    let end = if span.end.row > old_end || (span.end.row == old_end && span.end.col > 0) {
        rebase_point(span.end, old_end, old_height, new_height)
    } else if span.end.row == old_end && span.end.col == 0 {
        TuiGridPoint {
            row: min(span.end.row, new_end),
            col: 0,
        }
    } else {
        span.end
    };
    TuiSelectionSpan {
        start: span.start,
        end,
    }
}

/// Rebases one content point around a resized row range.
fn rebase_point(
    point: TuiGridPoint,
    old_end: usize,
    old_height: usize,
    new_height: usize,
) -> TuiGridPoint {
    if point.row < old_end {
        return point;
    }
    let row = if new_height >= old_height {
        point
            .row
            .saturating_add(new_height.saturating_sub(old_height))
    } else {
        point
            .row
            .saturating_sub(old_height.saturating_sub(new_height))
    };
    TuiGridPoint {
        row,
        col: point.col,
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
