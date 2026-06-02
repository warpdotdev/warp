//! Unit tests for the pure context-menu builders in [`super`].

use super::*;
use crate::menu::MenuItem;

use crate::workspace::view::git_graph::data::CommitNode;

fn commit() -> CommitNode {
    CommitNode {
        hash: "abcdef1234567890".into(),
        short_hash: "abcdef1".into(),
        parents: vec!["0".into()],
        author_name: "Ada".into(),
        author_email: "ada@x".into(),
        author_time: 0,
        subject: "Do a thing".into(),
        refs: vec![],
    }
}

/// Labels of the menu items, with separators shown as `"--"`.
fn labels(items: &[MenuItem<GitGraphAction>]) -> Vec<&str> {
    items
        .iter()
        .map(|i| match i {
            MenuItem::Item(f) => f.label(),
            MenuItem::Separator => "--",
            _ => "?",
        })
        .collect()
}

#[test]
fn commit_menu_full_matches_screenshot_order() {
    let items = build_commit_menu(&commit(), true);
    assert_eq!(
        labels(&items),
        vec![
            "Add Tag…",
            "Create Branch…",
            "--",
            "Checkout…",
            "Cherry Pick…",
            "Revert…",
            "Drop…",
            "--",
            "Merge into current branch…",
            "Rebase current branch on this Commit…",
            "Reset current branch to this Commit…",
            "--",
            "Copy Commit Hash to Clipboard",
            "Copy Commit Subject to Clipboard",
        ]
    );
}

#[test]
fn commit_menu_read_only_shows_only_copy() {
    let items = build_commit_menu(&commit(), false);
    assert_eq!(
        labels(&items),
        vec![
            "Copy Commit Hash to Clipboard",
            "Copy Commit Subject to Clipboard",
        ]
    );
}

#[test]
fn commit_copy_actions_carry_hash_and_subject() {
    let items = build_commit_menu(&commit(), true);
    let copy_actions: Vec<&GitGraphAction> = items
        .iter()
        .filter_map(|i| i.item_on_select_action())
        .filter(|a| matches!(a, GitGraphAction::CopyToClipboard(_)))
        .collect();
    assert_eq!(
        copy_actions,
        vec![
            &GitGraphAction::CopyToClipboard("abcdef1234567890".into()),
            &GitGraphAction::CopyToClipboard("Do a thing".into()),
        ]
    );
}

#[test]
fn tag_menu_full_and_read_only() {
    let full = build_tag_menu(3, "v1.0", true);
    assert_eq!(
        labels(&full),
        vec![
            "View Details",
            "Delete Tag…",
            "Push Tag…",
            "--",
            "Create Archive",
            "Copy Tag Name to Clipboard",
        ]
    );
    // View Details selects the anchor row.
    assert_eq!(
        full[0].item_on_select_action(),
        Some(&GitGraphAction::SelectCommit(3))
    );

    // Read-only: only the navigation + copy items remain, in their own groups.
    let ro = build_tag_menu(3, "v1.0", false);
    assert_eq!(
        labels(&ro),
        vec!["View Details", "--", "Copy Tag Name to Clipboard"]
    );
}

#[test]
fn remote_branch_menu_full_matches_screenshot() {
    let items = build_remote_branch_menu("origin/feature", true);
    assert_eq!(
        labels(&items),
        vec![
            "Checkout Branch…",
            "Delete Remote Branch…",
            "Merge into current branch…",
            "Pull into current branch…",
            "--",
            "Create Archive",
            "Unselect in Branches Dropdown",
            "--",
            "Copy Branch Name to Clipboard",
        ]
    );
}

#[test]
fn remote_branch_unselect_uses_full_remote_ref() {
    let items = build_remote_branch_menu("origin/feature", true);
    let unselect = items
        .iter()
        .find_map(|i| match i.item_on_select_action() {
            Some(GitGraphAction::ToggleBranch(r)) => Some(r.clone()),
            _ => None,
        })
        .expect("unselect present");
    assert_eq!(unselect, "refs/remotes/origin/feature");
}

#[test]
fn remote_branch_read_only_keeps_unselect_and_copy() {
    let items = build_remote_branch_menu("origin/feature", false);
    assert_eq!(
        labels(&items),
        vec![
            "Unselect in Branches Dropdown",
            "--",
            "Copy Branch Name to Clipboard",
        ]
    );
}

#[test]
fn local_branch_non_current_menu_matches_screenshot() {
    // A branch other than the checked-out one: the full operation set.
    let items = build_local_branch_menu("feature", false, true);
    assert_eq!(
        labels(&items),
        vec![
            "Checkout Branch…",
            "Rename Branch…",
            "Delete Branch…",
            "Merge into current branch…",
            "Rebase current branch on Branch…",
            "Push Branch…",
            "--",
            "Create Archive",
            "Unselect in Branches Dropdown",
            "--",
            "Copy Branch Name to Clipboard",
        ]
    );
}

#[test]
fn local_branch_current_menu_omits_self_only_ops() {
    // The checked-out branch: no checkout-self / delete / merge-into-current /
    // rebase-onto-self.
    let items = build_local_branch_menu("main", true, true);
    assert_eq!(
        labels(&items),
        vec![
            "Rename Branch…",
            "Push Branch…",
            "--",
            "Create Archive",
            "Unselect in Branches Dropdown",
            "--",
            "Copy Branch Name to Clipboard",
        ]
    );
}

#[test]
fn local_branch_unselect_uses_heads_ref() {
    let items = build_local_branch_menu("main", false, true);
    let unselect = items
        .iter()
        .find_map(|i| match i.item_on_select_action() {
            Some(GitGraphAction::ToggleBranch(r)) => Some(r.clone()),
            _ => None,
        })
        .expect("unselect present");
    assert_eq!(unselect, "refs/heads/main");
}
