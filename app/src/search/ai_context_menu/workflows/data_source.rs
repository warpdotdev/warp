use fuzzy_match::FuzzyMatchResult;
use warpui::{AppContext, SingletonEntity};

use super::search_item::WorkflowSearchItem;
use crate::cloud_object::CloudModelType;
use crate::cloud_object::model::persistence::CloudModel;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::workspaces::user_workspaces::UserWorkspaces;

const MAX_RESULTS: usize = 50;
/// Base score for zero-state results. Each item gets an additional bonus based on
/// recency so the mixer's score-based ordering places more recent items higher.
const ZERO_STATE_BASE_SCORE: i64 = 1000;

pub struct WorkflowDataSource;

impl WorkflowDataSource {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }
}

impl SyncDataSource for WorkflowDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = &query.text;

        // Get all workflows from CloudModel
        let cloud_model = CloudModel::as_ref(app);
        let _user_workspaces = UserWorkspaces::as_ref(app);

        // Get workflows from all spaces the user has access to
        let mut workflow_results = Vec::new();

        // Collect non-welcome workflows, sorted by revision timestamp in zero state
        let mut workflows: Vec<_> = cloud_model
            .get_all_active_workflows()
            .filter(|w| !w.metadata.is_welcome_object)
            .collect();

        // Always sort by revision timestamp ascending so that position-based
        // scores assign higher values to more recently updated items. This ensures
        // recency acts as a tiebreaker when fuzzy scores are similar.
        workflows.sort_by(|a, b| {
            let a_ts = a.metadata.revision.as_ref().map(|r| r.timestamp());
            let b_ts = b.metadata.revision.as_ref().map(|r| r.timestamp());
            a_ts.cmp(&b_ts)
        });

        let total_workflows = workflows.len();
        for (index, workflow) in workflows.into_iter().enumerate() {
            let workflow_name = workflow.model().display_name();
            // Use workflow content for hover details, with first few lines as preview
            let workflow_content = workflow.model().data.content();
            let content_lines: Vec<&str> = workflow_content.lines().take(3).collect();
            let content_preview = content_lines.join("\n");
            let workflow_description = if content_preview.is_empty() {
                None
            } else {
                Some(if content_preview.len() > 200 {
                    format!("{}...", &content_preview[..197])
                } else {
                    content_preview
                })
            };
            let workflow_uid = workflow.id.uid();
            let recency_bonus = (30 * (index + 1) / total_workflows) as i64;

            let (match_result, is_match_on_name) = if query_text.is_empty() {
                // Zero state: score encodes recency so the mixer orders newest items highest.
                (
                    FuzzyMatchResult {
                        score: ZERO_STATE_BASE_SCORE + recency_bonus,
                        matched_indices: vec![],
                    },
                    false,
                )
            } else {
                // Fuzzy match against workflow name
                let name_match =
                    fuzzy_match::match_indices_case_insensitive(&workflow_name, query_text);

                // Also try matching against description if available
                let description_match = workflow_description
                    .as_deref()
                    .and_then(|desc| fuzzy_match::match_indices_case_insensitive(desc, query_text));

                // Use the best match, tracking whether it was on the name
                let (mut result, on_name) = match (name_match, description_match) {
                    (Some(name), Some(desc)) if desc.score > name.score => (desc, false),
                    (Some(name), _) => (name, true),
                    (None, Some(desc)) => (desc, false),
                    (None, None) => continue, // No match, skip this workflow
                };
                // Add a recency bonus (capped at 30) so more recently updated
                // items rank higher among results with similar fuzzy scores,
                // regardless of the total size of the workflows collection.
                result.score += recency_bonus;
                (result, on_name)
            };

            let search_item = WorkflowSearchItem {
                workflow_name,
                workflow_description,
                workflow_uid,
                match_result,
                is_match_on_name,
            };

            workflow_results.push(QueryResult::from(search_item));
        }

        // Sort by score and take the top results
        workflow_results.sort_by_key(|b| std::cmp::Reverse(b.score()));
        workflow_results.truncate(MAX_RESULTS);

        Ok(workflow_results)
    }
}

impl warpui::Entity for WorkflowDataSource {
    type Event = ();
}
