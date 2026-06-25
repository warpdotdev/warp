//! Lane layout algorithm: arranges a commit sequence (newest -> oldest) into
//! per-row graph drawing data.
//!
//! Design notes (see specs/git-graph/TECH.md for details):
//! - Scan top-down, maintaining `lanes`: each lane records "the commit hash the
//!   next row is expected to land on".
//! - **No lane compaction**: once a lane is assigned a column, its column number
//!   stays fixed for its lifetime; after it ends the column is left empty and may
//!   be reused by a new lane. This way the same lane in adjacent rows is
//!   naturally column-aligned, so rendering only needs to draw each row
//!   independently with no global scroll-offset math; the cost is that empty
//!   columns may appear in the graph (acceptable).
//! - **First parent continues the current column**: a merge commit's merged-in
//!   branch naturally "merges back" into the mainline, producing the standard
//!   git-graph diamond look.
//!
//! The emitted `color_idx` is the lane's creation index (0, 1, 2, ...,
//! monotonically increasing, not taken modulo); the rendering layer applies
//! `% palette length` itself to pick a color, so tests can assert deterministic
//! values.

use super::data::{CommitNode, RefKind};

/// A continuing lane that passes vertically through a row without touching that
/// row's commit node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PassingLane {
    pub col: usize,
    pub color_idx: usize,
}

/// One endpoint of a polyline that connects to this row's commit node.
///
/// In [`GraphRow::from_children`], `col` is the source column (upper half:
/// child -> this node); in [`GraphRow::to_parents`], `col` is the target column
/// (lower half: this node -> parent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Connection {
    pub col: usize,
    pub color_idx: usize,
}

/// Graph drawing data for a single row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphRow {
    /// Column of this row's commit node (the dot).
    pub node_col: usize,
    /// Color index of the lane the node sits in.
    pub node_color: usize,
    /// Whether the node continues from the previous row (i.e. it is reached by an
    /// existing lane rather than being a branch tip). Determines whether a
    /// vertical line from the top of the row down to the node is drawn at
    /// `node_col`.
    pub node_continues_up: bool,
    /// Other continuing lanes that pass vertically through this row.
    pub passing: Vec<PassingLane>,
    /// Columns this node connects down to each parent (lower half). The first
    /// parent is usually equal to `node_col`.
    pub to_parents: Vec<Connection>,
    /// Child commits merging into this node from above (upper half); non-empty
    /// when this node acts as a merge point.
    pub from_children: Vec<Connection>,
}

/// The per-row layout result for the whole graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphLayout {
    pub rows: Vec<GraphRow>,
    /// Maximum number of columns needed for rendering (used to size the lane area).
    pub max_lanes: usize,
}

/// Internal state of one active lane.
struct Lane {
    /// The commit hash this lane expects the next row to land on.
    expected: String,
    color_idx: usize,
}

/// Arrange a commit sequence (git log order: newest -> oldest, children before
/// parents) into a per-row lane layout.
pub(crate) fn assign_lanes(commits: &[CommitNode]) -> GraphLayout {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut next_color: usize = 0;
    let mut rows = Vec::with_capacity(commits.len());
    let mut max_lanes = 0;

    for commit in commits {
        // 1. Find all columns expecting this commit (multiple columns = multiple
        //    child commits merging into this node).
        let incoming: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(j, lane)| match lane {
                Some(l) if l.expected == commit.hash => Some(j),
                _ => None,
            })
            .collect();

        // Whether the node is reached by an existing lane (not a branch tip).
        let node_continues_up = !incoming.is_empty();

        // 2. Determine the node's column and color.
        let (node_col, node_color) = match incoming.first() {
            // An existing lane points at this commit: land on the leftmost one.
            Some(&first) => (first, lanes[first].as_ref().unwrap().color_idx),
            // No lane points at it: this is a branch tip, open the leftmost empty
            // column.
            None => {
                let col = first_empty(&lanes);
                ensure_len(&mut lanes, col);
                let color = next_color;
                next_color += 1;
                (col, color)
            }
        };

        // 3. The remaining incoming columns (other than node_col) are recorded as
        //    from_children and end on this row.
        let from_children: Vec<Connection> = incoming
            .iter()
            .filter(|&&j| j != node_col)
            .map(|&j| Connection {
                col: j,
                color_idx: lanes[j].as_ref().unwrap().color_idx,
            })
            .collect();
        for &j in &incoming {
            if j != node_col {
                lanes[j] = None;
            }
        }

        // 4. Other surviving columns pass vertically through this row (incoming
        //    columns other than node_col have already ended and won't appear here).
        let passing: Vec<PassingLane> = lanes
            .iter()
            .enumerate()
            .filter_map(|(j, lane)| {
                if j == node_col {
                    return None;
                }
                lane.as_ref().map(|l| PassingLane {
                    col: j,
                    color_idx: l.color_idx,
                })
            })
            .collect();

        // 5. Process the parent commits, building to_parents and updating lanes.
        let mut to_parents: Vec<Connection> = Vec::new();
        if let Some((first_parent, extra_parents)) = commit.parents.split_first() {
            // The first parent continues the node_col column, keeping the node's
            // color (the mainline stays continuous).
            lanes[node_col] = Some(Lane {
                expected: first_parent.clone(),
                color_idx: node_color,
            });
            to_parents.push(Connection {
                col: node_col,
                color_idx: node_color,
            });

            // Extra parents: reuse an existing column that points at the parent,
            // otherwise open a new column.
            for parent in extra_parents {
                if let Some(existing) = find_lane(&lanes, parent) {
                    to_parents.push(Connection {
                        col: existing,
                        color_idx: lanes[existing].as_ref().unwrap().color_idx,
                    });
                } else {
                    let col = first_empty(&lanes);
                    ensure_len(&mut lanes, col);
                    let color = next_color;
                    next_color += 1;
                    lanes[col] = Some(Lane {
                        expected: parent.clone(),
                        color_idx: color,
                    });
                    to_parents.push(Connection {
                        col,
                        color_idx: color,
                    });
                }
            }
        } else {
            // Root commit: this lane ends here.
            lanes[node_col] = None;
        }

        // Measure the width before trimming trailing empty columns (trimming only
        // affects gap reuse on later rows, not the maximum width).
        max_lanes = max_lanes.max(lanes.len());
        trim_trailing_none(&mut lanes);

        rows.push(GraphRow {
            node_col,
            node_color,
            node_continues_up,
            passing,
            to_parents,
            from_children,
        });
    }

    GraphLayout { rows, max_lanes }
}

