use super::{ReviewChangesTarget, review_changes_target};

const PR_URL: &str = "https://github.com/warpdotdev/warp/pull/1234";

#[test]
fn dirty_tree_opens_local_code_review() {
    assert_eq!(
        review_changes_target(Some(true), None),
        ReviewChangesTarget::LocalCodeReview
    );
}

#[test]
fn dirty_tree_opens_local_code_review_even_when_a_pr_exists() {
    assert_eq!(
        review_changes_target(Some(true), Some(PR_URL)),
        ReviewChangesTarget::LocalCodeReview
    );
}

/// Regression for APP-5148: after the agent commits and pushes its accepted
/// edits the working tree is clean, so the control must follow the changes to
/// the branch's pull request instead of opening an empty local diff.
#[test]
fn clean_tree_with_a_pr_opens_the_remote_pull_request() {
    assert_eq!(
        review_changes_target(Some(false), Some(PR_URL)),
        ReviewChangesTarget::RemotePullRequest {
            url: PR_URL.to_owned()
        }
    );
}

#[test]
fn clean_tree_without_a_pr_reviews_nothing() {
    assert_eq!(
        review_changes_target(Some(false), None),
        ReviewChangesTarget::NothingToReview
    );
}

#[test]
fn unknown_dirty_state_opens_local_code_review() {
    assert_eq!(
        review_changes_target(None, None),
        ReviewChangesTarget::LocalCodeReview
    );
    assert_eq!(
        review_changes_target(None, Some(PR_URL)),
        ReviewChangesTarget::LocalCodeReview
    );
}
