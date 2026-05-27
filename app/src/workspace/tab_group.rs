//! Tab group data model. Gated at runtime by `FeatureFlag::GroupedTabs`.

use uuid::Uuid;

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

/// A group of tabs. Member tabs point to their
/// group via `TabData::group_id`.
#[derive(Clone)]
pub struct TabGroup {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub collapsed: bool,
}

impl TabGroup {
    /// Creates a new, untitled, expanded tab group with a fresh id.
    pub fn new() -> Self {
        Self {
            id: TabGroupId::new(),
            name: None,
            collapsed: false,
        }
    }
}

impl Default for TabGroup {
    fn default() -> Self {
        Self::new()
    }
}
