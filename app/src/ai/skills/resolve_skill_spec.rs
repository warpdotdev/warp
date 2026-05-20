//! Skill resolution for agent runs.
//!
//! This module exists primarily for `warp agent run --skill ...` (and related flows) where we need to
//! resolve a CLI-provided `--skill` specifier (`SkillSpec`) into a concrete `SKILL.md` file and its
//! parsed instruction body.
//!
//! While `SkillManager` maintains a cached view of known skills and can list skills "in scope",
//! agent runs need a single-shot resolver that:
//! - Works when invoked from directories *above* any detected repos (ambient/root-scope case).
//! - Supports qualified specs (`repo:skill` and `org/repo:skill`) and returns good errors on ambiguity.
//! - Applies consistent skill-directory precedence (e.g. `.claude/` vs `.codex/`, etc.).
//! - Falls back to scanning disk when the manager cache has not warmed yet.

use std::path::{Path, PathBuf};

use ai::skills::{
    home_skills_path, parse_skill, ParsedSkill, SkillProvider, SKILL_PROVIDER_DEFINITIONS,
};
use command::blocking::Command;
use command::r#async::Command as AsyncCommand;
use warp_cli::skill::SkillSpec;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::AppContext;
use warpui::SingletonEntity as _;

use super::SkillManager;
use crate::warp_managed_paths_watcher::warp_managed_skill_dirs;

const SKILL_FILE_NAME: &str = "SKILL.md";

#[derive(Debug, Clone)]
pub struct ResolvedSkill {
    pub skill_path: PathBuf,
    pub name: String,
    pub instructions: String,
    /// The full parsed skill, used for proto conversion when sending to server.
    pub parsed_skill: ParsedSkill,
}

fn resolve_from_skill_dirs_by_directory_scan(
    spec: &SkillSpec,
    skill_dirs: impl IntoIterator<Item = PathBuf>,
) -> Result<Option<ResolvedSkill>, ResolveSkillError> {
    if spec.is_full_path() {
        return Ok(None);
    }

    for skill_dir in skill_dirs {
        let path = skill_dir.join(&spec.skill_identifier).join(SKILL_FILE_NAME);

        if path.exists() {
            let parsed = parse_skill(&path).map_err(|err| ResolveSkillError::ParseFailed {
                path: path.clone(),
                message: err.to_string(),
            })?;

            return Ok(Some(to_resolved_skill(path, parsed)));
        }
    }

    Ok(None)
}

fn home_skill_dirs_for_resolution() -> Vec<PathBuf> {
    let mut skill_dirs = Vec::new();
    for provider in SKILL_PROVIDER_DEFINITIONS.iter() {
        if provider.provider == SkillProvider::Warp {
            for dir in warp_managed_skill_dirs() {
                push_unique_path(&mut skill_dirs, dir);
            }
        } else if let Some(dir) = home_skills_path(provider.provider) {
            push_unique_path(&mut skill_dirs, dir);
        }
    }
    skill_dirs
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveSkillError {
    #[error("Skill '{skill}' not found")]
    NotFound { skill: String },
    #[error("Repository '{repo}' not found")]
    RepoNotFound { repo: String },
    #[error("Skill '{skill}' is ambiguous; specify as repo:skill_name")]
    Ambiguous {
        skill: String,
        candidates: Vec<PathBuf>,
    },
    #[error("Repository '{repo}' found but belongs to org '{found}', expected '{expected}'")]
    OrgMismatch {
        repo: String,
        expected: String,
        found: String,
    },
    #[error("Failed to parse skill file {path}: {message}")]
    ParseFailed { path: PathBuf, message: String },
    #[error("Failed to clone repository '{org}/{repo}': {message}")]
    CloneFailed {
        org: String,
        repo: String,
        message: String,
    },
}

/// Resolve a `SkillSpec` (from the `--skill` CLI arg) into a concrete SKILL.md file.
///
/// Resolution flow:
/// - If the spec is repo-qualified (`repo:skill` or `org/repo:skill`):
///   - Find candidate repo roots from `SkillManager` (fallback: a direct child of `working_dir`).
///   - Optionally filter by `org` using the repo's `origin` remote when available.
///   - For each candidate repo root, try to resolve the skill (cache-first, then disk scan).
/// - If the spec is unqualified (`skill`):
///   1. Check cached home-directory skills.
///   2. Scan home/global skill directories directly for cold-start resolution.
///   3. Try the current repo root (if `working_dir` is inside a detected repo).
///   4. Otherwise, search across repos *under* `working_dir` (ambient/root-scope support).
///      - If multiple matches exist, return an ambiguity error.
///   5. Finally, fall back to scanning the filesystem relative to `working_dir`.
///
/// Once a SKILL.md is selected, we return the parsed instruction body (front matter stripped).
pub fn resolve_skill_spec(
    spec: &SkillSpec,
    working_dir: &Path,
    ctx: &AppContext,
) -> Result<ResolvedSkill, ResolveSkillError> {
    let skill_manager = SkillManager::as_ref(ctx);

    match &spec.repo {
        Some(repo) => resolve_repo_qualified(spec, repo, working_dir, skill_manager, ctx),
        None => resolve_unqualified(spec, working_dir, ctx, skill_manager),
    }
}

/// Clone a repository from GitHub into the working directory for skill resolution.
///
/// Uses HTTPS format: `https://github.com/org/repo.git`
///
/// This is used in sandboxed environments to auto-clone repos when a fully-qualified
/// skill spec references a repo that doesn't exist locally.
pub async fn clone_repo_for_skill(
    org: &str,
    repo: &str,
    working_dir: &Path,
) -> Result<(), ResolveSkillError> {
    let repo_url = format!("https://github.com/{org}/{repo}.git");
    let target_dir = working_dir.join(repo);

    // Check if target already exists.
    if target_dir.exists() {
        if target_dir.join(".git").is_dir() {
            log::info!(
                "Target directory {} already exists and appears to be a git repo, skipping clone",
                target_dir.display()
            );
            return Ok(());
        }

        return Err(ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: format!(
                "Target directory {} already exists but is not a git repository",
                target_dir.display()
            ),
        });
    }

    log::info!("Cloning {} into {}", repo_url, target_dir.display());
    log::debug!(
        "[GIT OPERATION] resolve_skill_spec.rs clone_repo_for_skill git clone {} {}",
        repo_url,
        target_dir.display()
    );

    let output = AsyncCommand::new("git")
        .arg("clone")
        .arg(&repo_url)
        .arg(&target_dir)
        .current_dir(working_dir)
        .output()
        .await
        .map_err(|e| ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: format!("Failed to execute git clone: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ResolveSkillError::CloneFailed {
            org: org.to_string(),
            repo: repo.to_string(),
            message: stderr.trim().to_string(),
        });
    }

    log::info!("Successfully cloned {org}/{repo}");
    Ok(())
}

