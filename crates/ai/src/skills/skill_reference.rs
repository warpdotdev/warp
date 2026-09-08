use std::fmt;

use serde::{Deserialize, Serialize};
use warp_util::local_or_remote_path::LocalOrRemotePath;

/// An unique reference to a skill.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum SkillReference {
    /// A skill identified by the path to its SKILL.md file.
    Path(LocalOrRemotePath),
    /// A bundled skill distributed with Warp.
    BundledSkillId(String),
}

impl SkillReference {
    /// A user-facing label for status and error copy (e.g. "Failed to read
    /// skill {label}"). Returns the path for a path-based skill, or the bare
    /// id for a bundled skill.
    ///
    /// Unlike [`Display`](fmt::Display), which renders the canonical
    /// `@warp-skill:<id>` reference form for bundled skills, this omits the
    /// internal `@warp-skill:` prefix so bundled-skill copy reads the same way
    /// as path-based skill copy. Use this for anything a user sees; keep
    /// `Display` for canonical/round-trippable reference strings.
    pub fn display_label(&self) -> String {
        match self {
            SkillReference::Path(path) => path.display_path(),
            SkillReference::BundledSkillId(id) => id.clone(),
        }
    }
}

impl fmt::Display for SkillReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SkillReference::Path(path) => path.display_path().fmt(f),
            SkillReference::BundledSkillId(id) => write!(f, "@warp-skill:{id}"),
        }
    }
}

impl From<SkillReference> for warp_multi_agent_api::skill_descriptor::SkillReference {
    fn from(reference: SkillReference) -> Self {
        match reference {
            SkillReference::Path(path) => {
                warp_multi_agent_api::skill_descriptor::SkillReference::Path(path.display_path())
            }
            SkillReference::BundledSkillId(id) => {
                warp_multi_agent_api::skill_descriptor::SkillReference::BundledSkillId(id)
            }
        }
    }
}
