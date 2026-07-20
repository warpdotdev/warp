use std::path::PathBuf;

use ai::agent::action::InsertReviewComment;
use chrono::{DateTime, Local};

use super::comment::ImportedCommentDetails;
use super::{PendingImportedReviewComment, PendingImportedReviewCommentTarget};
use crate::code_review::comments::diff_hunk_parser::parse_diff_hunk;

#[derive(Debug)]
pub enum ConversionError {
    InvalidTimestamp(String),
    InvalidFilePath(PathBuf),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversionError::InvalidTimestamp(ts) => {
                write!(f, "Invalid timestamp: {}", ts)
            }
            ConversionError::InvalidFilePath(path) => {
                write!(
                    f,
                    "Pending imported review comment targets must use repo-relative paths, invalid path: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ConversionError {}

pub(crate) fn convert_insert_review_comments(
    comments: &[InsertReviewComment],
) -> Vec<PendingImportedReviewComment> {
    comments
        .iter()
        .cloned()
        .filter_map(
            |comment| match PendingImportedReviewComment::try_from(comment) {
                Ok(comment) => Some(comment),
                Err(e) => {
                    log::warn!("Failed to convert InsertReviewComment: {e}");
                    None
                }
            },
        )
        .collect()
}

impl TryFrom<InsertReviewComment> for PendingImportedReviewComment {
    type Error = ConversionError;

    fn try_from(comment: InsertReviewComment) -> Result<Self, Self::Error> {
        // Parse timestamp - try RFC3339 format
        let last_update_time: DateTime<Local> =
            DateTime::parse_from_rfc3339(&comment.last_modified_timestamp)
                .map(|dt| dt.with_timezone(&Local))
                .map_err(|_| {
                    ConversionError::InvalidTimestamp(comment.last_modified_timestamp.clone())
                })?;

        let target = match comment.comment_location {
            None => PendingImportedReviewCommentTarget::General,
            Some(location) => match location.line {
                None => PendingImportedReviewCommentTarget::File {
                    relative_file_path: PathBuf::from(location.relative_file_path),
                },
                Some(line) => {
                    // Use the start of the range as the target line for attaching the comment.
                    match parse_diff_hunk(
                        &line.diff_hunk_text,
                        line.comment_line_range.start,
                        line.side,
                    ) {
                        Ok((line_location, diff_content)) => {
                            PendingImportedReviewCommentTarget::Line {
                                relative_file_path: PathBuf::from(location.relative_file_path),
                                line: line_location,
                                diff_content,
                            }
                        }
                        Err(err) => {
                            log::warn!(
                                "Error parsing comment at line {} from unified diff hunk: {}",
                                line.comment_line_range.start,
                                err
                            );
                            PendingImportedReviewCommentTarget::File {
                                relative_file_path: PathBuf::from(location.relative_file_path),
                            }
                        }
                    }
                }
            },
        };

        if let Some(file_path) = target.file_path()
            && file_path.is_absolute()
        {
            return Err(ConversionError::InvalidFilePath(file_path.to_owned()));
        }

        let github_details = ImportedCommentDetails {
            author: comment.author,
            github_comment_id: comment.comment_id,
            github_parent_id: comment.parent_comment_id,
            html_url: comment.html_url.clone(),
        };

        Ok(Self {
            github_details,
            body: comment.comment_body,
            last_update_time,
            target,
        })
    }
}
