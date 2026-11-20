//! Space data model. Gated at runtime by `FeatureFlag::Spaces`.

use std::collections::HashMap;

use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::EntityId;

use crate::tab::{SelectedTabColor, TabData};
use crate::workspace::tab_group::{TabGroup, TabGroupId};

/// Stable identity for a space.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SpaceId(pub Uuid);

impl SpaceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SpaceId {
    fn default() -> Self {
        Self::new()
    }
}

/// A named, colored set of tabs and tab groups within a window. Only one space's tabs are shown
/// at a time.
#[derive(Clone)]
pub struct Space {
    pub id: SpaceId,
    pub name: Option<String>,
    pub color: SelectedTabColor,
}

impl Space {
    /// Creates a new, untitled space with a fresh id.
    pub fn new() -> Self {
        Self {
            id: SpaceId::new(),
            name: None,
            color: SelectedTabColor::default(),
        }
    }
}

impl Default for Space {
    fn default() -> Self {
        Self::new()
    }
}

/// Tab state of a space that is not currently shown, or of tabs moving between spaces. Mirrors the
/// workspace's live tab fields so a switch can swap them wholesale.
#[derive(Default)]
pub(crate) struct StashedSpace {
    pub(crate) tabs: Vec<TabData>,
    pub(crate) active_tab_index: usize,
    pub(crate) tab_mru_order: Vec<EntityId>,
    pub(crate) tab_groups: HashMap<TabGroupId, TabGroup>,
}

impl StashedSpace {
    /// Adds `other`'s tabs and groups after this space's own, keeping this space's active tab.
    /// Pinned tabs join the end of the pinned prefix so pinned tabs stay at the front.
    pub(crate) fn append(&mut self, other: StashedSpace) {
        let pinned_boundary = self
            .tabs
            .iter()
            .take_while(|tab| is_effectively_pinned(tab, &self.tab_groups))
            .count();
        let (pinned, unpinned): (Vec<_>, Vec<_>) = other
            .tabs
            .into_iter()
            .partition(|tab| is_effectively_pinned(tab, &other.tab_groups));
        if !self.tabs.is_empty() && self.active_tab_index >= pinned_boundary {
            self.active_tab_index += pinned.len();
        }
        self.tabs.splice(pinned_boundary..pinned_boundary, pinned);
        self.tabs.extend(unpinned);
        self.tab_mru_order.extend(other.tab_mru_order);
        self.tab_groups.extend(other.tab_groups);
    }
}

/// Space-local counterpart of `Workspace::is_tab_effectively_pinned`.
fn is_effectively_pinned(tab: &TabData, tab_groups: &HashMap<TabGroupId, TabGroup>) -> bool {
    FeatureFlag::PinnedTabs.is_enabled()
        && (tab.pinned
            || tab
                .group_id
                .and_then(|group_id| tab_groups.get(&group_id))
                .is_some_and(|group| group.pinned))
}
