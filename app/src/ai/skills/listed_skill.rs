use ai::skills::{ParsedSkill, SkillProvider, SkillReference, SkillScope};
use serde::{Deserialize, Serialize};
use black_core::ui::icons::Icon;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct SkillDescriptor {
    pub reference: SkillReference,
    pub name: String,
    pub description: String,
    /// The scope of the skill (home directory vs project directory).
    pub scope: SkillScope,
    /// The provider/origin of the skill (Claude, Codex, or Warp).
    /// None if the skill path didn't match a known provider directory.
    pub provider: SkillProvider,
    /// Override icon for this skill. When set, rendering code should use this
    /// instead of deriving the icon from the provider.
    #[serde(skip)]
    pub icon_override: Option<Icon>,
}

impl SkillDescriptor {
    /// Returns whether this skill is from a project directory (vs home directory).
    pub fn is_project_skill(&self) -> bool {
        self.scope == SkillScope::Project
    }

    pub fn new_bundled(id: String, skill: ParsedSkill, icon: Icon) -> Self {
        Self {
            provider: SkillProvider::Warp,
            scope: SkillScope::Bundled,
            reference: SkillReference::BundledSkillId(id),
            name: skill.name,
            description: skill.description,
            icon_override: Some(icon),
        }
    }
}

impl From<ParsedSkill> for SkillDescriptor {
    fn from(skill: ParsedSkill) -> Self {
        Self {
            provider: skill.provider,
            scope: skill.scope,
            reference: SkillReference::Path(skill.path),
            name: skill.name,
            description: skill.description,
            icon_override: None,
        }
    }
}
