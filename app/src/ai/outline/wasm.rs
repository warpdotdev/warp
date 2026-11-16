use std::path::{Path, PathBuf};

use warpui::{Entity, ModelContext, SingletonEntity};

use super::OutlineStatus;

pub struct RepoOutlines {}

impl RepoOutlines {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {}
    }

    pub fn new_with_indexing_enabled(
        _indexing_enabled: bool,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self {}
    }

    pub fn get_outline(&self, _path: &Path) -> Option<(&OutlineStatus, PathBuf)> {
        None
    }

    pub fn is_directory_indexed(&self, _directory: &Path) -> bool {
        false
    }
}

impl Entity for RepoOutlines {
    type Event = ();
}

impl SingletonEntity for RepoOutlines {}
