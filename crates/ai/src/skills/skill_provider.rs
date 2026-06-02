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

/// Returns the skill provider for a location, if it matches a known skill provider directory.
///
/// Local locations retain home-directory-aware matching. All other locations are
/// classified by provider-directory structure using their standardized path representation.
pub fn get_provider_for_path(path: &LocalOrRemotePath) -> Option<SkillProvider> {
    path.to_local_path()
        .and_then(get_home_provider_for_local_path)
        .or_else(|| get_provider_for_structural_path(path))
}

fn get_home_provider_for_local_path(path: &Path) -> Option<SkillProvider> {
    SKILL_PROVIDER_DEFINITIONS
        .iter()
        .find(|definition| {
            home_skills_path(definition.provider)
                .into_iter()
                .any(|home_skills_path| path.starts_with(home_skills_path))
        })
        .map(|definition| definition.provider)
}

/// Returns the directory containing a provider's skills root when `skills_root` has a known
/// provider directory suffix, preserving the original local or remote location encoding.
///
/// For example, `/repo/.agents/skills` resolves to `/repo`, regardless of whether the location
/// is encoded with Unix or Windows path separators.
pub fn provider_parent_directory_for_skills_root(
    skills_root: &LocalOrRemotePath,
) -> Option<LocalOrRemotePath> {
    match_provider_skills_root(skills_root).map(|(_, parent_directory)| parent_directory)
}

fn get_provider_for_structural_path(path: &LocalOrRemotePath) -> Option<SkillProvider> {
    let mut current = Some(path.clone());
    while let Some(candidate) = current {
        if let Some((provider, _)) = match_provider_skills_root(&candidate) {
            return Some(provider);
        }
        current = candidate.parent();
    }
    None
}

fn match_provider_skills_root(
    skills_root: &LocalOrRemotePath,
) -> Option<(SkillProvider, LocalOrRemotePath)> {
    for definition in SKILL_PROVIDER_DEFINITIONS.iter() {
        let mut parent_directory = skills_root.clone();
        let mut matches_provider = true;
        for component in definition.skills_path.components().rev() {
            let expected_component = component.as_os_str().to_str()?;
            if parent_directory.file_name() != Some(expected_component) {
                matches_provider = false;
                break;
            }
            parent_directory = parent_directory.parent()?;
        }
        if matches_provider {
            return Some((definition.provider, parent_directory));
        }
    }
    None
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
#[path = "skill_provider_tests.rs"]
mod tests;
