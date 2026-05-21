use itertools::Itertools;
use warpui::{AppContext, SingletonEntity};

use super::notebook_search_item::NotebookSearchItem;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObject, Space};
use crate::notebooks::CloudNotebook;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::search::notebook_embedding::embedded_fuzzy_match::FuzzyMatchEmbeddedObjectResult;
use crate::search::notebook_embedding::is_embed_accessible;
use crate::search::notebook_embedding::searcher::EmbeddingSearchItemAction;

pub struct CloudNotebooksDataSource {
    /// The space containing the object we are embedding into.
    embedding_space: Space,
    notebooks: Vec<CloudNotebook>,
}

impl CloudNotebooksDataSource {
    pub fn new(notebook_space: Space, app: &mut AppContext) -> Self {
        let cloud_model = CloudModel::as_ref(app);
        Self {
            embedding_space: notebook_space,
            notebooks: cloud_model
                .get_all_active_notebooks()
                .filter(|notebook| notebook.id.into_server().is_some()) // Filter out local notebooks.
                .cloned()
                .collect(),
        }
    }
}

impl SyncDataSource for CloudNotebooksDataSource {
    type Action = EmbeddingSearchItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_str = query.text.as_str();
        Ok(self
            .notebooks
            .clone()
            .into_iter()
            .filter_map(move |notebook| -> Option<QueryResult<Self::Action>> {
                FuzzyMatchEmbeddedObjectResult::try_match(
                    query_str,
                    &notebook.model().title,
                    notebook.breadcrumbs(app).as_str(),
                )
                .map(|match_result| {
                    let is_accessible =
                        is_embed_accessible(self.embedding_space, notebook.permissions.owner);
                    NotebookSearchItem {
                        cloud_notebook: notebook,
                        fuzzy_matched_notebook: match_result,
                        is_accessible,
                    }
                    .into()
                })
            })
            .collect_vec())
    }
}
