//! Settings page for the worktree panel and where git worktrees are created.
use std::cell::RefCell;
use std::collections::HashMap;

use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::settings::ToggleableSetting as _;
use warp_errors::report_if_error;
use warpui::elements::{ChildView, Element, MouseStateHandle};
use warpui::keymap::ContextPredicate;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    Action, AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle, id,
};

use super::settings_page::{
    LocalOnlyIconState, MatchData, PageTitle, PageType, SettingsPageMeta, SettingsPageViewHandle,
    SettingsWidget, render_body_item,
};
use super::{
    SettingActionPairContexts, SettingActionPairDescriptions, SettingsAction, SettingsSection,
    ToggleSettingActionPair, ToggleState, flags,
};
use crate::appearance::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::settings::{
    CustomWorktreeDirectory, DeleteWorktreeOnPrMerge, FetchBeforeCreatingWorktree,
    WorktreeLocation, WorktreeLocationSetting, WorktreePanelVisible, WorktreeSettings,
};
use crate::view_components::{Dropdown, DropdownItem};

const CUSTOM_DIRECTORY_INPUT_WIDTH: f32 = 280.;

#[derive(Clone, Debug, PartialEq)]
pub enum WorktreesPageAction {
    TogglePanelVisible,
    SetLocation(WorktreeLocation),
    ToggleFetchBeforeCreate,
    ToggleDeleteOnPrMerge,
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    if !FeatureFlag::WorktreePanel.is_enabled() {
        return;
    }
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new(
                    "Show worktrees in tools panel",
                    "Hide worktrees in tools panel",
                ),
                builder(SettingsAction::Worktrees(
                    WorktreesPageAction::TogglePanelVisible,
                )),
                SettingActionPairContexts::new(
                    context.clone() & !id!(flags::WORKTREE_PANEL_VISIBLE),
                    context.clone() & id!(flags::WORKTREE_PANEL_VISIBLE),
                ),
                None,
            ),
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new(
                    "Fetch from origin before creating worktrees",
                    "Don't fetch from origin before creating worktrees",
                ),
                builder(SettingsAction::Worktrees(
                    WorktreesPageAction::ToggleFetchBeforeCreate,
                )),
                SettingActionPairContexts::new(
                    context.clone() & !id!(flags::FETCH_BEFORE_CREATING_WORKTREE),
                    context.clone() & id!(flags::FETCH_BEFORE_CREATING_WORKTREE),
                ),
                None,
            ),
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new(
                    "Delete worktrees when their pull request is merged",
                    "Keep worktrees when their pull request is merged",
                ),
                builder(SettingsAction::Worktrees(
                    WorktreesPageAction::ToggleDeleteOnPrMerge,
                )),
                SettingActionPairContexts::new(
                    context.clone() & !id!(flags::DELETE_WORKTREE_ON_PR_MERGE),
                    context.clone() & id!(flags::DELETE_WORKTREE_ON_PR_MERGE),
                ),
                None,
            ),
        ],
        app,
    );
}

pub struct WorktreesSettingsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    location_dropdown: ViewHandle<Dropdown<WorktreesPageAction>>,
    custom_directory_editor: ViewHandle<EditorView>,
}

