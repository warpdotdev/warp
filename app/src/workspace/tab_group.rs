//! Tab group data model. Gated at runtime by `FeatureFlag::GroupedTabs`.

use uuid::Uuid;
use warpui::elements::DraggableState;

use crate::features::FeatureFlag;
use crate::tab::SelectedTabColor;

/// Whether the automatic tab grouping mode is available at all. Layered over `GroupedTabs`
/// because the mode has nothing to put tabs into where groups themselves are unavailable.
pub fn auto_tab_grouping_available() -> bool {
    FeatureFlag::GroupedTabs.is_enabled() && FeatureFlag::AutoTabGrouping.is_enabled()
}

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
    /// The project key this group is keyed by, when automatic grouping created
    /// it. `None` for a group the user made, which is what tells the two apart
    /// after a restart.
    pub project_key: Option<String>,
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
            project_key: None,
        }
    }
}

impl Default for TabGroup {
    fn default() -> Self {
        Self::new()
    }
}
