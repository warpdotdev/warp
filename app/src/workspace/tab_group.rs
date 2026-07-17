//! Tab group data model. Gated at runtime by `FeatureFlag::GroupedTabs`.

use std::collections::HashMap;

use uuid::Uuid;
use warpui::elements::DraggableState;

use crate::tab::SelectedTabColor;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;

/// Stable identity for a tab group.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TabGroupId(pub Uuid);

impl TabGroupId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TabGroupId {
    fn default() -> Self {
        Self::new()
    }
}

/// A named group of tabs in the vertical tabs panel.
/// Member tabs reference their group via `TabData::group_id`.
#[derive(Clone)]
pub struct TabGroup {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub color: SelectedTabColor,
    pub collapsed: bool,
    pub draggable_state: DraggableState,
    /// True when this whole group is pinned to the front of the tab list.
    pub pinned: bool,
}

impl TabGroup {
    /// Creates a new, untitled, expanded tab group with a fresh id.
    pub fn new() -> Self {
        Self {
            id: TabGroupId::new(),
            name: None,
            color: SelectedTabColor::default(),
            collapsed: false,
            draggable_state: Default::default(),
            pinned: false,
        }
    }
}

impl Default for TabGroup {
    fn default() -> Self {
        Self::new()
    }
}

/// Picks the color to assign to a newly created tab group when the
/// `assign_color_to_new_tab_groups` setting is enabled. Iterates the tab color
/// palette ([`TAB_COLOR_OPTIONS`]) in order and returns the first color not
/// already claimed by an existing group, cycling back to the first palette
/// color once every color is in use (Chrome-style). Groups whose color is
/// [`SelectedTabColor::Unset`] or [`SelectedTabColor::Cleared`] never count as
/// used, so they don't block the cycle.
pub(crate) fn next_group_color(existing: &HashMap<TabGroupId, TabGroup>) -> SelectedTabColor {
    let used: Vec<SelectedTabColor> = existing.values().map(|group| group.color).collect();
    next_unused_group_color(&used)
}

/// Pure color-selection core: given the colors already used by existing groups,
/// returns the first palette color not present, or the first palette color once
/// all are used. Split out from [`next_group_color`] so the selection logic is
/// unit-testable without constructing full [`TabGroup`] values.
fn next_unused_group_color(used: &[SelectedTabColor]) -> SelectedTabColor {
    for &option in TAB_COLOR_OPTIONS.iter() {
        let candidate = SelectedTabColor::Color(option);
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    // Every palette color is already in use — cycle back to the first option.
    SelectedTabColor::Color(TAB_COLOR_OPTIONS[0])
}

#[cfg(test)]
#[path = "tab_group_tests.rs"]
mod tests;
