//! Git Graph 视图。
//!
//! 渲染：左侧 panel 的提交图谱列表（泳道图 + 短 hash + 引用标签 + subject），
//! 点击某行加载并展示该提交详情（完整信息 + 变更文件）。
//!
//! 状态直接持有在视图内（单实例、无共享），不引入单独的 Model 间接层——待后续
//! 出现跨视图共享需求时再抽。

use std::sync::Arc;

use warpui::elements::{
    Container, CrossAxisAlignment, Element, Empty, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Shrinkable, Text, UniformList, UniformListState,
};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use warp_core::ui::Icon;
use warpui::ui_components::components::UiComponent;

use super::data::{ChangedFile, CommitDetail, CommitNode};
use super::layout::{assign_lanes, GraphLayout, GraphRow};
use super::row_canvas::GitGraphRowCanvas;
use crate::appearance::Appearance;
use crate::ui_components::buttons::icon_button;

/// 首屏加载的提交数上限（分页懒加载留待后续）。
const COMMIT_PAGE_SIZE: usize = 200;

/// 视图自身的 action。
#[derive(Debug, Clone)]
pub(crate) enum GitGraphAction {
    /// 选中列表中第 N 行提交并加载其详情。
    SelectCommit(usize),
    /// 手动重新加载当前仓库的图谱。
    Refresh,
}

/// 视图向外发出的事件。暂无。
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

/// 选中提交详情的加载状态。
enum DetailState {
    /// 未选中任何提交。
    None,
    Loading,
    Loaded(CommitDetail),
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
    /// 每行的鼠标状态句柄（供 [`Hoverable`] 点击/悬停使用），与 `commits` 等长。
    row_mouse_states: Arc<Vec<MouseStateHandle>>,
    /// 当前选中行下标。
    selected: Option<usize>,
    /// 选中提交的详情。
    detail: DetailState,
    /// 提交列表滚动状态。
    list_state: UniformListState,
    /// 详情区文件列表的滚动状态。
    detail_list_state: UniformListState,
    /// 刷新按钮的鼠标状态。
    refresh_mouse_state: MouseStateHandle,
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
            row_mouse_states: Arc::new(Vec::new()),
            selected: None,
            detail: DetailState::None,
            list_state: UniformListState::new(),
            detail_list_state: UniformListState::new(),
            refresh_mouse_state: MouseStateHandle::default(),
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

    /// 清空选中与详情（仓库变化/重新加载时调用）。
    fn clear_selection(&mut self) {
        self.selected = None;
        self.detail = DetailState::None;
    }

    /// 重新加载当前工作目录的提交图谱。
    fn reload(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_selection();

        let Some(dir) = self.working_dir.clone() else {
            self.commits = Arc::new(Vec::new());
            self.layout = Arc::new(empty_layout());
            self.row_mouse_states = Arc::new(Vec::new());
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
                            view.row_mouse_states =
                                Arc::new((0..commits.len()).map(|_| MouseStateHandle::default()).collect());
                            view.commits = Arc::new(commits);
                            view.state = LoadState::Loaded;
                        }
                        Err(err) => {
                            view.commits = Arc::new(Vec::new());
                            view.layout = Arc::new(empty_layout());
                            view.row_mouse_states = Arc::new(Vec::new());
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

    /// 选中某行并异步加载其详情。
    fn select_commit(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let Some(commit) = self.commits.get(index) else {
            return;
        };
        let hash = commit.hash.clone();
        self.selected = Some(index);
        self.detail = DetailState::Loading;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let Some(dir) = self.working_dir.clone() else {
                return;
            };
            ctx.spawn(
                async move { super::data::load_commit_detail(&dir, &hash).await },
                move |view, result, ctx| {
                    if view.selected != Some(index) {
                        // 选中已变化，丢弃过期结果。
                        return;
                    }
                    view.detail = match result {
                        Ok(detail) => DetailState::Loaded(detail),
                        Err(err) => DetailState::Error(err.to_string()),
                    };
                    ctx.notify();
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = hash;
            self.detail = DetailState::None;
            ctx.notify();
        }
    }

    /// 渲染可点击的提交列表（每行 = 泳道 + 文字，包一层 [`Hoverable`] 派发选中）。
    fn render_commit_list(&self) -> Box<dyn Element> {
        let commits = self.commits.clone();
        let layout = self.layout.clone();
        let mouse_states = self.row_mouse_states.clone();
        let list = UniformList::new(self.list_state.clone(), commits.len(), move |range, app| {
            let appearance = Appearance::as_ref(app);
            let lane_count = layout.max_lanes;
            let rows: Vec<Box<dyn Element>> = range
                .filter_map(|i| {
                    let commit = commits.get(i)?;
                    let row = layout.rows.get(i)?;
                    let element = render_graph_row(row, lane_count, commit, appearance);
                    let state = mouse_states.get(i).cloned().unwrap_or_default();
                    Some(
                        Hoverable::new(state, move |_| element)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(GitGraphAction::SelectCommit(i));
                            })
                            .finish(),
                    )
                })
                .collect();
            rows.into_iter()
        });
        list.finish()
    }

    /// 渲染选中提交的详情区。
    fn render_detail(&self, appearance: &Appearance) -> Box<dyn Element> {
        match &self.detail {
            DetailState::None => Empty::new().finish(),
            DetailState::Loading => render_message("Loading commit details…".to_string(), appearance),
            DetailState::Error(err) => {
                render_message(format!("Failed to load details: {err}"), appearance)
            }
            DetailState::Loaded(detail) => {
                let commit = self.selected.and_then(|i| self.commits.get(i));
                render_detail_body(commit, detail, &self.detail_list_state, appearance)
            }
        }
    }

    /// 顶部条：左侧提交计数 / 状态，右侧刷新按钮。
    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = match &self.state {
            LoadState::Loaded => format!("{} commits", self.commits.len()),
            LoadState::Loading => "Loading…".to_string(),
            _ => String::new(),
        };
        let refresh = icon_button(
            appearance,
            Icon::Refresh,
            false,
            self.refresh_mouse_state.clone(),
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GitGraphAction::Refresh);
        })
        .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(text_line(label, appearance, true))
                .with_child(refresh)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(6.)
        .finish()
    }
}

