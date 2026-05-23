//! Async data source for the inline repos menu.

use std::path::PathBuf;

use black_ui::{AppContext, Entity, SingletonEntity};

use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{AsyncDataSource, BoxFuture, DataSourceRunErrorWrapper};
use crate::terminal::input::repos::AcceptRepo;

pub struct RepoMenuDataSource;

impl RepoMenuDataSource {
    pub fn new() -> Self {
        Self {}
    }
}

impl AsyncDataSource for RepoMenuDataSource {
    type Action = AcceptRepo;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let workspace_paths: Vec<PathBuf> = PersistedWorkspace::as_ref(app)
            .workspaces()
            .map(|m| m.path)
            .collect();

        let query_text = query.text.trim().to_lowercase();

        Box::pin(async move {
            #[cfg(feature = "local_fs")]
            {
                use crate::terminal::input::repos::search_item::RepoSearchItem;
                use crate::util::git::get_repo_git_summary;

                let futures: Vec<_> = workspace_paths
                    .into_iter()
                    .map(|path| async move {
                        let summary = get_repo_git_summary(&path).await;
                        RepoSearchItem::new(path, summary)
                    })
                    .collect();

                let mut items: Vec<RepoSearchItem> = futures::future::join_all(futures).await;
                items.sort_by(|a, b| a.display_name.cmp(&b.display_name));

                let results: Vec<QueryResult<AcceptRepo>> = if query_text.is_empty() {
                    items.into_iter().map(QueryResult::from).collect()
                } else {
                    items
                        .into_iter()
                        .filter_map(|item| {
                            let match_result = fuzzy_match::match_indices_case_insensitive(
                                &item.display_name,
                                &query_text,
                            )?;
                            if match_result.score < 25 {
                                return None;
                            }
                            Some(QueryResult::from(
                                item.with_name_match_result(Some(match_result)),
                            ))
                        })
                        .collect()
                };

                Ok(results)
            }

            #[cfg(not(feature = "local_fs"))]
            {
                let _ = workspace_paths;
                let _ = query_text;
                Ok(vec![])
            }
        })
    }
}

impl Entity for RepoMenuDataSource {
    type Event = ();
}
