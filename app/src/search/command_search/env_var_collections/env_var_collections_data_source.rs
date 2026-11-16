use itertools::Itertools;
use warpui::{AppContext, SingletonEntity};

use super::EnvVarCollectionSearchItem;
use crate::cloud_object::model::persistence::CloudModel;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::env_var_collections::fuzzy_match::FuzzyMatchEnvVarCollectionResult;
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

pub struct EnvVarCollectionDataSource {}

impl EnvVarCollectionDataSource {
    /// Creates a new EnvVarCollectionDataSource containing personal and team EVCs.
    pub fn new() -> Self {
        Self {}
    }
}

impl SyncDataSource for EnvVarCollectionDataSource {
    type Action = CommandSearchItemAction;

    /// Runs fuzzy matching of the query against all EVCs (specifically, against their names and descriptions).
    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_str = query.text.as_str();
        let env_var_collections = CloudModel::as_ref(app).get_all_active_env_var_collections();

        Ok(env_var_collections
            .flat_map(
                move |env_var_collection| -> Option<QueryResult<Self::Action>> {
                    FuzzyMatchEnvVarCollectionResult::try_match(
                        query_str,
                        &env_var_collection.model().string_model.clone(),
                        "",
                    )
                    .map(|match_result| {
                        EnvVarCollectionSearchItem {
                            env_var_collection: env_var_collection.clone(),
                            fuzzy_matched_env_var_collection: match_result,
                        }
                        .into()
                    })
                },
            )
            .collect_vec())
    }
}
