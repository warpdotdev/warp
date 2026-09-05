use std::fs;
use std::path::{Path, PathBuf};

use super::parse_skill::{ParsedSkill, parse_skill};
use super::skill_provider::SkillScope;

/// Bounds the total bytes of skill file content retained from one directory scan. Mirrors the
/// batch budget the remote/project skill read path enforces (`REMOTE_CONTEXT_MAX_BATCH_BYTES`);
/// without it, a catalog made up of many individually-valid skill files (each under the per-file
/// cap) can still retain unbounded total content, reproducing the multi-GB catalog growth this
/// cap is meant to contain (Sentry 7259255054).
const MAX_SKILLS_BATCH_BYTES: u64 = 5 * 1024 * 1024;

/// The environment variable that specifies extra skill directories to index at
/// personal (home) precedence. Value is a comma-separated list of paths; each
/// path is itself a skills directory whose **direct children** are skill folders
/// containing `SKILL.md`. Relative paths are resolved against the environment
/// working directory (see [`resolve_skills_dirs`]).
pub const WARP_SKILL_DIRS_ENV: &str = "WARP_SKILL_DIRS";

/// Parse the `WARP_SKILL_DIRS` environment variable into a list of directory paths.
///
/// Splits on commas, trims leading/trailing whitespace from each entry, and
/// drops blank entries. Returns an empty vec when the variable is unset or empty.
///
/// A leading `~` is expanded to the home directory. Entries are otherwise
/// returned verbatim — callers resolve relative paths against the environment
/// working directory (see [`resolve_skills_dirs`]) rather than letting them
/// depend on the process's current working directory.
pub fn parse_skills_dirs_env() -> Vec<PathBuf> {
    let Ok(val) = std::env::var(WARP_SKILL_DIRS_ENV) else {
        return Vec::new();
    };
    val.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|entry| PathBuf::from(shellexpand::tilde(entry).into_owned()))
        .collect()
}

/// Resolve entries parsed from [`WARP_SKILL_DIRS_ENV`] against a base directory.
///
/// Absolute entries pass through unchanged; relative entries are joined onto
/// `base` (the environment working directory) so their meaning does not depend
/// on the agent process's current working directory, which environment setup
/// may have changed.
pub fn resolve_skills_dirs(base: &Path, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    dirs.into_iter()
        .map(|dir| {
            if dir.is_absolute() {
                dir
            } else {
                base.join(dir)
            }
        })
        .collect()
}

/// Read skills from a slice of directories, treating each as a personal (home)
/// tier skills root.
///
/// Each directory in `dirs` is expected to contain individual skill folders as
/// **direct children** (e.g. `<dir>/<skill-name>/SKILL.md`). This matches the
/// layout used by `WARP_SKILL_DIRS`. Entries that are not directories are skipped
/// with a warning.
///
/// Skills loaded this way are assigned `SkillScope::Home` so they are always
/// in scope — the same precedence as personal skills from `~/.agents/skills`.
pub fn read_skills_for_skills_dirs(dirs: &[PathBuf]) -> Vec<ParsedSkill> {
    dirs.iter()
        .filter(|dir| {
            if dir.is_dir() {
                return true;
            }
            log::warn!(
                "WARP_SKILL_DIRS: skipping '{}' — not a directory or does not exist",
                dir.display()
            );
            false
        })
        .flat_map(|dir| read_skills(dir))
        .map(|mut skill| {
            skill.scope = SkillScope::Home;
            skill
        })
        .collect()
}

/// Read all skills from a directory containing skill subdirectories
///
/// # Arguments
/// * `path` - The path to a skills directory, e.g. `.claude/skills`
///
/// # Returns
/// * `Vec<ParsedSkill>` - List of successfully parsed skills (invalid files and errors are
///   silently ignored). Stops accepting further skills once their combined content would exceed
///   `MAX_SKILLS_BATCH_BYTES`, logging a warning about the skipped remainder.
pub fn read_skills(path: &Path) -> Vec<ParsedSkill> {
    let mut skills = Vec::new();
    let mut batch_bytes: u64 = 0;

    // Read all entries in the directory, return empty vec on error
    let Ok(entries) = fs::read_dir(path) else {
        return skills;
    };

    // `fs::read_dir` order is filesystem-dependent, so sort candidate skill files by path first.
    // Otherwise, which skills survive the batch quota below would vary between scans or
    // filesystems rather than being a stable, predictable selection.
    let mut skill_file_paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry_path| entry_path.is_dir())
        .map(|entry_path| entry_path.join("SKILL.md"))
        .filter(|skill_file_path| skill_file_path.exists())
        .collect();
    skill_file_paths.sort();

    for skill_file_path in skill_file_paths {
        // Attempt to parse the skill file, ignoring errors
        if let Ok(parsed_skill) = parse_skill(&skill_file_path) {
            let content_bytes = parsed_skill.content.len() as u64;
            if batch_bytes.saturating_add(content_bytes) > MAX_SKILLS_BATCH_BYTES {
                log::warn!(
                    "Reached skills read batch limit ({MAX_SKILLS_BATCH_BYTES} bytes) in {}; \
                     skipping {} and remaining files",
                    path.display(),
                    skill_file_path.display()
                );
                break;
            }
            batch_bytes = batch_bytes.saturating_add(content_bytes);
            skills.push(parsed_skill);
        }
    }

    skills
}

#[cfg(test)]
#[path = "read_skills_tests.rs"]
mod read_skills_test;
