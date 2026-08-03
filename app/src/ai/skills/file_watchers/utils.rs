use std::path::{Path, PathBuf};

use ai::skills::{
    ParsedSkill, SKILL_PROVIDER_DEFINITIONS, SkillProvider, home_skills_path, parse_skill,
    provider_parent_directory_for_skills_root, read_skills,
};
use anyhow::Error;
use repo_metadata::{RepoMetadataModel, RepositoryIdentifier};
use walkdir::{DirEntry, WalkDir};
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

/// Finds project skill files from stored standing results.
///
/// Symlinked project skills are resolved while evaluating standing queries on the process that
/// owns the repository. This consumer treats those results as authoritative for both local and
/// remote repositories; direct filesystem discovery remains confined to metadata-failure fallback.
pub(super) fn find_project_skill_files_in_tree(
    repo_id: &RepositoryIdentifier,
    repo_metadata: &RepoMetadataModel,
    ctx: &AppContext,
) -> Vec<LocalOrRemotePath> {
    repo_metadata
        .standing_query_results(repo_id, ctx)
        .into_iter()
        .flat_map(|results| results.project_skills())
        .filter(|content| !content.is_directory)
        .map(|content| local_or_remote_path_for_repo_path(repo_id, &content.path))
        .collect()
}

/// Finds local project skill files by discovering provider directories on the filesystem.
///
/// This is a local-only fallback for repositories whose repo metadata indexing fails. Successful
/// local and remote project refreshes should use [`find_project_skill_files_in_tree`] so the
/// normal metadata-backed path remains shared.
pub(super) fn find_local_project_skill_files_on_filesystem(
    scan_root: &Path,
) -> Vec<LocalOrRemotePath> {
    let direct_skill_file = scan_root.join("SKILL.md");
    if is_skill_file(&direct_skill_file) {
        return vec![LocalOrRemotePath::Local(direct_skill_file)];
    }

    find_local_provider_directories_on_filesystem(scan_root)
        .into_iter()
        .flat_map(|provider_dir| std::fs::read_dir(provider_dir).into_iter().flatten())
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let skill_dir = entry.path();
            if !skill_dir.is_dir() {
                return None;
            }
            let skill_file = skill_dir.join("SKILL.md");
            skill_file
                .exists()
                .then_some(LocalOrRemotePath::Local(skill_file))
        })
        .collect()
}

fn find_local_provider_directories_on_filesystem(scan_root: &Path) -> Vec<PathBuf> {
    let mut provider_dirs = Vec::new();
    let mut entries = WalkDir::new(scan_root).follow_links(false).into_iter();
    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else {
            continue;
        };
        if is_ignored_fallback_scan_entry(&entry) {
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            continue;
        }
        if entry.file_type().is_dir() && is_project_provider_path(entry.path()) {
            provider_dirs.push(entry.into_path());
            entries.skip_current_dir();
        }
    }
    provider_dirs.sort();
    provider_dirs
}

fn is_ignored_fallback_scan_entry(entry: &DirEntry) -> bool {
    entry.file_name().to_str() == Some(".git")
}

fn is_project_provider_path(path: &Path) -> bool {
    SKILL_PROVIDER_DEFINITIONS
        .iter()
        .any(|provider| path.ends_with(&provider.skills_path))
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
    extract_skill_parent_directory(&LocalOrRemotePath::Local(path.to_path_buf())).is_ok()
}

pub fn extract_skill_parent_directory(
    path: &LocalOrRemotePath,
) -> Result<LocalOrRemotePath, Error> {
    let is_warp_home_skill = path
        .to_local_path()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "SKILL.md")
        && path
            .to_local_path()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .is_some_and(|parent| warp_managed_skill_dirs().iter().any(|dir| parent == dir));
    if is_warp_home_skill {
        return dirs::home_dir()
            .map(LocalOrRemotePath::Local)
            .ok_or_else(|| {
                anyhow::anyhow!("Home directory not available for {}", path.display_path())
            });
    }
    if path.file_name() != Some("SKILL.md") {
        return Err(anyhow::anyhow!("Not a skill path: {}", path.display_path()));
    }

    let Some(skill_dir) = path.parent() else {
        return Err(anyhow::anyhow!("Not a skill path: {}", path.display_path()));
    };
    let Some(skills_root) = skill_dir.parent() else {
        return Err(anyhow::anyhow!("Not a skill path: {}", path.display_path()));
    };

    provider_parent_directory_for_skills_root(&skills_root)
        .ok_or_else(|| anyhow::anyhow!("Not a skill path: {}", path.display_path()))
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