fn resolve_repo_qualified(
    spec: &SkillSpec,
    repo: &str,
    working_dir: &Path,
    skill_manager: &SkillManager,
    _ctx: &AppContext,
) -> Result<ResolvedSkill, ResolveSkillError> {
    // Find directories with skills where the directory name matches the repo.
    // This includes both repo roots and subdirectories.
    let mut candidate_repo_roots: Vec<PathBuf> = skill_manager
        .directories_with_skills()
        .into_iter()
        .filter(|dir| dir.file_name().is_some_and(|n| n == repo))
        .collect();

    // Fallback: if we don't know about the repo yet, check a direct child directory.
    if candidate_repo_roots.is_empty() {
        let direct_child = working_dir.join(repo);
        if direct_child.is_dir() {
            candidate_repo_roots.push(direct_child);
        }
    }

    if candidate_repo_roots.is_empty() {
        return Err(ResolveSkillError::RepoNotFound {
            repo: repo.to_string(),
        });
    }

    // If org is specified, validate it when we can determine the org.
    if let Some(expected_org) = &spec.org {
        let mut filtered = Vec::new();
        let mut first_mismatch: Option<String> = None;

        for repo_root in &candidate_repo_roots {
            match get_git_remote_org(repo_root) {
                Some(found_org) if &found_org != expected_org => {
                    if first_mismatch.is_none() {
                        first_mismatch = Some(found_org);
                    }
                }
                _ => filtered.push(repo_root.clone()),
            }
        }

        if filtered.is_empty() {
            return Err(ResolveSkillError::OrgMismatch {
                repo: repo.to_string(),
                expected: expected_org.clone(),
                found: first_mismatch.unwrap_or_else(|| "unknown".to_string()),
            });
        }

        candidate_repo_roots = filtered;
    }

    // Try each matching repo root in a stable order.
    candidate_repo_roots.sort();

    for repo_root in candidate_repo_roots {
        match resolve_in_single_repo_root(spec, &repo_root, skill_manager) {
            Ok(resolved) => return Ok(resolved),
            Err(ResolveSkillError::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }
    }

    Err(ResolveSkillError::NotFound {
        skill: spec.skill_identifier.clone(),
    })
}

fn resolve_unqualified(
    spec: &SkillSpec,
    working_dir: &Path,
    ctx: &AppContext,
    skill_manager: &SkillManager,
) -> Result<ResolvedSkill, ResolveSkillError> {
    // If the skill_path is a full path, skip cache lookup and go straight to disk resolution.
    // Full paths don't match skill names in the cache.
    if spec.is_full_path() {
        if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, working_dir)? {
            return Ok(resolved);
        }
        return Err(ResolveSkillError::NotFound {
            skill: spec.skill_identifier.clone(),
        });
    }

    // Get all skill paths matching the requested name from the cache.
    let all_matching_paths = skill_manager.skill_paths_by_name(&spec.skill_identifier);
    let home_dir = dirs::home_dir();

    // Per the skills spec, home directory skills take precedence over project skills.
    // Check home directory skills first.
    let home_skill_paths = skill_manager.home_skill_paths();
    let home_matches: Vec<PathBuf> = all_matching_paths
        .iter()
        .filter(|p| home_skill_paths.contains(p))
        .cloned()
        .collect();

    if let Some(skill_path) = best_match_by_directory_precedence(home_matches, home_dir.as_deref())
    {
        return parsed_skill_from_manager_or_disk(skill_manager, &skill_path)
            .map(|parsed| to_resolved_skill(skill_path, parsed));
    }

    if let Some(resolved) =
        resolve_from_skill_dirs_by_directory_scan(spec, home_skill_dirs_for_resolution())?
    {
        return Ok(resolved);
    }

    // Next, try to scope to the current repo root (if known).
    let repo_root = repo_metadata::repositories::DetectedRepositories::as_ref(ctx)
        .get_root_for_path(&LocalOrRemotePath::Local(working_dir.to_path_buf()))
        .and_then(|r| PathBuf::try_from(r).ok());

    if let Some(repo_root) = repo_root {
        match resolve_in_single_repo_root(spec, &repo_root, skill_manager) {
            Ok(resolved) => return Ok(resolved),
            Err(ResolveSkillError::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }
    }

    // If we're not in a known repo, try searching across repos under the working directory.
    let in_scope_matches: Vec<PathBuf> = all_matching_paths
        .into_iter()
        .filter(|p| {
            // Only include project skills (not home skills) that are under working_dir
            skill_manager.skill_paths_in_scope(working_dir).contains(p)
        })
        .collect();

    if in_scope_matches.len() == 1 {
        let skill_path = in_scope_matches[0].clone();
        return parsed_skill_from_manager_or_disk(skill_manager, &skill_path)
            .map(|parsed| to_resolved_skill(skill_path, parsed));
    }

    if in_scope_matches.len() > 1 {
        return Err(ResolveSkillError::Ambiguous {
            skill: spec.skill_identifier.clone(),
            candidates: in_scope_matches,
        });
    }

    // Fallback: if SkillManager hasn't cached anything yet, try resolving relative to the working dir.
    if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, working_dir)? {
        return Ok(resolved);
    }

    Err(ResolveSkillError::NotFound {
        skill: spec.skill_identifier.clone(),
    })
}