/// Sentinel hash for the synthetic "uncommitted changes" row. Real commit hashes
/// are never empty, so an empty hash unambiguously marks the sentinel.
pub(crate) const UNCOMMITTED_HASH: &str = "";

/// Hash of the checked-out commit (the one carrying a `RefKind::Head` label),
/// used to anchor the uncommitted row's connection to HEAD.
fn head_commit_hash(commits: &[CommitNode]) -> Option<&str> {
    commits
        .iter()
        .find(|c| c.refs.iter().any(|r| r.kind == RefKind::Head))
        .map(|c| c.hash.as_str())
}

/// Build the per-row layout, prepending a synthetic "uncommitted changes" row
/// (a node on the HEAD lane that connects down to the HEAD commit) when
/// `has_uncommitted` and a HEAD commit is present in `commits`. When added, the
/// uncommitted row is row 0, so callers offset their commit indexing by one.
///
/// Falls back to a plain layout when there are no uncommitted changes, or when
/// no HEAD commit is in view (e.g. the branch filter excludes the current
/// branch) — there's nothing to anchor the row to.
pub(crate) fn build_layout(commits: &[CommitNode], has_uncommitted: bool) -> GraphLayout {
    if has_uncommitted {
        if let Some(head) = head_commit_hash(commits) {
            let sentinel = CommitNode {
                hash: UNCOMMITTED_HASH.to_string(),
                short_hash: String::new(),
                parents: vec![head.to_string()],
                author_name: String::new(),
                author_email: String::new(),
                author_time: 0,
                subject: String::new(),
                refs: Vec::new(),
            };
            let mut with_sentinel = Vec::with_capacity(commits.len() + 1);
            with_sentinel.push(sentinel);
            with_sentinel.extend_from_slice(commits);
            return assign_lanes(&with_sentinel);
        }
    }
    assign_lanes(commits)
}

/// Return the index of the first empty column; if all are full, return the end
/// (= length, so the caller must call [`ensure_len`]).
fn first_empty(lanes: &[Option<Lane>]) -> usize {
    lanes
        .iter()
        .position(Option::is_none)
        .unwrap_or(lanes.len())
}

/// Ensure `lanes` has at least `col + 1` slots (padding with `None`).
fn ensure_len(lanes: &mut Vec<Option<Lane>>, col: usize) {
    if col >= lanes.len() {
        lanes.resize_with(col + 1, || None);
    }
}

/// Find the existing lane column that expects `hash`.
fn find_lane(lanes: &[Option<Lane>], hash: &str) -> Option<usize> {
    lanes
        .iter()
        .position(|lane| matches!(lane, Some(l) if l.expected == hash))
}

/// Drop trailing consecutive empty columns to avoid unbounded growth.
fn trim_trailing_none(lanes: &mut Vec<Option<Lane>>) {
    while matches!(lanes.last(), Some(None)) {
        lanes.pop();
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
