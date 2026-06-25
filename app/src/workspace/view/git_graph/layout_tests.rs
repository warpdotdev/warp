//! Unit tests for [`super::assign_lanes`], covering linear / fork / merge /
//! octopus-merge / multi-root / branch-tip DAG shapes.

use super::*;
use crate::workspace::view::git_graph::data::{CommitNode, RefKind, RefLabel};

/// Build a commit that only cares about hash and parents (the other fields have
/// no effect on layout).
fn node(hash: &str, parents: &[&str]) -> CommitNode {
    CommitNode {
        hash: hash.to_string(),
        short_hash: hash.chars().take(8).collect(),
        parents: parents.iter().map(|s| s.to_string()).collect(),
        author_name: String::new(),
        author_email: String::new(),
        author_time: 0,
        subject: String::new(),
        refs: Vec::new(),
    }
}

fn conn(col: usize, color_idx: usize) -> Connection {
    Connection { col, color_idx }
}

fn pass(col: usize, color_idx: usize) -> PassingLane {
    PassingLane { col, color_idx }
}

/// Sequence of per-row node column numbers, for quickly asserting the overall
/// shape.
fn node_cols(layout: &GraphLayout) -> Vec<usize> {
    layout.rows.iter().map(|r| r.node_col).collect()
}

/// A commit marked as the checked-out HEAD.
fn head_node(hash: &str, parents: &[&str]) -> CommitNode {
    let mut commit = node(hash, parents);
    commit.refs.push(RefLabel {
        kind: RefKind::Head,
        name: "main".to_string(),
    });
    commit
}

#[test]
fn build_layout_prepends_uncommitted_row_anchored_to_head() {
    // "C" is the checked-out HEAD; with uncommitted changes a synthetic row 0
    // connects down to it.
    let commits = [head_node("C", &["B"]), node("B", &["A"]), node("A", &[])];
    let layout = build_layout(&commits, true);

    // Synthetic uncommitted row + the 3 commits.
    assert_eq!(layout.rows.len(), 4);
    // Row 0 (uncommitted): a branch tip (nothing above) connecting down one lane.
    assert!(!layout.rows[0].node_continues_up);
    assert_eq!(layout.rows[0].to_parents, vec![conn(0, 0)]);
    // Row 1 (HEAD commit "C"): now reached from the uncommitted lane above.
    assert!(layout.rows[1].node_continues_up);
    assert_eq!(layout.rows[1].node_col, 0);
}

#[test]
fn build_layout_without_uncommitted_is_plain() {
    let commits = [node("C", &["B"]), node("B", &[])];
    let layout = build_layout(&commits, false);
    assert_eq!(layout.rows.len(), 2);
    assert!(!layout.rows[0].node_continues_up);
}

#[test]
fn build_layout_with_no_head_in_view_falls_back_to_plain() {
    // No commit carries a HEAD ref → nothing to anchor the uncommitted row to.
    let commits = [node("C", &["B"]), node("B", &[])];
    let layout = build_layout(&commits, true);
    assert_eq!(layout.rows.len(), 2); // no sentinel row added
}

#[test]
fn linear_history_stays_in_single_lane() {
    let commits = [node("C", &["B"]), node("B", &["A"]), node("A", &[])];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 1);
    assert_eq!(node_cols(&layout), vec![0, 0, 0]);

    // The first commit is a branch tip (no upstream continuation); the rest are
    // reached by an existing lane.
    assert!(!layout.rows[0].node_continues_up);
    assert!(layout.rows[1].node_continues_up);
    assert!(layout.rows[2].node_continues_up);

    // Middle commit: both the incoming and outgoing connections are in the same
    // column.
    assert_eq!(layout.rows[1].to_parents, vec![conn(0, 0)]);
    assert!(layout.rows[1].passing.is_empty());
    assert!(layout.rows[1].from_children.is_empty());

    // Root commit: no parents and no outgoing connections.
    assert!(layout.rows[2].to_parents.is_empty());
    assert!(layout.rows[2].from_children.is_empty());
}

