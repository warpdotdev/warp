//! Publishes factory playbook skills into the skill roots that third-party
//! harnesses (Claude Code, Codex) search on their own, so a factory agent on
//! either harness receives the same factory skill set as the Oz harness.
//!
//! Oz reads factory skills directly from the directories the server lists in
//! `WARP_SKILL_DIRS` (see `crate::ai::agent_sdk::driver::AgentDriver::load_skills_dirs`).
//! Third-party harnesses discover skills from their own home-directory skill
//! roots instead, so this module reads the same `WARP_SKILL_DIRS` directories
//! and symlinks each skill folder into the harness's skill root, under the
//! skill's own name. The published name must match the real skill name
//! (rather than some namespaced alias) because an agent prompt, or another
//! skill, may reference a skill by that name. Skill frontmatter is never
//! rewritten.
//!
//! A factory skill wins over any existing entry with the same name already in
//! the harness's skill root. Every such override is logged (see
//! `logging-and-error-reporting`) with enough detail to debug later — there is
//! no user-facing surface for this today, so the log is for us, not the user.
//! An existing *real* (non-symlink) entry is moved aside rather than deleted,
//! so an override is always recoverable and never silently destroys a user's
//! own directory.
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

/// Suffix appended to a real (non-symlink) file or directory this module
/// moves aside so a factory skill can take over its name. The original
/// content is preserved under this name rather than deleted.
const PRE_FACTORY_BACKUP_SUFFIX: &str = ".pre-factory-backup";

/// Resolve the factory skill source directories from `WARP_SKILL_DIRS`, most
/// specific first — the same directories and precedence order Oz uses (see
/// `ai::skills::read_skills_for_skills_dirs`).
pub(super) fn factory_skill_source_dirs(working_dir: &Path) -> Vec<PathBuf> {
    resolve_skills_dirs(working_dir, parse_skills_dirs_env())
}

/// Publish every factory skill found under `source_dirs` into `skill_root` as
/// a symlink under the skill's own name, pointing at the real skill folder.
/// Returns the number of skills published.
///
/// `source_dirs` is most-specific-first: when two directories contain a skill
/// folder with the same name, only the one from the first (most specific)
/// directory is published under that name — the same precedence Oz applies
/// when it reads these directories directly. This precedence choice among our
/// own source directories is not logged as a conflict; only an override of an
/// entry that did not come from this pass is (see [`publish_factory_skill`]).
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

/// Publish a single factory skill folder as `<skill_root>/<skill_name>`,
/// symlinked to `source_dir`. Returns the published symlink path.
///
/// A factory skill wins over whatever already occupies `<skill_root>/<skill_name>`:
/// - An existing symlink (most commonly our own from an earlier run) is replaced outright.
/// - An existing real file or directory is moved aside to
///   `<skill_root>/<skill_name>.pre-factory-backup` (or a numbered variant if that's
///   already taken) rather than deleted, so nothing is lost.
///
/// Every override is logged via `safe_warn!` for later debugging — there is no
/// user-facing channel for this today.
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
    let target = skill_root.join(skill_name);

    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let previous_target = fs::read_link(&target).ok();
            fs::remove_file(&target).with_context(|| {
                format!("failed to remove existing symlink {}", target.display())
            })?;
            safe_warn!(
                safe: ("Factory skill publish: overriding an existing skill entry with the factory version"),
                full: (
                    "Factory skill publish: overriding skill '{skill_name}' at {} (was a symlink to {:?}) with {}",
                    target.display(), previous_target, source_dir.display()
                )
            );
        }
        Ok(_) => {
            let backup = reserve_conflict_backup_path(&target)?;
            fs::rename(&target, &backup).with_context(|| {
                format!(
                    "failed to move existing skill entry {} aside to {}",
                    target.display(),
                    backup.display()
                )
            })?;
            safe_warn!(
                safe: ("Factory skill publish: moved a real, non-symlink skill entry aside so the factory version could take over its name"),
                full: (
                    "Factory skill publish: moved existing skill '{skill_name}' from {} to {} before publishing factory skill from {}",
                    target.display(), backup.display(), source_dir.display()
                )
            );
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

/// Find an unused path to move an overridden, real (non-symlink) skill entry aside to,
/// by appending [`PRE_FACTORY_BACKUP_SUFFIX`] to `target`'s file name, then a numeric
/// suffix if that's already taken. Bails rather than risk silently colliding with (and
/// losing) an earlier backup.
fn reserve_conflict_backup_path(target: &Path) -> Result<PathBuf> {
    let name = target.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        anyhow::anyhow!("skill target path {} has no file name", target.display())
    })?;
    let first_choice = target.with_file_name(format!("{name}{PRE_FACTORY_BACKUP_SUFFIX}"));
    if !first_choice.exists() {
        return Ok(first_choice);
    }
    const MAX_BACKUP_ATTEMPTS: u32 = 20;
    for suffix in 2..=MAX_BACKUP_ATTEMPTS {
        let candidate =
            target.with_file_name(format!("{name}{PRE_FACTORY_BACKUP_SUFFIX}-{suffix}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "could not find a free backup path for {} after {MAX_BACKUP_ATTEMPTS} attempts",
        target.display()
    )
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