/// 一行普通文字（单行、不换行）。
fn text_line(text: String, appearance: &Appearance, dim: bool) -> Box<dyn Element> {
    let theme = appearance.theme();
    let color = if dim {
        theme.sub_text_color(theme.background())
    } else {
        theme.foreground()
    };
    Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
        .with_color(color.into())
        .finish()
}

/// 渲染居中的单行提示文案（用于空 / 加载中 / 错误等状态）。
fn render_message(text: String, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(text_line(text, appearance, true))
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

/// 渲染详情区主体：元信息 + 完整信息 + 变更文件列表。
fn render_detail_body(
    commit: Option<&CommitNode>,
    detail: &CommitDetail,
    list_state: &UniformListState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();

    let mut meta = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
    if let Some(c) = commit {
        meta = meta.with_child(text_line(c.hash.clone(), appearance, true));
        meta = meta.with_child(text_line(
            format!("{} <{}>", c.author_name, c.author_email),
            appearance,
            false,
        ));
        if detail.committer_name != c.author_name {
            meta = meta.with_child(text_line(
                format!("committed by {}", detail.committer_name),
                appearance,
                true,
            ));
        }
    }

    // 完整提交信息（可换行）。
    meta = meta.with_child(
        Container::new(
            Text::new(detail.message.clone(), font, size)
                .with_color(appearance.theme().foreground().into())
                .finish(),
        )
        .with_vertical_padding(6.)
        .finish(),
    );
    meta = meta.with_child(text_line(
        format!("{} changed files", detail.files.len()),
        appearance,
        true,
    ));

    // 文件列表（虚拟化、可滚动）。
    let files = Arc::new(detail.files.clone());
    let file_list = UniformList::new(list_state.clone(), files.len(), move |range, app| {
        let appearance = Appearance::as_ref(app);
        let rows: Vec<Box<dyn Element>> = range
            .filter_map(|i| files.get(i).map(|f| render_file_row(f, appearance)))
            .collect();
        rows.into_iter()
    });

    Container::new(
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(meta.finish())
            .with_child(Shrinkable::new(1.0, file_list.finish()).finish())
            .finish(),
    )
    .with_horizontal_padding(12.)
    .with_vertical_padding(8.)
    .finish()
}

/// 渲染一个变更文件行：路径 + `+增 -删`。
fn render_file_row(file: &ChangedFile, appearance: &Appearance) -> Box<dyn Element> {
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();
    let theme = appearance.theme();
    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline(file.path.clone(), font, size)
                    .with_color(theme.foreground().into())
                    .finish(),
            )
            .with_child(
                Container::new(
                    Text::new_inline(
                        format!("+{} -{}", file.additions, file.deletions),
                        font,
                        size,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_padding_left(8.)
                .finish(),
            )
            .finish(),
    )
    .with_vertical_padding(2.)
    .finish()
}

impl Entity for GitGraphView {
    type Event = GitGraphEvent;
}

impl TypedActionView for GitGraphView {
    type Action = GitGraphAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GitGraphAction::SelectCommit(index) => self.select_commit(*index, ctx),
            GitGraphAction::Refresh => self.reload(ctx),
        }
    }
}

impl View for GitGraphView {
    fn ui_name() -> &'static str {
        "GitGraphView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // 单层纵向 column 直接承接 panel 的有界高度；用 Shrinkable 因子在列表与详情之间
        // 分配空间（嵌套两层 MainAxisSize::Max 会导致内层收到无限约束而 panic）。
        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);

        // 有工作目录时显示顶部条（含刷新按钮）。
        if self.working_dir.is_some() {
            column = column.with_child(self.render_header(appearance));
        }

        column = match &self.state {
            LoadState::NoRepo => column.with_child(render_message(
                "Current directory is not a git repository".to_string(),
                appearance,
            )),
            LoadState::Loading => {
                column.with_child(render_message("Loading commit history…".to_string(), appearance))
            }
            LoadState::Error(err) => column.with_child(render_message(
                format!("Failed to load git history: {err}"),
                appearance,
            )),
            LoadState::Loaded if self.commits.is_empty() => {
                column.with_child(render_message("No commits yet".to_string(), appearance))
            }
            LoadState::Loaded if self.selected.is_some() => column
                // 列表与详情按 2:1 分配高度。
                .with_child(Shrinkable::new(2.0, self.render_commit_list()).finish())
                .with_child(Shrinkable::new(1.0, self.render_detail(appearance)).finish()),
            LoadState::Loaded => {
                column.with_child(Shrinkable::new(1.0, self.render_commit_list()).finish())
            }
        };

        column.finish()
    }
}
