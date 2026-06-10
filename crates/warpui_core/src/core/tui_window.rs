use std::collections::HashMap;

use crate::core::tui_view::AnyTuiViewHandle;
use crate::{AnyTuiView, EntityId};

#[derive(Default)]
pub(in crate::core) struct TuiWindow {
    pub views: HashMap<EntityId, Box<dyn AnyTuiView>>,
    pub root_view: Option<AnyTuiViewHandle>,
    pub focused_view: Option<EntityId>,
}
