//! Makes the skills listed in `WARP_SKILL_DIRS` available to third-party
//! harnesses (Claude Code, Codex), by symlinking them into a skill root each
//! harness already searches on its own.
//!
//! Oz reads `WARP_SKILL_DIRS` directly (see
//! `crate::ai::agent_sdk::driver::AgentDriver::load_skills_dirs`). Third-party
//! harnesses discover skills from their own skill roots instead, so this
//! module reads the same `WARP_SKILL_DIRS` directories and symlinks each
//! skill folder into the harness's skill root, under the skill's own name.
//! The published name must match the real skill name (rather than some
//! namespaced alias) because an agent prompt, or another skill, may
//! reference a skill by that name. Skill frontmatter is never rewritten.
//!
//! An existing symlink at a skill's target (most commonly ours, from an
//! earlier publish) is always replaced outright — that is not a conflict
//! with anything that predates us. A real, non-symlink entry is a genuine
//! conflict, and how it is resolved depends on [`is_sandbox`]:
//! - In a sandbox, we own the whole filesystem, so the published skill wins:
//!   the conflicting entry is renamed aside with a `.backup` suffix rather
//!   than deleted, so nothing is lost.
//! - Outside a sandbox (for example the self-hosted direct backend, which
//!   runs on a host we do not own) we never modify an entry that predates
//!   us. The skill is instead published under a `warp-<name>` alias, unless
//!   that alias also conflicts with a real entry, in which case the skill is
//!   not published at all.
//!
//! Every conflict is logged (see `logging-and-error-reporting`) with enough
//! detail to debug later — there is no user-facing surface for this today,
//! so the log is for us, not the user.
//!
//! A symlink — never a copy — keeps a skill's relative paths (for example a
//! helper script the skill invokes) pointing at the real, versioned skill
//! tree.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use ai::skills::{parse_skills_dirs_env, resolve_skills_dirs};
use anyhow::{Context, Result};
use warp_core::safe_warn;

/// Environment variable `AgentDriver` sets to `"1"` when it detects an
/// isolation platform (a Namespace instance, a Docker Sandbox, Kubernetes, or
/// a plain Docker container) — see `AgentDriver::new`. Its absence means we
/// are running directly on a host, such as the self-hosted direct backend,
/// which we do not necessarily own.
const IS_SANDBOX_ENV: &str = "IS_SANDBOX";

/// Suffix appended to a real (non-symlink) file or directory this module
/// moves aside, in a sandbox, so a published skill can take over its name.
/// The original content is preserved under this name rather than deleted.
const SANDBOX_BACKUP_SUFFIX: &str = ".backup";

/// Prefix used to publish a skill under an alternate name outside a sandbox,
/// when a real, non-symlink entry already occupies its real name.
const NON_SANDBOX_ALTERNATE_NAME_PREFIX: &str = "warp-";

/// Whether this run is executing inside a sandbox we control (see
/// [`IS_SANDBOX_ENV`]), as opposed to directly on a host we do not
/// necessarily own.
pub(super) fn is_sandbox(resolved_env_vars: &HashMap<OsString, OsString>) -> bool {
    resolved_env_vars.contains_key(OsStr::new(IS_SANDBOX_ENV))
}

/// Resolve the `WARP_SKILL_DIRS` source directories, most specific first —
/// the same directories and precedence order Oz uses (see
/// `ai::skills::read_skills_for_skills_dirs`).
pub(super) fn warp_skill_source_dirs(working_dir: &Path) -> Vec<PathBuf> {
    resolve_skills_dirs(working_dir, parse_skills_dirs_env())
}

