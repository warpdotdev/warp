//! Git Graph 视图。
//!
//! 渲染：左侧 panel 的提交图谱列表（泳道图 + 短 hash + 引用标签 + subject），
//! 点击某行加载并展示该提交详情（完整信息 + 变更文件）。
//!
//! 状态直接持有在视图内（单实例、无共享），不引入单独的 Model 间接层——待后续
//! 出现跨视图共享需求时再抽。

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_channel::Sender;
use pathfinder_color::ColorU;
use warpui::clipboard::ClipboardContent;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::elements::{
    resizable_state_handle, Align, ChildView, ClippedScrollStateHandle, ClippedScrollable,
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DragBarSide, Element, Empty, Fill,
    Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
    Resizable, ResizableStateHandle, ScrollbarWidth, SelectableArea, SelectionHandle, Shrinkable,
    Text, UniformList, UniformListState,
};
use warpui::keymap::macros::id;
use warpui::keymap::FixedBinding;
use warpui::units::Pixels;
use warpui::{
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use warp_core::ui::Icon;
use warpui::ui_components::components::UiComponent;

use super::data::{ChangedFile, CommitDetail, CommitNode, RefKind, RefLabel};
use super::layout::{assign_lanes, GraphLayout, GraphRow};
use super::row_canvas::GitGraphRowCanvas;
use crate::appearance::Appearance;
use crate::settings::{GitSettings, GitSettingsChangedEvent};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::item_highlight::ItemHighlightState;
use crate::view_components::dropdown::{Dropdown, DropdownItem};

/// 每页加载的提交数。
const COMMIT_PAGE_SIZE: usize = 200;

/// 距离列表末尾还剩这么多行时就预取下一页（无限滚动的提前量，避免滚到底才触发）。
const LOAD_MORE_PREFETCH: usize = 10;

/// 注册视图级键绑定：聚焦 Git Graph 面板时 Cmd/Ctrl+C 复制详情区选中的文本。
/// 作用域限定到本视图，不会影响终端等其它上下文的复制。
pub(crate) fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "cmdorctrl-c",
        GitGraphAction::CopySelection,
        id!(GitGraphView::ui_name()),
    )]);
}

/// 视图自身的 action。
/// 实现 `PartialEq` 以满足仓库下拉的 [`DropdownItemAction`] 约束。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GitGraphAction {
    /// 选中列表中第 N 行提交并加载其详情。
    SelectCommit(usize),
    /// 切换到发现列表中第 N 个仓库（多仓库时由顶部下拉派发）。
    SelectRepository(usize),
    /// 手动重新扫描工作目录并重新加载图谱。
    Refresh,
    /// 关闭详情区（取消选中）。
    CloseDetail,
    /// 把详情区当前选中的文本复制到剪贴板（Cmd/Ctrl+C）。
    CopySelection,
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
    /// 仓库发现的锚点目录（由左侧 panel 在活跃目录变化时推入）：在它自身所属仓库
    /// 之外，还会按 [`GitSettings::git_graph_scan_depth`] 向下扫描子目录里的独立仓库。
    scan_anchor: Option<PathBuf>,
    /// 发现到的仓库根列表（锚点所属仓库在最前）。多于 1 个时顶部展示仓库下拉。
    repositories: Arc<Vec<PathBuf>>,
    /// 当前选中（正在展示历史）的仓库在 `repositories` 中的下标。
    selected_repo: Option<usize>,
    /// 多仓库时顶部的仓库选择下拉（子视图，派发 [`GitGraphAction::SelectRepository`]）。
    repo_dropdown: ViewHandle<Dropdown<GitGraphAction>>,
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
    /// 详情区整体（提交信息 + 变更文件列表）的滚动状态：信息与文件统一在一个
    /// 可滚动区域内，长提交信息也能滚动查看完整内容。
    detail_scroll_state: ClippedScrollStateHandle,
    /// 刷新按钮的鼠标状态。
    refresh_mouse_state: MouseStateHandle,
    /// 是否可能还有更多提交可加载（上一页取满即认为有）。
    has_more: bool,
    /// 是否正在加载下一页（防重入）。
    loading_more: bool,
    /// 列表可见行区间的发送端：[`UniformList`] 上报可见区间，驱动滚动到底自动加载。
    visible_range_sender: Sender<Range<usize>>,
    /// 底部"加载更多"指示行的闪烁动画状态。
    loading_shimmer: ShimmeringTextStateHandle,
    /// 详情区高度的可拖动状态。
    detail_resizable_state: ResizableStateHandle,
    /// 详情区关闭按钮的鼠标状态。
    detail_close_mouse_state: MouseStateHandle,
    /// 详情区文本选区状态（拖拽框选），跨重渲染保持。
    detail_selection_handle: SelectionHandle,
    /// 详情区当前选中的文本，供 Cmd/Ctrl+C 复制；由 [`SelectableArea`] 的回调写入。
    detail_selected_text: Arc<RwLock<Option<String>>>,
}

