use itertools::Itertools;
use warpui::{AppContext, SingletonEntity};

use super::workflow_search_item::WorkflowSearchItem;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObject, Space};
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::search::notebook_embedding::embedded_fuzzy_match::FuzzyMatchEmbeddedObjectResult;
use crate::search::notebook_embedding::is_embed_accessible;
use crate::search::notebook_embedding::searcher::EmbeddingSearchItemAction;
use crate::workflows::CloudWorkflow;

pub struct CloudWorkflowsDataSource {
    /// The space containing the object we are embedding into.
    embedding_space: Space,
    workflows: Vec<CloudWorkflow>,
}

impl CloudWorkflowsDataSource {
    pub fn new(notebook_space: Space, app: &mut AppContext) -> Self {
        let cloud_model = CloudModel::as_ref(app);
        Self {
            embedding_space: notebook_space,
            workflows: cloud_model
                .get_all_active_workflows()
                .filter(|workflow| workflow.id.into_server().is_some()) // Filter out local workflows.
                .cloned()
                .collect(),
        }
    }
}

impl SyncDataSource for CloudWorkflowsDataSource {
    type Action = EmbeddingSearchItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_str = query.text.as_str();
        Ok(self
            .workflows
            .clone()
            .into_iter()
            .filter_map(move |workflow| -> Option<QueryResult<Self::Action>> {
                FuzzyMatchEmbeddedObjectResult::try_match(
                    query_str,
                    workflow.model().data.name(),
                    workflow.breadcrumbs(app).as_str(),
                )
                .map(|match_result| {
                    let is_accessible =
                        is_embed_accessible(self.embedding_space, workflow.permissions.owner);
                    WorkflowSearchItem {
                        cloud_workflow: workflow,
                        fuzzy_matched_workflow: match_result,
                        is_accessible,
                    }
                    .into()
                })
            })
            .collect_vec())
    }
}

#[cfg(test)]
#[path = "workflows_data_source_tests.rs"]
mod tests;
