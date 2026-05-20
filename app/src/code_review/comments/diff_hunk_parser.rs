//! Utilities for parsing unified diff hunks and extracting specific line content.

use ai::agent::action::CommentSide;
use num_traits::SaturatingSub;
use warp_editor::render::model::LineCount;

use crate::{
    code::editor::line::EditorLineLocation,
    code_review::{comments::LineDiffContent, diff_state::DiffLineType},
    util::git::parse_unified_diff_header,
};

#[derive(Debug)]
pub(crate) enum DiffHunkParseError {
    EmptyHunk,
    InvalidHeader(anyhow::Error),
    UnexpectedHunkHeader { line_index: usize },
    LineNotFound { target_line: usize },
}

impl std::fmt::Display for DiffHunkParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffHunkParseError::EmptyHunk => write!(f, "Empty diff hunk"),
            DiffHunkParseError::InvalidHeader(err) => write!(f, "Invalid header: {err}"),
            DiffHunkParseError::UnexpectedHunkHeader { line_index } => {
                write!(f, "Unexpected hunk header at line index {line_index}")
            }
            DiffHunkParseError::LineNotFound { target_line } => {
                write!(f, "Target line {target_line} not found in hunk")
            }
        }
    }
}

impl From<anyhow::Error> for DiffHunkParseError {
    fn from(err: anyhow::Error) -> Self {
        DiffHunkParseError::InvalidHeader(err)
    }
}

/// Build the result tuple from a diff line's type and content.
fn build_line_result(
    line: &str,
    line_type: &DiffLineType,
    line_index_in_hunk: usize,
    new_file_line: LineCount,
) -> Result<(EditorLineLocation, LineDiffContent), DiffHunkParseError> {
    // EditorLineLocation expects 0-based line numbers, but diff hunks are 1-based.
    let line_num = new_file_line.saturating_sub(&LineCount::from(1));

    let editor_line_location = match line_type {
        DiffLineType::Context | DiffLineType::Add => EditorLineLocation::Current {
            line_number: line_num,
            line_range: line_num..line_num,
        },
        DiffLineType::Delete => EditorLineLocation::Removed {
            line_number: line_num,
            line_range: line_num..line_num,
            index: 0,
        },
        DiffLineType::HunkHeader => {
            return Err(DiffHunkParseError::UnexpectedHunkHeader {
                line_index: line_index_in_hunk,
            });
        }
    };

    let lines_added = usize::from(matches!(line_type, DiffLineType::Add));
    let lines_removed = usize::from(matches!(line_type, DiffLineType::Delete));

    let line_diff_content = LineDiffContent {
        content: line.to_string(),
        lines_added: LineCount::from(lines_added),
        lines_removed: LineCount::from(lines_removed),
    };

    Ok((editor_line_location, line_diff_content))
}

fn get_diff_line_from_diff_hunk(
    diff_hunk: &str,
    target_line_number: usize,
    side: CommentSide,
) -> Result<(EditorLineLocation, LineDiffContent), DiffHunkParseError> {
    let parsed_lines: Vec<&str> = diff_hunk.lines().collect();
    let diff_hunk_header = parsed_lines
        .first()
        .ok_or(DiffHunkParseError::EmptyHunk)
        .and_then(|line| parse_unified_diff_header(line).map_err(Into::into))?;

    let mut index_in_file = match side {
        CommentSide::Left => diff_hunk_header.old_start_line,
        CommentSide::Right => diff_hunk_header.new_start_line,
    };

    for (index_in_hunk, line) in parsed_lines.iter().enumerate().skip(1) {
        let line_type = match line.chars().next().unwrap_or_default() {
            '@' => {
                return Err(DiffHunkParseError::UnexpectedHunkHeader {
                    line_index: index_in_hunk,
                });
            }
            '+' => DiffLineType::Add,
            '-' => DiffLineType::Delete,
            _ => DiffLineType::Context,
        };

        match side {
            CommentSide::Left => {
                if matches!(line_type, DiffLineType::Delete | DiffLineType::Context) {
                    if index_in_file == target_line_number {
                        return build_line_result(
                            line,
                            &line_type,
                            index_in_hunk,
                            LineCount::from(index_in_file),
                        );
                    }
                    index_in_file += 1;
                }
            }
            CommentSide::Right => {
                if matches!(line_type, DiffLineType::Add | DiffLineType::Context) {
                    if index_in_file == target_line_number {
                        return build_line_result(
                            line,
                            &line_type,
                            index_in_hunk,
                            LineCount::from(index_in_file),
                        );
                    }
                    index_in_file += 1;
                }
            }
        }
    }

    Err(DiffHunkParseError::LineNotFound {
        target_line: target_line_number,
    })
}

/// Given a diff hunk and a target start and end line within it, return the EditorLineLocation and LineDiffContent for the target.
///
/// # Arguments
/// * `diff_hunk` - A unified diff hunk string starting with `@@ -old_file_hunk_start,old_hunk_line_count +new_file_hunk_start,new_hunk_line_count @@`
/// * `target_new_file_line` - The 1-indexed line number in the new file that the comment was attached to.
/// * `side` - Optionally specify which side of the diff the target line is in. If None, tries Right first, then Left.
///
/// # Returns
/// * the `EditorLineLocation` representing the comment, where the line_range represents the location in the new file
/// * the `LineDiffContent` from the parsed diff hunk.
pub(crate) fn parse_diff_hunk(
    diff_hunk: &str,
    target_line_number: usize,
    side: Option<CommentSide>,
) -> Result<(EditorLineLocation, LineDiffContent), DiffHunkParseError> {
    match side {
        Some(side) => get_diff_line_from_diff_hunk(diff_hunk, target_line_number, side),
        None => {
            // Try Right first (new file), then fall back to Left (old file)
            let mut diff_line_result =
                get_diff_line_from_diff_hunk(diff_hunk, target_line_number, CommentSide::Right);
            if matches!(
                diff_line_result,
                Err(DiffHunkParseError::LineNotFound { .. })
            ) {
                diff_line_result =
                    get_diff_line_from_diff_hunk(diff_hunk, target_line_number, CommentSide::Left);
            }
            diff_line_result
        }
    }
}

#[cfg(test)]
#[path = "diff_hunk_parser_tests.rs"]
mod tests;
