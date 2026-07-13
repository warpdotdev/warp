use std::time::Duration;

use instant::Instant;

use super::{TuiPoint, TuiRect};

pub(super) const AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(100);

// Hold thresholds for the 1 → 2 → 4 → 8 row auto-scroll acceleration ramp.
const AUTO_SCROLL_TWO_ROWS_AFTER: Duration = Duration::from_millis(500);
const AUTO_SCROLL_FOUR_ROWS_AFTER: Duration = Duration::from_millis(1_500);
const AUTO_SCROLL_EIGHT_ROWS_AFTER: Duration = Duration::from_millis(3_000);

/// How a drag event changed the parked auto-scroll state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiAutoScrollDragUpdate {
    InBounds,
    Refreshed,
    Armed,
}

/// One due viewport scroll resolved from a parked drag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TuiAutoScrollStep {
    pub(super) position: TuiPoint,
    pub(super) area: TuiRect,
    pub(super) rows: isize,
}

/// Persistent edge, cadence, and acceleration state for one selection gesture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TuiAutoScrollState {
    target: Option<TuiAutoScrollTarget>,
}

impl TuiAutoScrollState {
    /// Tracks the latest drag position and reports its edge transition.
    pub(super) fn track_drag(
        &mut self,
        position: TuiPoint,
        area: TuiRect,
        now: Instant,
    ) -> TuiAutoScrollDragUpdate {
        let Some(edge) = TuiAutoScrollEdge::at(position, area) else {
            self.stop();
            return TuiAutoScrollDragUpdate::InBounds;
        };
        if let Some(target) = self
            .target
            .as_mut()
            .filter(|target| target.edge.direction() == edge.direction())
        {
            target.position = position;
            target.area = area;
            target.edge = edge;
            return TuiAutoScrollDragUpdate::Refreshed;
        }
        self.target = Some(TuiAutoScrollTarget {
            position,
            area,
            edge,
            started_at: now,
            next_step_at: now,
        });
        TuiAutoScrollDragUpdate::Armed
    }

    /// Takes one due scroll step and advances the cadence deadline.
    pub(super) fn take_due_step(&mut self, now: Instant) -> Option<TuiAutoScrollStep> {
        let target = self.target.as_mut()?;
        if now < target.next_step_at {
            return None;
        }
        target.next_step_at = now + AUTO_SCROLL_INTERVAL;
        Some(TuiAutoScrollStep {
            position: target.position,
            area: target.area,
            rows: target.rows(now),
        })
    }

    /// Returns whether repaint-driven auto-scroll is active.
    pub(super) fn is_active(&self) -> bool {
        self.target.is_some()
    }

    /// Stops repaint-driven auto-scroll.
    pub(super) fn stop(&mut self) {
        self.target = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TuiAutoScrollTarget {
    position: TuiPoint,
    area: TuiRect,
    edge: TuiAutoScrollEdge,
    started_at: Instant,
    next_step_at: Instant,
}

impl TuiAutoScrollTarget {
    /// Returns the signed row delta for the current hold time and overshoot.
    fn rows(&self, now: Instant) -> isize {
        let held_for = now.saturating_duration_since(self.started_at);
        let time_rows = if held_for >= AUTO_SCROLL_EIGHT_ROWS_AFTER {
            8
        } else if held_for >= AUTO_SCROLL_FOUR_ROWS_AFTER {
            4
        } else if held_for >= AUTO_SCROLL_TWO_ROWS_AFTER {
            2
        } else {
            1
        };
        let distance_rows = 1 + usize::from(self.edge.overshoot().saturating_sub(1)) / 2;
        let max_rows = (usize::from(self.area.height) / 2).max(1);
        self.edge.direction() * time_rows.max(distance_rows).min(max_rows) as isize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiAutoScrollEdge {
    Top { overshoot: u16 },
    Bottom { overshoot: u16 },
}

impl TuiAutoScrollEdge {
    /// Resolves a pointer parked at or beyond a selectable edge.
    fn at(position: TuiPoint, area: TuiRect) -> Option<Self> {
        if area.is_empty() {
            return None;
        }
        if position.y <= area.y {
            Some(Self::Top {
                overshoot: area.y.saturating_sub(position.y).saturating_add(1),
            })
        } else if position.y >= area.bottom() {
            Some(Self::Bottom {
                overshoot: position.y.saturating_sub(area.bottom()).saturating_add(1),
            })
        } else {
            None
        }
    }

    /// Returns the signed vertical direction for this edge.
    fn direction(self) -> isize {
        match self {
            Self::Top { .. } => -1,
            Self::Bottom { .. } => 1,
        }
    }

    /// Returns the pointer's cell distance beyond this edge.
    fn overshoot(self) -> u16 {
        match self {
            Self::Top { overshoot } | Self::Bottom { overshoot } => overshoot,
        }
    }
}

#[cfg(test)]
#[path = "auto_scroll_tests.rs"]
mod tests;
