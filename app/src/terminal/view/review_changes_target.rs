//! Routing for the agent block footer's "Review changes" control.
//!
//! The control historically always opened Warp's local Code Review pane against
//! the working tree. Once the agent's accepted edits are committed and pushed
//! the tree is clean, so that pane has nothing to show — the destination has to
//! follow the changes to the branch's pull request instead.

/// Where the agent block footer's "Review changes" control should take the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewChangesTarget {
    /// Open Warp's local Code Review pane against the working tree.
    LocalCodeReview,
    /// Open the pull request for the current branch.
    RemotePullRequest { url: String },
    /// The working tree is clean and no pull request could be resolved, so
    /// there is no diff to show; the caller explains that instead of opening an
    /// empty pane.
    NothingToReview,
}

/// Picks the destination for the agent footer's "Review changes" control.
///
/// `working_tree_is_dirty` is `None` while the repo's git status metadata is
/// still loading. The local pane is the right destination then, because it
/// defers its own open until that metadata arrives and can still route on it.
pub(crate) fn review_changes_target(
    working_tree_is_dirty: Option<bool>,
    pr_url: Option<&str>,
) -> ReviewChangesTarget {
    match (working_tree_is_dirty, pr_url) {
        (Some(false), Some(url)) => ReviewChangesTarget::RemotePullRequest {
            url: url.to_owned(),
        },
        (Some(false), None) => ReviewChangesTarget::NothingToReview,
        (Some(true), _) | (None, _) => ReviewChangesTarget::LocalCodeReview,
    }
}

#[cfg(test)]
#[path = "review_changes_target_tests.rs"]
mod tests;
