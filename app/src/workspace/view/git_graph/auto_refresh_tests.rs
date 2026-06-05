//! Unit tests for [`super::capture_anchor`] (position snapshot by hash),
//! [`super::relocate_view`] (position restore by hash),
//! [`super::detail_refresh_after_reload`] (detail-pane fate after a reload),
//! [`super::throttle_signal`] (the reload-rate throttle), and
//! [`super::should_reload`] (the reload predicate).

use super::*;
use crate::workspace::view::git_graph::data::CommitNode;

/// Build a commit that only carries a hash (the only field [`relocate_view`]
/// looks at).
fn node(hash: &str) -> CommitNode {
    CommitNode {
        hash: hash.to_string(),
        short_hash: hash.chars().take(7).collect(),
        parents: Vec::new(),
        author_name: String::new(),
        author_email: String::new(),
        author_time: 0,
        subject: String::new(),
        refs: Vec::new(),
    }
}

fn commits(hashes: &[&str]) -> Vec<CommitNode> {
    hashes.iter().map(|h| node(h)).collect()
}

#[test]
fn selection_and_anchor_shift_when_new_commits_are_prepended() {
    // The common case: a `git commit` in the terminal prepends "n1"/"n2" at the
    // top. The user had "c" selected and "b" at the top of the viewport; both
    // should track to their new indices so the view doesn't jump.
    let new = commits(&["n1", "n2", "a", "b", "c", "d"]);
    let anchor = relocate_view(&new, Some("c"), Some("b"));
    assert_eq!(anchor.selected, Some(4));
    assert_eq!(anchor.scroll_to, 3);
}

#[test]
fn missing_anchor_falls_back_to_the_selected_row() {
    // The scroll anchor "b" was dropped (e.g. rebased away); fall back to the
    // still-present selected row.
    let new = commits(&["a", "c"]);
    let anchor = relocate_view(&new, Some("c"), Some("b"));
    assert_eq!(anchor.selected, Some(1));
    assert_eq!(anchor.scroll_to, 1);
}

#[test]
fn missing_selection_and_anchor_fall_back_to_the_top() {
    let new = commits(&["a", "b"]);
    let anchor = relocate_view(&new, Some("x"), Some("y"));
    assert_eq!(anchor.selected, None);
    assert_eq!(anchor.scroll_to, 0);
}

#[test]
fn no_prior_selection_anchors_on_the_viewport_top_only() {
    let new = commits(&["a", "b", "c"]);
    let anchor = relocate_view(&new, None, Some("b"));
    assert_eq!(anchor.selected, None);
    assert_eq!(anchor.scroll_to, 1);
}

mod capture {
    use super::super::capture_anchor;
    use super::commits;

    #[test]
    fn captures_the_selected_and_viewport_top_hashes() {
        let list = commits(&["a", "b", "c", "d"]);
        let (selected, anchor) = capture_anchor(&list, Some(2), 1, false);
        assert_eq!(selected.as_deref(), Some("c"));
        assert_eq!(anchor.as_deref(), Some("b"));
    }

    #[test]
    fn uncommitted_row_offsets_the_viewport_anchor_only() {
        // `visible_start` is a UniformList index that counts the synthetic
        // uncommitted row at index 0; `selected` is already a commit index.
        let list = commits(&["a", "b", "c"]);
        let (selected, anchor) = capture_anchor(&list, Some(0), 2, true);
        assert_eq!(selected.as_deref(), Some("a"));
        assert_eq!(anchor.as_deref(), Some("b"));
    }

    #[test]
    fn viewport_on_the_uncommitted_row_anchors_on_the_newest_commit() {
        // The uncommitted row itself is at the viewport top: it has no hash,
        // so the anchor degrades to the newest commit below it.
        let list = commits(&["a", "b"]);
        let (_, anchor) = capture_anchor(&list, None, 0, true);
        assert_eq!(anchor.as_deref(), Some("a"));
    }

    #[test]
    fn no_selection_captures_no_selected_hash() {
        let list = commits(&["a", "b"]);
        let (selected, anchor) = capture_anchor(&list, None, 1, false);
        assert_eq!(selected, None);
        assert_eq!(anchor.as_deref(), Some("b"));
    }

    #[test]
    fn out_of_range_indices_capture_nothing() {
        // A selection index past the end (the list shrank under it) must not
        // panic and yields no hash.
        let list = commits(&["a"]);
        let (selected, anchor) = capture_anchor(&list, Some(5), 9, false);
        assert_eq!(selected, None);
        assert_eq!(anchor, None);
    }
}

mod detail_refresh {
    use super::super::{detail_refresh_after_reload, DetailRefresh};

