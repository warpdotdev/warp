//! Git Graph 视图。
//!
//! Phase 2：先打通"flag → 按钮 → 视图 → 取数 → 渲染"全链路，主体渲染为**纯文本
//! 提交列表**（短 hash + 引用标签 + subject）。图谱泳道绘制留待 Phase 3 接入
//! [`super::layout`] 与自定义 row canvas。
//!
//! 状态直接持有在视图内（单实例、无共享），不引入单独的 Model 间接层——待后续
//! 出现跨视图共享需求时再抽。

use std::sync::Arc;

use warpui::elements::{
    Container, CrossAxisAlignment, Element, Flex, MainAxisAlignment, MainAxisSize, ParentElement,
    Shrinkable, Text, UniformList, UniformListState,
};
use warpui::{AppContext, Entity, SingletonEntity, View, ViewContext};

use super::data::CommitNode;
use super::layout::{assign_lanes, GraphLayout, GraphRow};
use super::row_canvas::GitGraphRowCanvas;
use crate::appearance::Appearance;

/// 首屏加载的提交数上限（分页懒加载留待 Phase 4）。
const COMMIT_PAGE_SIZE: usize = 200;

/// 视图向外发出的事件。Phase 2 暂无。
pub(crate) enum GitGraphEvent {}

/// 提交图谱的加载状态。
enum LoadState {
    /// 当前工作目录不在任何 git 仓库内，或尚未指定目录。
    NoRepo,
    /// 正在加载。
    Loading,
    /// 已加载（`commits` 有效；可能为空 = 仓库无提交）。
    Loaded,
    /// 加载失败，附带错误文案。
    Error(String),
}

pub(crate) struct GitGraphView {
    /// 当前跟随的工作目录（由左侧 panel 在活跃目录变化时推入）。
    working_dir: Option<std::path::PathBuf>,
    /// 已加载的提交（用 `Arc` 便于零拷贝移动进 [`UniformList`] 的构建闭包）。
    commits: Arc<Vec<CommitNode>>,
    /// 由 [`assign_lanes`] 算出的逐行泳道布局，与 `commits` 一一对应。
    layout: Arc<GraphLayout>,
    state: LoadState,
    /// 列表滚动状态（保留滚动位置）。
    list_state: UniformListState,
}

/// 空布局，用于未加载/出错时。
fn empty_layout() -> GraphLayout {
    GraphLayout {
        rows: Vec::new(),
        max_lanes: 0,
    }
}

impl GitGraphView {
    pub(crate) fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            working_dir: None,
            commits: Arc::new(Vec::new()),
            layout: Arc::new(empty_layout()),
            state: LoadState::NoRepo,
            list_state: UniformListState::new(),
        }
    }

    /// 设置要展示的工作目录；变化时触发重新加载。
    pub(crate) fn set_working_directory(
        &mut self,
        dir: Option<std::path::PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.working_dir == dir {
            return;
        }
        self.working_dir = dir;
        self.reload(ctx);
    }

    /// 重新加载当前工作目录的提交图谱。
    fn reload(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(dir) = self.working_dir.clone() else {
            self.commits = Arc::new(Vec::new());
            self.layout = Arc::new(empty_layout());
            self.state = LoadState::NoRepo;
            ctx.notify();
            return;
        };

        self.state = LoadState::Loading;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            // 用于在结果返回时判断仓库是否已被切走（任务是 detach 的，无需句柄）。
            let expected = dir.clone();
            ctx.spawn(
                async move { super::data::load_commit_graph(&dir, COMMIT_PAGE_SIZE, 0).await },
                move |view, result, ctx| {
                    if view.working_dir.as_deref() != Some(expected.as_path()) {
                        // 仓库已切换，丢弃过期结果。
                        return;
                    }
                    match result {
                        Ok(commits) => {
                            view.layout = Arc::new(assign_lanes(&commits));
                            view.commits = Arc::new(commits);
                            view.state = LoadState::Loaded;
                        }
                        Err(err) => {
                            view.commits = Arc::new(Vec::new());
                            view.layout = Arc::new(empty_layout());
                            view.state = LoadState::Error(err.to_string());
                        }
                    }
                    ctx.notify();
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = dir;
            self.state = LoadState::NoRepo;
            ctx.notify();
        }
    }
}

/// 渲染居中的单行提示文案（用于空 / 加载中 / 错误等状态）。
fn render_message(text: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
    )
    .with_horizontal_padding(12.)
    .with_vertical_padding(8.)
    .finish()
}

/// 渲染一行图谱：左侧泳道绘制 + 右侧提交文字。
fn render_graph_row(
    row: &GraphRow,
    lane_count: usize,
    commit: &CommitNode,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(GitGraphRowCanvas::new(row.clone(), lane_count).finish())
        .with_child(render_commit_text(commit, appearance))
        .finish()
}

/// 渲染提交文字列：短 hash + 引用标签 + subject。
fn render_commit_text(commit: &CommitNode, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();
    let dim = theme.sub_text_color(theme.background());
    let fg = theme.foreground();

    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                Text::new_inline(commit.short_hash.clone(), font, size)
                    .with_color(dim.into())
                    .finish(),
            )
            .with_padding_right(8.)
            .finish(),
        );

    if !commit.refs.is_empty() {
        let label = commit
            .refs
            .iter()
            .map(|r| r.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        row = row.with_child(
            Container::new(
                Text::new_inline(format!("({label})"), font, size)
                    .with_color(fg.into())
                    .finish(),
            )
            .with_padding_right(8.)
            .finish(),
        );
    }

    row = row.with_child(
        Text::new_inline(commit.subject.clone(), font, size)
            .with_color(fg.into())
            .finish(),
    );

    Container::new(row.finish())
        .with_padding_left(6.)
        .with_padding_right(12.)
        .finish()
}

impl Entity for GitGraphView {
    type Event = GitGraphEvent;
}

impl View for GitGraphView {
    fn ui_name() -> &'static str {
        "GitGraphView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let content: Box<dyn Element> = match &self.state {
            LoadState::NoRepo => {
                render_message("Current directory is not a git repository".to_string(), appearance)
            }
            LoadState::Loading => render_message("Loading commit history…".to_string(), appearance),
            LoadState::Error(err) => {
                render_message(format!("Failed to load git history: {err}"), appearance)
            }
            LoadState::Loaded if self.commits.is_empty() => {
                render_message("No commits yet".to_string(), appearance)
            }
            LoadState::Loaded => {
                let commits = self.commits.clone();
                let layout = self.layout.clone();
                let list = UniformList::new(self.list_state.clone(), commits.len(), {
                    move |range, app| {
                        let appearance = Appearance::as_ref(app);
                        let lane_count = layout.max_lanes;
                        let rows: Vec<Box<dyn Element>> = range
                            .filter_map(|i| {
                                let commit = commits.get(i)?;
                                let row = layout.rows.get(i)?;
                                Some(render_graph_row(row, lane_count, commit, appearance))
                            })
                            .collect();
                        rows.into_iter()
                    }
                });
                Shrinkable::new(1.0, list.finish()).finish()
            }
        };

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_child(content)
            .finish()
    }
}
