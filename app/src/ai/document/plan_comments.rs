//! Data model for code-review-style comments attached to a planning document.
//!
//! Unlike code review comments (which target diff lines/files), plan comments target a range of
//! text within a single rich-text plan document, anchored via the editor's [`Anchor`] system so
//! the range follows edits. A comment can also be `General` (applies to the whole plan).

use std::fmt::{Display, Formatter};
use std::ops::Range;

use chrono::{DateTime, Local};
use string_offset::CharOffset;
use warp_editor::content::anchor::Anchor;
use warp_editor::content::selection_model::BufferSelectionModel;
use warpui::{Entity, ModelContext};

/// Locally-generated identifier for a plan comment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlanCommentId(uuid::Uuid);

impl PlanCommentId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for PlanCommentId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for PlanCommentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a plan comment is attached within the plan document.
#[derive(Clone, Debug)]
pub enum PlanCommentTarget {
    /// Anchored to a range of text in the plan document. The `head`/`tail` anchors are created
    /// from the plan editor's [`BufferSelectionModel`] and are kept in sync as the document is
    /// edited. `quoted_text` is a snapshot of the selected text at creation time, used as the card
    /// title and for context when submitting to the agent.
    DocumentRange {
        head: Anchor,
        tail: Anchor,
        quoted_text: String,
    },
    /// Applies to the whole plan document.
    General,
}

/// A single comment attached to a planning document.
#[derive(Clone, Debug)]
pub struct PlanComment {
    pub id: PlanCommentId,
    /// The user-authored comment body (markdown).
    pub body: String,
    pub target: PlanCommentTarget,
    pub last_update_time: DateTime<Local>,
    /// True if the anchored range no longer resolves (its text was deleted). Outdated comments are
    /// still shown but are excluded when submitting to the agent.
    pub outdated: bool,
}

impl PlanComment {
    /// Creates a new comment anchored to a document range.
    pub fn new_range(body: String, head: Anchor, tail: Anchor, quoted_text: String) -> Self {
        Self {
            id: PlanCommentId::new(),
            body,
            target: PlanCommentTarget::DocumentRange {
                head,
                tail,
                quoted_text,
            },
            last_update_time: Local::now(),
            outdated: false,
        }
    }

    /// Creates a new general (document-level) comment.
    pub fn new_general(body: String) -> Self {
        Self {
            id: PlanCommentId::new(),
            body,
            target: PlanCommentTarget::General,
            last_update_time: Local::now(),
            outdated: false,
        }
    }

    /// The quoted text snapshot for range comments, if any.
    pub fn quoted_text(&self) -> Option<&str> {
        match &self.target {
            PlanCommentTarget::DocumentRange { quoted_text, .. } => Some(quoted_text.as_str()),
            PlanCommentTarget::General => None,
        }
    }

    /// Resolves the comment's current character range using the plan editor's selection model.
    ///
    /// Returns `None` for general comments or when either anchor no longer resolves (e.g. the
    /// commented text was deleted).
    pub fn resolve_range(
        &self,
        selection_model: &BufferSelectionModel,
    ) -> Option<Range<CharOffset>> {
        match &self.target {
            PlanCommentTarget::DocumentRange { head, tail, .. } => {
                let head = selection_model.resolve_anchor(head)?;
                let tail = selection_model.resolve_anchor(tail)?;
                Some(if head <= tail { head..tail } else { tail..head })
            }
            PlanCommentTarget::General => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanCommentBatchEvent {
    Changed,
}

/// A collection of comments attached to a single planning document.
#[derive(Default)]
pub struct PlanCommentBatch {
    comments: Vec<PlanComment>,
}

impl Entity for PlanCommentBatch {
    type Event = PlanCommentBatchEvent;
}

impl PlanCommentBatch {
    pub fn comments(&self) -> &[PlanComment] {
        &self.comments
    }

    pub fn get(&self, id: PlanCommentId) -> Option<&PlanComment> {
        self.comments.iter().find(|comment| comment.id == id)
    }

    /// Inserts a new comment or updates an existing one (matched by ID) in place.
    pub fn upsert_comment(&mut self, comment: PlanComment, ctx: &mut ModelContext<Self>) {
        if let Some(existing) = self.comments.iter_mut().find(|c| c.id == comment.id) {
            *existing = comment;
        } else {
            self.comments.push(comment);
        }
        ctx.emit(PlanCommentBatchEvent::Changed);
    }

    pub fn delete_comment(&mut self, id: PlanCommentId, ctx: &mut ModelContext<Self>) {
        self.comments.retain(|comment| comment.id != id);
        ctx.emit(PlanCommentBatchEvent::Changed);
    }

    pub fn clear_all(&mut self, ctx: &mut ModelContext<Self>) {
        self.comments.clear();
        ctx.emit(PlanCommentBatchEvent::Changed);
    }

    /// Updates the `outdated` flag for the comment with the given ID. Returns `true` if the flag
    /// changed.
    pub fn set_outdated(&mut self, id: PlanCommentId, outdated: bool) -> bool {
        if let Some(comment) = self.comments.iter_mut().find(|c| c.id == id) {
            if comment.outdated != outdated {
                comment.outdated = outdated;
                return true;
            }
        }
        false
    }
}

/// Builds the user-facing query text sent to the agent when submitting plan comments.
///
/// Centralizing the formatting here keeps the submission call site simple and makes it easy to
/// swap in a dedicated, server-formatted `PlanReview` input type in the future (which would
/// require proto/server changes). Outdated comments are skipped.
pub fn build_plan_comment_prompt(comments: &[PlanComment]) -> String {
    let mut text = String::from(
        "Please revise the plan to address the following comments. Update the planning document accordingly.\n",
    );

    for comment in comments.iter().filter(|comment| !comment.outdated) {
        let body = comment.body.trim();
        match comment.quoted_text() {
            Some(quoted) if !quoted.trim().is_empty() => {
                // Collapse the quoted snippet to a single line for a concise reference.
                let snippet = quoted.split('\n').next().unwrap_or("").trim();
                text.push_str(&format!("\n- On \"{snippet}\": {body}"));
            }
            _ => {
                text.push_str(&format!("\n- {body}"));
            }
        }
    }

    text
}

#[cfg(test)]
#[path = "plan_comments_tests.rs"]
mod tests;
