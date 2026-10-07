use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_errors::report_error;

// Embedded objects are matched on object name and its drive location breadcrumb.
#[derive(Clone, Debug)]
pub struct FuzzyMatchEmbeddedObjectResult {
    pub name_match_result: Option<FuzzyMatchResult>,
    pub breadcrumb_match_result: Option<FuzzyMatchResult>,
}

impl FuzzyMatchEmbeddedObjectResult {
    /// Attempts to fuzzy match the embedded object. Returns `None` if the embedded object was not matched.
    pub fn try_match(
        query: &str,
        name: &str,
        breadcrumb: &str,
    ) -> Option<FuzzyMatchEmbeddedObjectResult> {
        let name_match_result = fuzzy_match::match_indices_case_insensitive(name, query);
        let breadcrumb_match_result =
            fuzzy_match::match_indices_case_insensitive(breadcrumb, query);
        match (&name_match_result, &breadcrumb_match_result) {
            (None, None) => None,
            _ => Some(FuzzyMatchEmbeddedObjectResult {
                name_match_result,
                breadcrumb_match_result,
            }),
        }
    }

    /// Returns the fuzzy match score of the embedded item. Its name and drive location breadcrumb
    /// are weighted equally.
    pub fn score(&self) -> OrderedFloat<f64> {
        let scores = self
            .name_match_result
            .iter()
            .chain(self.breadcrumb_match_result.iter())
            .map(|result| result.score);

        let (sum, count) = scores.fold((0, 0), |(acc_sum, acc_count), score| {
            (acc_sum + score, acc_count + 1)
        });

        if count == 0 {
            // This branch should never be executed because an embedded object search result should
            // always have some match with the query, otherwise it should not appear as a result.
            report_error!("Embedded object has neither a name nor breadcrumb match result.");
            OrderedFloat(f64::MIN)
        } else {
            // All attributes have equal weight, so just take the avg score
            OrderedFloat((sum / (count as i64)) as f64)
        }
    }
}
