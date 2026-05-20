use std::collections::HashSet;
use std::path::PathBuf;

use crate::drive::settings::WarpDriveSettings;
use crate::search::action::CommandBindingDataSource;
use crate::search::binding_source::BindingSource;
use crate::search::command_palette::files;
use crate::search::command_palette::launch_config;
use crate::search::command_palette::mixer::{CommandPaletteItemAction, ItemSummary};
use crate::search::command_palette::new_session::NewSessionDataSource;
use crate::search::command_palette::repos::RepoDataSource;
use crate::search::command_palette::{navigation, tabs, CommandPaletteMixer};
use crate::search::data_source::QueryResult;
use crate::search::files::model::FileSearchModel;
use crate::search::mixer::AddAsyncSourceOptions;
use crate::search::QueryFilter;
use crate::session_management::SessionSource;
use crate::settings::AISettings;
use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warpui::keymap::BindingId;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::conversations;
use super::warp_drive;

/// Store of all of the [`crate::search::DataSource`]s for the command palette.
pub struct DataSourceStore {
    actions_data_source: ModelHandle<CommandBindingDataSource>,
    sessions_data_source: ModelHandle<navigation::DataSource>,
    warp_drive_data_source: ModelHandle<warp_drive::DataSource>,
    launch_config_data_source: ModelHandle<launch_config::DataSource>,
    new_session_data_source: Option<ModelHandle<NewSessionDataSource>>,
    all_conversation_data_source: ModelHandle<conversations::DataSource>,
    repo_data_source: ModelHandle<RepoDataSource>,
    tabs_data_source: Option<ModelHandle<tabs::DataSource>>,
}

impl DataSourceStore {
    pub fn new(
        binding_source: ModelHandle<BindingSource>,
        active_session_handle: ModelHandle<SessionSource>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let actions_data_source =
            ctx.add_model(|ctx| CommandBindingDataSource::new(binding_source.clone(), ctx));

        let sessions_data_source =
            ctx.add_model(|_| navigation::DataSource::new(active_session_handle));

        let warp_drive_data_source = ctx.add_model(warp_drive::DataSource::new);

        let launch_config_data_source = ctx.add_model(launch_config::DataSource::new);

        let new_session_data_source = (FeatureFlag::ShellSelector.is_enabled()
            && cfg!(feature = "local_tty"))
        .then_some(ctx.add_model(|ctx| NewSessionDataSource::new(binding_source, ctx)));

        let all_conversation_data_source: ModelHandle<conversations::DataSource> =
            ctx.add_model(|_| conversations::DataSource::new());

        let repo_data_source = ctx.add_model(|_| RepoDataSource::new());

        Self {
            actions_data_source,
            sessions_data_source,
            warp_drive_data_source,
            launch_config_data_source,
            new_session_data_source,
            all_conversation_data_source,
            repo_data_source,
            tabs_data_source: None,
        }
    }

    /// Resets the [`CommandPaletteMixer`] to the set of data sources that are relevant for the command palette.
    pub fn reset_search_mixer(
        &mut self,
        mixer: ModelHandle<CommandPaletteMixer>,
        is_shared_session_viewer: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);

            if ContextFlag::LaunchConfigurations.is_enabled() {
                mixer.add_sync_source(
                    self.launch_config_data_source.clone(),
                    HashSet::from([QueryFilter::LaunchConfigurations]),
                );
            }

            mixer.add_sync_source(
                self.sessions_data_source.clone(),
                HashSet::from([QueryFilter::Sessions]),
            );