/// Publish every skill found under `source_dirs` into `skill_root` as a
/// symlink under the skill's own name, pointing at the real skill folder.
/// Returns the number of skills published. See [`publish_skill`] for the
/// conflict-resolution behavior `is_sandbox` selects.
///
/// `source_dirs` is most-specific-first: when two directories contain a skill
/// folder with the same name, only the one from the first (most specific)
/// directory is published under that name — the same precedence Oz applies
/// when it reads these directories directly. This precedence choice among our
/// own source directories is not logged as a conflict; only a conflict with
/// an entry that did not come from this pass is (see [`publish_skill`]).
///
/// A failure to publish one skill (an unreadable directory, a missing
/// `SKILL.md`, a filesystem error) is logged and does not stop the rest of
/// the skills from publishing. Does nothing (not even creating `skill_root`)
/// when `source_dirs` is empty.
pub(super) fn publish_skill_dirs(
    skill_root: &Path,
    source_dirs: &[PathBuf],
    is_sandbox: bool,
) -> usize {
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
                    safe: ("WARP_SKILL_DIRS publish: skipping an unreadable source directory"),
                    full: ("WARP_SKILL_DIRS publish: skipping '{}' — {err}", source_dir.display())
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    safe_warn!(
                        safe: ("WARP_SKILL_DIRS publish: failed to read a directory entry"),
                        full: (
                            "WARP_SKILL_DIRS publish: failed to read an entry in '{}': {err}",
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
            match publish_skill(skill_root, name, &source_path, is_sandbox) {
                Ok(Some(_)) => published += 1,
                Ok(None) => {
                    // Deliberately skipped (a real conflict outside a sandbox whose
                    // alternate name also conflicts) — already logged by publish_skill.
                }
                Err(err) => {
                    safe_warn!(
                        safe: ("WARP_SKILL_DIRS publish: failed to publish a skill"),
                        full: ("WARP_SKILL_DIRS publish: failed to publish '{name}': {err:#}")
                    );
                }
            }
        }
    }
    published
}

/// State of the filesystem entry, if any, at a publish target.
enum TargetState {
    Missing,
    /// An existing symlink, and what it pointed to (if readable).
    Symlink(Option<PathBuf>),
    /// A real, non-symlink file or directory.
    RealEntry,
}

fn inspect_target(target: &Path) -> Result<TargetState> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Ok(TargetState::Symlink(fs::read_link(target).ok()))
        }
        Ok(_) => Ok(TargetState::RealEntry),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(TargetState::Missing),
        Err(err) => {
            Err(anyhow::Error::from(err).context(format!("failed to inspect {}", target.display())))
        }
    }
}

fn create_symlink_at(source_dir: &Path, target: &Path) -> Result<Option<PathBuf>> {
    create_symlink(source_dir, target).with_context(|| {
        format!(
            "failed to symlink {} -> {}",
            target.display(),
            source_dir.display()
        )
    })?;
    Ok(Some(target.to_path_buf()))
}

