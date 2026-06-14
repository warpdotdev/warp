//! The "Git" settings tab. Contains the Git Graph panel visibility toggle and the repository scan
//! depth; future git-related options (branch filtering, connector style, etc.) can simply be added
//! to `widgets` as a new [`SettingsWidget`], with no change to the page structure.

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

/// The selectable repository scan depth options (value + display label). See
/// [`crate::settings::GitSettings`] for the meaning of each depth. Only the common values 0–3 are
/// offered; deeper scans are rarely needed and can still be set by editing the toml directly.
const SCAN_DEPTH_OPTIONS: &[(u32, &str)] = &[
    (0, "Current folder only"),
    (1, "1 level (direct subfolders)"),
    (2, "2 levels"),
    (3, "3 levels"),
];

/// This page's action. `PartialEq` is required to satisfy the scan depth dropdown's
/// [`crate::view_components::dropdown::DropdownItemAction`] bound.
#[derive(Clone, Debug, PartialEq)]
pub enum GitSettingsPageAction {
    /// Toggle the visibility of the Git Graph panel.
    ToggleShowGitGraph,
    /// Set the repository scan depth.
    SetScanDepth(u32),
}

pub struct GitSettingsPageView {
    page: PageType<Self>,
    /// The repository scan depth dropdown (a subview that dispatches [`GitSettingsPageAction::SetScanDepth`]).
    scan_depth_dropdown: ViewHandle<Dropdown<GitSettingsPageAction>>,
}

impl GitSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let scan_depth_dropdown = ctx.add_typed_action_view(Dropdown::new);
        Self::update_scan_depth_dropdown(scan_depth_dropdown.clone(), ctx);

        // Refresh the dropdown selection when the depth changes elsewhere (cloud sync / reset / edits on this page).
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

    /// Refresh the scan depth dropdown's menu items and selection from the current setting value.
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

/// The Git Graph panel visibility toggle.
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

/// The Git Graph repository scan depth dropdown.
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
