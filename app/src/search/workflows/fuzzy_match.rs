use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;

use crate::report_error;
use crate::workflows::workflow::Workflow;

#[derive(Clone, Debug)]
/// Result of fuzzy matching a [`Workflow`].
pub struct FuzzyMatchWorkflowResult {
    pub name_match_result: Option<FuzzyMatchResult>,
    pub content_match_result: Option<FuzzyMatchResult>,
    pub description_match_result: Option<FuzzyMatchResult>,
    pub folder_match_result: Option<FuzzyMatchResult>,
}

impl FuzzyMatchWorkflowResult {
    /// Attempts to fuzzy match the `workflow`. Returns `None` if the `workflow` was not matched.
    pub fn try_match(
        query: &str,
        workflow: &Workflow,
        breadcrumbs: &str,
    ) -> Option<FuzzyMatchWorkflowResult> {
        let name_match_result = fuzzy_match::match_indices_case_insensitive(workflow.name(), query);
        let content_match_result =
            fuzzy_match::match_indices_case_insensitive(workflow.content(), query);
        let description_match_result = workflow.description().as_ref().and_then(|description| {
            fuzzy_match::match_indices_case_insensitive(description.as_str(), query)
        });
        let folder_match_result = fuzzy_match::match_indices_case_insensitive(breadcrumbs, query);
        match (
            &name_match_result,
            &content_match_result,
            &description_match_result,
            &folder_match_result,
        ) {
            (None, None, None, None) => None,
            _ => Some(FuzzyMatchWorkflowResult {
                name_match_result,
                content_match_result,
                description_match_result,
                folder_match_result,
            }),
        }
    }

    /// Returns a dummy [`FuzzyMatchWorkflowResult`] for an item that is unmatched.
    pub fn no_match() -> FuzzyMatchWorkflowResult {
        Self {
            name_match_result: Some(FuzzyMatchResult::no_match()),
            content_match_result: Some(FuzzyMatchResult::no_match()),
            description_match_result: Some(FuzzyMatchResult::no_match()),
            folder_match_result: Some(FuzzyMatchResult::no_match()),
        }
    }

    /// Returns the fuzzy match score of the workflow. The command has the highest weight, followed
    /// by the title, then the description and breadcrumbs.
    pub fn score(&self) -> OrderedFloat<f64> {
        let scores = self
            .name_match_result
            .iter()
            .map(|result| (result.score as f64) * 0.3)
            .chain(
                self.content_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.5),
            )
            .chain(
                self.description_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.1),
            )
            .chain(
                self.folder_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.1),
            );

        let (weighted_sum, count) = scores.fold((0.0, 0), |(acc_sum, acc_count), score| {
            (acc_sum + score, acc_count + 1)
        });

        if count == 0 {
            // This branch should never be executed because a workflows search result should
            // always have some match with the query, otherwise it should not appear as a
            // result.
            report_error!("Workflow has neither a name nor command match result.");
            OrderedFloat(f64::MIN)
        } else {
            // All attributes have equal weight, so just take the avg score
            OrderedFloat(weighted_sum)
        }
    }
}

#[cfg(test)]
#[path = "fuzzy_match_tests.rs"]
mod tests;