fn resolve_in_single_repo_root(
    spec: &SkillSpec,
    repo_root: &Path,
    skill_manager: &SkillManager,
) -> Result<ResolvedSkill, ResolveSkillError> {
    // If the skill_path is a full path, skip cache lookup and go straight to disk resolution.
    // Full paths don't match skill names in the cache.
    if spec.is_full_path() {
        if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, repo_root)? {
            return Ok(resolved);
        }
        return Err(ResolveSkillError::NotFound {
            skill: spec.skill_identifier.clone(),
        });
    }

    // Prefer cached skills (fast path).
    // Use the name-based cache and filter to paths within this repo root.
    let repo_skill_paths = skill_manager.skill_paths_in_scope(repo_root);
    let cached_paths: Vec<PathBuf> = skill_manager
        .skill_paths_by_name(&spec.skill_identifier)
        .into_iter()
        .filter(|p| repo_skill_paths.contains(p))
        .collect();

    if let Some(best_path) = best_match_by_directory_precedence(cached_paths, Some(repo_root)) {
        let parsed = parsed_skill_from_manager_or_disk(skill_manager, &best_path)?;
        return Ok(to_resolved_skill(best_path, parsed));
    }

    // Cold start fallback: check disk in precedence order.
    if let Some(resolved) = resolve_from_root_path_by_directory_scan(spec, repo_root)? {
        return Ok(resolved);
    }

    Err(ResolveSkillError::NotFound {
        skill: spec.skill_identifier.clone(),
    })
}

