use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui_core::ModelContext;

use super::model::{ProjectContextModel, ProjectRule};

/// No-op stand-in for non-`local_fs` builds. File-based global rules require
/// filesystem watchers that don't exist on WASM, so callers see an empty
/// view here.
#[derive(Debug, Default)]
pub(crate) struct GlobalRules;

impl GlobalRules {
    pub(crate) fn index(&mut self, _ctx: &mut ModelContext<ProjectContextModel>) {}
    pub(crate) fn active_rules(&self) -> impl Iterator<Item = ProjectRule> + '_ {
        std::iter::empty()
    }

    pub(crate) fn paths(&self) -> impl Iterator<Item = LocalOrRemotePath> + '_ {
        std::iter::empty()
    }

    pub(crate) fn first_rule_parent(&self) -> Option<LocalOrRemotePath> {
        None
    }
}
