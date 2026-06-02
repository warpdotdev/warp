use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ai::skills::{
    home_skills_path, parse_skill, provider_parent_directory_for_skills_root, read_skills,
    ParsedSkill, SkillProvider, SKILL_PROVIDER_DEFINITIONS,
};
use anyhow::Error;
use repo_metadata::file_tree_update::RepoNodeMetadata;
use repo_metadata::local_model::GetContentsArgs;
use repo_metadata::{RepoContent, RepoMetadataModel, RepoMetadataUpdate, RepositoryIdentifier};
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

/// Returns whether an incremental metadata update can alter project skills for a repository.
///
/// Local provider-directory changes are included because symlinked skill directories are
/// hydrated from the local filesystem rather than represented directly in repo metadata.
pub(super) fn update_might_affect_project_skills(
    repo_id: &RepositoryIdentifier,
    update: &RepoMetadataUpdate,
    known_skill_files: Option<&HashSet<LocalOrRemotePath>>,
) -> bool {
    if update.remove_entries.iter().any(|removed_path| {
        let removed_path = local_or_remote_path_for_repo_path(repo_id, removed_path);
        known_skill_files
            .into_iter()
            .flatten()
            .any(|known_skill_file| known_skill_file.starts_with(&removed_path))
    }) {
        return true;
    }

    let is_local_repo = matches!(repo_id, RepositoryIdentifier::Local(_));
    update.update_entries.iter().any(|entry_update| {
        if is_local_repo
            && is_project_provider_path(&entry_update.parent_path_to_replace.to_local_path_lossy())
        {
            return true;
        }

        entry_update.subtree_metadata.iter().any(|node| match node {
            RepoNodeMetadata::File(file) => {
                let path = local_or_remote_path_for_repo_path(repo_id, &file.path);
                extract_skill_parent_directory(&path).is_ok()
            }
            RepoNodeMetadata::Directory(directory) => {
                is_local_repo && is_project_provider_path(&directory.path.to_local_path_lossy())
            }
        })
    })
}
/// Finds project skill files and local symlinked skill files with one metadata traversal.
///
/// Local provider directories are included in the metadata query so filesystem hydration can
/// supplement indexed files with directory symlinks. Remote repositories only return indexed
/// skill files because their filesystems are unavailable to the client.
pub(super) fn find_project_skill_files_in_tree(
    repo_id: &RepositoryIdentifier,
    repo_metadata: &RepoMetadataModel,
    ctx: &AppContext,
) -> Vec<LocalOrRemotePath> {
    let include_local_provider_directories = matches!(repo_id, RepositoryIdentifier::Local(_));
    let repo_id_for_filter = repo_id.clone();
    let args = GetContentsArgs {
        include_folders: include_local_provider_directories,
        ..GetContentsArgs::default()
    }
    .include_ignored()
    .with_filter(move |content| match content {
        RepoContent::File(file) => {
            let path = local_or_remote_path_for_repo_path(&repo_id_for_filter, &file.path);
            extract_skill_parent_directory(&path).is_ok()
        }
        RepoContent::Directory(directory) => {
            include_local_provider_directories
                && is_project_provider_path(&directory.path.to_local_path_lossy())
        }
    });

    let mut skill_files = Vec::new();
    let mut local_provider_directories = Vec::new();
    for content in repo_metadata
        .get_repo_contents(repo_id, args, ctx)
        .unwrap_or_default()
    {
        match content {
            RepoContent::File(file) => {
                skill_files.push(local_or_remote_path_for_repo_path(repo_id, &file.path));
            }
            RepoContent::Directory(directory) => {
                if let Some(path) = directory.path.to_local_path() {
                    local_provider_directories.push(path);
                }
            }
        }
    }

    skill_files.extend(
        find_symlinked_skill_files_in_local_provider_directories(local_provider_directories)
            .into_iter()
            .map(LocalOrRemotePath::Local),
    );
    skill_files
}

/// Reads local project skills by discovering provider directories on the filesystem.
///
/// This is a local-only fallback for repositories whose repo metadata indexing fails. Successful
/// local and remote project refreshes should use [`find_project_skill_files_in_tree`] so the
/// normal metadata-backed path remains shared.
pub(super) fn read_local_project_skills_from_filesystem(scan_root: &Path) -> Vec<ParsedSkill> {
    let direct_skill_file = scan_root.join("SKILL.md");
    if is_skill_file(&direct_skill_file) {
        return read_skills_from_files([direct_skill_file]);
    }

    read_skills_from_directories(find_local_provider_directories_on_filesystem(scan_root))
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

/// Finds symlinked skill directories under loaded local provider directories in a repository.
///
/// Repo metadata intentionally skips directory symlinks to avoid duplicate trees/cycles. Project
/// skill refreshes are still triggered by repo metadata, but local hydration supplements the tree
/// with `SKILL.md` files from symlinked skill directories so existing symlink handling is preserved.
fn find_symlinked_skill_files_in_local_provider_directories(
    provider_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    provider_dirs
        .into_iter()
        .flat_map(|provider_dir| {
            std::fs::read_dir(provider_dir)
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let skill_dir = entry.path();
                    if skill_dir.is_symlink() && skill_dir.is_dir() {
                        let skill_file = skill_dir.join("SKILL.md");
                        if skill_file.exists() {
                            return Some(skill_file);
                        }
                    }
                    None
                })
        })
        .collect()
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
