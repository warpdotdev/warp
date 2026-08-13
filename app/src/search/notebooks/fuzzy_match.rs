use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_errors::report_error;
use warpui::elements::{Highlight, Text};
use warpui::{AppContext, SingletonEntity};

use crate::appearance::Appearance;
use crate::cloud_object::CloudObject;
use crate::notebooks::CloudNotebook;
use crate::notebooks::manager::NotebookManager;
use crate::search::result_renderer::ItemHighlightState;
use crate::server::ids::SyncId;

#[derive(Clone, Debug)]
/// Result of fuzzy matching a [`Notebook`].
pub struct FuzzyMatchNotebookResult {
    pub name_match_result: Option<fuzzy_match::FuzzyMatchResult>,
    pub content_match_result: Option<fuzzy_match::FuzzyMatchResult>,
    pub folder_match_result: Option<fuzzy_match::FuzzyMatchResult>,
}

/// Renders the text element used in both command palette and command search
/// that displays notebook content with matching highlights. If there is a match,
/// the content is truncated up to the first match so we display only relevant information.
pub fn render_notebook_matched_content_with_highlight(
    notebook_id: SyncId,
    model_data: &str,
    content_match_result: &Option<FuzzyMatchResult>,
    highlight_state: ItemHighlightState,
    app: &AppContext,
) -> Text {
    let appearance = Appearance::as_ref(app);
    let parsed_raw_text = NotebookManager::as_ref(app)
        .notebook_raw_text(notebook_id)
        .unwrap_or(model_data);

    if let Some(match_result) = content_match_result {
        let first_match_index = match_result.matched_indices.first().unwrap_or(&0);
        // We only want the content starting at the match
        let matched_text_slice = parsed_raw_text
            .get(*first_match_index..)
            .unwrap_or_default();
        // Adjust the highlight indexes to be offset from the first index
        let adjusted_content_match_result: Vec<usize> = match_result
            .matched_indices
            .iter()
            .map(|idx| *idx - first_match_index)
            .collect();

        Text::new_inline(
            matched_text_slice.to_owned(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_single_highlight(
            Highlight::new()
                .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
            adjusted_content_match_result,
        )
    } else {
        Text::new_inline(
            parsed_raw_text.to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
    }
}

impl FuzzyMatchNotebookResult {
    /// Attempts to fuzzy match the `notebook`. Returns `None` if the `notebook` was not matched.
    pub fn try_match(
        query: &str,
        notebook: &CloudNotebook,
        app: &AppContext,
    ) -> Option<FuzzyMatchNotebookResult> {
        let name_match_result =
            fuzzy_match::match_indices_case_insensitive(&notebook.model().title, query);
        let parsed_raw_text = NotebookManager::as_ref(app)
            .notebook_raw_text(notebook.id)
            .unwrap_or(notebook.model().data.as_str());
        let content_match_result =
            fuzzy_match::match_indices_case_insensitive(parsed_raw_text, query);
        let folder_match_result =
            fuzzy_match::match_indices_case_insensitive(notebook.breadcrumbs(app).as_str(), query);
        match (
            &name_match_result,
            &content_match_result,
            &folder_match_result,
        ) {
            (None, None, None) => None,
            _ => Some(FuzzyMatchNotebookResult {
                name_match_result,
                content_match_result,
                folder_match_result,
            }),
        }
    }

    /// Returns a dummy [`FuzzyMatchNotebookResult`] for an item that is unmatched.
    pub fn no_match() -> FuzzyMatchNotebookResult {
        Self {
            name_match_result: Some(FuzzyMatchResult::no_match()),
            content_match_result: Some(FuzzyMatchResult::no_match()),
            folder_match_result: Some(FuzzyMatchResult::no_match()),
        }
    }

    /// Returns the fuzzy match score of the notebook.
    /// Currently weighted 60% name, 20% content, 20% breadcrumbs.
    pub fn score(&self) -> OrderedFloat<f64> {
        let scores = self
            .name_match_result
            .iter()
            .map(|result| (result.score as f64) * 0.6)
            .chain(
                self.content_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.2),
            )
            .chain(
                self.folder_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.2),
            );

        let (weighted_sum, count) = scores.fold((0.0, 0), |(acc_sum, acc_count), score| {
            (acc_sum + score, acc_count + 1)
        });

        if count == 0 {
            // This branch should never be executed because a notebooks search result should
            // always have some match with the query, otherwise it should not appear as a
            // result.
            report_error!("Notebook has neither a name nor content match result.");
            OrderedFloat(f64::MIN)
        } else {
            OrderedFloat(weighted_sum)
        }
    }
}
