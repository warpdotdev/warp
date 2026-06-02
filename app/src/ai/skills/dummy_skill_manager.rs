use ai::skills::{ParsedSkill, SkillProvider, SkillReference};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::ai::skills::{SkillDescriptor, SkillPathQuery};

pub struct SkillManager {}

impl SkillManager {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {}
    }

    pub fn get_skills_for_working_directory(
        &self,
        _working_directory: Option<&LocalOrRemotePath>,
        _ctx: &AppContext,
    ) -> Vec<SkillDescriptor> {
        vec![]
    }

    pub fn skill_by_path<P: SkillPathQuery + ?Sized>(
        &self,
        _skill_path: &P,
    ) -> Option<&ParsedSkill> {
        None
    }
    pub fn reference_for_skill_path<P: SkillPathQuery + ?Sized>(
        &self,
        skill_path: &P,
    ) -> SkillReference {
        SkillReference::Path(skill_path.to_skill_location())
    }

    pub fn skill_by_reference(&self, _reference: &SkillReference) -> Option<&ParsedSkill> {
        None
    }

    pub fn active_skill_by_reference(
        &self,
        _reference: &SkillReference,
        _ctx: &AppContext,
    ) -> Option<&ParsedSkill> {
        None
    }

    pub fn active_bundled_skill(&self, _id: &str, _ctx: &AppContext) -> Option<&ParsedSkill> {
        None
    }

    pub fn skill_exists_for_any_provider(
        &self,
        _skill: &SkillDescriptor,
        _providers: &[SkillProvider],
    ) -> bool {
        false
    }

    pub fn best_supported_provider(
        &self,
        skill: &SkillDescriptor,
        _supported_providers: &[SkillProvider],
    ) -> SkillProvider {
        skill.provider
    }
}

impl Entity for SkillManager {
    type Event = ();
}

impl SingletonEntity for SkillManager {}
