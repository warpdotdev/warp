mod telemetry;
pub use telemetry::{SkillOpenOrigin, SkillTelemetryEvent};

cfg_if::cfg_if! {
    if #[cfg(not(feature = "local_fs"))] {
        mod dummy_skill_manager;
        pub use dummy_skill_manager::SkillManager;
    }
}

pub use ai::skills::SkillReference;

#[cfg(not(target_family = "wasm"))]
mod global_skills;
#[cfg(not(target_family = "wasm"))]
pub use global_skills::{filter_skills_by_spec, resolve_skill_repos};

mod listed_skill;
pub use listed_skill::SkillDescriptor;

mod skill_utils;
pub use skill_utils::{
    icon_override_for_skill_name, list_skills_if_changed, render_skill_button,
    skill_path_from_file_path,
};

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
