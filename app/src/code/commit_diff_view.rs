//! 主区只读 diff pane 的视图：展示某个提交对单个文件的改动（`commit~1..commit`，即
//! "这一个提交自身改了什么"）。由 Git Graph 提交详情里点击变更文件触发。
//!
//! 复用编辑器的 diff 叠加机制：先把文件在**父提交处**的完整内容灌进编辑器作为 diff base，
//! 再叠加该提交对此文件的 deltas（由 unified diff hunks 转换而来）。**不注册 FileModel**
//! （不调用 [`InlineDiffView::register_file`]）→ 编辑器没有文件后端，保持只读、不可保存，
//! 避免把"历史版本"误存回工作区文件。

use std::path::Path;

use ai::diff_validation::DiffType;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::standardized_path::StandardizedPath;
use warpui::elements::ChildView;
use warpui::text_layout::ClipConfig;
use warpui::{
    AppContext, Element, Entity, ModelHandle, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::code::diff_viewer::{DiffViewer, DisplayMode};
use crate::code::editor::view::{CodeEditorEvent, CodeEditorRenderOptions, CodeEditorView};
use crate::code::inline_diff::InlineDiffView;
use crate::code_review::diff_state::{convert_hunks_to_diff_deltas, DiffHunk};
use crate::editor::InteractionState;
use crate::menu::{MenuItem, MenuItemFields};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions};
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};

/// commit diff pane 头部 overflow 菜单的 action。
#[derive(Debug, Clone)]
pub enum CommitDiffMenuAction {
    /// 最大化 / 还原本 pane。
    ToggleMaximized,
}

/// 只读地展示单个提交对单个文件的改动。
pub struct CommitDiffView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    /// 实际承载 diff 渲染的内联 diff 视图（编辑器 + 叠加的 deltas）。
    diff_view: ViewHandle<InlineDiffView>,
    /// 头部 / 标签标题：`文件名 @ 短 hash`。
    header_title: String,
}

impl CommitDiffView {
    /// `repo_relative_path` 仓库相对路径；`short_hash` 短 commit hash（仅用于标题）；
    /// `base_content` 文件在父提交处的完整内容（新增文件 / 根提交为空串）；
    /// `hunks` 该提交对此文件的 unified diff hunks。
    pub fn new(
        repo_relative_path: String,
        short_hash: String,
        base_content: String,
        hunks: Vec<DiffHunk>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let header_title = Self::compute_title(&repo_relative_path, &short_hash);

        let pane_configuration = ctx.add_model({
            let title = header_title.clone();
            // 标题用 set_title 设定（而非 ::new 传入）以保证标签立即渲染，参照 CodeDiffPane。
            move |ctx| {
                let mut cfg = PaneConfiguration::new("");
                cfg.set_title(title, ctx);
                cfg
            }
        });

        let diff_view = Self::build_diff_view(&repo_relative_path, &base_content, &hunks, ctx);

        Self {
            pane_configuration,
            focus_handle: None,
            diff_view,
            header_title,
        }
    }

    /// 复用同一个 pane 打开另一个文件的 diff：替换内容、更新标题，**保持 pane 不被销毁**
    /// （header / 关闭按钮 / 焦点状态都不变）。供"连点多个文件复用同一 diff pane"使用。
    pub fn load(
        &mut self,
        repo_relative_path: String,
        short_hash: String,
        base_content: String,
        hunks: Vec<DiffHunk>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.header_title = Self::compute_title(&repo_relative_path, &short_hash);
        self.pane_configuration.update(ctx, {
            let title = self.header_title.clone();
            move |cfg, ctx| cfg.set_title(title, ctx)
        });
        self.diff_view = Self::build_diff_view(&repo_relative_path, &base_content, &hunks, ctx);
        ctx.notify();
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// 头部 / 标签标题：`文件名 @ 短 hash`。
    fn compute_title(repo_relative_path: &str, short_hash: &str) -> String {
        let file_name = Path::new(repo_relative_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo_relative_path.to_string());
        format!("{file_name} @ {short_hash}")
    }

    /// 构建只读 diff 视图：编辑器灌入父版本完整内容作 base，叠加该提交的 deltas；
    /// 不注册 FileModel（无文件后端 → 不可保存），并强制 Selectable（FullPane 默认可编辑）。
    fn build_diff_view(
        repo_relative_path: &str,
        base_content: &str,
        hunks: &[DiffHunk],
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<InlineDiffView> {
        let editor = ctx.add_typed_action_view(|ctx| {
            CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::FillMaxHeight).lazy_layout(),
                ctx,
            )
        });
        editor.update(ctx, |editor_view, ctx| {
            editor_view.set_language_with_local_path(Path::new(repo_relative_path), ctx);
            editor_view.reset(InitialBufferState::plain_text(base_content), ctx);
        });

        let diff_type = DiffType::update(convert_hunks_to_diff_deltas(hunks), None);
        let standardized_path = StandardizedPath::try_new(repo_relative_path).ok();
        let diff_view = ctx.add_typed_action_view(|ctx| {
            InlineDiffView::new(
                editor.clone(),
                Some(diff_type),
                Some(DisplayMode::FullPane),
                standardized_path,
                ctx,
            )
        });

        diff_view.update(ctx, |view, ctx| {
            view.editor().clone().update(ctx, |editor_view, ctx| {
                editor_view.set_interaction_state(InteractionState::Selectable, ctx);
            });
        });

        // 点击编辑器内容获得焦点时，把焦点上抛为 PaneEvent::FocusSelf，让 pane group 激活本 pane
        // （否则点击 diff 内容区不激活、左上角无活跃标记——点击被编辑器选区消费、不会冒泡到 pane）。
        let editor = diff_view.as_ref(ctx).editor().clone();
        ctx.subscribe_to_view(&editor, |_me, _editor, event, ctx| {
            if matches!(event, CodeEditorEvent::Focused) {
                ctx.emit(PaneEvent::FocusSelf);
            }
        });

        diff_view
    }
}

impl Entity for CommitDiffView {
    type Event = PaneEvent;
}

impl TypedActionView for CommitDiffView {
    type Action = ();

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl View for CommitDiffView {
    fn ui_name() -> &'static str {
        "CommitDiffView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.diff_view).finish()
    }
}

impl BackingView for CommitDiffView {
    type PaneHeaderOverflowMenuAction = CommitDiffMenuAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            CommitDiffMenuAction::ToggleMaximized => ctx.emit(PaneEvent::ToggleMaximized),
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        let editor = self.diff_view.as_ref(ctx).editor().clone();
        ctx.focus(&editor);
    }

    fn pane_header_overflow_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<Self::PaneHeaderOverflowMenuAction>> {
        let is_maximized = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_maximized(ctx));
        vec![MenuItemFields::toggle_pane_action(is_maximized)
            .with_on_select_action(CommitDiffMenuAction::ToggleMaximized)
            .into_item()]
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::Standard(StandardHeader {
            title: self.header_title.clone(),
            title_secondary: None,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            // 关闭按钮与 overflow 菜单常显（不依赖悬停），便于关闭/最大化只读 diff pane。
            options: StandardHeaderOptions {
                always_show_icons: true,
                ..StandardHeaderOptions::default()
            },
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
