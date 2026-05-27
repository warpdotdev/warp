//! Tab group data model. Gated at runtime by `FeatureFlag::GroupedTabs`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warpui::elements::DraggableState;

/// Stable identity for a tab group.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
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

/// A group of tabs the user has clustered together. Member tabs point to
/// their group via `TabData::group_id`; `Workspace::tab_groups` is the source
/// of truth for the group's metadata.
///
/// `Debug` is implemented manually so that `draggable_state` (which doesn't
/// implement `Debug`) can be skipped.
#[derive(Clone, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub collapsed: bool,
    /// Transient drag state for the group header. Bound to a `Draggable`
    /// in the vertical tabs panel so the entire group's contiguous run of
    /// member tabs can be reordered as a single block. Not persisted: drag
    /// state is meaningful only during an in-flight drag, so the field is
    /// skipped on serialize/deserialize and reinitialized to its default.
    #[serde(skip)]
    pub draggable_state: DraggableState,
}

impl std::fmt::Debug for TabGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabGroup")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("collapsed", &self.collapsed)
            .finish_non_exhaustive()
    }
}
