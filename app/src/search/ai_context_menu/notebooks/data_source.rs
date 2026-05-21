use super::search_item::NotebookSearchItem;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::CloudModelType;
use crate::notebooks::manager::{NotebookManager, NotebookSource};
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::workspaces::user_workspaces::UserWorkspaces;
use fuzzy_match::FuzzyMatchResult;
use warpui::{AppContext, SingletonEntity};

const MAX_RESULTS: usize = 50;
/// Base score for zero-state results. Each item gets an additional bonus based on
/// recency so the mixer's score-based ordering places more recent items higher.
const ZERO_STATE_BASE_SCORE: i64 = 1000;

pub struct NotebookDataSource {
    is_plan: bool,
}

impl NotebookDataSource {
    #[allow(dead_code)]
    pub fn new(is_plan: bool) -> Self {
        Self { is_plan }
    }
}

impl SyncDataSource for NotebookDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = &query.text;

        // Get all notebooks from CloudModel
        let cloud_model = CloudModel::as_ref(app);
        let _user_workspaces = UserWorkspaces::as_ref(app);

        // Get notebooks from all spaces the user has access to
        let mut notebook_results = Vec::new();
        let notebook_manager = NotebookManager::as_ref(app);

        let mut notebooks: Vec<_> = cloud_model
            .get_all_active_notebooks()
            .filter(|notebook| {
                // Notebooks and plans have separate filters.
                self.is_plan == notebook.model().ai_document_id.is_some()
            })
            .filter(|notebook| !notebook.metadata.is_welcome_object)
            .collect();

        // Always sort by revision timestamp ascending so that position-based
        // scores assign higher values to more recently updated items. This ensures
        // recency acts as a tiebreaker when fuzzy scores are similar.
        notebooks.sort_by(|a, b| {
            let a_ts = a.metadata.revision.as_ref().map(|r| r.timestamp());
            let b_ts = b.metadata.revision.as_ref().map(|r| r.timestamp());
            a_ts.cmp(&b_ts)
        });

        let total_notebooks = notebooks.len();
        for (index, notebook) in notebooks.into_iter().enumerate() {
            let notebook_name = notebook.model().display_name();
            // Use the first few lines of raw text (without markdown) as description for hover info
            let raw_text = notebook_manager
                .notebook_raw_text(notebook.id)
                .unwrap_or(notebook.model().data.as_str());
            let content_lines: Vec<&str> = raw_text.lines().take(3).collect();
            let content_preview = content_lines.join("\n");
            let notebook_description = if content_preview.is_empty() {
                None
            } else {
                Some(if content_preview.len() > 200 {
                    // Use char_indices to find the last valid character boundary before position 197
                    let truncated = content_preview
                        .char_indices()
                        .take_while(|(i, _)| *i <= 197)
                        .last()
                        .map(|(i, c)| &content_preview[..i + c.len_utf8()])
                        .unwrap_or("");
                    format!("{truncated}...")
                } else {
                    content_preview
                })
            };
            let notebook_uid = notebook.id.uid();

            // Check if this notebook is currently open
            let is_open = notebook_manager
                .find_pane(&NotebookSource::Existing(notebook.id))
                .is_some();
            let recency_bonus = (30 * (index + 1) / total_notebooks) as i64;

            let (base_match_result, is_match_on_name) = if query_text.is_empty() {
                // Zero state: score encodes recency so the mixer orders newest items highest.
                (
                    FuzzyMatchResult {
                        score: ZERO_STATE_BASE_SCORE + recency_bonus,
                        matched_indices: vec![],
                    },
                    false,
                )
            } else {
                // Fuzzy match against notebook name
                let name_match =
                    fuzzy_match::match_indices_case_insensitive(&notebook_name, query_text);

                // Also try matching against description if available
                let description_match = notebook_description
                    .as_deref()
                    .and_then(|desc| fuzzy_match::match_indices_case_insensitive(desc, query_text));

                // Use the best match, tracking whether it was on the name
                let (mut result, on_name) = match (name_match, description_match) {
                    (Some(name), Some(desc)) if desc.score > name.score => (desc, false),
                    (Some(name), _) => (name, true),
                    (None, Some(desc)) => (desc, false),
                    (None, None) => continue, // No match, skip this notebook
                };
                // Add a recency bonus, capped at 30.
                result.score += recency_bonus;
                (result, on_name)
            };
            let mut match_result = base_match_result;

            // Heavily prioritize open notebooks by adding a large bonus to their score
            if is_open {
                match_result.score += 10000;
            }

            let ai_document_uid = notebook.model().ai_document_id;
            let search_item = NotebookSearchItem {
                notebook_name,
                notebook_description,
                notebook_uid,
                match_result,
                ai_document_uid: ai_document_uid.map(|id| id.to_string()),
                is_match_on_name,
            };

            notebook_results.push(QueryResult::from(search_item));
        }

        // Sort by score and take the top results
        notebook_results.sort_by_key(|b| std::cmp::Reverse(b.score()));
        notebook_results.truncate(MAX_RESULTS);

        Ok(notebook_results)
    }
}

impl warpui::Entity for NotebookDataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
