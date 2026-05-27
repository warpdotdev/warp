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

/// A group of tabs. Member tabs point to their
/// group via `TabData::group_id`.
#[derive(Clone)]
pub struct TabGroup {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub collapsed: bool,
}
