use std::collections::HashMap;

use warp_editor::render::model::LineCount;
use warpui::{Entity, ModelContext};

use super::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentId, PendingImportedReviewComment,
};
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::EditorReviewComment;
use crate::code_review::diff_state::DiffMode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewCommentBatchEvent {
    Changed { should_reposition_comments: bool },
}

#[derive(Clone, Debug, Default)]
pub struct ReviewCommentBatch {
    /// Comments that are attached to local editors and visible to the user.
    pub comments: Vec<AttachedReviewComment>,
    /// Imported comments waiting for editors and diffs to load before they can be displayed to the user.
    /// Comments are grouped by base branch.
    pending_imported_comments: HashMap<DiffMode, Vec<PendingImportedReviewComment>>,
}

impl Entity for ReviewCommentBatch {
    type Event = ReviewCommentBatchEvent;
}

impl ReviewCommentBatch {
    pub fn from_comments(comments: Vec<AttachedReviewComment>) -> Self {
        Self {
            comments,
            pending_imported_comments: HashMap::new(),
        }
    }

    pub(crate) fn get_review_comment_by_id(&self, id: CommentId) -> Option<&AttachedReviewComment> {
        self.comments.iter().find(|comment| comment.id == id)
    }

    pub(super) fn get_mut_review_comment_by_id(
        &mut self,
        id: CommentId,
    ) -> Option<&mut AttachedReviewComment> {
        self.comments.iter_mut().find(|comment| comment.id == id)
    }

    pub(crate) fn diffset_comment(&self) -> Option<&AttachedReviewComment> {
        self.comments
            .iter()
            .find(|comment| matches!(comment.target, AttachedReviewCommentTarget::General))
    }

    pub(crate) fn has_only_outdated_comments(&self) -> bool {
        self.comments.iter().all(|comment| comment.outdated)
    }

    /// `file` should be the host-aware absolute path for the editor file.
    pub fn file_comments<'a>(
        &'a self,
        file: &'a LocalOrRemotePath,
    ) -> impl Iterator<Item = &'a AttachedReviewComment> + 'a {
        self.comments.iter().filter(move |comment| {
            comment
                .target
                .absolute_file_path()
                .is_some_and(|comment_file| comment_file == file)
        })
    }

    /// `file` should be the host-aware absolute path for the editor file.
    pub fn comment_line_numbers_for_file<'a>(
        &'a self,
        file: &'a LocalOrRemotePath,
    ) -> impl Iterator<Item = LineCount> + 'a {
        self.file_comments(file).filter_map(move |comment| {
            if let AttachedReviewCommentTarget::Line {
                absolute_file_path: comment_file_path,
                line,
                ..
            } = &comment.target
            {
                if comment_file_path == file {
                    line.line_number()
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    pub(crate) fn editor_comments_for_file(
        &self,
        file: &LocalOrRemotePath,
    ) -> Vec<EditorReviewComment> {
        self.file_comments(file)
            .filter(|comment| !comment.outdated)
            .filter_map(|comment| EditorReviewComment::try_from(comment.clone()).ok())
            .collect()
    }

    pub(crate) fn upsert_comment(
        &mut self,
        comment: AttachedReviewComment,
        ctx: &mut ModelContext<Self>,
    ) {
        self.upsert_comments_inner(vec![comment]);
        ctx.emit(ReviewCommentBatchEvent::Changed {
            should_reposition_comments: false,
        });
    }

    #[cfg(feature = "local_fs")]
    pub(crate) fn upsert_imported_comments(
        &mut self,
        comments: Vec<AttachedReviewComment>,
        ctx: &mut ModelContext<Self>,
    ) {
        if comments.is_empty() {
            return;
        }
        self.upsert_comments_inner(comments);
        ctx.emit(ReviewCommentBatchEvent::Changed {
            should_reposition_comments: true,
        });
    }

    /// Comments with existing IDs are updated.
    /// New comments are inserted into the batch.
    pub fn upsert_comments(
        &mut self,
        comments: Vec<AttachedReviewComment>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.upsert_comments_inner(comments);
        ctx.emit(ReviewCommentBatchEvent::Changed {
            should_reposition_comments: false,
        });
    }

    fn upsert_comments_inner(&mut self, comments: Vec<AttachedReviewComment>) {
        let (existing_comments, new_comments): (
            Vec<AttachedReviewComment>,
            Vec<AttachedReviewComment>,
        ) = comments
            .into_iter()
            .partition(|c| self.get_review_comment_by_id(c.id).is_some());

        self.comments.extend(new_comments);
        for c in existing_comments {
            if let Some(existing_entry) = self.get_mut_review_comment_by_id(c.id) {
                *existing_entry = c;
            }
        }
    }

    pub(crate) fn take_comments(&mut self) -> Vec<AttachedReviewComment> {
        std::mem::take(&mut self.comments)
    }

    /// Deleting a comment does NOT remove the associated diff hunk from the batch's
    /// diff set because that hunk may be referenced by another comment.
    /// In the future, we may investigate a cleaner way to do this.
    pub(crate) fn delete_comment(&mut self, id: CommentId, ctx: &mut ModelContext<Self>) {
        self.comments.retain(|comment| comment.id != id);
        ctx.emit(ReviewCommentBatchEvent::Changed {
            should_reposition_comments: false,
        });
    }

    pub(crate) fn clear_all(&mut self, ctx: &mut ModelContext<Self>) {
        self.comments.clear();
        ctx.emit(ReviewCommentBatchEvent::Changed {
            should_reposition_comments: false,
        });
    }

    /// Stores imported comments that are waiting for diffs and editors to load before they can be flattened,
    /// relocated, and inserted into `comments`.
    #[cfg(feature = "local_fs")]
    pub(crate) fn add_pending_imported_comments(
        &mut self,
        comments: Vec<PendingImportedReviewComment>,
        base_branch: DiffMode,
        ctx: &mut ModelContext<Self>,
    ) {
        self.pending_imported_comments
            .entry(base_branch)
            .or_default()
            .extend(comments);
        ctx.emit(ReviewCommentBatchEvent::Changed {
            should_reposition_comments: true,
        });
    }

    /// Takes all pending imported comments for the given diff mode, leaving the pending list empty.
    /// Used when diffs have loaded and comments can be relocated.
    pub(crate) fn take_pending_imported_comments_for_branch(
        &mut self,
        branch: &DiffMode,
    ) -> Vec<PendingImportedReviewComment> {
        if let Some(pending_comments) = self.pending_imported_comments.get_mut(branch) {
            std::mem::take(pending_comments)
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
