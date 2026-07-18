use super::*;
use crate::workspace::view::source_control::data::CommitNode;

fn commit(hash: &str, parents: &[&str]) -> CommitNode {
    CommitNode {
        hash: hash.to_string(),
        parents: parents.iter().map(ToString::to_string).collect(),
        author: String::new(),
        timestamp: 0,
        subject: hash.to_string(),
        body: String::new(),
        refs: Vec::new(),
        stats: None,
    }
}

#[test]
fn lays_out_linear_history_in_one_lane() {
    let layout = layout_commits(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);

    assert_eq!(layout.max_lanes, 1);
    assert_eq!(
        layout
            .rows
            .iter()
            .map(|row| row.node_lane)
            .collect::<Vec<_>>(),
        vec![0, 0, 0]
    );
}

#[test]
fn lays_out_merge_with_two_lanes() {
    let layout = layout_commits(&[
        commit("merge", &["left", "right"]),
        commit("left", &["base"]),
        commit("right", &["base"]),
        commit("base", &[]),
    ]);

    assert_eq!(layout.max_lanes, 2);
    assert_eq!(layout.rows[0].node_lane, 0);
    assert_eq!(layout.rows[2].node_lane, 1);
    assert_eq!(layout.rows[3].node_lane, 0);
}

#[test]
fn lays_out_multiple_roots_without_panicking() {
    let layout = layout_commits(&[commit("one", &[]), commit("two", &[])]);

    assert_eq!(layout.rows.len(), 2);
    assert_eq!(layout.max_lanes, 1);
}
