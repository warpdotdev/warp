//! Unit tests for the pure context-menu builders in [`super`].

use super::*;
use crate::menu::MenuItem;
use crate::workspace::view::git_graph::data::CommitNode;

fn commit() -> CommitNode {
    CommitNode {
        hash: "abcdef1234567890".into(),
        short_hash: "abcdef12".into(),
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
fn commit_add_tag_action_opens_add_tag_dialog_with_hash() {
    // "Add Tag…" routes to the dedicated AddTag dialog (input + "Push to remote"
    // checkbox), not the plain text-input prompt — carries the commit hash so
    // the dialog can build the AddTag op once the name is submitted.
    let items = build_commit_menu(&commit(), true);
    assert_eq!(
        items[0].item_on_select_action(),
        Some(&GitGraphAction::OpenAddTag {
            hash: "abcdef1234567890".into()
        })
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
fn short_hash_menu_copies_eight_char_hash_regardless_of_write_flag() {
    // A right-click on the short hash yields a single copy item carrying the
    // 8-char short hash (not the full 40-char commit hash); copying is read-only
    // so the write flag doesn't change it.
    let kind = MenuKind::ShortHash { index: 0 };
    for write_enabled in [true, false] {
        let items = build_menu(&kind, &commit(), write_enabled);
        assert_eq!(labels(&items), vec!["Copy Short Hash to Clipboard"]);
        assert_eq!(
            items[0].item_on_select_action(),
            Some(&GitGraphAction::CopyToClipboard("abcdef12".into()))
        );
    }
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

#[test]
fn uncommitted_menu_offers_stash_reset_and_clean_when_writable() {
    let items = build_uncommitted_menu(true);
    assert_eq!(
        labels(&items),
        vec![
            "Stash uncommitted changes…",
            "--",
            "Reset uncommitted changes…",
            "Clean untracked files…",
        ]
    );
    // Stash opens the stash dialog (message input + untracked checkbox).
    assert_eq!(
        items[0].item_on_select_action(),
        Some(&GitGraphAction::PromptStash)
    );
    // Reset (after the separator) opens the mixed/hard picker dialog.
    assert_eq!(
        items[2].item_on_select_action(),
        Some(&GitGraphAction::PromptResetUncommitted)
    );
    // Clean seeds its op with the directories checkbox on by default.
    assert_eq!(
        items[3].item_on_select_action(),
        Some(&GitGraphAction::BeginWriteOp(GitWriteOp::CleanUntracked {
            directories: true
        }))
    );
}

#[test]
fn uncommitted_menu_is_empty_when_read_only() {
    assert!(build_uncommitted_menu(false).is_empty());
}

#[test]
fn stash_menu_full_matches_screenshot_order() {
    let items = build_stash_menu("stash@{0}", &commit(), true);
    assert_eq!(
        labels(&items),
        vec![
            "Apply Stash…",
            "Create Branch from Stash…",
            "Pop Stash…",
            "Drop Stash…",
            "--",
            "Copy Stash Name to Clipboard",
            "Copy Stash Hash to Clipboard",
        ]
    );
    // Create-branch goes through a text-input prompt carrying the selector.
    assert_eq!(
        items[1].item_on_select_action(),
        Some(&GitGraphAction::PromptInput(PromptKind::StashBranch {
            selector: "stash@{0}".to_string()
        }))
    );
}

#[test]
fn stash_menu_read_only_shows_only_copy() {
    let items = build_stash_menu("stash@{0}", &commit(), false);
    assert_eq!(
        labels(&items),
        vec![
            "Copy Stash Name to Clipboard",
            "Copy Stash Hash to Clipboard"
        ]
    );
    // Name copies the selector; hash copies the full commit hash.
    assert_eq!(
        items[0].item_on_select_action(),
        Some(&GitGraphAction::CopyToClipboard("stash@{0}".to_string()))
    );
    assert_eq!(
        items[1].item_on_select_action(),
        Some(&GitGraphAction::CopyToClipboard(commit().hash))
    );
}
