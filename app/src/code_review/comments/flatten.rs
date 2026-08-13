use ai::agent::action::{
    ReviewCommentThread, format_review_comment_thread, group_review_comment_threads,
};

use super::comment::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentId, CommentOrigin,
};
use super::pending_imported::{PendingImportedReviewComment, PendingImportedReviewCommentTarget};
use crate::code::buffer_location::LocalOrRemotePath;

/// Converts pending imported provider comments into attached review comments by:
/// * flattening threaded replies
/// * formatting markdown bodies
/// * converting repo-relative file paths to absolute file paths
pub(crate) fn attach_pending_imported_comments(
    pending_comments: Vec<PendingImportedReviewComment>,
    repo_path: &LocalOrRemotePath,
) -> Vec<AttachedReviewComment> {
    if pending_comments.is_empty() {
        return Vec::new();
    }

    group_review_comment_threads(&pending_comments)
        .into_iter()
        .map(|thread| {
            if let Some(missing_parent_id) = thread.missing_parent_id() {
                log::warn!(
                    "Importing orphaned comment (ID {:?}) with parent ID {:?}",
                    thread.root().github_comment_id(),
                    missing_parent_id
                );
            }
            attach_pending_imported_thread(thread, repo_path)
        })
        .collect()
}

fn attach_pending_imported_thread(
    thread: ReviewCommentThread<'_, PendingImportedReviewComment>,
    repo_path: &LocalOrRemotePath,
) -> AttachedReviewComment {
    let root = thread.root();
    let last_update_time = thread
        .comments()
        .iter()
        .map(|comment| comment.last_update_time)
        .max()
        .unwrap_or(root.last_update_time);

    let target = match &root.target {
        PendingImportedReviewCommentTarget::Line {
            relative_file_path,
            line,
            diff_content,
        } => AttachedReviewCommentTarget::Line {
            absolute_file_path: repo_path.join(&relative_file_path.to_string_lossy()),
            line: line.clone(),
            content: diff_content.clone(),
        },
        PendingImportedReviewCommentTarget::File { relative_file_path } => {
            AttachedReviewCommentTarget::File {
                absolute_file_path: repo_path.join(&relative_file_path.to_string_lossy()),
            }
        }
        PendingImportedReviewCommentTarget::General => AttachedReviewCommentTarget::General,
    };

    let origin = CommentOrigin::ImportedFromGitHub(root.github_details_without_parent());

    AttachedReviewComment {
        id: CommentId::new(),
        content: format_review_comment_thread(&thread),
        target,
        last_update_time,
        base: None,
        head: None,
        outdated: false,
        origin,
    }
}
