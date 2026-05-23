//! Skill provider definitions and utilities.
//!
//! This module defines the supported skill providers (i.e. Agents, Claude, Codex, Warp) and their
//! associated skills directory paths. It provides utilities for looking up providers
//! from paths and vice versa.
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use dirs::home_dir;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString, VariantNames};
use warp_core::ui::color::CLAUDE_ORANGE;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_util::local_or_remote_path::LocalOrRemotePath;

/// Represents a skill provider/origin (Agents, Claude, Codex, or Warp).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    VariantNames,
)]
pub enum SkillProvider {
    Warp,
    Agents,
    Claude,
    Codex,
    Cursor,
    Gemini,
    Copilot,
    Droid,
    Github,
    OpenCode,
}

/// Represents the scope of a skill (home directory vs project directory).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    Display,
    EnumString,
    VariantNames,
)]
pub enum SkillScope {
    /// Skills from the user's home directory (e.g., `~/.agents/skills`).
    #[default]
    Home,
    /// Skills from a project directory (e.g., `./repo/.agents/skills`).
    Project,
    /// Bundled skills distributed with Warp.
    Bundled,
}

/// Definition of a skill provider including its directory path.
pub struct SkillProviderDefinition {
    pub provider: SkillProvider,
    /// Relative path from root (repo or home), constructed with platform-aware joining.
    pub skills_path: PathBuf,
}

impl SkillProvider {
    /// Returns the default icon for this provider.
    pub fn icon(&self) -> Icon {
        match self {
            SkillProvider::Claude => Icon::ClaudeLogo,
            SkillProvider::Codex => Icon::OpenAILogo,
            SkillProvider::Gemini => Icon::GeminiLogo,
            SkillProvider::Droid => Icon::DroidLogo,
            SkillProvider::OpenCode => Icon::OpenCodeLogo,
            SkillProvider::Warp
            | SkillProvider::Agents
            | SkillProvider::Cursor
            | SkillProvider::Copilot
            | SkillProvider::Github => Icon::WarpLogoLight,
        }
    }

    /// Returns the icon fill for this provider, using `fallback` for providers that
    /// don't require a specific color. Claude uses its branded salmon color instead.
    pub fn icon_fill(&self, fallback: Fill) -> Fill {
        match self {
            SkillProvider::Claude => Fill::Solid(CLAUDE_ORANGE),
            _ => fallback,
        }
    }
}

/// All provider definitions. Order determines precedence (first = highest priority).
pub static SKILL_PROVIDER_DEFINITIONS: LazyLock<Vec<SkillProviderDefinition>> =
    LazyLock::new(|| {
        vec![
            SkillProviderDefinition {
                provider: SkillProvider::Agents,
                skills_path: PathBuf::from(".agents").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Warp,
                skills_path: PathBuf::from(".warp").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Claude,
                skills_path: PathBuf::from(".claude").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Codex,
                skills_path: PathBuf::from(".codex").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Cursor,
                skills_path: PathBuf::from(".cursor").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Gemini,
                skills_path: PathBuf::from(".gemini").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Copilot,
                skills_path: PathBuf::from(".copilot").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Droid,
                skills_path: PathBuf::from(".factory").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::Github,
                skills_path: PathBuf::from(".github").join("skills"),
            },
            SkillProviderDefinition {
                provider: SkillProvider::OpenCode,
                skills_path: PathBuf::from(".opencode").join("skills"),
            },
        ]
    });

/// Returns the precedence rank of a provider based on its position in [`SKILL_PROVIDER_DEFINITIONS`].
pub fn provider_rank(provider: SkillProvider) -> usize {
    SKILL_PROVIDER_DEFINITIONS
        .iter()
        .position(|def| def.provider == provider)
        // NOTE: Each SkillProvider should map to a unique SkillProviderDefinition
        // so we should never reach this path.
        .unwrap_or(usize::MAX)
}

pub fn home_skills_path(provider: SkillProvider) -> Option<PathBuf> {
    if provider == SkillProvider::Warp {
        return warp_core::paths::warp_home_skills_dir();
    }
    let definition = SKILL_PROVIDER_DEFINITIONS
        .iter()
        .find(|def| def.provider == provider)?;
    home_dir().map(|home_dir| home_dir.join(&definition.skills_path))
}

