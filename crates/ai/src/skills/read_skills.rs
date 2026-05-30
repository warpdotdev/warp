use std::fs;
use std::path::Path;

use super::parse_skill::{parse_skill, ParsedSkill};

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
