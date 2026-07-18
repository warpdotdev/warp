use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_errors::report_error;

use crate::env_vars::EnvVarCollection;

#[derive(Clone, Debug)]
/// Result of fuzzy matching an [`EnvVarCollection`].
pub struct FuzzyMatchEnvVarCollectionResult {
    pub title_match_result: Option<FuzzyMatchResult>,
    pub var_name_match_result: Option<FuzzyMatchResult>,
    pub description_match_result: Option<FuzzyMatchResult>,
    pub breadcrumbs_match_result: Option<FuzzyMatchResult>,
}

impl FuzzyMatchEnvVarCollectionResult {
    /// Attempts to fuzzy match the `env_var_collection`. Returns `None` if the `env_var_collection` was not matched.
    pub fn try_match(
        query: &str,
        env_var_collection: &EnvVarCollection,
        breadcrumbs: &str,
    ) -> Option<FuzzyMatchEnvVarCollectionResult> {
        let title_match_result = env_var_collection
            .title
            .as_ref()
            .and_then(|title| fuzzy_match::match_indices_case_insensitive(title.as_str(), query));
        // We're passing each var into match_indices_case_insensitive,
        // taking the max result, and returning the matched indices of the joined string
        // we're rendering (separated by a comma and a space)
        let mut pos = 0;
        let var_name_match_result = env_var_collection
            .vars
            .iter()
            .filter_map(|var| {
                let name = &var.name;
                let curr_pos = pos;
                pos += name.len() + 2; // + 2 to account for the comma/space
                fuzzy_match::match_indices_case_insensitive(name, query)
                    .map(|result| (curr_pos, result))
            })
            .max_by_key(|(_, result)| result.score);

        let var_name_match_result =
            var_name_match_result.map(|(pos, mut result)| FuzzyMatchResult {
                score: result.score,
                matched_indices: {
                    result.matched_indices.iter_mut().for_each(|index| {
                        *index += pos;
                    });
                    result.matched_indices
                },
            });

        let description_match_result =
            env_var_collection
                .description
                .as_ref()
                .and_then(|description| {
                    fuzzy_match::match_indices_case_insensitive(description.as_str(), query)
                });
        let breadcrumbs_match_result =
            fuzzy_match::match_indices_case_insensitive(breadcrumbs, query);

        match (
            &title_match_result,
            &description_match_result,
            &var_name_match_result,
            &breadcrumbs_match_result,
        ) {
            (None, None, None, None) => None,
            _ => Some(FuzzyMatchEnvVarCollectionResult {
                title_match_result,
                var_name_match_result,
                description_match_result,
                breadcrumbs_match_result,
            }),
        }
    }

    /// Returns a dummy [`FuzzyMatchEnvVarCollectionResult`] for an item that is unmatched.
    pub fn no_match() -> FuzzyMatchEnvVarCollectionResult {
        Self {
            title_match_result: Some(FuzzyMatchResult::no_match()),
            var_name_match_result: Some(FuzzyMatchResult::no_match()),
            description_match_result: Some(FuzzyMatchResult::no_match()),
            breadcrumbs_match_result: Some(FuzzyMatchResult::no_match()),
        }
    }

    /// Returns the fuzzy match score of the EVC. The EVC name, description, and breadcrumbs
    /// are weighted equally.
    pub fn score(&self) -> OrderedFloat<f64> {
        let scores = self
            .title_match_result
            .iter()
            .map(|result| (result.score as f64) * 0.5)
            .chain(
                self.var_name_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.3),
            )
            .chain(
                self.description_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.1),
            )
            .chain(
                self.breadcrumbs_match_result
                    .iter()
                    .map(|result| (result.score as f64) * 0.1),
            );

        let (weighted_sum, count) = scores.fold((0.0, 0), |(acc_sum, acc_count), score| {
            (acc_sum + score, acc_count + 1)
        });

        if count == 0 {
            // This branch should never be executed because a EVC search result should
            // always have some match with the query, otherwise it should not appear as a
            // result.
            report_error!("EVC has no component which matches the result.");
            OrderedFloat(f64::MIN)
        } else {
            // All attributes have equal weight, so just take the avg score
            OrderedFloat(weighted_sum)
        }
    }
}
