use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{Entity, ModelContext};

use crate::workspace::view::global_search::SearchConfig;
use crate::workspace::view::global_search::view::GlobalSearchEvent;

pub struct GlobalSearch {}

impl Entity for GlobalSearch {
    type Event = GlobalSearchEvent;
}

impl GlobalSearch {
    pub fn new() -> Self {
        GlobalSearch {}
    }

    pub fn abort_search(&mut self, _ctx: &mut ModelContext<Self>) {}

    pub fn run_search(
        &mut self,
        _pattern: String,
        _roots: Vec<LocalOrRemotePath>,
        _search_config: SearchConfig,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
}

impl Default for GlobalSearch {
    fn default() -> Self {
        Self::new()
    }
}
