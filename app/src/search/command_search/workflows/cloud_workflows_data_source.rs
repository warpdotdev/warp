use std::sync::Arc;

use futures_lite::future::yield_now;
use warpui::{AppContext, SingletonEntity};

use super::WorkflowSearchItem;
use crate::cloud_object::model::persistence::CloudModel;
use crate::search::QueryFilter;
use crate::search::async_snapshot_data_source::AsyncSnapshotDataSource;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{BoxFuture, DataSourceRunErrorWrapper};
use crate::search::workflows::fuzzy_match::FuzzyMatchWorkflowResult;
use crate::server::ids::SyncId;
use crate::settings::AISettings;
use crate::workflows::{CloudWorkflowModel, WorkflowSource};
use crate::workspaces::user_workspaces::UserWorkspaces;

pub(crate) struct WorkflowMatchCandidate {
    pub id: SyncId,
    pub model: Arc<CloudWorkflowModel>,
    pub source: WorkflowSource,
}

pub(crate) struct CloudWorkflowsSnapshot {
    candidates: Vec<WorkflowMatchCandidate>,
    query_text: String,
    filter_to_agent_mode: bool,
    filter_to_command_workflows: bool,
}

/// Creates an async data source for cloud workflows (i.e. those that exist in Warp Drive).
pub fn cloud_workflows_data_source()
-> AsyncSnapshotDataSource<CloudWorkflowsSnapshot, CommandSearchItemAction> {
    AsyncSnapshotDataSource::new(
        |query: &Query, app: &AppContext| {
            let is_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
            let filter_to_agent_mode = query.filters.contains(&QueryFilter::AgentModeWorkflows);
            let filter_to_command_workflows =
                query.filters.contains(&QueryFilter::Workflows) || !is_ai_enabled;

            let cloud_model = CloudModel::as_ref(app);
            let user_workspaces = UserWorkspaces::as_ref(app);

            let candidates: Vec<WorkflowMatchCandidate> = user_workspaces
                .all_user_spaces(app)
                .into_iter()
                .flat_map(|space| {
                    let source: WorkflowSource = space.into();
                    cloud_model
                        .active_workflows_in_space(space, app)
                        .map(move |cloud_workflow| WorkflowMatchCandidate {
                            id: cloud_workflow.id,
                            model: cloud_workflow.shared_model(),
                            source,
                        })
                })
                .collect();

            CloudWorkflowsSnapshot {
                candidates,
                query_text: query.text.clone(),
                filter_to_agent_mode,
                filter_to_command_workflows,
            }
        },
        fuzzy_match_cloud_workflows,
    )
}

pub(crate) fn fuzzy_match_cloud_workflows(
    snapshot: CloudWorkflowsSnapshot,
) -> BoxFuture<'static, Result<Vec<QueryResult<CommandSearchItemAction>>, DataSourceRunErrorWrapper>>
{
    Box::pin(async move {
        let mut results = Vec::new();

        // Workflows are a small dataset with moderate per-item cost (4 fuzzy matches against
        // short strings), so we use a medium chunk size.
        for chunk in snapshot.candidates.chunks(256) {
            for candidate in chunk {
                let is_agent_mode = candidate.model.data.is_agent_mode_workflow();

                if (snapshot.filter_to_command_workflows && is_agent_mode)
                    || (snapshot.filter_to_agent_mode && !is_agent_mode)
                {
                    continue;
                }

                if let Some(match_result) = FuzzyMatchWorkflowResult::try_match(
                    &snapshot.query_text,
                    &candidate.model.data,
                    "",
                ) {
                    results.push(
                        WorkflowSearchItem {
                            identity: super::WorkflowIdentity::Cloud {
                                id: candidate.id,
                                model: candidate.model.clone(),
                            },
                            source: candidate.source,
                            fuzzy_matched_workflow: match_result,
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
