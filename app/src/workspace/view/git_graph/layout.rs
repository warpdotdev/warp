//! 泳道布局算法：把提交序列（新→旧）编排成逐行的图谱绘制数据。
//!
//! 设计要点（详见 specs/git-graph/TECH.md）：
//! - 自顶向下扫描，维护 `lanes`：每条 lane 记录"下一行期望落到的提交 hash"。
//! - **不做 lane 压缩**：一条 lane 分配到某列后，存活期间列号不变，收束后该列
//!   留空、可被新 lane 复用。这样相邻行的同一条 lane 列号天然对齐，渲染只需逐行
//!   独立绘制，无需全局滚动偏移运算；代价是图里可能出现空列（可接受）。
//! - **第一父接续本列**：合并提交的被合并分支会自然地"汇回"主线，得到标准的
//!   git-graph 菱形观感。
//!
//! 输出的 `color_idx` 是 lane 的创建序号（0,1,2,...，单调递增、不取模），由渲染层
//! 自行 `% 调色板长度` 取色，以便测试断言确定值。

use super::data::CommitNode;

/// 一条竖直穿过某行、且不接触本行提交节点的延续泳道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PassingLane {
    pub col: usize,
    pub color_idx: usize,
}

/// 一端连接到本行提交节点的折线端点。
///
/// 在 [`GraphRow::from_children`] 中 `col` 是来源列（上半段：子→本节点）；
/// 在 [`GraphRow::to_parents`] 中 `col` 是目标列（下半段：本节点→父）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Connection {
    pub col: usize,
    pub color_idx: usize,
}

/// 一行的图谱绘制数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphRow {
    /// 本行提交节点（圆点）所在列。
    pub node_col: usize,
    /// 节点所在 lane 的颜色序号。
    pub node_color: usize,
    /// 节点是否由上一行延续而来（即由已存在的 lane 抵达，而非分支 tip）。
    /// 决定渲染时是否在 `node_col` 画一段从行顶到节点的竖线。
    pub node_continues_up: bool,
    /// 竖直穿过本行的其它延续泳道。
    pub passing: Vec<PassingLane>,
    /// 本节点向下连到各父所在列（下半段）。第一父通常等于 `node_col`。
    pub to_parents: Vec<Connection>,
    /// 各子提交从上方汇入本节点（上半段）；本节点作为合并点时非空。
    pub from_children: Vec<Connection>,
}

/// 整张图的逐行布局结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphLayout {
    pub rows: Vec<GraphRow>,
    /// 渲染所需的最大列数（用于决定泳道区宽度）。
    pub max_lanes: usize,
}

/// 一条活跃 lane 的内部状态。
struct Lane {
    /// 该 lane 下一行期望落到的提交 hash。
    expected: String,
    color_idx: usize,
}

/// 把提交序列（git log 顺序：新→旧，子在父之前）编排为逐行泳道布局。
pub(crate) fn assign_lanes(commits: &[CommitNode]) -> GraphLayout {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut next_color: usize = 0;
    let mut rows = Vec::with_capacity(commits.len());
    let mut max_lanes = 0;

    for commit in commits {
        // 1. 找出所有期望落到本提交的列（多列 = 多个子提交汇入本节点）。
        let incoming: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(j, lane)| match lane {
                Some(l) if l.expected == commit.hash => Some(j),
                _ => None,
            })
            .collect();

        // 节点是否由已存在的 lane 抵达（非分支 tip）。
        let node_continues_up = !incoming.is_empty();

        // 2. 确定节点列与节点颜色。
        let (node_col, node_color) = match incoming.first() {
            // 已有 lane 指向本提交：落在最左的那条。
            Some(&first) => (first, lanes[first].as_ref().unwrap().color_idx),
            // 无 lane 指向：这是一条分支 tip，新开最左空列。
            None => {
                let col = first_empty(&lanes);
                ensure_len(&mut lanes, col);
                let color = next_color;
                next_color += 1;
                (col, color)
            }
        };

        // 3. 其余汇入列（非 node_col）记为 from_children，并在本行收束。
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

        // 4. 其它存活列竖直穿过本行（incoming 的非 node_col 列已收束，不会在此出现）。
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

        // 5. 处理父提交，生成 to_parents 并更新 lanes。
        let mut to_parents: Vec<Connection> = Vec::new();
        if let Some((first_parent, extra_parents)) = commit.parents.split_first() {
            // 第一父接续 node_col 列，沿用节点颜色（主线连续）。
            lanes[node_col] = Some(Lane {
                expected: first_parent.clone(),
                color_idx: node_color,
            });
            to_parents.push(Connection {
                col: node_col,
                color_idx: node_color,
            });

            // 额外父：已有列指向它则复用，否则新开一列。
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
                    to_parents.push(Connection { col, color_idx: color });
                }
            }
        } else {
            // 根提交：本 lane 到此为止。
            lanes[node_col] = None;
        }

        // 在裁剪尾部空列之前统计宽度（裁剪只影响后续行的空洞复用，不影响最大宽度）。
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

/// 返回第一个空列的索引；若全满则返回末尾（= 长度，调用方需 [`ensure_len`]）。
fn first_empty(lanes: &[Option<Lane>]) -> usize {
    lanes
        .iter()
        .position(Option::is_none)
        .unwrap_or(lanes.len())
}

/// 确保 `lanes` 至少有 `col + 1` 个槽位（用 `None` 填充）。
fn ensure_len(lanes: &mut Vec<Option<Lane>>, col: usize) {
    if col >= lanes.len() {
        lanes.resize_with(col + 1, || None);
    }
}

/// 查找期望落到 `hash` 的现存 lane 列。
fn find_lane(lanes: &[Option<Lane>], hash: &str) -> Option<usize> {
    lanes
        .iter()
        .position(|lane| matches!(lane, Some(l) if l.expected == hash))
}

/// 去掉尾部连续的空列，避免无限增长。
fn trim_trailing_none(lanes: &mut Vec<Option<Lane>>) {
    while matches!(lanes.last(), Some(None)) {
        lanes.pop();
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
