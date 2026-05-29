//! "Git" 设置分页。当前只有 Git Graph 面板显隐开关；后续 git 相关参数（分支过滤、
//! 连线样式等）作为新的 [`SettingsWidget`] 追加到 `widgets` 即可，页面结构无需再动。

use warp_core::settings::ToggleableSetting as _;
use warpui::elements::Element;
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    render_body_item, MatchData, PageType, SettingsPageEvent, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::{LocalOnlyIconState, SettingsSection, ToggleState};
use crate::appearance::Appearance;
use crate::report_if_error;
use crate::settings::GitSettings;

/// 本页的 action。
#[derive(Clone, Debug)]
pub enum GitSettingsPageAction {
    /// 切换 Git Graph 面板显隐。
    ToggleShowGitGraph,
}

pub struct GitSettingsPageView {
    page: PageType<Self>,
}

impl GitSettingsPageView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        let widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
            vec![Box::new(ShowGitGraphToggleWidget::default())];
        Self {
            page: PageType::new_uncategorized(widgets, None),
        }
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
