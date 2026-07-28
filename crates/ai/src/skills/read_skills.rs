use std::fs;
use std::path::{Path, PathBuf};

use super::parse_skill::{ParsedSkill, parse_skill, parse_skill_content_at_location};
use super::skill_provider::{SkillProvider, SkillScope, get_provider_for_path};
use warp_util::local_or_remote_path::LocalOrRemotePath;

/// The environment variable that specifies extra skill directories to index at
/// personal (home) precedence. Value is a comma-separated list of paths; each
/// path is itself a skills directory whose **direct children** are skill folders
/// containing `SKILL.md`.
pub const SKILLS_DIRS_ENV: &str = "SKILLS_DIRS";

/// Parse the `SKILLS_DIRS` environment variable into a list of directory paths.
///
/// Splits on commas, trims leading/trailing whitespace from each entry, and
/// drops blank entries. Returns an empty vec when the variable is unset or empty.
pub fn parse_skills_dirs_env() -> Vec<PathBuf> {
    let Ok(val) = std::env::var(SKILLS_DIRS_ENV) else {
        return Vec::new();
    };
    val.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Read skills from a slice of directories, treating each as a personal (home)
/// tier skills root.
///
/// Each directory in `dirs` is expected to contain individual skill folders as
/// **direct children** (e.g. `<dir>/<skill-name>/SKILL.md`). This matches the
/// layout used by `SKILLS_DIRS`. Entries that are not directories or cannot be
/// read are skipped with a warning.
///
/// Skills loaded this way are assigned `SkillScope::Home` so they are always
/// in scope — the same precedence as personal skills from `~/.agents/skills`.
pub fn read_skills_for_skills_dirs(dirs: &[PathBuf]) -> Vec<ParsedSkill> {
    let mut skills = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            log::warn!(
                "SKILLS_DIRS: skipping '{}' — not a directory or does not exist",
                dir.display()
            );
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            log::warn!("SKILLS_DIRS: cannot read directory '{}'", dir.display());
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let entry_path = entry.path();
            if !entry_path.is_dir() {
                continue;
            }
            let skill_file_path = entry_path.join("SKILL.md");
            if !skill_file_path.exists() {
                continue;
            }
            let Ok(content) = fs::read_to_string(&skill_file_path) else {
                log::warn!(
                    "SKILLS_DIRS: cannot read skill file '{}'",
                    skill_file_path.display()
                );
                continue;
            };
            let location = LocalOrRemotePath::Local(skill_file_path);
            let provider = get_provider_for_path(&location).unwrap_or(SkillProvider::Agents);
            match parse_skill_content_at_location(location, &content, provider, SkillScope::Home) {
                Ok(skill) => skills.push(skill),
                Err(err) => {
                    log::warn!(
                        "SKILLS_DIRS: failed to parse skill in '{}': {err}",
                        entry_path.display()
                    );
                }
            }
        }
    }
    skills
}

/// Read all skills from a directory containing skill subdirectories
///
/// # Arguments
/// * `path` - The path to a skills directory, e.g. `.claude/skills`
///
/// # Returns
/// * `Vec<ParsedSkill>` - List of successfully parsed skills (invalid files and errors are silently ignored)
pub fn read_skills(path: &Path) -> Vec<ParsedSkill> {
    let mut skills = Vec::new();

    // Read all entries in the directory, return empty vec on error
    let Ok(entries) = fs::read_dir(path) else {
        return skills;
    };

    for entry in entries {
        // Skip entries that fail to read
        let Ok(entry) = entry else {
            continue;
        };

        let entry_path = entry.path();

        // Only process directories
        if !entry_path.is_dir() {
            continue;
        }

        // Look for SKILL.md file in the subdirectory
        let skill_file_path = entry_path.join("SKILL.md");

        if skill_file_path.exists() {
            // Attempt to parse the skill file, ignoring errors
            if let Ok(parsed_skill) = parse_skill(&skill_file_path) {
                skills.push(parsed_skill);
            }
        }
    }

    skills
}

#[cfg(test)]
#[path = "read_skills_tests.rs"]
mod read_skills_test;
