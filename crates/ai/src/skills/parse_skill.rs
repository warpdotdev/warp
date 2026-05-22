use std::fmt::Display;
use std::fs;
use std::ops::Range;
use std::path::Path;

use super::parser::parse_markdown_content;
use super::skill_provider::{get_provider_for_path, get_scope_for_path, SkillProvider, SkillScope};
use anyhow::Result;
use lazy_static::lazy_static;
use regex::Regex;
use thiserror::Error;
use warp_util::local_or_remote_path::LocalOrRemotePath;

const MAX_SKILL_DESCRIPTION_CHARS: usize = 512;

lazy_static! {
    static ref BLOCK_SEPARATOR: Regex =
        Regex::new(r"\n\s*\n").expect("Block separator regex should be valid");
    static ref INCOMPLETE_SENTENCE: Regex =
        Regex::new(r"[^.!?]*$").expect("Incomplete sentence regex should be valid");
}
/// Parse skill markdown content that was fetched outside the local filesystem.
///
/// This is used for remote project skills, whose SKILL.md body arrives through
/// the remote file-read transport rather than `std::fs`.
pub fn parse_skill_content_at_location(
    path: LocalOrRemotePath,
    content: &str,
    provider: SkillProvider,
    scope: SkillScope,
) -> Result<ParsedSkill> {
    let parsed = parse_markdown_content(content)?;
    let name = match parsed
        .front_matter
        .get("name")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        Some(name) => name.to_string(),
        None => derive_skill_name_from_path(&path)?,
    };

    let description = match parsed
        .front_matter
        .get("description")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        Some(description) => description.to_string(),
        None => truncate_skill_description(
            &derive_description_from_content(&parsed.content, parsed.line_range.as_ref())
                .unwrap_or_default(),
        ),
    };

    Ok(ParsedSkill {
        path,
        name,
        description,
        content: parsed.content,
        line_range: parsed.line_range,
        provider,
        scope,
    })
}

#[derive(Error, Debug)]
pub enum ParseSkillError {
    /// This should never happen in practice since we would never read the skill
    /// file to begin with if the path didn't have a valid parent directory.
    #[error("Could not derive skill name from path")]
    CouldNotDeriveSkillNameFromPath,
}

/// Represents a parsed skill with validated fields
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSkill {
    pub path: LocalOrRemotePath,
    pub name: String,
    pub description: String,
    /// The entire content of the file (including front matter)
    pub content: String,
    /// The line range where the markdown content (without front matter) is located (1-indexed)
    /// None if there is no front matter (content is the entire file)
    pub line_range: Option<Range<usize>>,
    /// The provider of the skill (Agents, Claude, Codex, or Warp), determined from the path.
    pub provider: SkillProvider,
    /// The scope of the skill (home directory vs project directory).
    pub scope: SkillScope,
}

impl ParsedSkill {
    /// Returns true if this skill is bundled with Warp (not a user-editable file).
    pub fn is_bundled(&self) -> bool {
        self.scope == SkillScope::Bundled
    }
}

impl Display for ParsedSkill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Skill: {}", self.path.display_path())
    }
}

/// Parse a skill markdown file and validate required fields
///
/// # Arguments
/// * `path` - Path to the skill markdown file to parse
///
/// # Returns
/// * `Result<ParsedSkill>` - Parsed skill with validated name and description
pub fn parse_skill(path: &Path) -> Result<ParsedSkill> {
    let provider = get_provider_for_path(path).unwrap_or(SkillProvider::Agents);
    let scope = get_scope_for_path(path);
    parse_skill_internal(
        LocalOrRemotePath::Local(path.to_path_buf()),
        provider,
        scope,
    )
}

/// Parse a bundled skill markdown file.
///
/// Unlike `parse_skill`, this function does not require the path to match a known
/// skill provider directory. Bundled skills are always assigned `SkillProvider::Warp`
/// and `SkillScope::Bundled`.
///
/// # Arguments
/// * `path` - Path to the skill markdown file to parse
///
/// # Returns
/// * `Result<ParsedSkill>` - Parsed skill with validated name and description
pub fn parse_bundled_skill(path: &Path) -> Result<ParsedSkill> {
    parse_skill_internal(
        LocalOrRemotePath::Local(path.to_path_buf()),
        SkillProvider::Warp,
        SkillScope::Bundled,
    )
}

fn parse_skill_internal(
    path: LocalOrRemotePath,
    provider: SkillProvider,
    scope: SkillScope,
) -> Result<ParsedSkill> {
    let local_path = path
        .to_local_path()
        .expect("parse_skill_internal only reads local files");
    let content = fs::read_to_string(local_path)?;
    parse_skill_content_at_location(path, &content, provider, scope)
}

fn derive_skill_name_from_path(path: &LocalOrRemotePath) -> Result<String> {
    path.parent()
        .and_then(|parent| parent.file_name().map(str::to_owned))
        .ok_or(ParseSkillError::CouldNotDeriveSkillNameFromPath.into())
}

fn derive_description_from_content(
    content: &str,
    line_range: Option<&Range<usize>>,
) -> Option<String> {
    first_paragraph_from_markdown(&extract_markdown_body(content, line_range))
}

fn extract_markdown_body(content: &str, line_range: Option<&Range<usize>>) -> String {
    let Some(line_range) = line_range else {
        return content.to_string();
    };

    let start = line_range.start.saturating_sub(1);
    let end = line_range.end.saturating_sub(1);
    let lines: Vec<&str> = content.lines().collect();
    if start >= lines.len() {
        return String::new();
    }

    let end = end.min(lines.len());
    lines[start..end].join("\n")
}

fn first_paragraph_from_markdown(markdown: &str) -> Option<String> {
    for block in BLOCK_SEPARATOR.split(markdown) {
        let paragraph: String = block
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join(" ");
        let paragraph = paragraph.trim();
        if !paragraph.is_empty() {
            return Some(paragraph.to_string());
        }
    }
    None
}

fn truncate_skill_description(description: &str) -> String {
    let description = description.trim();
    if description.is_empty() {
        return String::new();
    }

    let chars: Vec<char> = description.chars().collect();
    if chars.len() <= MAX_SKILL_DESCRIPTION_CHARS {
        return description.to_string();
    }

    let truncated: String = chars[..MAX_SKILL_DESCRIPTION_CHARS].iter().collect();

    // Drop the trailing incomplete sentence using regex
    let at_sentence = INCOMPLETE_SENTENCE
        .replace(&truncated, "")
        .trim()
        .to_string();
    if !at_sentence.is_empty() {
        return at_sentence;
    }

    // No sentence boundary found — fall back to word boundary
    truncated
        .rfind(char::is_whitespace)
        .map(|pos| truncated[..pos].trim().to_string())
        .unwrap_or(truncated)
}

#[cfg(test)]
#[path = "parse_skill_tests.rs"]
mod parse_skill_test;
