use std::collections::{HashMap, HashSet};

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

    // Build a set of all GitHub comment IDs for orphan detection.
    let existing_ids: HashSet<&str> = pending_comments
        .iter()
        .map(|c| c.github_comment_id())
        .collect();

    let mut roots: HashMap<&str, &PendingImportedReviewComment> = HashMap::new();
    let mut parent_to_children: HashMap<&str, Vec<&PendingImportedReviewComment>> = HashMap::new();

    for comment in pending_comments.iter() {
        match comment.github_parent_comment_id() {
            Some(parent_id) if existing_ids.contains(parent_id) => {
                parent_to_children
                    .entry(parent_id)
                    .or_default()
                    .push(comment);
            }
            Some(missing_parent_id) => {
                // Orphaned comment - parent doesn't exist, treat as root.
                log::warn!(
                    "Importing orphaned comment (ID {:?}) with parent ID {:?}",
                    comment.github_comment_id(),
                    missing_parent_id
                );
                roots.insert(comment.github_comment_id(), comment);
            }
            None => {
                // Already a root comment.
                roots.insert(comment.github_comment_id(), comment);
            }
        };
    }

    let mut root_comments: Vec<_> = roots.values().copied().collect();
    root_comments.sort_by_key(|c| c.github_comment_id());

    root_comments
        .into_iter()
        .map(|root| flatten_pending_imported_thread(root, &parent_to_children, repo_path))
        .collect()
}

fn flatten_pending_imported_thread(
    root: &PendingImportedReviewComment,
    children_map: &HashMap<&str, Vec<&PendingImportedReviewComment>>,
    repo_path: &LocalOrRemotePath,
) -> AttachedReviewComment {
    const THREAD_REPLY_DIVIDER: &str = "\n---\n";

    let mut thread_comments = Vec::new();
    collect_pending_imported_thread_dfs(root, children_map, &mut thread_comments);

    let last_update_time = thread_comments
        .iter()
        .map(|c| c.last_update_time)
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

    let mut combined_body = String::new();
    for (i, comment) in thread_comments.iter().enumerate() {
        if i > 0 {
            combined_body.push_str(THREAD_REPLY_DIVIDER);
        }

        combined_body.push_str(&format!(
            "**@{}**:\n{}",
            comment.author(),
            comment.body.as_str()
        ));
    }

    let origin = CommentOrigin::ImportedFromGitHub(Box::new(root.github_details_without_parent()));

    AttachedReviewComment {
        id: CommentId::new(),
        content: combined_body,
        target,
        last_update_time,
        base: None,
        head: None,
        outdated: false,
        origin,
    }
}

fn collect_pending_imported_thread_dfs<'a>(
    comment: &'a PendingImportedReviewComment,
    children_map: &HashMap<&str, Vec<&'a PendingImportedReviewComment>>,
    result: &mut Vec<&'a PendingImportedReviewComment>,
) {
    result.push(comment);

    // Get children of this comment.
    if let Some(children) = children_map.get(comment.github_comment_id()) {
        let mut sorted_children = children.to_vec();
        sorted_children.sort_by(|a, b| a.last_update_time.cmp(&b.last_update_time));
        for child in sorted_children {
            collect_pending_imported_thread_dfs(child, children_map, result);
        }
    }
}
