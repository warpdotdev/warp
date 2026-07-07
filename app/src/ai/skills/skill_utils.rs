//! Utility functions for working with skills.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use ai::skills::{
    provider_parent_directory_for_skills_root, provider_rank, ParsedSkill, SkillPathOrigin,
    SkillProvider,
};
use lazy_static::lazy_static;
use siphasher::sip::SipHasher;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::prelude::MouseStateHandle;
use warpui::{AppContext, Element, EventContext, SingletonEntity};

use super::{SkillDescriptor, SkillManager};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::view_util::render_provider_icon_button;
use crate::ai::blocklist::BlocklistAIHistoryModel;

lazy_static! {
    static ref CONTENT_HASHER: SipHasher = SipHasher::new_with_keys(0, 0);
}

/// Tries to insert or update a skill descriptor in the deduplication map.
/// If a skill with the same (directory, content) key already exists, keeps the one
/// from the higher-priority provider based on [`SKILL_PROVIDER_DEFINITIONS`].
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
fn try_insert_skill(
    dedup_map: &mut HashMap<u64, SkillDescriptor>,
    descriptor: SkillDescriptor,
    dir_path: &LocalOrRemotePath,
    content: &str,
) {
    let mut hasher = *CONTENT_HASHER;
    // Hash the directory path and content to create a unique key for deduplication.
    dir_path.hash(&mut hasher);
    content.hash(&mut hasher);
    let key = hasher.finish();
    match dedup_map.entry(key) {
        Entry::Vacant(e) => {
            e.insert(descriptor);
        }
        Entry::Occupied(mut e) => {
            // Prefer the skill from the higher-priority provider.
            if provider_rank(descriptor.provider) < provider_rank(e.get().provider) {
                e.insert(descriptor);
            }
        }
    }
}

/// Accumulates file-backed skills from one or more catalogs and keeps the best
/// representative for each owning-directory-and-content pair.
#[derive(Default)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub(crate) struct SkillDeduplicator {
    dedup_map: HashMap<u64, SkillDescriptor>,
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
impl SkillDeduplicator {
    pub(crate) fn insert(&mut self, dir_path: &LocalOrRemotePath, skill: &ParsedSkill) {
        try_insert_skill(
            &mut self.dedup_map,
            SkillDescriptor::from(skill.clone()),
            dir_path,
            &skill.content,
        );
    }

    pub(crate) fn extend_paths(
        &mut self,
        skill_paths: &[(LocalOrRemotePath, LocalOrRemotePath)],
        skills_by_path: &HashMap<LocalOrRemotePath, ParsedSkill>,
    ) {
        for (dir_path, path) in skill_paths {
            if let Some(skill) = skills_by_path.get(path) {
                self.insert(dir_path, skill);
            }
        }
    }

    pub(crate) fn into_descriptors(self) -> Vec<SkillDescriptor> {
        self.dedup_map.into_values().collect()
    }
}

/// Deduplicates paths from one indexed catalog when identical content is installed under the
/// same directory across multiple providers, keeping the single best representative per
/// [`SKILL_PROVIDER_DEFINITIONS`] (index 0 = highest priority).
///
/// Two skills are considered duplicates only when they share the same owning directory
/// **and** identical content — which is the common case when a tool like `npx skills`
/// symlinks the same skill under `~/.agents/skills/`, `~/.warp/skills/`, `~/.claude/skills/`, etc.
///
/// Each element of `skill_paths` is a `(dir_path, skill_file_path)` tuple where
/// `dir_path` is the directory that owns the skill.
#[cfg(test)]
pub(crate) fn unique_skills(
    skill_paths: &[(LocalOrRemotePath, LocalOrRemotePath)],
    skills_by_path: &HashMap<LocalOrRemotePath, ParsedSkill>,
) -> Vec<SkillDescriptor> {
    let mut deduplicator = SkillDeduplicator::default();
    deduplicator.extend_paths(skill_paths, skills_by_path);
    deduplicator.into_descriptors()
}

/// Returns the list of skills if they have changed since the last time we sent them to the server.
/// Skills are always included except when the current list matches the last list sent.
pub fn list_skills_if_changed(
    working_directory: Option<&LocalOrRemotePath>,
    path_origin: &SkillPathOrigin,
    conversation_id: Option<AIConversationId>,
    app: &AppContext,
) -> Option<Vec<SkillDescriptor>> {
    let current_skills = SkillManager::as_ref(app).get_skills_for_working_directory_with_origin(
        working_directory,
        path_origin,
        app,
    );

    let previous_skills: Option<Vec<SkillDescriptor>> =
        conversation_id.and_then(|conversation_id| {
            let history_model = BlocklistAIHistoryModel::as_ref(app);
            history_model
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.latest_skills())
        });

    // If there are no previous skills, we consider the skills changed and push the current skills to the context
    let skills_changed = previous_skills
        .map(|previous_skills| {
            let previous_skills_set: HashSet<SkillDescriptor> =
                HashSet::from_iter(previous_skills.iter().cloned());
            let current_skills_set: HashSet<SkillDescriptor> =
                HashSet::from_iter(current_skills.iter().cloned());

            previous_skills_set != current_skills_set
        })
        .unwrap_or(true);

    if skills_changed {
        Some(current_skills)
    } else {
        None
    }
}

/// Renders an 'open skill' button for blocklist AI actions and the code diff view.
pub fn render_skill_button<F>(
    button_label: &str,
    button_handle: MouseStateHandle,
    appearance: &Appearance,
    skill_provider: SkillProvider,
    icon_override: Option<Icon>,
    on_click: F,
) -> Box<dyn Element>
where
    F: FnMut(&mut EventContext) + 'static,
{
    let theme = appearance.theme();
    let logo_fill = internal_colors::fg_overlay_6(theme);

    let icon = icon_override.unwrap_or_else(|| skill_provider.icon());

    let color = if icon_override.is_some() {
        logo_fill
    } else {
        skill_provider.icon_fill(logo_fill)
    };

    render_provider_icon_button(
        button_label,
        button_handle,
        appearance,
        icon,
        color,
        on_click,
    )
}

/// Returns a branded icon override for well-known skill names.
pub fn icon_override_for_skill_name(name: &str) -> Option<Icon> {
    match name {
        "stripe-projects-cli" => Some(Icon::StripeLogo),
        _ => None,
    }
}

pub fn skill_path_from_location(location: &LocalOrRemotePath) -> Option<LocalOrRemotePath> {
    let mut current = Some(location.clone());
    while let Some(candidate_skill_dir) = current {
        if candidate_skill_dir
            .parent()
            .and_then(|provider_dir| provider_parent_directory_for_skills_root(&provider_dir))
            .is_some()
        {
            return Some(candidate_skill_dir.join("SKILL.md"));
        }
        current = candidate_skill_dir.parent();
    }
    None
}

#[cfg(test)]
#[path = "skill_utils_tests.rs"]
mod tests;
