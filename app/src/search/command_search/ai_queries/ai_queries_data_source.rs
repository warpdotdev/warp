use itertools::Itertools;
use warpui::{AppContext, SingletonEntity};

use super::AIQuerySearchResultItem;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::search::ai_queries::fuzzy_match::FuzzyMatchAIQueryResults;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

/// Manages querying the AI queries in history for Command Search.
pub struct AIQueriesDataSource {}

impl AIQueriesDataSource {
    pub fn new() -> Self {
        Self {}
    }
}

impl SyncDataSource for AIQueriesDataSource {
    type Action = CommandSearchItemAction;

    /// Performs a query on the AI queries in history and returns a collection of matches.
    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_str = query.text.as_str();

        let ai_queries: Vec<_> = BlocklistAIHistoryModel::as_ref(app)
            .all_ai_queries(None)
            .collect();
        // Only show the most recent query for each unique query text.
        // all_ai_queries() returns results sorted by start_time ascending, so reversing
        // before unique_by ensures we keep the most recent entry per query text.
        let mut unique_queries = ai_queries
            .into_iter()
            .rev()
            .unique_by(|query| query.query_text.clone())
            .collect_vec();
        // Reverse back to ascending start_time order (most recent on bottom).
        unique_queries.reverse();

        Ok(unique_queries
            .into_iter()
            .filter_map(|ai_query| -> Option<QueryResult<Self::Action>> {
                FuzzyMatchAIQueryResults::try_match(query_str, &ai_query.query_text).map(
                    |match_result| {
                        AIQuerySearchResultItem {
                            query_text: ai_query.query_text.to_owned(),
                            fuzzy_match_results: match_result,
                            start_time: ai_query.start_time,
                            output_status: ai_query.output_status,
                            working_directory: ai_query.working_directory,
                        }
                        .into()
                    },
                )
            })
            .collect_vec())
    }
}
