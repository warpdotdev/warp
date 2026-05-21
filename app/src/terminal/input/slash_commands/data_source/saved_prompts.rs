use std::sync::Arc;

use futures_lite::future::yield_now;
use ordered_float::OrderedFloat;
use warp_core::ui::appearance::Appearance;
use warpui::fonts::FamilyId;
use warpui::{AppContext, SingletonEntity};

use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::CloudObject;
use crate::search::async_snapshot_data_source::AsyncSnapshotDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{BoxFuture, DataSourceRunErrorWrapper};
use crate::search::FuzzyMatchWorkflowResult;
use crate::server::ids::SyncId;
use crate::settings::AISettings;
use crate::workflows::CloudWorkflowModel;

use super::{AcceptSlashCommandOrSavedPrompt, InlineItem};

pub(super) struct SavedPromptCandidate {
    pub(super) id: SyncId,
    pub(super) model: Arc<CloudWorkflowModel>,
    pub(super) breadcrumbs: String,
}

pub(crate) struct SavedPromptsSnapshot {
    candidates: Vec<SavedPromptCandidate>,
    query_text: String,
    font_family: FamilyId,
    /// Saved prompts are Agent Mode workflows destined for the AI agent, so they shouldn't be
    /// surfaced when AI is globally disabled. Captured at snapshot time so the async match step
    /// can bail without touching app state.
    ai_enabled: bool,
}

/// Creates an async data source for saved prompts (Agent Mode workflows) in the slash command
/// menu.
///
/// We use fuzzy search rather than Tantivy full-text search. Switching to Tantivy would avoid
/// cloning all workflow data on each query, but would require exposing the Tantivy reader for
/// off-thread use.
pub(crate) fn saved_prompts_data_source(
) -> AsyncSnapshotDataSource<SavedPromptsSnapshot, AcceptSlashCommandOrSavedPrompt> {
    AsyncSnapshotDataSource::new(
        |query: &Query, app: &AppContext| {
            let ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
            // Skip the workflow scan entirely when AI is off; the match step will return empty.
            let candidates: Vec<SavedPromptCandidate> = if ai_enabled {
                CloudModel::as_ref(app)
                    .get_all_active_workflows()
                    .filter(|cw| cw.model().data.is_agent_mode_workflow())
                    .map(|cw| SavedPromptCandidate {
                        id: cw.id,
                        model: cw.shared_model(),
                        breadcrumbs: cw.breadcrumbs(app),
                    })
                    .collect()
            } else {
                vec![]
            };

            SavedPromptsSnapshot {
                candidates,
                query_text: query.text.trim().to_owned(),
                font_family: Appearance::as_ref(app).ui_font_family(),
                ai_enabled,
            }
        },
        fuzzy_match_saved_prompts,
    )
}

pub(crate) fn fuzzy_match_saved_prompts(
    snapshot: SavedPromptsSnapshot,
) -> BoxFuture<
    'static,
    Result<Vec<QueryResult<AcceptSlashCommandOrSavedPrompt>>, DataSourceRunErrorWrapper>,
> {
    Box::pin(async move {
        if !snapshot.ai_enabled || snapshot.query_text.is_empty() {
            return Ok(vec![]);
        }

        // For single-character queries, use prefix matching on the name instead of
        // fuzzy search to avoid missing valid results.
        let prefix_char = (snapshot.query_text.chars().count() == 1)
            .then(|| snapshot.query_text.chars().next().unwrap());
        let mut results = Vec::new();

        for chunk in snapshot.candidates.chunks(128) {
            for candidate in chunk {
                if let Some(query_char) = prefix_char {
                    if candidate
                        .model
                        .data
                        .name_starts_with_char_ignore_case(query_char)
                    {
                        let name_match_result = Some(fuzzy_match::FuzzyMatchResult {
                            score: 100,
                            matched_indices: vec![0],
                        });
                        // All prefix matches share the same score; ordering among them
                        // is determined by the candidate insertion order, which is fine
                        // for single-character queries.
                        let item = InlineItem {
                            action: AcceptSlashCommandOrSavedPrompt::SavedPrompt {
                                id: candidate.id,
                            },
                            icon_path: "bundled/svg/prompt.svg",
                            name: candidate.model.data.name().to_owned(),
                            description: None,
                            font_family: snapshot.font_family,
                            name_match_result,
                            description_match_result: None,
                            score: OrderedFloat(100.0),
                            compact_layout: false,
                        };
                        results.push(QueryResult::from(item));
                    }
                } else {
                    let match_result = FuzzyMatchWorkflowResult::try_match(
                        &snapshot.query_text,
                        &candidate.model.data,
                        &candidate.breadcrumbs,
                    );

                    if let Some(match_result) = match_result {
                        let score = match_result.score();

                        // Avoid spamming results with extremely weak matches.
                        if score <= OrderedFloat(25.0) {
                            continue;
                        }

                        let item = InlineItem {
                            action: AcceptSlashCommandOrSavedPrompt::SavedPrompt {
                                id: candidate.id,
                            },
                            icon_path: "bundled/svg/prompt.svg",
                            name: candidate.model.data.name().to_owned(),
                            description: None,
                            font_family: snapshot.font_family,
                            name_match_result: match_result.name_match_result,
                            description_match_result: match_result.content_match_result,
                            score,
                            compact_layout: false,
                        };
                        results.push(QueryResult::from(item));
                    }
                }
            }
            yield_now().await;
        }

        Ok(results)
    })
}

#[cfg(test)]
#[path = "saved_prompts_tests.rs"]
mod tests;
