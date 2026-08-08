//! Serializable, structured mirror of the proto `ApplyFileDiffsResult.Failure`
//! variants, produced by the client after applying (or failing to apply) diffs.
//!
//! Lives in `crates/ai` because the API-conversion code lives here and
//! `DiffApplicationError` (in the `app` crate) cannot cross the layering
//! boundary.

use std::ops::Range;

use itertools::Itertools as _;
use serde::{Deserialize, Serialize};

/// Maximum number of bytes of search-block text that is included in a single
/// [`DiffSearchBlockFailure`] when populating the proto. Text beyond this
/// limit is truncated and [`DiffSearchBlockFailure::truncated`] is set to
/// `true`.
pub const MAX_DIFF_MATCH_FAILURE_BYTES: usize = 1024;

// ---------------------------------------------------------------------------
// Per-search-block failure detail
// ---------------------------------------------------------------------------

/// Detail about one search block that failed to match.
///
/// Carries the raw (pre-truncation) search text so that the proto-conversion
/// layer can apply the byte cap and set `truncated` accordingly.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffSearchBlockFailure {
    /// The search text of the failing block (from the raw diff, not truncated).
    /// Sensitive — never include in logs or telemetry in cleartext.
    pub search: String,
    /// Expected 1-indexed, exclusive-end line range the block was thought to
    /// occupy, if line numbers were present in the diff.
    pub expected_range: Option<Range<usize>>,
}

/// Redacted `Debug` impl — never prints `search` in cleartext.
impl std::fmt::Debug for DiffSearchBlockFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffSearchBlockFailure")
            .field("search", &"<redacted>")
            .field("expected_range", &self.expected_range)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// DiffApplicationFailure
// ---------------------------------------------------------------------------

/// Structured, serializable failure produced when `edit_files` cannot apply
/// one or more diffs to the user's filesystem.
///
/// Mirrors the proto `ApplyFileDiffsResult.Failure` variants so it can be
/// converted losslessly in both directions.
///
/// Sensitive fields (file paths, search-block text) are **never** printed
/// through the auto-derived `Debug` path — this type has a manual impl.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiffApplicationFailure {
    /// Some search blocks could not be located in the file. Optionally, some
    /// were already matching (noop). These are combined here because the
    /// `DiffApplicationError::UnmatchedDiffs` source carries both counters.
    UnmatchedDiffs {
        /// Sensitive: file path.
        file: String,
        /// Number of search blocks that could not be matched at all.
        fuzzy_match_failure_count: u8,
        /// Number of search blocks whose replace text was identical to the
        /// matched content (changes already applied / no-op).
        changes_already_applied_count: u8,
        /// Per-block details for the fuzzy-match failures.
        search_block_failures: Vec<DiffSearchBlockFailure>,
    },
    /// The diff was a no-op for this file (changes already applied).
    ChangesAlreadyApplied {
        /// Sensitive: file path.
        file: String,
    },
    /// The file to edit was not found.
    MissingFile {
        /// Sensitive: file path.
        file: String,
    },
    /// The file could not be read (I/O error, permissions, remote
    /// connectivity, etc.).
    ReadFailed {
        /// Sensitive: file path.
        file: String,
    },
    /// A file the diff tried to create already exists.
    AlreadyExists {
        /// Sensitive: file path.
        file: String,
    },
    /// The diff contained multiple attempts to create the same file.
    MultipleFileCreation {
        /// Sensitive: file path.
        file: String,
    },
    /// The diff contained multiple attempts to rename the same file.
    MultipleFileRenames {
        /// Sensitive: file path.
        file: String,
    },
    /// A diff attempted to modify a file that was simultaneously deleted.
    MutatedDeletedFile {
        /// Sensitive: file path.
        file: String,
    },
    /// No diffs were applicable (all were filtered before application).
    NoDiffsApplicable,
    /// File read/write is not available on this remote session type.
    RemoteFileOperationsUnsupported,
    /// Pre-rendered opaque message (save errors, uncategorized failures).
    /// The server surfaces this message verbatim.
    Opaque {
        /// Sensitive: may contain user-facing I/O error strings or paths.
        message: String,
    },
}

