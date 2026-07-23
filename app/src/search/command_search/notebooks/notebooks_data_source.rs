use std::sync::Arc;

use futures_lite::future::yield_now;
use warpui::{AppContext, SingletonEntity};

use super::NotebookSearchItem;
use crate::cloud_object::model::persistence::CloudModel;
use crate::notebooks::CloudNotebookModel;
use crate::notebooks::manager::NotebookManager;
use crate::search::async_snapshot_data_source::AsyncSnapshotDataSource;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{BoxFuture, DataSourceRunErrorWrapper};
use crate::server::ids::SyncId;

pub(crate) struct NotebookMatchCandidate {
    id: SyncId,
    model: Arc<CloudNotebookModel>,
    raw_text: Option<Arc<str>>,
}

pub(crate) struct NotebooksSnapshot {
    candidates: Vec<NotebookMatchCandidate>,
    query_text: String,
}

/// Creates an async data source for cloud notebooks.
///
/// The snapshot captures `Arc<CloudNotebookModel>` and `Arc<str>` (raw text) per notebook,
/// avoiding deep clones of notebook data on each keystroke.
pub fn notebooks_data_source() -> AsyncSnapshotDataSource<NotebooksSnapshot, CommandSearchItemAction>
{
    AsyncSnapshotDataSource::new(
        |query: &Query, app: &AppContext| {
            let notebook_manager = NotebookManager::as_ref(app);
            let candidates: Vec<NotebookMatchCandidate> = CloudModel::as_ref(app)
                .get_all_active_notebooks()
                .map(|notebook| NotebookMatchCandidate {
                    id: notebook.id,
                    model: notebook.shared_model(),
                    raw_text: notebook_manager.notebook_raw_text_shared(notebook.id),
                })
                .collect();

            NotebooksSnapshot {
                candidates,
                query_text: query.text.clone(),
            }
        },
        fuzzy_match_notebooks,
    )
}

pub(crate) fn fuzzy_match_notebooks(
    snapshot: NotebooksSnapshot,
) -> BoxFuture<'static, Result<Vec<QueryResult<CommandSearchItemAction>>, DataSourceRunErrorWrapper>>
{
    Box::pin(async move {
        let mut results = Vec::new();

        // Notebooks have the highest per-item cost (fuzzy matching against potentially long
        // content), so we use a smaller chunk size for tighter cancellation granularity.
        for chunk in snapshot.candidates.chunks(128) {
            for candidate in chunk {
                let searchable_text: &str = candidate
                    .raw_text
                    .as_deref()
                    .unwrap_or(&candidate.model.data);

                let name_match_result = fuzzy_match::match_indices_case_insensitive(
                    &candidate.model.title,
                    &snapshot.query_text,
                );
                let content_match_result = fuzzy_match::match_indices_case_insensitive(
                    searchable_text,
                    &snapshot.query_text,
                );

                if name_match_result.is_some() || content_match_result.is_some() {
                    results.push(
                        NotebookSearchItem {
                            id: candidate.id,
                            model: candidate.model.clone(),
                            name_match_result,
                            content_match_result,
                        }
                        .into(),
                    );
                }
            }
            yield_now().await;
        }

        Ok(results)
    })
}