/// 空布局，用于未加载/出错时。
fn empty_layout() -> GraphLayout {
    GraphLayout {
        rows: Vec::new(),
        max_lanes: 0,
    }
}

impl GitGraphView {
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        // UniformList 通过此 channel 上报当前可见行区间，触发滚动到底的自动加载。
        let (visible_range_sender, visible_range_receiver) = async_channel::unbounded();
        let _ = ctx.spawn_stream_local(
            visible_range_receiver,
            Self::on_visible_range,
            |_, _| {},
        );

        let repo_dropdown = ctx.add_typed_action_view(Dropdown::new);
        // 收缩到仓库名宽度，放进顶部条左侧时才不会撑满、把右侧刷新按钮挤出去。
        repo_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_main_axis_size(MainAxisSize::Min, ctx);
        });

        // 扫描深度变化时，对当前锚点重新发现仓库（用户在设置里调深度后面板即时生效）。
        ctx.subscribe_to_model(&GitSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, GitSettingsChangedEvent::GitGraphScanDepth { .. }) {
                me.discover(ctx);
            }
        });

        Self {
            scan_anchor: None,
            repositories: Arc::new(Vec::new()),
            selected_repo: None,
            repo_dropdown,
            commits: Arc::new(Vec::new()),
            layout: Arc::new(empty_layout()),
            state: LoadState::NoRepo,
            row_mouse_states: Arc::new(Vec::new()),
            selected: None,
            detail: DetailState::None,
            list_state: UniformListState::new(),
            detail_scroll_state: ClippedScrollStateHandle::new(),
            refresh_mouse_state: MouseStateHandle::default(),
            has_more: false,
            loading_more: false,
            visible_range_sender,
            loading_shimmer: ShimmeringTextStateHandle::new(),
            detail_resizable_state: resizable_state_handle(220.0),
            detail_close_mouse_state: MouseStateHandle::default(),
            detail_selection_handle: SelectionHandle::default(),
            detail_selected_text: Arc::new(RwLock::new(None)),
        }
    }

    /// 设置仓库发现的锚点目录；变化时触发重新发现仓库。
    pub(crate) fn set_working_directory(
        &mut self,
        dir: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.scan_anchor == dir {
            return;
        }
        self.scan_anchor = dir;
        self.discover(ctx);
    }

    /// 当前选中仓库的路径。
    fn current_repo_path(&self) -> Option<PathBuf> {
        self.selected_repo
            .and_then(|i| self.repositories.get(i).cloned())
    }

    /// 扫描锚点目录、发现其中的所有 git 仓库（异步），完成后填充仓库列表并加载选中仓库。
    /// 尽量保持原先选中的仓库（路径仍在新列表中则继续选它），否则默认选第一个。
    fn discover(&mut self, ctx: &mut ViewContext<Self>) {
        // 记住当前选中仓库，发现完成后尽量保持选中同一个。
        let previous = self.current_repo_path();
        self.clear_selection();

        let Some(anchor) = self.scan_anchor.clone() else {
            self.set_repositories(Vec::new(), None, ctx);
            return;
        };

        self.state = LoadState::Loading;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let depth = *GitSettings::as_ref(ctx).git_graph_scan_depth as usize;
            let expected = anchor.clone();
            ctx.spawn(
                async move { super::data::discover_repositories(&anchor, depth).await },
                move |view, repos, ctx| {
                    if view.scan_anchor.as_deref() != Some(expected.as_path()) {
                        // 锚点已切换，丢弃过期结果。
                        return;
                    }
                    // 保持原选中仓库；找不到则默认第一个。
                    let selected = previous
                        .and_then(|p| repos.iter().position(|r| *r == p))
                        .or_else(|| (!repos.is_empty()).then_some(0));
                    view.set_repositories(repos, selected, ctx);
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (anchor, previous);
            self.set_repositories(Vec::new(), None, ctx);
        }
    }

    /// 应用一次仓库发现的结果：更新列表与下拉，再加载选中仓库（无选中则进入 NoRepo 占位）。
    fn set_repositories(
        &mut self,
        repos: Vec<PathBuf>,
        selected: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.repositories = Arc::new(repos);
        self.selected_repo = selected;
        self.update_repo_dropdown(ctx);

        if self.selected_repo.is_some() {
            self.reload(ctx);
        } else {
            self.commits = Arc::new(Vec::new());
            self.layout = Arc::new(empty_layout());
            self.row_mouse_states = Arc::new(Vec::new());
            self.state = LoadState::NoRepo;
            ctx.notify();
        }
    }

    /// 用当前仓库列表与选中项刷新顶部仓库下拉的菜单项与选中态。
    fn update_repo_dropdown(&self, ctx: &mut ViewContext<Self>) {
        let repos = self.repositories.clone();
        let selected = self.selected_repo;
        self.repo_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                repos
                    .iter()
                    .enumerate()
                    .map(|(i, path)| {
                        // 展示目录名，悬停 tooltip 给出完整路径（同名仓库可借此区分）。
                        DropdownItem::new(repo_display_name(path), GitGraphAction::SelectRepository(i))
                            .with_tooltip(path.to_string_lossy().to_string())
                    })
                    .collect(),
                ctx,
            );
            if let Some(sel) = selected {
                dropdown.set_selected_by_index(sel, ctx);
            }
            // 只有一个仓库时无可切换项，置灰不可点（仅用于一致地展示当前仓库名）。
            if repos.len() <= 1 {
                dropdown.set_disabled(ctx);
            } else {
                dropdown.set_enabled(ctx);
            }
        });
    }

    /// 切换当前展示的仓库。
    ///
    /// 不在此同步调用 [`Self::update_repo_dropdown`]：本方法由下拉项点击经 `dispatch_typed_action`
    /// **同步**冒泡而来，此刻 [`Dropdown`] 视图正被其自身 `handle_action` 可变借用，再 `.update()`
    /// 它会重入借用而崩溃。表头选中态由 [`Dropdown`] 收到 `ItemSelected` 时自更新，无需我们干预；
    /// 列表/选中的权威重建只在异步的 [`Self::set_repositories`] 里做（不存在重入）。
    fn select_repository(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if self.selected_repo == Some(index) || index >= self.repositories.len() {
            return;
        }
        self.selected_repo = Some(index);
        self.reload(ctx);
    }

    /// 清空选中与详情（仓库变化/重新加载时调用）。
    fn clear_selection(&mut self) {
        self.selected = None;
        self.detail = DetailState::None;
        self.clear_detail_text_selection();
    }

    /// 清空详情区的文本框选状态（切换提交/关闭详情时调用，避免旧选区坐标残留）。
    fn clear_detail_text_selection(&mut self) {
        self.detail_selection_handle.clear();
        if let Ok(mut guard) = self.detail_selected_text.write() {
            *guard = None;
        }
    }

    /// 重新加载当前选中仓库的提交图谱。
    fn reload(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_selection();
        self.has_more = false;
        self.loading_more = false;
        // 重新加载会把提交重置回第一页，滚动位置也回到顶部（顶部即最新提交），
        // 否则用户滚到下面刷新后会停在中下部，被迫手动滚回。
        self.list_state.scroll_to(0);

        let Some(dir) = self.current_repo_path() else {
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
                    if view.current_repo_path().as_deref() != Some(expected.as_path()) {
                        // 仓库已切换，丢弃过期结果。
                        return;
                    }
                    match result {
                        Ok(commits) => {
                            view.has_more = commits.len() == COMMIT_PAGE_SIZE;
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
                            view.has_more = false;
                            let raw = err.to_string();
                            // 目录不在任何 git 仓库内时 `git log` 会报 "not a git repository"，
                            // 这不是错误，归一到 NoRepo 占位（而非展示吓人的原始报错）。
                            view.state = if raw.contains("not a git repository") {
                                LoadState::NoRepo
                            } else {
                                LoadState::Error(clean_git_error(&raw))
                            };
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

    /// 加载下一页提交并追加到列表末尾。
    fn load_more(&mut self, ctx: &mut ViewContext<Self>) {
        if self.loading_more || !self.has_more {
            return;
        }
        let Some(dir) = self.current_repo_path() else {
            return;
        };
        let skip = self.commits.len();
        self.loading_more = true;
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let expected = dir.clone();
            ctx.spawn(
                async move { super::data::load_commit_graph(&dir, COMMIT_PAGE_SIZE, skip).await },
                move |view, result, ctx| {
                    view.loading_more = false;
                    // 仓库已切换、或起始位置已变（被 reload 打断），丢弃过期结果。
                    if view.current_repo_path().as_deref() != Some(expected.as_path())
                        || view.commits.len() != skip
                    {
                        ctx.notify();
                        return;
                    }
                    match result {
                        Ok(batch) => {
                            view.has_more = batch.len() == COMMIT_PAGE_SIZE;
                            let mut combined = (*view.commits).clone();
                            combined.extend(batch);
                            view.layout = Arc::new(assign_lanes(&combined));
                            view.row_mouse_states = Arc::new(
                                (0..combined.len()).map(|_| MouseStateHandle::default()).collect(),
                            );
                            view.commits = Arc::new(combined);
                        }
                        Err(_) => {
                            view.has_more = false;
                        }
                    }
                    ctx.notify();
                },
            );
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (dir, skip);
            self.loading_more = false;
        }
    }

    /// [`UniformList`] 上报的当前可见行区间回调。可见区间逼近列表末尾且还有更多页时，
    /// 自动加载下一页（无限滚动）。`load_more` 自身做了防重入与"无更多页"的保护。
    fn on_visible_range(&mut self, range: Range<usize>, ctx: &mut ViewContext<Self>) {
        if range.end + LOAD_MORE_PREFETCH >= self.commits.len() {
            self.load_more(ctx);
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
        self.clear_detail_text_selection();
        // 切换提交后详情内容整体替换，滚动位置回到顶部（否则会停在上一个提交的偏移处）。
        self.detail_scroll_state.scroll_to(Pixels::zero());
        ctx.notify();

        #[cfg(not(target_family = "wasm"))]
        {
            let Some(dir) = self.current_repo_path() else {
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
    /// 渲染提交列表。还有更多页时末尾追加一行带闪烁动画的"加载更多"指示，滚动到它即
    /// 自动加载下一页（无限滚动）。
    fn render_commit_list(&self) -> Box<dyn Element> {
        let commits = self.commits.clone();
        let layout = self.layout.clone();
        let mouse_states = self.row_mouse_states.clone();
        let has_more = self.has_more;
        let shimmer = self.loading_shimmer.clone();
        let selected = self.selected;
        let commit_count = commits.len();
        let total = commit_count + usize::from(has_more);

        let list = UniformList::new(self.list_state.clone(), total, move |range, app| {
            let appearance = Appearance::as_ref(app);
            let lane_count = layout.max_lanes;
            let rows: Vec<Box<dyn Element>> = range
                .filter_map(|i| {
                    if i < commit_count {
                        let commit = commits.get(i)?;
                        let row = layout.rows.get(i)?;
                        let element = render_graph_row(row, lane_count, commit, appearance);
                        let state = mouse_states.get(i).cloned().unwrap_or_default();
                        let is_selected = selected == Some(i);
                        Some(
                            // 悬停/选中时套一层高亮背景（复用左侧面板列表通用的 [`ItemHighlightState`]：
                            // 悬停淡、选中略深，随鼠标进出即时切换）。
                            Hoverable::new(state, move |mouse_state| {
                                let highlight = ItemHighlightState::new(is_selected, mouse_state);
                                let mut container = Container::new(element);
                                if let Some(bg) = highlight.background_color(appearance) {
                                    container = container.with_background_color(bg.into_solid());
                                }
                                if let Some(radius) = highlight.corner_radius() {
                                    container = container.with_corner_radius(radius);
                                }
                                container.finish()
                            })
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(GitGraphAction::SelectCommit(i));
                            })
                            .finish(),
                        )
                    } else {
                        // 末行：加载更多指示（闪烁动画，滚动到此自动触发加载）。
                        Some(render_loading_more_row(appearance, shimmer.clone()))
                    }
                })
                .collect();
            rows.into_iter()
        })
        // 上报可见行区间，逼近末尾时由 on_visible_range 触发自动加载。
        .notify_visible_items(self.visible_range_sender.clone());
        list.finish()
    }

    /// 把详情区包进可拖动高度的 [`Resizable`]（顶部拖条上下拉），列表占其余空间。
    fn render_resizable_detail(&self, appearance: &Appearance) -> Box<dyn Element> {
        Resizable::new(
            self.detail_resizable_state.clone(),
            self.render_detail(appearance),
        )
        .with_dragbar_side(DragBarSide::Top)
        .on_resize(move |ctx, _| {
            ctx.notify();
        })
        .with_bounds_callback(Box::new(|window_size| {
            let min = 100.0;
            let max = (window_size.y() * 0.7).max(min);
            (min, max)
        }))
        .finish()
    }

    /// 渲染选中提交的详情区（顶部带关闭按钮）。
    fn render_detail(&self, appearance: &Appearance) -> Box<dyn Element> {
        let body: Box<dyn Element> = match &self.detail {
            DetailState::None => Empty::new().finish(),
            DetailState::Loading => render_message("Loading commit details…".to_string(), appearance),
            DetailState::Error(err) => {
                render_message(format!("Failed to load details: {err}"), appearance)
            }
            DetailState::Loaded(detail) => {
                let commit = self.selected.and_then(|i| self.commits.get(i));
                render_detail_body(
                    commit,
                    detail,
                    self.detail_scroll_state.clone(),
                    self.detail_selection_handle.clone(),
                    self.detail_selected_text.clone(),
                    appearance,
                )
            }
        };

        let close = icon_button(
            appearance,
            Icon::X,
            false,
            self.detail_close_mouse_state.clone(),
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GitGraphAction::CloseDetail);
        })
        .finish();

        let header = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(text_line("Commit details".to_string(), appearance, true))
                .with_child(close)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(4.)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(header)
            .with_child(Shrinkable::new(1.0, body).finish())
            .finish()
    }

    /// 顶部条：左侧为仓库选择下拉，右侧为刷新按钮。
    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        // 有仓库就显示下拉（单仓库时下拉被置灰、仅展示当前仓库名，保持布局一致）；
        // 无仓库（NoRepo）时左侧留空，不显示没有内容的空下拉。
        let left: Box<dyn Element> = if self.repositories.is_empty() {
            Empty::new().finish()
        } else {
            ChildView::new(&self.repo_dropdown).finish()
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
                .with_child(left)
                .with_child(refresh)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(6.)
        .finish()
    }
}

/// 仓库下拉里展示的名字：取目录名（完整路径由 tooltip 给出）。
fn repo_display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
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

/// 渲染列表底部的"加载更多"指示行：闪烁文字动画（[`ShimmeringTextElement`] 在 paint
/// 内自驱重绘，约 30fps），仅在还有更多页时出现，滚动到它即触发自动加载。
fn render_loading_more_row(
    appearance: &Appearance,
    shimmer: ShimmeringTextStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let bg = theme.background();
    let base_color = theme.sub_text_color(bg).into_solid();
    let shimmer_color = theme.foreground().into_solid();
    let text = ShimmeringTextElement::new(
        "Loading more commits…",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
        base_color,
        shimmer_color,
        ShimmerConfig::default(),
        shimmer,
    )
    .finish();
    Container::new(text)
        .with_horizontal_padding(12.)
        .with_vertical_padding(4.)
        .finish()
}

/// 把 `run_git_command` 的原始报错（形如 `Git command failed: <stderr>, <stdout>`）压成
/// 一行简洁文案：去掉前缀、只取首行、去掉尾部多余的逗号/空白。
fn clean_git_error(raw: &str) -> String {
    raw.strip_prefix("Git command failed: ")
        .unwrap_or(raw)
        .lines()
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_end_matches(',')
        .trim()
        .to_string()
}

/// 渲染详情区内的小提示文案（左对齐单行，用于详情加载中 / 出错）。
fn render_message(text: String, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(text_line(text, appearance, true))
        .with_horizontal_padding(12.)
        .with_vertical_padding(8.)
        .finish()
}

/// 渲染整块面板的占位状态：在剩余空间内垂直 + 水平居中，可选一个装饰图标、必有标题、
/// 可选副标题。用于 NoRepo / Loading / Error / 空仓库等"整屏"状态，避免文字挤在左上角。
fn render_centered_placeholder(
    icon: Option<Icon>,
    title: String,
    subtitle: Option<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // 内容块：图标/标题/副标题竖向堆叠、彼此水平居中（默认 MainAxisSize::Min，按内容收缩）。
    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Center);

    if let Some(icon) = icon {
        let icon_el = ConstrainedBox::new(
            icon.to_warpui_icon(theme.sub_text_color(theme.background()))
                .finish(),
        )
        .with_width(32.)
        .with_height(32.)
        .finish();
        content = content.with_child(Container::new(icon_el).with_vertical_padding(8.).finish());
    }

    content = content.with_child(
        Text::new_inline(title, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(theme.foreground().into())
            .finish(),
    );

    if let Some(subtitle) = subtitle {
        content = content.with_child(
            Container::new(
                Text::new(subtitle, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
            )
            .with_vertical_padding(4.)
            .with_horizontal_padding(24.)
            .finish(),
        );
    }

    // Shrinkable 占满剩余空间（外层是 MainAxisSize::Max column），Align 在该空间内把内容块
    // 两轴居中——这才有可居中的宽度，单靠 Flex 的 CrossAxisAlignment 会因列宽只裹文字而无效。
    Shrinkable::new(1.0, Align::new(content.finish()).finish()).finish()
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

    for ref_label in &commit.refs {
        row = row.with_child(render_ref_badge(ref_label, appearance));
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

/// 引用标签的徽标配色（按种类）。
fn ref_badge_color(kind: RefKind) -> ColorU {
    match kind {
        RefKind::Head => ColorU { r: 0x4e, g: 0xc9, b: 0x7a, a: 0xff }, // 绿
        RefKind::LocalBranch => ColorU { r: 0x4f, g: 0xc1, b: 0xff, a: 0xff }, // 蓝
        RefKind::RemoteBranch => ColorU { r: 0xd6, g: 0x7c, b: 0xff, a: 0xff }, // 紫
        RefKind::Tag => ColorU { r: 0xe6, g: 0xd2, b: 0x4f, a: 0xff }, // 黄
    }
}

/// 渲染一个引用标签徽标：圆角半透明底 + 同色文字，右侧留间距。
fn render_ref_badge(label: &RefLabel, appearance: &Appearance) -> Box<dyn Element> {
    let color = ref_badge_color(label.kind);
    let bg = ColorU { a: 0x33, ..color };
    let badge = Container::new(
        Text::new_inline(
            label.name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(color.into())
        .finish(),
    )
    .with_background_color(bg)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
    .with_horizontal_padding(5.)
    .with_vertical_padding(1.)
    .finish();

    Container::new(badge).with_padding_right(4.).finish()
}

/// 渲染详情区主体：可框选的元信息文本块 + 变更文件列表。
///
/// 元信息（完整 hash + 作者 + 提交者 + 提交信息）合并成单个可选 [`Text`]，外包
/// [`SelectableArea`] 以支持拖拽框选；选中文本写入 `selected_text`，由 Cmd/Ctrl+C
/// 复制。文件列表是虚拟化的，暂不参与框选。
fn render_detail_body(
    commit: Option<&CommitNode>,
    detail: &CommitDetail,
    scroll_state: ClippedScrollStateHandle,
    selection_handle: SelectionHandle,
    selected_text: Arc<RwLock<Option<String>>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let font = appearance.ui_font_family();
    let size = appearance.ui_font_size();

    // 把各段元信息拼成一个多行字符串：hash / 作者 / 提交者（若不同）/ 空行 / 完整信息。
    let mut meta_text = String::new();
    if let Some(c) = commit {
        meta_text.push_str(&c.hash);
        meta_text.push('\n');
        meta_text.push_str(&format!("{} <{}>", c.author_name, c.author_email));
        if detail.committer_name != c.author_name {
            meta_text.push('\n');
            meta_text.push_str(&format!("committed by {}", detail.committer_name));
        }
        meta_text.push('\n');
    }
    meta_text.push('\n');
    meta_text.push_str(detail.message.trim_end());

    let meta_element = Text::new(meta_text, font, size)
        .with_color(appearance.theme().foreground().into())
        .with_selectable(true)
        .finish();
    let selectable_meta = SelectableArea::new(
        selection_handle,
        move |args, _, _| {
            if let Ok(mut guard) = selected_text.write() {
                *guard = args.selection;
            }
        },
        meta_element,
    )
    .finish();

    let files_label = text_line(
        format!("{} changed files", detail.files.len()),
        appearance,
        true,
    );

    // 提交信息 + 标题 + 全部变更文件行拼成一列，整列交给 [`ClippedScrollable`] 统一滚动。
    // 不再虚拟化文件列表：单个提交的文件数有限，且把信息与文件放进同一个滚动区域，
    // 长提交信息才能和文件一起滚动查看完整内容（虚拟化的 UniformList 需要有界视口，
    // 无法嵌进按自然高度布局的滚动容器）。
    let mut content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(Container::new(selectable_meta).with_vertical_padding(6.).finish())
        .with_child(files_label);
    for file in detail.files.iter() {
        content = content.with_child(render_file_row(file, appearance));
    }

    let theme = appearance.theme();
    let scrollable = ClippedScrollable::vertical(
        scroll_state,
        content.finish(),
        ScrollbarWidth::Auto,
        theme.nonactive_ui_detail().into(),
        theme.active_ui_detail().into(),
        Fill::None,
    )
    .with_overlayed_scrollbar()
    .finish();

    Container::new(scrollable)
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
            GitGraphAction::SelectRepository(index) => self.select_repository(*index, ctx),
            // 手动刷新时重新扫描仓库（用户可能新增/删除了子仓库），再加载选中仓库。
            GitGraphAction::Refresh => self.discover(ctx),
            GitGraphAction::CloseDetail => {
                self.clear_selection();
                ctx.notify();
            }
            GitGraphAction::CopySelection => {
                let text = self
                    .detail_selected_text
                    .read()
                    .ok()
                    .and_then(|guard| guard.clone())
                    .filter(|t| !t.is_empty());
                if let Some(text) = text {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                }
            }
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

        // 有锚点目录时显示顶部条（仓库下拉 + 刷新按钮）。
        if self.scan_anchor.is_some() {
            column = column.with_child(self.render_header(appearance));
        }

        column = match &self.state {
            LoadState::NoRepo => column.with_child(render_centered_placeholder(
                Some(Icon::GitBranch),
                "Not a Git repository".to_string(),
                None,
                appearance,
            )),
            LoadState::Loading => column.with_child(render_centered_placeholder(
                None,
                "Loading commit history…".to_string(),
                None,
                appearance,
            )),
            LoadState::Error(err) => column.with_child(render_centered_placeholder(
                None,
                "Failed to load git history".to_string(),
                Some(err.clone()),
                appearance,
            )),
            LoadState::Loaded if self.commits.is_empty() => {
                column.with_child(render_centered_placeholder(
                    None,
                    "No commits yet".to_string(),
                    None,
                    appearance,
                ))
            }
            LoadState::Loaded if self.selected.is_some() => column
                // 列表填充上方空间；详情区高度可拖动（顶部拖条）。
                .with_child(Shrinkable::new(1.0, self.render_commit_list()).finish())
                .with_child(self.render_resizable_detail(appearance)),
            LoadState::Loaded => {
                column.with_child(Shrinkable::new(1.0, self.render_commit_list()).finish())
            }
        };

        column.finish()
    }
}
