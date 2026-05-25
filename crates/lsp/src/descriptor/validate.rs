//! Validation rules for parsed `[[editor.language_servers]]` entries.
//!
//! Invalid entries are dropped (not auto-edited in the user's settings file);
//! each error becomes one line in the user-visible settings-error surface.

use std::fmt;

/// A single validation error against a single descriptor entry. `entry_name`
/// is `None` for entries that fail before a `name` could be parsed (e.g. an
/// entry missing the `name` field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspDescriptorError {
    pub entry_name: Option<String>,
    pub kind: LspDescriptorErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspDescriptorErrorKind {
    /// Two entries share the same `name`. The offending name is in the
    /// enclosing `LspDescriptorError::entry_name`.
    DuplicateName,
    /// Entry's `filetypes` array is empty.
    EmptyFiletypes,
    /// Entry is missing the required `name` field.
    MissingName,
    /// Entry is missing the required `command` field.
    MissingCommand,
    /// The entry's overall shape did not match the expected schema (e.g. it
    /// is not a table at all, or a field has the wrong type). `reason` is
    /// the underlying deserialize error.
    MalformedEntry { reason: String },
    /// A glob pattern in `filetypes` failed to compile. `pattern` is the
    /// offending source; `reason` is the underlying compile error.
    InvalidGlob { pattern: String, reason: String },
    /// A glob pattern uses a feature outside the supported syntax — either
    /// `**` (path-spanning) or `{a,b}` brace alternation. v1 only supports
    /// basename-only matching with `*`, `?`, and character classes.
    UnsupportedGlobFeature {
        pattern: String,
        feature: &'static str,
    },
}

impl fmt::Display for LspDescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let entry_label = match &self.entry_name {
            Some(name) => format!("entry `{name}`"),
            None => "entry without `name`".to_string(),
        };
        match &self.kind {
            LspDescriptorErrorKind::DuplicateName => {
                write!(f, "{entry_label}: duplicate `name`")
            }
            LspDescriptorErrorKind::EmptyFiletypes => {
                write!(f, "{entry_label}: `filetypes` must be a non-empty array")
            }
            LspDescriptorErrorKind::MissingName => write!(f, "{entry_label}: missing required `name`"),
            LspDescriptorErrorKind::MissingCommand => {
                write!(f, "{entry_label}: missing required `command`")
            }
            LspDescriptorErrorKind::MalformedEntry { reason } => {
                write!(f, "{entry_label}: malformed entry: {reason}")
            }
            LspDescriptorErrorKind::InvalidGlob { pattern, reason } => write!(
                f,
                "{entry_label}: invalid glob `{pattern}`: {reason}",
            ),
            LspDescriptorErrorKind::UnsupportedGlobFeature { pattern, feature } => write!(
                f,
                "{entry_label}: glob `{pattern}` uses unsupported feature `{feature}` (not allowed in v1)",
            ),
        }
    }
}

impl std::error::Error for LspDescriptorError {}

/// Returns `Some(UnsupportedGlobFeature)` if the glob pattern uses a feature
/// outside the v1 supported subset (`**` or `{a,b}` brace alternation).
/// Returns `None` for the empty pattern; callers should check non-emptiness
/// separately.
pub fn check_supported_glob_features(pattern: &str) -> Option<LspDescriptorErrorKind> {
    if pattern.contains("**") {
        return Some(LspDescriptorErrorKind::UnsupportedGlobFeature {
            pattern: pattern.to_string(),
            feature: "**",
        });
    }
    // `{a,b}` brace alternation: present any time we see a `{` followed by
    // `,` before the matching `}`. We don't run a full parser here — the
    // detection is a heuristic that rejects globset's superset feature.
    if let Some(open) = pattern.find('{') {
        if let Some(close_offset) = pattern[open..].find('}') {
            let inner = &pattern[open + 1..open + close_offset];
            if inner.contains(',') {
                return Some(LspDescriptorErrorKind::UnsupportedGlobFeature {
                    pattern: pattern.to_string(),
                    feature: "{a,b}",
                });
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
