//! Space data model. Gated at runtime by `FeatureFlag::Spaces`.

use std::collections::HashMap;

use uuid::Uuid;
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

/// Tab state of a space that is not currently shown. Mirrors the workspace's live tab fields so a
/// switch can swap them wholesale.
#[derive(Default)]
pub(crate) struct StashedSpace {
    pub(crate) tabs: Vec<TabData>,
    pub(crate) active_tab_index: usize,
    pub(crate) tab_mru_order: Vec<EntityId>,
    pub(crate) tab_groups: HashMap<TabGroupId, TabGroup>,
}
