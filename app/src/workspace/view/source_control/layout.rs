use std::collections::HashMap;

use super::data::CommitNode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSegment {
    pub from_lane: usize,
    pub to_lane: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphRow {
    pub node_lane: usize,
    pub segments: Vec<GraphSegment>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GraphLayout {
    pub rows: Vec<GraphRow>,
    pub max_lanes: usize,
}

/// Assigns stable lanes to the visible commit page. Parents outside the page remain represented
/// until the end of the page, which keeps lines continuous when another history page is appended.
pub fn layout_commits(commits: &[CommitNode]) -> GraphLayout {
    let mut lanes: Vec<String> = Vec::new();
    let visible: HashMap<_, _> = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.hash.as_str(), index))
        .collect();
    let mut rows = Vec::with_capacity(commits.len());
    let mut max_lanes = 1;

    for commit in commits {
        let node_lane = lanes
            .iter()
            .position(|hash| hash == &commit.hash)
            .unwrap_or_else(|| {
                lanes.push(commit.hash.clone());
                lanes.len() - 1
            });
        let previous_lanes = lanes.clone();

        lanes.remove(node_lane);
        for (parent_index, parent) in commit.parents.iter().enumerate() {
            if lanes.iter().any(|hash| hash == parent) {
                continue;
            }
            let insert_at = (node_lane + parent_index).min(lanes.len());
            lanes.insert(insert_at, parent.clone());
        }

        // Lanes for parents that do not occur in the loaded page can terminate here. Keeping the
        // first parent preserves the continuation into the next page without growing stale lanes.
        lanes.retain(|hash| {
            commit.parents.first() == Some(hash)
                || visible
                    .get(hash.as_str())
                    .is_some_and(|index| *index > rows.len())
        });

        let mut segments = Vec::new();
        for (from_lane, hash) in previous_lanes.iter().enumerate() {
            if hash == &commit.hash {
                for parent in &commit.parents {
                    if let Some(to_lane) = lanes.iter().position(|lane| lane == parent) {
                        segments.push(GraphSegment { from_lane, to_lane });
                    }
                }
            } else if let Some(to_lane) = lanes.iter().position(|lane| lane == hash) {
                segments.push(GraphSegment { from_lane, to_lane });
            }
        }

        max_lanes = max_lanes.max(previous_lanes.len()).max(lanes.len());
        rows.push(GraphRow {
            node_lane,
            segments,
        });
    }

    GraphLayout { rows, max_lanes }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
