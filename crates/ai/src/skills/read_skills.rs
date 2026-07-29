use std::fs;
use std::path::{Path, PathBuf};

use super::parse_skill::{ParsedSkill, parse_skill};
use super::skill_provider::SkillScope;

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
/// layout used by `SKILLS_DIRS`. Entries that are not directories are skipped
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
                "SKILLS_DIRS: skipping '{}' — not a directory or does not exist",
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