impl WorktreesSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let location_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(280.);
            dropdown
        });

        let ui_font_size = Appearance::as_ref(ctx).ui_font_size();
        let custom_directory_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(ui_font_size),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("~/worktrees", ctx);
            editor
        });
        ctx.subscribe_to_view(&custom_directory_editor, |me, _, event, ctx| {
            if matches!(event, EditorEvent::Blurred | EditorEvent::Enter) {
                me.save_custom_directory(ctx);
            }
        });

        ctx.subscribe_to_model(&WorktreeSettings::handle(ctx), |me, _, _, ctx| {
            me.sync_controls_from_settings(ctx);
            ctx.notify();
        });

        let mut view = Self {
            page: PageType::new_uncategorized(
                vec![
                    Box::new(WorktreePanelVisibleWidget::default()),
                    Box::new(WorktreeLocationWidget),
                    Box::new(CustomWorktreeDirectoryWidget),
                    Box::new(FetchBeforeCreatingWorktreeWidget::default()),
                    Box::new(DeleteWorktreeOnPrMergeWidget::default()),
                ],
                Some(PageTitle::new("Worktrees")),
            ),
            local_only_icon_tooltip_states: RefCell::new(HashMap::new()),
            location_dropdown,
            custom_directory_editor,
        };
        view.sync_controls_from_settings(ctx);
        view
    }

    fn sync_controls_from_settings(&mut self, ctx: &mut ViewContext<Self>) {
        let settings = WorktreeSettings::as_ref(ctx);
        let location = *settings.worktree_location;
        let custom_directory = settings.custom_worktree_directory.clone();

        self.location_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                WorktreeLocation::ALL
                    .into_iter()
                    .map(|location| {
                        DropdownItem::new(
                            location.as_dropdown_label(),
                            WorktreesPageAction::SetLocation(location),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_action(WorktreesPageAction::SetLocation(location), ctx);
        });

        let editor_text = self.custom_directory_editor.as_ref(ctx).buffer_text(ctx);
        if editor_text != custom_directory {
            self.custom_directory_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&custom_directory, ctx);
            });
        }
    }

    fn save_custom_directory(&mut self, ctx: &mut ViewContext<Self>) {
        let directory = self
            .custom_directory_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_owned();
        WorktreeSettings::handle(ctx).update(ctx, |settings, ctx| {
            if *settings.custom_worktree_directory != directory {
                report_if_error!(settings.custom_worktree_directory.set_value(directory, ctx));
            }
        });
    }

    fn local_only_icon_state(
        &self,
        storage_key: &str,
        sync_to_cloud: settings::SyncToCloud,
        app: &AppContext,
    ) -> LocalOnlyIconState {
        LocalOnlyIconState::for_setting(
            storage_key,
            sync_to_cloud,
            &mut self.local_only_icon_tooltip_states.borrow_mut(),
            app,
        )
    }
}

impl Entity for WorktreesSettingsPageView {
    type Event = ();
}

impl TypedActionView for WorktreesSettingsPageView {
    type Action = WorktreesPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WorktreesPageAction::TogglePanelVisible => {
                WorktreeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.worktree_panel_visible.toggle_and_save_value(ctx));
                });
            }
            WorktreesPageAction::SetLocation(location) => {
                WorktreeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.worktree_location.set_value(*location, ctx));
                });
            }
            WorktreesPageAction::ToggleFetchBeforeCreate => {
                WorktreeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .fetch_before_creating_worktree
                            .toggle_and_save_value(ctx)
                    );
                });
            }
            WorktreesPageAction::ToggleDeleteOnPrMerge => {
                WorktreeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .delete_worktree_on_pr_merge
                            .toggle_and_save_value(ctx)
                    );
                });
            }
        }
        ctx.notify();
    }
}

impl View for WorktreesSettingsPageView {
    fn ui_name() -> &'static str {
        "WorktreesSettingsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for WorktreesSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Worktrees
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::WorktreePanel.is_enabled() && cfg!(feature = "local_fs")
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

impl From<ViewHandle<WorktreesSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<WorktreesSettingsPageView>) -> Self {
        SettingsPageViewHandle::Worktrees(view_handle)
    }
}

#[derive(Default)]
struct WorktreePanelVisibleWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for WorktreePanelVisibleWidget {
    type View = WorktreesSettingsPageView;

    fn search_terms(&self) -> &str {
        "worktree worktrees tools panel sidebar show hide repository repositories"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let settings = WorktreeSettings::as_ref(app);
        render_body_item::<WorktreesPageAction>(
            "Show worktrees in tools panel".into(),
            None,
            view.local_only_icon_state(
                WorktreePanelVisible::storage_key(),
                WorktreePanelVisible::sync_to_cloud(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*settings.worktree_panel_visible)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(WorktreesPageAction::TogglePanelVisible);
                })
                .finish(),
            Some(
                "Adds a Worktrees view to the tools panel that lists repositories and their git \
                 worktrees, along with the status of coding agents running in each worktree."
                    .into(),
            ),
        )
    }
}

