//! Git Graph 面板：只读的 commit DAG 可视化（见 specs/git-graph）。
//!
//! 模块分层：
//! - [`data`]       提交数据类型 + `git log` 输出解析（纯函数）+ 异步取数。
//! - [`layout`]     把提交序列编排成逐行的泳道布局（纯函数，核心算法）。
//! - [`row_canvas`] 单行泳道的自定义绘制元素（竖线/圆点/正交折线）。
//! - [`view`]       左侧 panel 的 Git Graph 视图。
//!
//! 后续阶段会补充提交详情与分页懒加载。

pub(crate) mod data;
pub(crate) mod layout;
pub(crate) mod row_canvas;
pub(crate) mod view;

pub(crate) use view::{init, GitGraphView};
#[cfg(not(target_family = "wasm"))]
pub(crate) use view::GitGraphEvent;
