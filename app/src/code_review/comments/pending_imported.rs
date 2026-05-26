use std::path::PathBuf;

use chrono::{DateTime, Local};

use super::comment::{ImportedCommentDetails, LineDiffContent};
use crate::code::editor::line::EditorLineLocation;

/// Pending imported GitHub review comment.
///
/// This represents imported GitHub PR data after parsing timestamps and diff hunks, but before it
/// is:
/// * flattened (threads/replies combined),
/// * converted from repo-relative paths to absolute paths,
/// * attached/relocated against a local editor.
///
/// Invariants:
/// * `target` file paths are repo-relative.
/// * `body` is the raw GitHub comment body.
/// * no notion of `outdated`, since there is no local diff/editor state to compare to yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingImportedReviewComment {
    /// GitHub metadata such as remote comment IDs.
    pub(crate) github_details: ImportedCommentDetails,
    /// The raw comment body as provided by GitHub (no additional formatting).
    pub(crate) body: String,
    /// The most recent update time for this individual comment.
    pub(crate) last_update_time: DateTime<Local>,
    /// Where this comment was originally attached in the GitHub UI.
    pub(crate) target: PendingImportedReviewCommentTarget,
}

impl PendingImportedReviewComment {
    pub(crate) fn author(&self) -> &str {
        &self.github_details.author
    }

    pub(crate) fn github_comment_id(&self) -> &str {
        &self.github_details.github_comment_id
    }

    pub(crate) fn github_parent_comment_id(&self) -> Option<&str> {
        self.github_details.github_parent_id.as_deref()
    }

    /// Returns a copy of the GitHub metadata with any parent reference cleared.
    ///
    /// Used when collapsing a threaded comment into a single flattened comment.
    pub(crate) fn github_details_without_parent(&self) -> ImportedCommentDetails {
        let mut details = self.github_details.clone();
        details.github_parent_id = None;
        details
    }
}

/// Where a pending imported review comment applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingImportedReviewCommentTarget {
    /// A comment attached to a specific diff line in a file.
    Line {
        /// Repo-relative path to the file.
        relative_file_path: PathBuf,
        /// A line location derived from the provider's diff hunk.
        line: EditorLineLocation,
        /// The diff line content at the target location.
        diff_content: LineDiffContent,
    },
    /// A comment attached to a file, but not a specific line.
    File {
        /// Repo-relative path to the file.
        relative_file_path: PathBuf,
    },
    /// A comment that applies to the entire diffset / PR.
    General,
}

impl PendingImportedReviewCommentTarget {
    pub(crate) fn file_path(&self) -> Option<&PathBuf> {
        match self {
            PendingImportedReviewCommentTarget::Line {
                relative_file_path, ..
            } => Some(relative_file_path),
            PendingImportedReviewCommentTarget::File { relative_file_path } => {
                Some(relative_file_path)
            }
            PendingImportedReviewCommentTarget::General => None,
        }
    }
}