/// Publish a single skill folder as `<skill_root>/<skill_name>`, symlinked to
/// `source_dir`. Returns the published symlink path, or `None` when the skill
/// was deliberately not published (see the non-sandbox double-conflict case
/// below) — that is not an error.
///
/// An existing symlink at the target (most commonly our own, from an earlier
/// run) is always replaced outright; that is not a conflict with anything
/// that predates this pass.
///
/// A real, non-symlink entry at the target is a genuine conflict, resolved
/// according to `is_sandbox`:
/// - In a sandbox, the published skill wins: the conflicting entry is moved
///   aside to `<skill_root>/<skill_name>.backup` (or a numbered variant if
///   that's already taken) rather than deleted, so nothing is lost.
/// - Outside a sandbox, the conflicting entry is left untouched, and the
///   skill is instead published as `<skill_root>/warp-<skill_name>`. If that
///   alternate name is itself occupied by a real, non-symlink entry, the
///   skill is not published under either name.
///
/// Every conflict is logged via `safe_warn!` for later debugging — there is
/// no user-facing channel for this today.
pub(super) fn publish_skill(
    skill_root: &Path,
    skill_name: &str,
    source_dir: &Path,
    is_sandbox: bool,
) -> Result<Option<PathBuf>> {
    if !source_dir.join("SKILL.md").is_file() {
        anyhow::bail!(
            "source skill directory {} has no SKILL.md",
            source_dir.display()
        );
    }
    fs::create_dir_all(skill_root)
        .with_context(|| format!("failed to create skill root {}", skill_root.display()))?;
    let target = skill_root.join(skill_name);

    match inspect_target(&target)? {
        TargetState::Missing => return create_symlink_at(source_dir, &target),
        TargetState::Symlink(previous_target) => {
            fs::remove_file(&target).with_context(|| {
                format!("failed to remove existing symlink {}", target.display())
            })?;
            safe_warn!(
                safe: ("WARP_SKILL_DIRS publish: overriding an existing skill entry with the published version"),
                full: (
                    "WARP_SKILL_DIRS publish: overriding skill '{skill_name}' at {} (was a symlink to {:?}) with {}",
                    target.display(), previous_target, source_dir.display()
                )
            );
            return create_symlink_at(source_dir, &target);
        }
        TargetState::RealEntry => {}
    }

    // A real, non-symlink entry occupies `skill_name` — a genuine conflict
    // with something that predates this publish pass.
    if is_sandbox {
        let backup = reserve_conflict_backup_path(&target)?;
        fs::rename(&target, &backup).with_context(|| {
            format!(
                "failed to move existing skill entry {} aside to {}",
                target.display(),
                backup.display()
            )
        })?;
        safe_warn!(
            safe: ("WARP_SKILL_DIRS publish: replaced a conflicting skill entry in a sandbox, backing up the original"),
            full: (
                "WARP_SKILL_DIRS publish: replaced skill '{skill_name}' at {} with {}, backing up the original to {}",
                target.display(), source_dir.display(), backup.display()
            )
        );
        return create_symlink_at(source_dir, &target);
    }

    // Outside a sandbox: never modify an entry that predates us. Try the
    // `warp-<name>` alternate name instead.
    let alt_name = format!("{NON_SANDBOX_ALTERNATE_NAME_PREFIX}{skill_name}");
    let alt_target = skill_root.join(&alt_name);
    match inspect_target(&alt_target)? {
        TargetState::RealEntry => {
            safe_warn!(
                safe: ("WARP_SKILL_DIRS publish: a skill conflict outside a sandbox also collided under its alternate name; the skill was not published"),
                full: (
                    "WARP_SKILL_DIRS publish: skill '{skill_name}' conflicts with an existing entry at {} (left untouched); the alternate name {} is also occupied by a real entry, so the skill from {} was not published under either name",
                    target.display(), alt_target.display(), source_dir.display()
                )
            );
            Ok(None)
        }
        TargetState::Symlink(previous_alt_target) => {
            // Assume this is our own alias from an earlier, non-sandboxed run.
            fs::remove_file(&alt_target).with_context(|| {
                format!("failed to remove existing symlink {}", alt_target.display())
            })?;
            safe_warn!(
                safe: ("WARP_SKILL_DIRS publish: a skill conflicted outside a sandbox; the original was left as-is and the skill was published under an alternate name"),
                full: (
                    "WARP_SKILL_DIRS publish: skill '{skill_name}' conflicts with an existing entry at {} (left untouched); published {} as {} instead (was a symlink to {:?})",
                    target.display(), source_dir.display(), alt_target.display(), previous_alt_target
                )
            );
            create_symlink_at(source_dir, &alt_target)
        }
        TargetState::Missing => {
            safe_warn!(
                safe: ("WARP_SKILL_DIRS publish: a skill conflicted outside a sandbox; the original was left as-is and the skill was published under an alternate name"),
                full: (
                    "WARP_SKILL_DIRS publish: skill '{skill_name}' conflicts with an existing entry at {} (left untouched); published {} as {} instead",
                    target.display(), source_dir.display(), alt_target.display()
                )
            );
            create_symlink_at(source_dir, &alt_target)
        }
    }
}

/// Find an unused path to move an overridden, real (non-symlink) skill entry aside to,
/// by appending [`SANDBOX_BACKUP_SUFFIX`] to `target`'s file name, then a numeric
/// suffix if that's already taken. Bails rather than risk silently colliding with (and
/// losing) an earlier backup.
fn reserve_conflict_backup_path(target: &Path) -> Result<PathBuf> {
    let name = target.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        anyhow::anyhow!("skill target path {} has no file name", target.display())
    })?;
    let first_choice = target.with_file_name(format!("{name}{SANDBOX_BACKUP_SUFFIX}"));
    if !first_choice.exists() {
        return Ok(first_choice);
    }
    const MAX_BACKUP_ATTEMPTS: u32 = 20;
    for suffix in 2..=MAX_BACKUP_ATTEMPTS {
        let candidate = target.with_file_name(format!("{name}{SANDBOX_BACKUP_SUFFIX}-{suffix}"));
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
#[path = "skill_dirs_publish_tests.rs"]
mod tests;
