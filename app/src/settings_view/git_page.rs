//! "Git" 设置分页。包含 Git Graph 面板显隐开关与仓库扫描深度；后续 git 相关参数（分支
//! 过滤、连线样式等）作为新的 [`SettingsWidget`] 追加到 `widgets` 即可，页面结构无需再动。

use settings::{Setting as _, ToggleableSetting as _};
use warpui::elements::{Element, Flex};
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    add_setting, render_body_item, render_dropdown_item, MatchData, PageType, SettingsPageEvent,
    SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
};
use super::{LocalOnlyIconState, SettingsSection, ToggleState};
use crate::appearance::Appearance;
use crate::report_if_error;
use crate::settings::{GitSettings, GitSettingsChangedEvent};
use crate::view_components::{Dropdown, DropdownItem};

/// 仓库扫描深度的可选项（值 + 展示文案）。深度语义见 [`crate::settings::GitSettings`]。
/// 只提供 0–3 这几档常用值；更深的需求极少，手改 toml 仍可生效。
const SCAN_DEPTH_OPTIONS: &[(u32, &str)] = &[
    (0, "Current folder only"),
    (1, "1 level (direct subfolders)"),
    (2, "2 levels"),
    (3, "3 levels"),
];

/// 本页的 action。`PartialEq` 用于满足扫描深度下拉的 [`crate::view_components::dropdown::DropdownItemAction`] 约束。
#[derive(Clone, Debug, PartialEq)]
pub enum GitSettingsPageAction {
    /// 切换 Git Graph 面板显隐。
    ToggleShowGitGraph,
    /// 设置仓库扫描深度。
    SetScanDepth(u32),
}

pub struct GitSettingsPageView {
    page: PageType<Self>,
    /// 仓库扫描深度下拉（子视图，派发 [`GitSettingsPageAction::SetScanDepth`]）。
    scan_depth_dropdown: ViewHandle<Dropdown<GitSettingsPageAction>>,
}

impl GitSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let scan_depth_dropdown = ctx.add_typed_action_view(Dropdown::new);
        Self::update_scan_depth_dropdown(scan_depth_dropdown.clone(), ctx);

        // 深度在别处变化（云同步 / 重置 / 本页修改）时刷新下拉选中态。
        ctx.subscribe_to_model(&GitSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, GitSettingsChangedEvent::GitGraphScanDepth { .. }) {
                Self::update_scan_depth_dropdown(me.scan_depth_dropdown.clone(), ctx);
                ctx.notify();
            }
        });

        let widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(ShowGitGraphToggleWidget::default()),
            Box::new(ScanDepthWidget::default()),
        ];
        Self {
            page: PageType::new_uncategorized(widgets, None),
            scan_depth_dropdown,
        }
    }

    /// 用当前设置值刷新扫描深度下拉的菜单项与选中态。
    fn update_scan_depth_dropdown(
        dropdown: ViewHandle<Dropdown<GitSettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        dropdown.update(ctx, |dropdown, ctx| {
            let current = *GitSettings::as_ref(ctx).git_graph_scan_depth.value();
            let selected_index = SCAN_DEPTH_OPTIONS
                .iter()
                .position(|(value, _)| *value == current)
                .unwrap_or(0);
            dropdown.set_items(
                SCAN_DEPTH_OPTIONS
                    .iter()
                    .map(|(value, label)| {
                        DropdownItem::new(
                            label.to_string(),
                            GitSettingsPageAction::SetScanDepth(*value),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);
        });
    }
}

impl Entity for GitSettingsPageView {
    type Event = SettingsPageEvent;
}

impl View for GitSettingsPageView {
    fn ui_name() -> &'static str {
        "GitSettingsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for GitSettingsPageView {
    type Action = GitSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GitSettingsPageAction::ToggleShowGitGraph => {
                GitSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.show_git_graph.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            GitSettingsPageAction::SetScanDepth(depth) => {
                GitSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.git_graph_scan_depth.set_value(*depth, ctx));
                });
                ctx.notify();
            }
        }
    }
}

impl SettingsPageMeta for GitSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Git
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<GitSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<GitSettingsPageView>) -> Self {
        SettingsPageViewHandle::Git(view_handle)
    }
}

/// "Git Graph" 面板显隐开关。
#[derive(Default)]
struct ShowGitGraphToggleWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ShowGitGraphToggleWidget {
    type View = GitSettingsPageView;

    fn search_terms(&self) -> &str {
        "git graph commit history dag panel left tools"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let git_settings = GitSettings::as_ref(app);

        render_body_item::<GitSettingsPageAction>(
            "Git Graph".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*git_settings.show_git_graph)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(GitSettingsPageAction::ToggleShowGitGraph);
                })
                .finish(),
            Some("Adds a read-only commit graph (DAG) panel to the left side tools panel.".into()),
        )
    }
}

/// Git Graph 仓库扫描深度下拉。
#[derive(Default)]
struct ScanDepthWidget {}

impl SettingsWidget for ScanDepthWidget {
    type View = GitSettingsPageView;

    fn search_terms(&self) -> &str {
        "git graph repository scan depth subfolder subdirectory multiple repos"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();
        add_setting(
            &mut column,
            &GitSettings::as_ref(app).git_graph_scan_depth,
            || {
                render_dropdown_item::<GitSettingsPageAction>(
                    appearance,
                    "Repository scan depth:",
                    None,
                    None,
                    LocalOnlyIconState::Hidden,
                    None,
                    &view.scan_depth_dropdown,
                )
            },
        );
        column.finish()
    }
}
