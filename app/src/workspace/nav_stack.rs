use warp_editor::render::model::viewport::ScrollPositionSnapshot;
use warpui::units::Pixels;
use warpui::{navigation, AppContext, EntityId, WindowId};

use crate::pane_group::PaneId;
use crate::terminal::block_list_viewport::ScrollPosition;

/// Terminal scroll anchors within this many lines of the previous entry are
/// treated as near-duplicates and not recorded, so a trivial or net-zero
/// scroll twitch does not create an imperceptible Back destination.
const NEAR_DUPLICATE_TERMINAL_SCROLL_THRESHOLD_LINES: f32 = 10.0;

#[derive(Debug, Clone)]
pub enum ScrollSnapshot {
    Terminal(ScrollPosition),
    Editor(ScrollPositionSnapshot),
    CodeDiff {
        view_id: EntityId,
        selected_tab: usize,
        editor_scroll_snapshot: ScrollPositionSnapshot,
    },
    CodeReview {
        scroll_index: usize,
        scroll_offset_px: Pixels,
    },
}

impl ScrollSnapshot {
    pub fn same_position(&self, other: &Self) -> bool {
        match (self, other) {
            (ScrollSnapshot::Terminal(a), ScrollSnapshot::Terminal(b)) => a == b,
            (ScrollSnapshot::Editor(a), ScrollSnapshot::Editor(b)) => a == b,
            (
                ScrollSnapshot::CodeDiff {
                    view_id: view_a,
                    selected_tab: tab_a,
                    editor_scroll_snapshot: snapshot_a,
                },
                ScrollSnapshot::CodeDiff {
                    view_id: view_b,
                    selected_tab: tab_b,
                    editor_scroll_snapshot: snapshot_b,
                },
            ) => view_a == view_b && tab_a == tab_b && snapshot_a == snapshot_b,
            (
                ScrollSnapshot::CodeReview {
                    scroll_index: idx_a,
                    scroll_offset_px: off_a,
                },
                ScrollSnapshot::CodeReview {
                    scroll_index: idx_b,
                    scroll_offset_px: off_b,
                },
            ) => idx_a == idx_b && off_a == off_b,
            _ => false,
        }
    }

    /// Whether pushing `self` on top of `existing` would create a
    /// near-duplicate entry the user cannot meaningfully navigate between.
    pub fn is_near_duplicate(&self, existing: &Self) -> bool {
        match (self, existing) {
            (ScrollSnapshot::Terminal(a), ScrollSnapshot::Terminal(b)) => {
                a.is_within_lines(b, NEAR_DUPLICATE_TERMINAL_SCROLL_THRESHOLD_LINES)
            }
            _ => self.same_position(existing),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NavigationEntry {
    pub window_id: WindowId,
    pub tab_index: usize,
    pub pane_id: PaneId,
    pub scroll_snapshot: Option<ScrollSnapshot>,
}

impl navigation::NavigationEntry for NavigationEntry {
    fn should_push(&self, existing: &Self) -> bool {
        if self.window_id != existing.window_id
            || self.tab_index != existing.tab_index
            || self.pane_id != existing.pane_id
        {
            return true;
        }
        match (&self.scroll_snapshot, &existing.scroll_snapshot) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some(a), Some(b)) => !a.is_near_duplicate(b),
        }
    }
}

pub type NavigationStack = navigation::NavigationStack<NavigationEntry>;

pub fn init(app: &mut AppContext) {
    app.add_singleton_model(NavigationStack::new);
}

#[cfg(test)]
#[path = "nav_stack_tests.rs"]
mod tests;
