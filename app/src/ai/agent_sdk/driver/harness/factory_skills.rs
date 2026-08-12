//! Publishes factory playbook skills into the skill roots that third-party
//! harnesses (Claude Code, Codex) search on their own, so a factory agent on
//! either harness receives the same factory skill set as the Oz harness.
//!
//! Oz reads factory skills directly from the directories the server lists in
//! `WARP_SKILL_DIRS` (see `crate::ai::agent_sdk::driver::AgentDriver::load_skills_dirs`).
//! Third-party harnesses discover skills from their own home-directory skill
//! roots instead, so this module reads the same `WARP_SKILL_DIRS` directories
//! and symlinks each skill folder into the harness's skill root, prefixed
//! with `factory-` so a factory skill can never collide with a real,
//! user-owned skill folder. Skill frontmatter is never rewritten.
//!
//! A symlink — never a copy — keeps a skill's relative paths (for example a
//! helper script the skill invokes) pointing at the real, versioned skill
//! tree.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ai::skills::{parse_skills_dirs_env, resolve_skills_dirs};
use anyhow::{Context, Result};
use warp_core::safe_warn;

/// Prefix applied to every published factory skill's folder/symlink name, so
/// a factory skill can never collide with a real skill folder of the same
/// name.
pub(super) const FACTORY_SKILL_PREFIX: &str = "factory-";

/// Resolve the factory skill source directories from `WARP_SKILL_DIRS`, most
/// specific first — the same directories and precedence order Oz uses (see
/// `ai::skills::read_skills_for_skills_dirs`).
pub(super) fn factory_skill_source_dirs(working_dir: &Path) -> Vec<PathBuf> {
    resolve_skills_dirs(working_dir, parse_skills_dirs_env())
}

/// Publish every factory skill found under `source_dirs` into `skill_root` as
/// a `factory-<name>` symlink pointing at the real skill folder. Returns the
/// number of skills published.
///
/// `source_dirs` is most-specific-first: when two directories contain a skill
/// folder with the same name, only the one from the first (most specific)
/// directory is published under that name — the same precedence Oz applies
/// when it reads these directories directly.
///
/// A failure to publish one skill (an unreadable directory, a missing
/// `SKILL.md`, a filesystem error) is logged and does not stop the rest of
/// the skills from publishing. Does nothing (not even creating `skill_root`)
/// when `source_dirs` is empty.
pub(super) fn publish_factory_skills(skill_root: &Path, source_dirs: &[PathBuf]) -> usize {
    if source_dirs.is_empty() {
        return 0;
    }

    let mut published_names = HashSet::new();
    let mut published = 0usize;
    for source_dir in source_dirs {
        let entries = match fs::read_dir(source_dir) {
            Ok(entries) => entries,
            Err(err) => {
                safe_warn!(
                    safe: ("Factory skill publish: skipping an unreadable source directory"),
                    full: ("Factory skill publish: skipping '{}' — {err}", source_dir.display())
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    safe_warn!(
                        safe: ("Factory skill publish: failed to read a directory entry"),
                        full: (
                            "Factory skill publish: failed to read an entry in '{}': {err}",
                            source_dir.display()
                        )
                    );
                    continue;
                }
            };
            let source_path = entry.path();
            if !source_path.is_dir() || !source_path.join("SKILL.md").is_file() {
                // Not a skill folder.
                continue;
            }
            let Some(name) = source_path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !published_names.insert(name.to_owned()) {
                // A more specific directory already published a skill with this name.
                continue;
            }
            if let Err(err) = publish_factory_skill(skill_root, name, &source_path) {
                safe_warn!(
                    safe: ("Factory skill publish: failed to publish a factory skill"),
                    full: ("Factory skill publish: failed to publish '{name}': {err:#}")
                );
                continue;
            }
            published += 1;
        }
    }
    published
}

/// Publish a single factory skill folder as `<skill_root>/factory-<skill_name>`,
/// symlinked to `source_dir`. Returns the published symlink path.
///
/// Idempotent: replaces a stale `factory-`-prefixed symlink left by an
/// earlier run. Refuses to touch a target that already exists and is not a
/// symlink, since that could be a real, user-owned directory.
pub(super) fn publish_factory_skill(
    skill_root: &Path,
    skill_name: &str,
    source_dir: &Path,
) -> Result<PathBuf> {
    if !source_dir.join("SKILL.md").is_file() {
        anyhow::bail!(
            "source skill directory {} has no SKILL.md",
            source_dir.display()
        );
    }
    fs::create_dir_all(skill_root)
        .with_context(|| format!("failed to create skill root {}", skill_root.display()))?;
    let target = skill_root.join(format!("{FACTORY_SKILL_PREFIX}{skill_name}"));

    match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if !metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "{} already exists and is not a symlink; leaving it in place",
                    target.display()
                );
            }
            fs::remove_file(&target)
                .with_context(|| format!("failed to remove stale symlink {}", target.display()))?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(
                anyhow::Error::from(err).context(format!("failed to inspect {}", target.display()))
            );
        }
    }

    create_symlink(source_dir, &target).with_context(|| {
        format!(
            "failed to symlink {} -> {}",
            target.display(),
            source_dir.display()
        )
    })?;
    Ok(target)
}

#[cfg(unix)]
fn create_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn create_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
}

#[cfg(test)]
#[path = "factory_skills_tests.rs"]
mod tests;
