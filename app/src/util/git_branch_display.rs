use regex::Regex;
use serde::{Deserialize, Serialize};

use super::truncation::{truncate_from_beginning, truncate_from_end};

/// Maximum length of a user-supplied branch display pattern.
const MAX_GIT_BRANCH_NAME_PATTERN_LEN: usize = 256;

/// Which end of a long git branch name the prompt chip keeps.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Which end of a long git branch name the prompt chip keeps.",
    rename_all = "snake_case"
)]
pub enum GitBranchNameTruncation {
    /// Keep the end of the name. The start is replaced with an ellipsis.
    #[default]
    #[schemars(description = "Keep the end of a long branch name.")]
    End,
    /// Keep the start of the name, such as a ticket id.
    #[schemars(description = "Keep the start of a long branch name.")]
    Start,
}

impl GitBranchNameTruncation {
    pub fn as_dropdown_label(self) -> &'static str {
        match self {
            Self::End => "End of name",
            Self::Start => "Start of name",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::End => Self::Start,
            Self::Start => Self::End,
        }
    }
}

/// Whether `pattern` can be saved as a git branch display pattern.
///
/// An empty pattern is valid and means the full branch name is shown.
pub fn git_branch_name_pattern_is_valid(pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.len() > MAX_GIT_BRANCH_NAME_PATTERN_LEN {
        return false;
    }
    pattern.is_empty() || Regex::new(pattern).is_ok()
}

/// Formats a git branch name for the prompt chip.
///
/// A non-empty `pattern` replaces the name with the first regex capture group,
/// or the whole match when the pattern has no groups. An empty, invalid, or
/// non-matching pattern leaves the branch name unchanged. The result is then
/// shortened to `max_chars`.
pub fn format_git_branch_name(
    branch: &str,
    truncation: GitBranchNameTruncation,
    pattern: &str,
    max_chars: usize,
) -> String {
    let displayed = matched_branch_text(branch, pattern).unwrap_or_else(|| branch.to_string());
    match truncation {
        GitBranchNameTruncation::End => truncate_from_beginning(&displayed, max_chars),
        GitBranchNameTruncation::Start => truncate_from_end(&displayed, max_chars),
    }
}

/// The branch text selected by `pattern`, when the pattern matches.
fn matched_branch_text(branch: &str, pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.len() > MAX_GIT_BRANCH_NAME_PATTERN_LEN {
        return None;
    }
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(branch)?;
    let text = if re.captures_len() > 1 {
        caps.get(1)?.as_str()
    } else {
        caps.get(0)?.as_str()
    };
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
#[path = "git_branch_display_tests.rs"]
mod tests;
