//! [`super::assign_lanes`] 的单元测试，覆盖线性 / 分叉 / 合并 / 八爪合并 /
//! 多根 / 分支 tip 各种 DAG 形态。

use super::*;
use crate::workspace::view::git_graph::data::CommitNode;

/// 构造一个只关心 hash 与 parents 的提交（其余字段对布局无影响）。
fn node(hash: &str, parents: &[&str]) -> CommitNode {
    CommitNode {
        hash: hash.to_string(),
        short_hash: hash.chars().take(7).collect(),
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

/// 各行节点列号序列，便于快速断言整体形状。
fn node_cols(layout: &GraphLayout) -> Vec<usize> {
    layout.rows.iter().map(|r| r.node_col).collect()
}

#[test]
fn linear_history_stays_in_single_lane() {
    let commits = [node("C", &["B"]), node("B", &["A"]), node("A", &[])];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 1);
    assert_eq!(node_cols(&layout), vec![0, 0, 0]);

    // 首个提交是分支 tip（无上游延续），其余由已存在 lane 抵达。
    assert!(!layout.rows[0].node_continues_up);
    assert!(layout.rows[1].node_continues_up);
    assert!(layout.rows[2].node_continues_up);

    // 中间提交：上接下连都在同一列。
    assert_eq!(layout.rows[1].to_parents, vec![conn(0, 0)]);
    assert!(layout.rows[1].passing.is_empty());
    assert!(layout.rows[1].from_children.is_empty());

    // 根提交：无父、无后续连接。
    assert!(layout.rows[2].to_parents.is_empty());
    assert!(layout.rows[2].from_children.is_empty());
}

#[test]
fn fork_places_second_child_in_new_lane_and_merges_at_parent() {
    // A 同时是 C 与 B 的父；顺序（新→旧）为 C, B, A。
    let commits = [node("C", &["A"]), node("B", &["A"]), node("A", &[])];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    // C 在 col0、B 在新列 col1、A 汇回最左列 col0。
    assert_eq!(node_cols(&layout), vec![0, 1, 0]);

    // B 在新列，且第一列（C→A）作为穿过泳道。
    assert_eq!(layout.rows[1].node_col, 1);
    assert_eq!(layout.rows[1].passing, vec![pass(0, 0)]);
    assert_eq!(layout.rows[1].to_parents, vec![conn(1, 1)]);

    // A 收束第二条 lane：B 分支从 col1 汇入。
    assert_eq!(layout.rows[2].from_children, vec![conn(1, 1)]);
    assert!(layout.rows[2].to_parents.is_empty());
}

#[test]
fn merge_commit_opens_lane_for_second_parent() {
    // M 是合并提交，父为 [A(主线), B(被合并)]；B 的父是 A；顺序 M, B, A。
    let commits = [
        node("M", &["A", "B"]),
        node("B", &["A"]),
        node("A", &[]),
    ];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    assert_eq!(node_cols(&layout), vec![0, 1, 0]);

    // 合并行向两列分出：第一父在本列、第二父开新列。
    assert_eq!(layout.rows[0].to_parents, vec![conn(0, 0), conn(1, 1)]);
    assert!(layout.rows[0].from_children.is_empty());

    // 被合并分支 B 走 col1，主线 A 作为穿过泳道。
    assert_eq!(layout.rows[1].node_col, 1);
    assert_eq!(layout.rows[1].passing, vec![pass(0, 0)]);

    // A 行收束 col1（被合并分支汇回主线）。
    assert_eq!(layout.rows[2].node_col, 0);
    assert_eq!(layout.rows[2].from_children, vec![conn(1, 1)]);
}

#[test]
fn octopus_merge_opens_one_lane_per_extra_parent() {
    // 三父合并，三个父均为独立根；顺序 M, A, B, C。
    let commits = [
        node("M", &["A", "B", "C"]),
        node("A", &[]),
        node("B", &[]),
        node("C", &[]),
    ];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 3);
    assert_eq!(node_cols(&layout), vec![0, 0, 1, 2]);

    // 合并行向三列分出，颜色各异。
    assert_eq!(
        layout.rows[0].to_parents,
        vec![conn(0, 0), conn(1, 1), conn(2, 2)]
    );
}

#[test]
fn multiple_roots_keep_independent_lanes() {
    // 两条互不相关的历史：X→R1，Y→R2；顺序 X, Y, R1, R2。
    let commits = [
        node("X", &["R1"]),
        node("Y", &["R2"]),
        node("R1", &[]),
        node("R2", &[]),
    ];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    assert_eq!(node_cols(&layout), vec![0, 1, 0, 1]);

    // 两条根提交分别收束各自的列，互不干扰。
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
    // col1 在 B 收束后应被后来的分支 tip D 复用，而非一直右移。
    // 顺序：C(→A), B(→A), A(→Z), D(→Z), Z()
    // C、B 共享父 A（B 占 col1）；A 收束 col1 后，D 作为新 tip 应复用 col1。
    let commits = [
        node("C", &["A"]),
        node("B", &["A"]),
        node("A", &["Z"]),
        node("D", &["Z"]),
        node("Z", &[]),
    ];
    let layout = assign_lanes(&commits);

    assert_eq!(layout.max_lanes, 2);
    // A 在 col0 收束 B 的 col1；D 作为新 tip 复用 col1。
    assert_eq!(layout.rows[2].node_col, 0);
    assert_eq!(layout.rows[3].node_col, 1);
}