    #[test]
    fn uncommitted_detail_refreshes_in_place_while_the_tree_is_dirty() {
        // The uncommitted row is not hash-addressed, so relocation always
        // reports `selected: None` for it — that must read as "re-read the
        // uncommitted detail", not "the selection is gone, drop the detail"
        // (the bug: a large in-flight file change delivered its watcher event
        // after the row was clicked, and the reload blanked the loaded detail).
        assert_eq!(
            detail_refresh_after_reload(true, 3, None),
            DetailRefresh::RefreshUncommitted
        );
    }

    #[test]
    fn uncommitted_detail_clears_when_the_tree_becomes_clean() {
        // Everything was committed/stashed away: the uncommitted row itself is
        // gone from the graph, so its selection and detail go with it.
        assert_eq!(
            detail_refresh_after_reload(true, 0, None),
            DetailRefresh::Clear
        );
    }

    #[test]
    fn surviving_commit_selection_keeps_its_detail() {
        assert_eq!(
            detail_refresh_after_reload(false, 1, Some(4)),
            DetailRefresh::Keep
        );
    }

    #[test]
    fn vanished_commit_selection_clears_its_detail() {
        // The selected commit was amended/rebased away.
        assert_eq!(
            detail_refresh_after_reload(false, 1, None),
            DetailRefresh::Clear
        );
    }
}

mod throttle {
    use std::time::Duration;

    use super::super::{throttle_signal, ThrottleDecision};

    const INTERVAL: Duration = Duration::from_secs(3);

    #[test]
    fn first_signal_after_quiet_refreshes_immediately() {
        // Leading edge: a `git commit` in the terminal must show up right
        // away, never wait out a cooldown. `None` = never refreshed yet.
        assert_eq!(
            throttle_signal(None, false, INTERVAL),
            ThrottleDecision::RefreshNow
        );
    }

    #[test]
    fn signal_outside_the_window_refreshes_immediately() {
        assert_eq!(
            throttle_signal(Some(Duration::from_secs(4)), false, INTERVAL),
            ThrottleDecision::RefreshNow
        );
    }

    #[test]
    fn window_boundary_refreshes_immediately() {
        // elapsed == interval counts as "outside": the cooldown is half-open.
        assert_eq!(
            throttle_signal(Some(INTERVAL), false, INTERVAL),
            ThrottleDecision::RefreshNow
        );
    }

    #[test]
    fn first_signal_inside_the_window_defers_the_remainder() {
        // Trailing edge: an event storm's second signal schedules exactly one
        // catch-up at the window's end, so the graph still converges on the
        // final state instead of dropping it.
        assert_eq!(
            throttle_signal(Some(Duration::from_secs(1)), false, INTERVAL),
            ThrottleDecision::Defer(Duration::from_secs(2))
        );
    }

    #[test]
    fn further_signals_inside_the_window_are_dropped() {
        // A catch-up is already scheduled; it covers every later change too.
        assert_eq!(
            throttle_signal(Some(Duration::from_secs(1)), true, INTERVAL),
            ThrottleDecision::AlreadyDeferred
        );
    }

    #[test]
    fn a_stale_pending_does_not_block_an_overdue_refresh() {
        // The catch-up timer hasn't fired yet but the window has passed:
        // refresh now (the caller absorbs the pending catch-up into it).
        assert_eq!(
            throttle_signal(Some(Duration::from_secs(5)), true, INTERVAL),
            ThrottleDecision::RefreshNow
        );
    }
}

mod reload_predicate {
    use repo_metadata::watcher::TargetFile;
    use repo_metadata::RepositoryUpdate;

    use super::super::subscription::should_reload;

    #[test]
    fn local_commit_move_triggers_reload() {
        let update = RepositoryUpdate {
            commit_updated: true,
            ..Default::default()
        };
        assert!(should_reload(&update));
    }

    #[test]
    fn remote_ref_update_triggers_reload() {
        let update = RepositoryUpdate {
            remote_ref_updated: true,
            ..Default::default()
        };
        assert!(should_reload(&update));
    }

    #[test]
    fn an_empty_update_does_not_reload() {
        // An update with no signal at all — no commit/ref move and no file
        // change — must not trigger a reload.
        assert!(!should_reload(&RepositoryUpdate::default()));
    }

    #[test]
    fn a_non_ignored_file_change_triggers_reload() {
        // A working-tree file change refreshes the uncommitted-changes count
        // (the "reload the whole graph on a file change" policy).
        let mut update = RepositoryUpdate::default();
        update
            .modified
            .insert(TargetFile::new("src/a.rs".into(), false));
        assert!(should_reload(&update));
    }

    #[test]
    fn an_ignored_only_file_change_does_not_reload() {
        let mut update = RepositoryUpdate::default();
        update
            .modified
            .insert(TargetFile::new("target/x".into(), true));
        assert!(!should_reload(&update));
    }
}