#[test]
fn fork_places_second_child_in_new_lane_and_merges_at_parent() {
    // A is the parent of both C and B; order (newest -> oldest) is C, B, A.
    let commits = [node("C", &["A"]), node("B", &["A"]), node("A", &[])];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    // C is in col0, B in the new col1, and A merges back into the leftmost col0.
    assert_eq!(node_cols(&layout), vec![0, 1, 0]);

    // B is in the new column, and the first column (C->A) passes through as a lane.
    assert_eq!(layout.rows[1].node_col, 1);
    assert_eq!(layout.rows[1].passing, vec![pass(0, 0)]);
    assert_eq!(layout.rows[1].to_parents, vec![conn(1, 1)]);

    // A ends the second lane: the B branch merges in from col1.
    assert_eq!(layout.rows[2].from_children, vec![conn(1, 1)]);
    assert!(layout.rows[2].to_parents.is_empty());
}

#[test]
fn merge_commit_opens_lane_for_second_parent() {
    // M is a merge commit with parents [A (mainline), B (merged-in)]; B's parent
    // is A; order M, B, A.
    let commits = [node("M", &["A", "B"]), node("B", &["A"]), node("A", &[])];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    assert_eq!(node_cols(&layout), vec![0, 1, 0]);

    // The merge row branches into two columns: the first parent in the current
    // column, the second parent opening a new column.
    assert_eq!(layout.rows[0].to_parents, vec![conn(0, 0), conn(1, 1)]);
    assert!(layout.rows[0].from_children.is_empty());

    // The merged-in branch B runs in col1, with the mainline A passing through as
    // a lane.
    assert_eq!(layout.rows[1].node_col, 1);
    assert_eq!(layout.rows[1].passing, vec![pass(0, 0)]);

    // The A row ends col1 (the merged-in branch merges back into the mainline).
    assert_eq!(layout.rows[2].node_col, 0);
    assert_eq!(layout.rows[2].from_children, vec![conn(1, 1)]);
}

#[test]
fn octopus_merge_opens_one_lane_per_extra_parent() {
    // A three-parent merge where all three parents are independent roots; order
    // M, A, B, C.
    let commits = [
        node("M", &["A", "B", "C"]),
        node("A", &[]),
        node("B", &[]),
        node("C", &[]),
    ];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 3);
    assert_eq!(node_cols(&layout), vec![0, 0, 1, 2]);

    // The merge row branches into three columns, each a different color.
    assert_eq!(
        layout.rows[0].to_parents,
        vec![conn(0, 0), conn(1, 1), conn(2, 2)]
    );
}

#[test]
fn multiple_roots_keep_independent_lanes() {
    // Two unrelated histories: X->R1 and Y->R2; order X, Y, R1, R2.
    let commits = [
        node("X", &["R1"]),
        node("Y", &["R2"]),
        node("R1", &[]),
        node("R2", &[]),
    ];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    assert_eq!(node_cols(&layout), vec![0, 1, 0, 1]);

    // The two root commits each end their own column, without interfering.
    assert!(layout.rows[2].to_parents.is_empty());
    assert!(layout.rows[3].to_parents.is_empty());
    assert_eq!(layout.rows[1].passing, vec![pass(0, 0)]);
}

#[test]
fn single_commit_repository() {
    let commits = [node("only", &[])];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 1);
    assert_eq!(layout.rows.len(), 1);
    let row = &layout.rows[0];
    assert_eq!(row.node_col, 0);
    assert!(row.to_parents.is_empty());
    assert!(row.from_children.is_empty());
    assert!(row.passing.is_empty());
}

#[test]
fn empty_input_yields_empty_layout() {
    let layout = assign_lanes(&[]);
    assert_eq!(layout.max_lanes, 0);
    assert!(layout.rows.is_empty());
}

#[test]
fn freed_lane_column_is_reused_by_later_branch() {
    // After B ends col1, it should be reused by the later branch tip D rather than
    // shifting ever further right.
    // Order: C(->A), B(->A), A(->Z), D(->Z), Z()
    // C and B share parent A (B occupies col1); once A ends col1, D as a new tip
    // should reuse col1.
    let commits = [
        node("C", &["A"]),
        node("B", &["A"]),
        node("A", &["Z"]),
        node("D", &["Z"]),
        node("Z", &[]),
    ];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    // A in col0 ends B's col1; D as a new tip reuses col1.
    assert_eq!(layout.rows[2].node_col, 0);
    assert_eq!(layout.rows[3].node_col, 1);
}
