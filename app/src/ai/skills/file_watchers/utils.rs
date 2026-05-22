use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use ai::skills::{
    home_skills_path, parse_skill, read_skills, ParsedSkill, SkillProvider,
    SKILL_PROVIDER_DEFINITIONS,
};
use anyhow::Error;
use regex::Regex;
use repo_metadata::local_model::GetContentsArgs;
use repo_metadata::{RepoContent, RepoMetadataModel, RepositoryIdentifier};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::AppContext;

use crate::warp_managed_paths_watcher::warp_managed_skill_dirs;

fn local_or_remote_path_for_repo_path(
    repo_id: &RepositoryIdentifier,
    path: &StandardizedPath,
) -> LocalOrRemotePath {
    match repo_id {
        RepositoryIdentifier::Local(_) => LocalOrRemotePath::Local(path.to_local_path_lossy()),
        RepositoryIdentifier::Remote(remote) => {
            LocalOrRemotePath::Remote(RemotePath::new(remote.host_id.clone(), path.clone()))
        }
    }
}

/// Finds all skill files in a repository by querying the RepoMetadataModel tree.
///
/// Returns a list of paths to concrete `SKILL.md` files (e.g.,
/// `/repo/.agents/skills/deploy/SKILL.md`, `/repo/sub/.claude/skills/build/SKILL.md`).
pub fn find_skill_files_in_tree(
    repo_id: &RepositoryIdentifier,
    repo_metadata: &RepoMetadataModel,
    ctx: &AppContext,
) -> Vec<LocalOrRemotePath> {
    // Filter during traversal: only collect concrete SKILL.md files that match a known provider
    // path. This keeps project acquisition on repo metadata until local or remote file hydration.
    let args = GetContentsArgs {
        include_folders: false,
        ..GetContentsArgs::default()
    }
    .with_filter(|content| {
        let RepoContent::File(file) = content else {
            return false;
        };
        is_skill_file(&file.path.to_local_path_lossy())
    });
    repo_metadata
        .get_repo_contents(repo_id, args, ctx)
        .unwrap_or_default()
        .into_iter()
        // Only files should reach this iterator due to the GetContentsArgs::filter.
        // Keep the Directory arm for exhaustive matching in case RepoContent grows new variants.
        .filter_map(|content| match content {
            RepoContent::File(file) => {
                Some(local_or_remote_path_for_repo_path(repo_id, &file.path))
            }
            RepoContent::Directory(_) => None,
        })
        .collect()
}

/// Reads all skills from the given skill directories.
pub fn read_skills_from_directories(
    skill_dirs: impl IntoIterator<Item = PathBuf>,
) -> Vec<ParsedSkill> {
    skill_dirs
        .into_iter()
        .flat_map(|dir| read_skills(&dir))
        .collect()
}
/// Reads all skills from the given concrete skill files.
pub fn read_skills_from_files(skill_files: impl IntoIterator<Item = PathBuf>) -> Vec<ParsedSkill> {
    skill_files
        .into_iter()
        .filter_map(|path| parse_skill(&path).ok())
        .collect()
}

pub fn is_skill_file(path: &Path) -> bool {
    extract_skill_parent_directory(path).is_ok()
}

static SKILL_PROVIDER_PATHS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    // Collect the skill provider paths from the definitions
    SKILL_PROVIDER_DEFINITIONS
        .iter()
        .map(|p| p.skills_path.to_string_lossy().to_string())
        .collect()
});

// Pattern: {prefix}/{provider_path}/{skill-name}/SKILL.md
// where provider_path is 2 parts (e.g., ".agents/skills") and skill-name is 1 part
#[cfg(not(target_os = "windows"))]
static SKILL_FILE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(.+)/([^/]+/[^/]+)/[^/]+/SKILL\.md$")
        .expect("Failed to compile skill file pattern")
});

// On windows, the path separator is \
#[cfg(target_os = "windows")]
static SKILL_FILE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(.+)\\([^\\]+\\[^\\]+)\\[^\\]+\\SKILL\.md$")
        .expect("Failed to compile skill file pattern")
});

pub fn extract_skill_parent_directory(path: &Path) -> Result<PathBuf, Error> {
    let is_warp_home_skill = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "SKILL.md")
        && path
            .parent()
            .and_then(Path::parent)
            .is_some_and(|parent| warp_managed_skill_dirs().iter().any(|dir| parent == dir));
    if is_warp_home_skill {
        return dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Home directory not available for {}", path.display()));
    }
    let path_str = path.to_string_lossy();

    if let Some(captures) = SKILL_FILE_PATTERN.captures(&path_str) {
        if let Some(provider_path) = captures.get(2) {
            if SKILL_PROVIDER_PATHS.contains(provider_path.as_str()) {
                if let Some(parent_directory) = captures.get(1) {
                    return Ok(PathBuf::from(parent_directory.as_str()));
                }
            }
        }
    }

    Err(anyhow::anyhow!("Not a skill path: {}", path.display()))
}

/// Check if this path is a skill directory under a home directory provider path
/// E.g. ~/.agents/skills/skill-name
pub fn is_home_skill_directory(path: &Path) -> bool {
    let parent_directory = path.parent();
    if let Some(parent_directory) = parent_directory {
        is_home_provider_path(parent_directory)
    } else {
        false
    }
}

/// Check if this path is a home directory provider path
/// E.g. ~/.agents/skills
pub fn is_home_provider_path(path: &Path) -> bool {
    SKILL_PROVIDER_DEFINITIONS.iter().any(|provider| {
        if provider.provider == SkillProvider::Warp {
            return warp_managed_skill_dirs().iter().any(|dir| path == dir);
        }
        home_skills_path(provider.provider)
            .as_ref()
            .is_some_and(|home_skills_path| path == home_skills_path)
    })
}

#[cfg(test)]
#[path = "utils_tests.rs"]
mod tests;
