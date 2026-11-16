use fuzzy_match::match_indices_case_insensitive;
use itertools::Itertools;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::ProjectSearchItem;
use crate::projects::ProjectManagementModel;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

/// Manages querying projects for Command Search (existing, user-added projects only).
pub struct ProjectDataSource {
    project_model: ModelHandle<ProjectManagementModel>,
}

impl ProjectDataSource {
    /// Creates a new ProjectsDataSource with access to the project management model.
    pub fn new(app: &mut ModelContext<Self>) -> Self {
        Self {
            project_model: ProjectManagementModel::handle(app),
        }
    }

    pub fn top_n(
        &self,
        limit: usize,
        app: &AppContext,
    ) -> impl Iterator<Item = ProjectSearchItem> + use<> {
        // Create search items and sort them using the Ord implementation
        self.project_model
            .as_ref(app)
            .all_projects()
            .map(|project| {
                ProjectSearchItem::new(
                    project.path.clone(),
                    fuzzy_match::FuzzyMatchResult::no_match(),
                    project.last_used_at(),
                )
            })
            .k_largest(limit)
    }
}

impl Entity for ProjectDataSource {
    type Event = ();
}

impl SyncDataSource for ProjectDataSource {
    type Action = CommandPaletteItemAction;

    /// Performs a query on the projects and returns a collection of matches.
    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_str = query.text.as_str();

        // Get all projects from the project management model
        let projects = self
            .project_model
            .as_ref(app)
            .all_projects()
            .collect::<Vec<_>>();

        // Create search items and sort them using the Ord implementation
        let mut search_items: Vec<ProjectSearchItem> = projects
            .into_iter()
            .filter_map(|project| {
                let mut search_item = ProjectSearchItem::new(
                    project.path.clone(),
                    fuzzy_match::FuzzyMatchResult::no_match(),
                    project.last_used_at(),
                );

                // Perform fuzzy matching on the project display name
                let match_result = if query_str.is_empty() {
                    // If query is empty, show all projects with no highlighting
                    Some(fuzzy_match::FuzzyMatchResult::no_match())
                } else {
                    // Try to match the project name against the query
                    match_indices_case_insensitive(search_item.name.as_str(), query_str)
                };

                match_result.map(|mut match_result| {
                    // This is a hack to make sure these results have higher priority than other
                    // searchable items in the welcome palette. The tantivy search implementation
                    // tends to score things higher than fuzzy_match.
                    match_result.score *= 4;
                    search_item.match_result = match_result;

                    search_item
                })
            })
            .collect_vec();

        // Reverse sort to get the best matches first
        search_items.sort_by(|a, b| b.cmp(a));

        // Convert to QueryResult after sorting
        let results = search_items
            .into_iter()
            .map(QueryResult::from)
            .collect_vec();

        Ok(results)
    }
}