/// Redacted `Debug` impl — never prints sensitive fields (file paths, search
/// text, opaque messages) in cleartext.
impl std::fmt::Debug for DiffApplicationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffApplicationFailure::UnmatchedDiffs {
                fuzzy_match_failure_count,
                changes_already_applied_count,
                search_block_failures,
                ..
            } => f
                .debug_struct("UnmatchedDiffs")
                .field("file", &"<redacted>")
                .field("fuzzy_match_failure_count", fuzzy_match_failure_count)
                .field(
                    "changes_already_applied_count",
                    changes_already_applied_count,
                )
                .field("search_block_failures", search_block_failures)
                .finish(),
            DiffApplicationFailure::ChangesAlreadyApplied { .. } => f
                .debug_struct("ChangesAlreadyApplied")
                .field("file", &"<redacted>")
                .finish(),
            DiffApplicationFailure::MissingFile { .. } => f
                .debug_struct("MissingFile")
                .field("file", &"<redacted>")
                .finish(),
            DiffApplicationFailure::ReadFailed { .. } => f
                .debug_struct("ReadFailed")
                .field("file", &"<redacted>")
                .finish(),
            DiffApplicationFailure::AlreadyExists { .. } => f
                .debug_struct("AlreadyExists")
                .field("file", &"<redacted>")
                .finish(),
            DiffApplicationFailure::MultipleFileCreation { .. } => f
                .debug_struct("MultipleFileCreation")
                .field("file", &"<redacted>")
                .finish(),
            DiffApplicationFailure::MultipleFileRenames { .. } => f
                .debug_struct("MultipleFileRenames")
                .field("file", &"<redacted>")
                .finish(),
            DiffApplicationFailure::MutatedDeletedFile { .. } => f
                .debug_struct("MutatedDeletedFile")
                .field("file", &"<redacted>")
                .finish(),
            DiffApplicationFailure::NoDiffsApplicable => {
                write!(f, "NoDiffsApplicable")
            }
            DiffApplicationFailure::RemoteFileOperationsUnsupported => {
                write!(f, "RemoteFileOperationsUnsupported")
            }
            DiffApplicationFailure::Opaque { .. } => f
                .debug_struct("Opaque")
                .field("message", &"<redacted>")
                .finish(),
        }
    }
}

// ---------------------------------------------------------------------------
// render()
// ---------------------------------------------------------------------------

/// Render a list of structured failures as the agent-facing error string.
///
/// Reproduces the exact wording previously produced by
/// `DiffApplicationError::error_for_conversation`. This is the single source
/// of truth for the local `Display` impl, the markdown render in the GUI, the
/// SDK output, the TUI view, and the `message` back-compat field in the proto.
///
/// # Format
/// - 0 failures: `""` (callers should treat this as unexpected)
/// - 1 failure: just the message, no prefix
/// - ≥2 failures: each prefixed with `"* "`, joined by `"\n"`
pub fn render(failures: &[DiffApplicationFailure]) -> String {
    let messages: Vec<String> = failures.iter().map(render_one).collect();
    if messages.len() == 1 {
        messages.into_iter().next().unwrap_or_default()
    } else {
        messages
            .iter()
            .format_with("\n", |msg, f| f(&format_args!("* {msg}")))
            .to_string()
    }
}

fn render_one(failure: &DiffApplicationFailure) -> String {
    match failure {
        DiffApplicationFailure::UnmatchedDiffs {
            file,
            fuzzy_match_failure_count,
            changes_already_applied_count,
            ..
        } => {
            use std::fmt::Write as _;
            let mut message = String::new();
            if *fuzzy_match_failure_count > 0 {
                let _ = write!(message, "Could not apply all diffs to {file}.");
            }
            if *changes_already_applied_count > 0 {
                if !message.is_empty() {
                    message.push(' ');
                }
                let _ = write!(message, "The changes to {file} were already made.");
            }
            // Defensive fallback: both counts are zero, e.g. after a round-trip where
            // fuzzy_match_failure_count saturated to u8::MAX and was read back as 0, or
            // if the caller constructs a zero-count variant. PRODUCT invariant 9 requires
            // the agent always receives a non-empty message.
            if message.is_empty() {
                let _ = write!(message, "Could not apply all diffs to {file}.");
            }
            message
        }
        DiffApplicationFailure::ChangesAlreadyApplied { file } => {
            format!("The changes to {file} were already made.")
        }
        DiffApplicationFailure::MissingFile { file } => {
            format!("{file} does not exist. Is the path correct?")
        }
        DiffApplicationFailure::ReadFailed { file } => {
            format!("Could not read {file}")
        }
        DiffApplicationFailure::AlreadyExists { file } => {
            format!("Could not create {file} because it already exists.")
        }
        DiffApplicationFailure::MultipleFileCreation { file } => {
            format!("There can only be one attempt to create {file}.")
        }
        DiffApplicationFailure::MultipleFileRenames { file } => {
            format!("There can only be one attempt to rename {file}.")
        }
        DiffApplicationFailure::MutatedDeletedFile { file } => {
            format!("Could not mutate a deleted file {file}.")
        }
        DiffApplicationFailure::NoDiffsApplicable => "No diffs could be applied.".to_string(),
        DiffApplicationFailure::RemoteFileOperationsUnsupported => {
            "The file read/edit tool is not available on this remote session. Try using a different tool.".to_string()
        }
        DiffApplicationFailure::Opaque { message } => message.clone(),
    }
}

#[cfg(test)]
#[path = "diff_application_failure_tests.rs"]
mod tests;
