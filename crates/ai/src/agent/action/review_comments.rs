use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::DateTime;

use super::InsertReviewComment;

const THREAD_REPLY_DIVIDER: &str = "\n---\n";

/// Presentation-neutral data needed to reconstruct provider review comment threads.
pub trait ReviewCommentThreadItem {
    fn comment_id(&self) -> &str;
    fn parent_comment_id(&self) -> Option<&str>;
    fn author(&self) -> &str;
    fn body(&self) -> &str;
    fn compare_last_modified(&self, other: &Self) -> Ordering;
}

/// One root review comment and its replies in display order.
#[derive(Debug, PartialEq, Eq)]
pub struct ReviewCommentThread<'a, T> {
    comments: Vec<&'a T>,
    missing_parent_id: Option<&'a str>,
}

impl<'a, T> ReviewCommentThread<'a, T> {
    pub fn root(&self) -> &'a T {
        self.comments[0]
    }

    pub fn comments(&self) -> &[&'a T] {
        &self.comments
    }

    pub fn missing_parent_id(&self) -> Option<&'a str> {
        self.missing_parent_id
    }
}

/// Groups review comments into deterministic, depth-first provider threads.
///
/// Roots are ordered by comment ID. Replies are stably ordered by their
/// last-modified value. A comment whose parent is absent is retained as a root
/// and exposes that missing parent ID.
pub fn group_review_comment_threads<T: ReviewCommentThreadItem>(
    comments: &[T],
) -> Vec<ReviewCommentThread<'_, T>> {
    if comments.is_empty() {
        return Vec::new();
    }

    let existing_ids: HashSet<&str> = comments.iter().map(T::comment_id).collect();
    let mut roots: HashMap<&str, (&T, Option<&str>)> = HashMap::new();
    let mut parent_to_children: HashMap<&str, Vec<&T>> = HashMap::new();

    for comment in comments {
        match comment.parent_comment_id() {
            Some(parent_id) if existing_ids.contains(parent_id) => {
                parent_to_children
                    .entry(parent_id)
                    .or_default()
                    .push(comment);
            }
            missing_parent_id => {
                roots.insert(comment.comment_id(), (comment, missing_parent_id));
            }
        }
    }

    let mut roots: Vec<_> = roots.into_values().collect();
    roots.sort_by(|(a, _), (b, _)| a.comment_id().cmp(b.comment_id()));
    roots
        .into_iter()
        .map(|(root, missing_parent_id)| {
            let mut thread_comments = Vec::new();
            collect_review_comment_thread(root, &parent_to_children, &mut thread_comments);
            ReviewCommentThread {
                comments: thread_comments,
                missing_parent_id,
            }
        })
        .collect()
}

/// Formats a provider thread using the Markdown representation shared by clients.
pub fn format_review_comment_thread<T: ReviewCommentThreadItem>(
    thread: &ReviewCommentThread<'_, T>,
) -> String {
    thread
        .comments()
        .iter()
        .map(|comment| format!("**@{}**:\n{}", comment.author(), comment.body()))
        .collect::<Vec<_>>()
        .join(THREAD_REPLY_DIVIDER)
}

fn collect_review_comment_thread<'a, T: ReviewCommentThreadItem>(
    comment: &'a T,
    children_map: &HashMap<&str, Vec<&'a T>>,
    result: &mut Vec<&'a T>,
) {
    result.push(comment);

    if let Some(children) = children_map.get(comment.comment_id()) {
        let mut sorted_children = children.to_vec();
        sorted_children.sort_by(|a, b| a.compare_last_modified(b));
        for child in sorted_children {
            collect_review_comment_thread(child, children_map, result);
        }
    }
}

impl ReviewCommentThreadItem for InsertReviewComment {
    fn comment_id(&self) -> &str {
        &self.comment_id
    }

    fn parent_comment_id(&self) -> Option<&str> {
        self.parent_comment_id.as_deref()
    }

    fn author(&self) -> &str {
        &self.author
    }

    fn body(&self) -> &str {
        &self.comment_body
    }

    fn compare_last_modified(&self, other: &Self) -> Ordering {
        match (
            DateTime::parse_from_rfc3339(&self.last_modified_timestamp),
            DateTime::parse_from_rfc3339(&other.last_modified_timestamp),
        ) {
            (Ok(this), Ok(other)) => this.cmp(&other),
            _ => self
                .last_modified_timestamp
                .cmp(&other.last_modified_timestamp),
        }
    }
}

#[cfg(test)]
#[path = "review_comments_tests.rs"]
mod tests;