fn resolve_from_root_path_by_directory_scan(
    spec: &SkillSpec,
    root: &Path,
) -> Result<Option<ResolvedSkill>, ResolveSkillError> {
    // If the skill_path is a full path (contains "/" or ends with ".md"),
    // try to resolve it directly without iterating through SKILL_PROVIDER_DEFINITIONS.
    if spec.is_full_path() {
        // Reject absolute paths to prevent escaping the root directory
        let skill_path = Path::new(&spec.skill_identifier);
        if skill_path.is_absolute() {
            return Err(ResolveSkillError::NotFound {
                skill: spec.skill_identifier.clone(),
            });
        }

        let path = root.join(&spec.skill_identifier);
        if path.exists() {
            let parsed = parse_skill(&path).map_err(|err| ResolveSkillError::ParseFailed {
                path: path.clone(),
                message: err.to_string(),
            })?;

            return Ok(Some(to_resolved_skill(path, parsed)));
        }
        // If full path doesn't exist, return None (don't fall through to directory scan)
        return Ok(None);
    }

    // For simple skill names, iterate through SKILL_PROVIDER_DEFINITIONS in precedence order.
    for provider in SKILL_PROVIDER_DEFINITIONS.iter() {
        let path = root
            .join(&provider.skills_path)
            .join(&spec.skill_identifier)
            .join(SKILL_FILE_NAME);

        if path.exists() {
            let parsed = parse_skill(&path).map_err(|err| ResolveSkillError::ParseFailed {
                path: path.clone(),
                message: err.to_string(),
            })?;

            return Ok(Some(to_resolved_skill(path, parsed)));
        }
    }

    Ok(None)
}

fn parsed_skill_from_manager_or_disk(
    skill_manager: &SkillManager,
    skill_path: &Path,
) -> Result<ParsedSkill, ResolveSkillError> {
    if let Some(parsed) = skill_manager.skill_by_path(skill_path).cloned() {
        return Ok(parsed);
    }

    parse_skill(skill_path).map_err(|err| ResolveSkillError::ParseFailed {
        path: skill_path.to_path_buf(),
        message: err.to_string(),
    })
}

fn to_resolved_skill(skill_path: PathBuf, parsed: ParsedSkill) -> ResolvedSkill {
    let instructions = instructions_body(&parsed);
    ResolvedSkill {
        name: parsed.name.clone(),
        instructions,
        skill_path,
        parsed_skill: parsed,
    }
}

fn instructions_body(skill: &ParsedSkill) -> String {
    let Some(line_range) = &skill.line_range else {
        return skill.content.clone();
    };

    // line_range is 1-indexed, end-exclusive.
    let start = line_range.start.saturating_sub(1);
    let end = line_range.end.saturating_sub(1);

    let lines: Vec<&str> = skill.content.lines().collect();
    if start >= lines.len() {
        return String::new();
    }

    let end = end.min(lines.len());
    lines[start..end].join("\n").trim().to_string()
}

fn best_match_by_directory_precedence(
    mut matches: Vec<PathBuf>,
    root: Option<&Path>,
) -> Option<PathBuf> {
    if matches.is_empty() {
        return None;
    }

    // If we can't determine the root, fall back to a stable path sort.
    let Some(root) = root else {
        matches.sort();
        return matches.into_iter().next();
    };

    matches.sort_by(|a, b| {
        let a_rank = directory_precedence_rank(root, a);
        let b_rank = directory_precedence_rank(root, b);

        a_rank.cmp(&b_rank).then_with(|| a.cmp(b))
    });

    matches.into_iter().next()
}

fn directory_precedence_rank(root: &Path, skill_path: &Path) -> usize {
    for (idx, provider) in SKILL_PROVIDER_DEFINITIONS.iter().enumerate() {
        if skill_path.starts_with(root.join(&provider.skills_path)) {
            return idx;
        }
    }

    SKILL_PROVIDER_DEFINITIONS.len()
}

fn get_git_remote_org(repo_path: &Path) -> Option<String> {
    log::debug!(
        "[GIT OPERATION] resolve_skill_spec.rs get_git_remote_org git remote get-url origin"
    );
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8(output.stdout).ok()?.trim().to_string();
    parse_org_from_git_url(&url)
}

fn parse_org_from_git_url(url: &str) -> Option<String> {
    // SSH format: git@github.com:org/repo.git
    if let Some(rest) = url.strip_prefix("git@") {
        if let Some(path_start) = rest.find(':') {
            let path = &rest[path_start + 1..];
            if let Some(slash_pos) = path.find('/') {
                return Some(path[..slash_pos].to_string());
            }
        }
    }

    // HTTPS format: https://github.com/org/repo.git
    if url.starts_with("https://") || url.starts_with("http://") {
        let parts: Vec<&str> = url.split('/').collect();
        // https://github.com/org/repo.git
        if parts.len() >= 4 {
            return Some(parts[3].to_string());
        }
    }

    None
}

#[cfg(test)]
#[path = "resolve_skill_spec_tests.rs"]
mod tests;
