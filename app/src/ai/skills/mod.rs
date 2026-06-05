use std::path::{Path, PathBuf};

use warp_core::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use crate::terminal::model::session::SessionType;
mod telemetry;
pub use telemetry::{SkillOpenOrigin, SkillTelemetryEvent};

cfg_if::cfg_if! {
    if #[cfg(not(feature = "local_fs"))] {
        mod dummy_skill_manager;
        pub use dummy_skill_manager::SkillManager;
    }
}

pub use ai::skills::SkillReference;
/// Controls which path-based skills are available to the current session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillPathScope {
    /// Skills on the local filesystem.
    Local,
    /// Skills on the connected remote host, or no path-based skills while disconnected.
    Remote { host_id: Option<HostId> },
}

impl SkillPathScope {
    pub fn for_session_type(session_type: Option<SessionType>) -> Self {
        match session_type {
            Some(SessionType::WarpifiedRemote { host_id }) => Self::Remote { host_id },
            Some(SessionType::Local) | None => Self::Local,
        }
    }

    pub(super) fn includes(&self, path: &LocalOrRemotePath) -> bool {
        match (self, path) {
            (Self::Local, LocalOrRemotePath::Local(_)) => true,
            (
                Self::Remote {
                    host_id: Some(host_id),
                },
                LocalOrRemotePath::Remote(remote_path),
            ) => remote_path.host_id == *host_id,
            (Self::Local, LocalOrRemotePath::Remote(_))
            | (Self::Remote { .. }, LocalOrRemotePath::Local(_))
            | (Self::Remote { host_id: None }, LocalOrRemotePath::Remote(_)) => false,
        }
    }
}

#[cfg(not(target_family = "wasm"))]
mod global_skills;
#[cfg(not(target_family = "wasm"))]
pub use global_skills::{filter_skills_by_spec, resolve_skill_repos};

mod listed_skill;
pub use listed_skill::SkillDescriptor;

mod skill_utils;
pub use skill_utils::{
    icon_override_for_skill_name, list_skills_if_changed, render_skill_button,
    skill_path_from_location,
};
pub trait SkillPathQuery {
    fn to_skill_location(&self) -> LocalOrRemotePath;
}

impl SkillPathQuery for LocalOrRemotePath {
    fn to_skill_location(&self) -> LocalOrRemotePath {
        self.clone()
    }
}

impl SkillPathQuery for Path {
    fn to_skill_location(&self) -> LocalOrRemotePath {
        LocalOrRemotePath::Local(self.to_path_buf())
    }
}

impl SkillPathQuery for PathBuf {
    fn to_skill_location(&self) -> LocalOrRemotePath {
        LocalOrRemotePath::Local(self.clone())
    }
}

#[cfg(not(target_family = "wasm"))]
mod resolve_skill_spec;
#[cfg(not(target_family = "wasm"))]
pub use resolve_skill_spec::{
    clone_repo_for_skill, resolve_skill_spec, ResolveSkillError, ResolvedSkill,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        mod skill_manager;
        pub use skill_manager::{
            read_skills_from_directories, SkillManager, SkillWatcher,
        };
        #[cfg(test)]
        pub use skill_manager::BundledSkillActivation;
    }
}
