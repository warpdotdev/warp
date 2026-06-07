//! Validation rules for parsed `[[editor.language_servers]]` entries.
//!
//! Invalid entries are dropped (not auto-edited in the user's settings file);
//! each error becomes one line in the user-visible settings-error surface.

use std::fmt;

use crate::supported_servers::LSPServerType;

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
    /// `name` violates the character/length/format constraints (1–64 chars
    /// from `[A-Za-z0-9._-]`, not `.`/`..`, no leading `.`/`-`). `reason`
    /// names the specific rule that failed.
    InvalidName { reason: &'static str },
    /// `name` collides, case-insensitively, with a built-in server's binary
    /// display name (the string the footer's "Enable {name}" button shows).
    ReservedName,
    /// `command`, after leading-`~` expansion, is neither an absolute path nor
    /// a bare name (no `/` or `\`), so it would resolve against the spawned
    /// process's cwd.
    UnsafeCommandPath {
        command: String,
        reason: &'static str,
    },
}

impl fmt::Display for LspDescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redaction-safe: never write the raw user-typed value (the offending
        // pattern, command, or serde message) — it may contain a secret. The
        // entry `name` is included only when fully valid, so it too cannot
        // carry one. Raw values remain in the struct fields for `Debug`.
        let entry = match &self.entry_name {
            Some(name) if check_name(name).is_none() => name.as_str(),
            _ => "anonymous",
        };
        write!(f, "entry `{entry}`: ")?;
        match &self.kind {
            LspDescriptorErrorKind::DuplicateName => write!(f, "duplicate `name`"),
            LspDescriptorErrorKind::EmptyFiletypes => write!(f, "`filetypes` must be non-empty"),
            LspDescriptorErrorKind::MissingName => write!(f, "missing required `name`"),
            LspDescriptorErrorKind::MissingCommand => write!(f, "missing required `command`"),
            LspDescriptorErrorKind::MalformedEntry { .. } => write!(f, "malformed entry"),
            LspDescriptorErrorKind::InvalidGlob { .. } => write!(f, "invalid glob in `filetypes`"),
            LspDescriptorErrorKind::UnsupportedGlobFeature { feature, .. } => {
                write!(f, "`filetypes` glob uses unsupported feature `{feature}`")
            }
            LspDescriptorErrorKind::InvalidName { reason } => write!(f, "invalid `name`: {reason}"),
            LspDescriptorErrorKind::ReservedName => {
                write!(f, "`name` is reserved for a built-in language server")
            }
            LspDescriptorErrorKind::UnsafeCommandPath { reason, .. } => {
                write!(f, "unsafe `command`: {reason}")
            }
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

/// Maximum length of a descriptor `name`, in characters.
const MAX_NAME_LEN: usize = 64;

/// Returns `Some(InvalidName)` if `name` violates the constraints: 1–64
/// characters from `[A-Za-z0-9._-]`, not `.` or `..`, and not starting with
/// `.` or `-`. Returns `None` for an acceptable name.
pub fn check_name(name: &str) -> Option<LspDescriptorErrorKind> {
    if name.is_empty() {
        return Some(LspDescriptorErrorKind::InvalidName {
            reason: "must not be empty",
        });
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Some(LspDescriptorErrorKind::InvalidName {
            reason: "must be at most 64 characters",
        });
    }
    if name == "." || name == ".." {
        return Some(LspDescriptorErrorKind::InvalidName {
            reason: "must not be `.` or `..`",
        });
    }
    if name.starts_with('.') || name.starts_with('-') {
        return Some(LspDescriptorErrorKind::InvalidName {
            reason: "must not start with `.` or `-`",
        });
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Some(LspDescriptorErrorKind::InvalidName {
            reason: "must contain only ASCII letters, digits, `.`, `_`, or `-`",
        });
    }
    None
}

/// Returns `true` if `name` collides, case-insensitively, with a built-in
/// server's binary display name. Sourced from `LSPServerType::binary_name()`
/// so adding a built-in extends the reservation automatically.
pub fn is_reserved_name(name: &str) -> bool {
    LSPServerType::all().any(|server| name.eq_ignore_ascii_case(server.binary_name()))
}

/// Returns `Some(UnsafeCommandPath)` if `command` is none of: home-rooted
/// (`~` / `~/...`, which expands to the absolute home directory at launch), an
/// absolute path, or a bare name (no `/` or `\`). Relative paths with
/// separators would resolve against the spawned process's cwd (the workspace
/// root), so they are rejected at settings load. Only the literal form is
/// checked; the post-substitution command is not revalidated.
pub fn check_command(command: &str) -> Option<LspDescriptorErrorKind> {
    if is_home_rooted(command) || is_absolute_command(command) || is_bare_name(command) {
        None
    } else {
        Some(LspDescriptorErrorKind::UnsafeCommandPath {
            command: command.to_string(),
            reason: "must be an absolute path or a bare command name (no path separators)",
        })
    }
}

/// A leading `~` or `~/` expands to the (always absolute) home directory at
/// launch, so it satisfies the absolute-path requirement without resolving
/// the home directory here. Other-user forms (`~someuser`) are not recognized.
fn is_home_rooted(command: &str) -> bool {
    command == "~" || command.starts_with("~/")
}

/// A bare command name has no path separators and is resolved against `PATH`
/// by the OS process loader. `\` is treated as a separator on every platform
/// so that Windows relative forms are rejected even when validated on Unix.
fn is_bare_name(command: &str) -> bool {
    !command.contains('/') && !command.contains('\\')
}

/// Platform-specific absolute-path check, matching the OS whose loader will
/// run the command: on Unix a leading `/`; on Windows a drive-letter root
/// (`C:\`/`C:/`) or a UNC path. A leading `/`/`\` without a drive is
/// deliberately *not* absolute on Windows — it is current-drive-relative.
fn is_absolute_command(command: &str) -> bool {
    if cfg!(windows) {
        is_windows_absolute(command)
    } else {
        command.starts_with('/')
    }
}

/// Windows absolute forms: a UNC path (`\\server\share` / `//server/share`)
/// or a drive-letter root (`C:\` / `C:/`).
fn is_windows_absolute(command: &str) -> bool {
    if command.starts_with("\\\\") || command.starts_with("//") {
        return true;
    }
    let bytes = command.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
