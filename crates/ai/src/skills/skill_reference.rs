use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// An unique reference to a skill.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum SkillReference {
    /// A skill identified by the path to its SKILL.md file.
    Path(PathBuf),
    /// A bundled skill distributed with Warp.
    BundledSkillId(String),
}

impl fmt::Display for SkillReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SkillReference::Path(path) => path.display().fmt(f),
            SkillReference::BundledSkillId(id) => write!(f, "@warp-skill:{id}"),
        }
    }
}

impl From<SkillReference> for warp_multi_agent_api::skill_descriptor::SkillReference {
    fn from(reference: SkillReference) -> Self {
        match reference {
            SkillReference::Path(path) => {
                warp_multi_agent_api::skill_descriptor::SkillReference::Path(
                    path.to_string_lossy().to_string(),
                )
            }
            SkillReference::BundledSkillId(id) => {
                warp_multi_agent_api::skill_descriptor::SkillReference::BundledSkillId(id)
            }
        }
    }
}
