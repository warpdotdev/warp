use std::path::{Path, PathBuf};

use ai::skills::SkillPathOrigin;
use warp_util::local_or_remote_path::LocalOrRemotePath;

mod telemetry;
pub use telemetry::{SkillOpenOrigin, SkillTelemetryEvent};
#[cfg(all(not(target_family = "wasm"), feature = "local_fs"))]
mod remote;
#[cfg(all(not(target_family = "wasm"), feature = "local_fs"))]
pub(crate) use remote::{bundled_skill_snapshot_protos, wire_remote_bundled_skills};
#[cfg(feature = "local_fs")]
mod bundled;
#[cfg(all(not(target_family = "wasm"), feature = "local_fs"))]
pub(crate) use bundled::BundledSkill;
#[cfg(all(feature = "local_fs", test))]
pub use bundled::BundledSkillActivation;

cfg_if::cfg_if! {
    if #[cfg(not(feature = "local_fs"))] {
        mod dummy_skill_manager;
        pub use dummy_skill_manager::SkillManager;
    }
}

pub use ai::skills::SkillReference;

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub enum SkillManagerEvent {
    HomeSkillsChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActiveSkillLookupError {
    #[error("Bundled skills are not available on this remote session")]
    BundledSkillsUnavailable,
    #[error("Skill not found: {reference}")]
    NotFound { reference: SkillReference },
}

impl ActiveSkillLookupError {
    pub(crate) fn for_reference(reference: &SkillReference, path_origin: &SkillPathOrigin) -> Self {
        if matches!(path_origin, SkillPathOrigin::Unavailable)
            && matches!(reference, SkillReference::BundledSkillId(_))
        {
            Self::BundledSkillsUnavailable
        } else {
            Self::NotFound {
                reference: reference.clone(),
            }
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
    }
}