            if WarpDriveSettings::is_warp_drive_enabled(ctx) {
                let mut warp_drive_filters = HashSet::from([
                    QueryFilter::Notebooks,
                    QueryFilter::Plans,
                    QueryFilter::Drive,
                    QueryFilter::Workflows,
                ]);

                warp_drive_filters.insert(QueryFilter::EnvironmentVariables);

                if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    warp_drive_filters.insert(QueryFilter::AgentModeWorkflows);
                }
                mixer.add_sync_source(self.warp_drive_data_source.clone(), warp_drive_filters);
            }

            mixer.add_sync_source(
                self.actions_data_source.clone(),
                HashSet::from([QueryFilter::Actions]),
            );

            if let Some(new_session_data_source) = &self.new_session_data_source {
                mixer.add_sync_source(
                    new_session_data_source.clone(),
                    HashSet::from([QueryFilter::Actions]),
                );
            }

            if FeatureFlag::CommandPaletteFileSearch.is_enabled() && !is_shared_session_viewer {
                let file_search_model = FileSearchModel::as_ref(ctx);
                let repo_root = file_search_model.repo_root(ctx);
                let is_in_git_repo = repo_root.is_some();

                let files_data_source = if is_in_git_repo {
                    ctx.add_model(|_| files::data_source::FileDataSource::new())
                } else {
                    ctx.add_model(|ctx| files::data_source::FileDataSource::new_current_folder(ctx))
                };
                mixer.add_async_source(
                    files_data_source,
                    HashSet::from([QueryFilter::Files]),
                    AddAsyncSourceOptions {
                        debounce_interval: None,
                        run_in_zero_state: true,
                        run_when_unfiltered: true,
                    },
                    ctx,
                );
            }

            // Add conversation search if AI is enabled
            if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                mixer.add_sync_source(
                    self.all_conversation_data_source.clone(),
                    HashSet::from([QueryFilter::Conversations]),
                );
            }

            mixer.add_sync_source(
                self.repo_data_source.clone(),
                HashSet::from([QueryFilter::Repos]),
            );

            ctx.notify();
        });
    }

    /// Resets the [`CommandPaletteMixer`] to the set of data sources relevant for the Ctrl+Tab
    /// palette, which shows tabs sorted by MRU order.
    pub fn reset_ctrl_tab_mixer(
        &mut self,
        mixer: ModelHandle<CommandPaletteMixer>,
        tabs: Vec<crate::session_management::TabNavigationData>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.tabs_data_source.is_none() {
            self.tabs_data_source = Some(ctx.add_model(|_| tabs::DataSource::new()));
        }

        if let Some(tabs_data_source) = &self.tabs_data_source {
            tabs_data_source.update(ctx, |ds, _| ds.set_tabs(tabs));
            mixer.update(ctx, |mixer, ctx| {
                mixer.reset(ctx);
                mixer.add_sync_source(tabs_data_source.clone(), HashSet::from([QueryFilter::Tabs]));
                ctx.notify();
            });
        }
    }

    /// Restores the [`CommandPaletteMixer`] to the sessions-only source for Ctrl+Tab,
    /// undoing any previous `reset_ctrl_tab_mixer` call.
    pub fn restore_ctrl_tab_session_mixer(
        &self,
        mixer: ModelHandle<CommandPaletteMixer>,
        ctx: &mut ModelContext<Self>,
    ) {
        mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
            mixer.add_sync_source(
                self.sessions_data_source.clone(),
                HashSet::from([QueryFilter::Sessions]),
            );
            ctx.notify();
        });
    }

    /// Returns a [`QueryResult`] from the data sources identified by the `summary`. `None` if none
    /// of the data sources contained an item with given summary.
    pub fn query_result_from_summary(
        &self,
        summary: &ItemSummary,
        app: &AppContext,
    ) -> Option<QueryResult<CommandPaletteItemAction>> {
        match summary {
            ItemSummary::Action { binding_id } => self
                .actions_data_source
                .as_ref(app)
                .query_result(*binding_id),
            ItemSummary::Workflow { id } => self
                .warp_drive_data_source
                .as_ref(app)
                .query_result(id, app),
            ItemSummary::EnvVarCollection { id } => self
                .warp_drive_data_source
                .as_ref(app)
                .query_result(id, app),
            ItemSummary::Notebook { id } => self
                .warp_drive_data_source
                .as_ref(app)
                .query_result(id, app),
            ItemSummary::Session { pane_view_locator } => self
                .sessions_data_source
                .as_ref(app)
                .query_result(*pane_view_locator, app),
            ItemSummary::LaunchConfiguration => {
                // TODO(CLD-205): Launch configurations are not supported in the recent section of the
                // zero state yet.
                None
            }
            ItemSummary::CloudObject => {
                // We don't yet support all cloud objects in the command palette but
                // we have a `ViewInWarpDrive` action that supports all of them, so
                // this is necessary to make the compiler happy.
                None
            }
            ItemSummary::NewSession { id } => self
                .new_session_data_source
                .as_ref()
                .and_then(|source| source.as_ref(app).query_result(id)),
            ItemSummary::File {
                path,
                project_directory,
                line_and_column_arg,
            } => {
                // Create a file search item from the summary
                use crate::search::command_palette::files::search_item::FileSearchItem;
                use fuzzy_match::FuzzyMatchResult;

                let search_item = FileSearchItem {
                    path: PathBuf::from(path),
                    project_directory: project_directory.clone(),
                    match_result: FuzzyMatchResult::no_match(),
                    line_and_column_arg: *line_and_column_arg,
                    is_directory: false,
                };
                Some(QueryResult::from(search_item))
            }
            ItemSummary::Directory {
                path,
                project_directory,
            } => {
                // Create a directory search item from the summary
                use crate::search::command_palette::files::search_item::FileSearchItem;
                use fuzzy_match::FuzzyMatchResult;

                let search_item = FileSearchItem {
                    path: PathBuf::from(path),
                    project_directory: project_directory.clone(),
                    match_result: FuzzyMatchResult::no_match(),
                    line_and_column_arg: None,
                    is_directory: true,
                };
                Some(QueryResult::from(search_item))
            }
            ItemSummary::Project { path: _ } => {
                // For project summaries, we would need a project data source to reconstruct the item,
                // but this is typically handled by the welcome palette, not the command palette.
                // For now, return None as projects aren't expected in the regular command palette.
                None
            }
            ItemSummary::Conversation { id } => conversations::DataSource::query_result(id, app),

            ItemSummary::NewConversation => {
                // The new conversation item should not show up in the recent command list,
                // as its use is specific to the conversation filter.
                None
            }

            ItemSummary::ForkConversation => {
                // The forked conversation item should not show up in the recent command list,
                // as its use is specific to the conversation filter.
                None
            }

            ItemSummary::NoOp => {
                // No-op action (used for non-interactable separator items that don't do anything on click).
                None
            }

            ItemSummary::Tab { .. } => {
                // Tabs are only shown in the ctrl_tab palette, not in recent commands.
                None
            }
        }
    }

    /// Returns a [`QueryResult`] for a binding with `binding_id`. `None` if no result was found
    /// with the given ID.
    pub fn query_result_for_binding_id(
        &self,
        binding_id: BindingId,
        app: &AppContext,
    ) -> Option<QueryResult<CommandPaletteItemAction>> {
        self.query_result_from_summary(&ItemSummary::Action { binding_id }, app)
    }
}

impl Entity for DataSourceStore {
    type Event = ();
}

#[cfg(test)]
#[path = "data_sources_tests.rs"]
mod tests;
