use serde::{Deserialize, Serialize};

use super::truncation::{truncate_from_beginning, truncate_from_end};

/// Largest leading or trailing character count a user can save.
///
/// The prompt chip itself is capped separately. Counts above that cap are reduced while formatting
/// so the ellipsis still fits.
pub const GIT_BRANCH_NAME_EDGE_CHARS_LIMIT: usize = 40;

/// Characters kept from the start when the middle of a branch name is hidden.
pub const DEFAULT_GIT_BRANCH_NAME_PREFIX_CHARS: usize = 12;

/// Characters kept from the end when the middle of a branch name is hidden.
pub const DEFAULT_GIT_BRANCH_NAME_SUFFIX_CHARS: usize = 12;

/// Which part of a long git branch name the prompt chip replaces with an ellipsis.
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
    description = "Which part of a long git branch name the prompt chip replaces with an ellipsis.",
    rename_all = "snake_case"
)]
pub enum GitBranchNameTruncation {
    /// Hide the start of the name. This is the original prompt behavior.
    #[default]
    #[schemars(description = "Hide the start of a long branch name.")]
    OmitStart,
    /// Hide the end of the name so a prefix, such as a ticket id, stays visible.
    #[schemars(description = "Hide the end of a long branch name.")]
    OmitEnd,
    /// Hide the middle and keep a fixed number of characters from each end.
    #[schemars(description = "Hide the middle of a long branch name.")]
    OmitMiddle,
}

impl GitBranchNameTruncation {
    pub fn as_dropdown_label(self) -> &'static str {
        match self {
            Self::OmitStart => "Omit the start",
            Self::OmitEnd => "Omit the end",
            Self::OmitMiddle => "Omit the middle",
        }
    }

    /// Switches between hiding the start and hiding the end.
    ///
    /// Middle omission is not part of that pair. Toggling it shows the start of the name instead.
    pub fn toggled(self) -> Self {
        match self {
            Self::OmitStart | Self::OmitMiddle => Self::OmitEnd,
            Self::OmitEnd => Self::OmitStart,
        }
    }
}

/// Formats a git branch name for the prompt chip.
///
/// `prefix_chars` and `suffix_chars` apply only when `truncation` hides the middle. A shortened
/// name always keeps an ellipsis where text was removed.
pub fn format_git_branch_name(
    branch: &str,
    truncation: GitBranchNameTruncation,
    prefix_chars: usize,
    suffix_chars: usize,
    max_chars: usize,
) -> String {
    match truncation {
        GitBranchNameTruncation::OmitStart => truncate_from_beginning(branch, max_chars),
        GitBranchNameTruncation::OmitEnd => truncate_from_end(branch, max_chars),
        GitBranchNameTruncation::OmitMiddle => {
            truncate_middle(branch, prefix_chars, suffix_chars, max_chars)
        }
    }
}

/// Hides the middle of `text`.
///
/// Keeps `prefix_chars` from the start and `suffix_chars` from the end. A name no longer than those
/// two edges is shown in full. When the edges plus the ellipsis would pass `max_chars`, the suffix
/// shrinks before the prefix so a ticket id at the start survives.
fn truncate_middle(
    text: &str,
    prefix_chars: usize,
    suffix_chars: usize,
    max_chars: usize,
) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len <= max_chars && len <= prefix_chars.saturating_add(suffix_chars) {
        return text.to_string();
    }

    let mut prefix = prefix_chars.min(len);
    let mut suffix = suffix_chars.min(len);
    let edge_budget = max_chars.saturating_sub(1);
    if prefix + suffix > edge_budget {
        let overflow = prefix + suffix - edge_budget;
        let suffix_reduction = overflow.min(suffix);
        suffix -= suffix_reduction;
        prefix = prefix.saturating_sub(overflow - suffix_reduction);
    }
    if prefix + suffix >= len {
        return text.to_string();
    }
    if prefix == 0 && suffix == 0 {
        return "…".to_string();
    }
    let start: String = chars[..prefix].iter().collect();
    let end: String = chars[len - suffix..].iter().collect();
    format!("{start}…{end}")
}

#[cfg(test)]
#[path = "git_branch_display_tests.rs"]
mod tests;