struct WorktreeLocationWidget;

impl SettingsWidget for WorktreeLocationWidget {
    type View = WorktreesSettingsPageView;

    fn search_terms(&self) -> &str {
        "worktree worktrees location directory folder path where create new"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_body_item::<WorktreesPageAction>(
            "Worktree location".into(),
            None,
            view.local_only_icon_state(
                WorktreeLocationSetting::storage_key(),
                WorktreeLocationSetting::sync_to_cloud(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ChildView::new(&view.location_dropdown).finish(),
            Some(
                "Where new worktrees are created. Each repository gets its own directory, with \
                 one subdirectory per worktree. When using the repository option, add \
                 .worktrees to your .gitignore."
                    .into(),
            ),
        )
    }
}

struct CustomWorktreeDirectoryWidget;

impl SettingsWidget for CustomWorktreeDirectoryWidget {
    type View = WorktreesSettingsPageView;

    fn search_terms(&self) -> &str {
        "worktree worktrees custom directory folder path location"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        *WorktreeSettings::as_ref(app).worktree_location == WorktreeLocation::CustomDirectory
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let input = appearance
            .ui_builder()
            .text_input(view.custom_directory_editor.clone())
            .with_style(UiComponentStyles {
                width: Some(CUSTOM_DIRECTORY_INPUT_WIDTH),
                padding: Some(Coords {
                    top: 4.,
                    bottom: 4.,
                    left: 6.,
                    right: 6.,
                }),
                background: Some(appearance.theme().surface_2().into()),
                ..Default::default()
            })
            .build()
            .finish();
        render_body_item::<WorktreesPageAction>(
            "Custom worktree directory".into(),
            None,
            view.local_only_icon_state(
                CustomWorktreeDirectory::storage_key(),
                CustomWorktreeDirectory::sync_to_cloud(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            input,
            Some(
                "Worktrees are created in <directory>/<repository>/<worktree>. Leave empty to \
                 use Warp's data directory."
                    .into(),
            ),
        )
    }
}

#[derive(Default)]
struct FetchBeforeCreatingWorktreeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for FetchBeforeCreatingWorktreeWidget {
    type View = WorktreesSettingsPageView;

    fn search_terms(&self) -> &str {
        "worktree worktrees fetch pull remote origin main latest update new create"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let settings = WorktreeSettings::as_ref(app);
        render_body_item::<WorktreesPageAction>(
            "Fetch from origin before creating a worktree".into(),
            None,
            view.local_only_icon_state(
                FetchBeforeCreatingWorktree::storage_key(),
                FetchBeforeCreatingWorktree::sync_to_cloud(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*settings.fetch_before_creating_worktree)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(WorktreesPageAction::ToggleFetchBeforeCreate);
                })
                .finish(),
            Some(
                "New worktrees start from the latest version of their base branch on origin \
                 (e.g. origin/main). Also adjustable in the New worktree dialog."
                    .into(),
            ),
        )
    }
}

#[derive(Default)]
struct DeleteWorktreeOnPrMergeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for DeleteWorktreeOnPrMergeWidget {
    type View = WorktreesSettingsPageView;

    fn search_terms(&self) -> &str {
        "worktree worktrees delete remove cleanup pull request pr merged merge github"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let settings = WorktreeSettings::as_ref(app);
        render_body_item::<WorktreesPageAction>(
            "Delete worktree when its pull request is merged".into(),
            None,
            view.local_only_icon_state(
                DeleteWorktreeOnPrMerge::storage_key(),
                DeleteWorktreeOnPrMerge::sync_to_cloud(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*settings.delete_worktree_on_pr_merge)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(WorktreesPageAction::ToggleDeleteOnPrMerge);
                })
                .finish(),
            Some(
                "Checks pull requests with the GitHub CLI (gh) every few minutes. Worktrees with \
                 uncommitted changes or an open terminal are never deleted, and the branch is kept."
                    .into(),
            ),
        )
    }
}