/// Returns the skill provider for a given path, if it matches a known skill provider directory.
/// For example:
///   get_provider_for_path(Path::new("/repo/.claude/skills/my-skill/SKILL.md")) returns Some(SkillProvider::Claude).
/// Handles both SKILL.md files and files nested within a skill directory.
pub fn get_provider_for_path(path: &Path) -> Option<SkillProvider> {
    let path_components: Vec<_> = path.components().collect();

    for def in SKILL_PROVIDER_DEFINITIONS.iter() {
        if home_skills_path(def.provider)
            .into_iter()
            .any(|home_skills_path| path.starts_with(home_skills_path))
        {
            return Some(def.provider);
        }

        // Retrieves path components for the skill provider directory (i.e., [".claude", "skills"])
        let skill_components: Vec<_> = def.skills_path.components().collect();

        // Checks if some consecutive components of the path match the skill provider directory
        for window in path_components.windows(skill_components.len()) {
            if window == skill_components.as_slice() {
                return Some(def.provider);
            }
        }
    }
    None
}

/// Returns the skill provider for a location without discarding remote host identity.
///
/// Local locations retain home-directory-aware matching. Remote project locations are
/// classified only by their provider-directory structure, because their paths do not
/// belong to the local filesystem.
pub fn get_provider_for_location(path: &LocalOrRemotePath) -> Option<SkillProvider> {
    match path {
        LocalOrRemotePath::Local(path) => get_provider_for_path(path),
        LocalOrRemotePath::Remote(remote) => SKILL_PROVIDER_DEFINITIONS
            .iter()
            .find(|definition| {
                let provider_path = definition.skills_path.to_string_lossy();
                remote
                    .path
                    .ancestors()
                    .any(|ancestor| ancestor.ends_with(provider_path.as_ref()))
            })
            .map(|definition| definition.provider),
    }
}

/// Returns the skill scope (Home or Project) for a given path.
/// A skill is considered a "Home" skill if its path starts with the user's home directory.
/// Otherwise, it's a "Project" skill.
pub fn get_scope_for_path(path: &Path) -> SkillScope {
    for def in SKILL_PROVIDER_DEFINITIONS.iter() {
        if home_skills_path(def.provider)
            .into_iter()
            .any(|home_skills_path| path.starts_with(home_skills_path))
        {
            return SkillScope::Home;
        }
    }
    SkillScope::Project
}

#[cfg(test)]
mod tests {
    use super::{
        get_provider_for_location, get_provider_for_path, get_scope_for_path, home_skills_path,
        SkillProvider, SkillScope,
    };
    use warp_util::host_id::HostId;
    use warp_util::local_or_remote_path::LocalOrRemotePath;
    use warp_util::remote_path::RemotePath;
    use warp_util::standardized_path::StandardizedPath;

    #[test]
    fn warp_home_skills_path_uses_warp_home_path() {
        assert_eq!(
            home_skills_path(SkillProvider::Warp),
            warp_core::paths::warp_home_skills_dir()
        );
    }

    #[test]
    fn warp_home_skill_path_is_home_warp_skill() {
        let Some(warp_home_skills_dir) = warp_core::paths::warp_home_skills_dir() else {
            eprintln!("Skipping test: home directory not available");
            return;
        };
        let path = warp_home_skills_dir.join("my-skill").join("SKILL.md");

        assert_eq!(get_provider_for_path(&path), Some(SkillProvider::Warp));
        assert_eq!(get_scope_for_path(&path), SkillScope::Home);
    }

    #[test]
    fn remote_provider_path_is_classified_by_structure() {
        let path = LocalOrRemotePath::Remote(RemotePath::new(
            HostId::new("remote-host".to_string()),
            StandardizedPath::try_new("/repo/.claude/skills/my-skill/SKILL.md").unwrap(),
        ));

        assert_eq!(
            get_provider_for_location(&path),
            Some(SkillProvider::Claude)
        );
    }

    #[test]
    fn foreign_encoded_remote_provider_path_is_classified_by_structure() {
        let path = LocalOrRemotePath::Remote(RemotePath::new(
            HostId::new("remote-host".to_string()),
            StandardizedPath::try_new(r"C:\repo\.codex\skills\my-skill\SKILL.md").unwrap(),
        ));

        assert_eq!(get_provider_for_location(&path), Some(SkillProvider::Codex));
    }
}
