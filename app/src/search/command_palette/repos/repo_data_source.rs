use ai::workspace::WorkspaceMetadata;
use fuzzy_match::{FuzzyMatchResult, match_indices_case_insensitive};
use itertools::Itertools;
use warpui::{AppContext, Entity, SingletonEntity};

use super::RepoSearchItem;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

const MAX_REPOS_CONSIDERED: usize = 50;

pub struct RepoDataSource {}

impl Default for RepoDataSource {
    fn default() -> Self {
        Self::new()
    }
}

impl RepoDataSource {
    pub fn new() -> Self {
        Self {}
    }

    pub fn top_n(
        &self,
        limit: usize,
        app: &AppContext,
    ) -> impl Iterator<Item = RepoSearchItem> + use<> {
        PersistedWorkspace::as_ref(app)
            .workspaces()
            .filter(|cbm| cbm.path.is_dir())
            .sorted_by(WorkspaceMetadata::most_recently_navigated)
            .take(limit)
            .map(RepoSearchItem::new)
    }
}

impl Entity for RepoDataSource {
    type Event = ();
}

impl SyncDataSource for RepoDataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_str = query.text.as_str();

        let repos = self.top_n(MAX_REPOS_CONSIDERED, app);

        let results = repos
            .filter_map(|mut repo| {
                let match_result = if query_str.is_empty() {
                    Some(FuzzyMatchResult::no_match())
                } else {
                    match_indices_case_insensitive(repo.display_name.as_str(), query_str)
                };

                // Boost repo results so they compete fairly with other sources
                match_result.map(|mut match_result| {
                    match_result.score *= 4;
                    repo.match_result = match_result;
                    repo.into()
                })
            })
            .collect_vec();

        Ok(results)
    }
}
