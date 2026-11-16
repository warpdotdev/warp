use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;

/// Result of fuzzy matching the user AI queries in history.
#[derive(Clone, Debug)]
pub struct FuzzyMatchAIQueryResults {
    /// Result of the attempted fuzzy match on the query text including matched string indices and
    /// score.
    pub query_text_match_result: FuzzyMatchResult,
}

impl FuzzyMatchAIQueryResults {
    /// Attempt to fuzzy match the user's search text with the AI query in history.
    pub fn try_match(query: &str, ai_query: &str) -> Option<Self> {
        fuzzy_match::match_indices_case_insensitive(ai_query, query).map(|result| Self {
            query_text_match_result: result,
        })
    }

    /// Returns the score of the match if any, and the lowest number possible if there was no match.
    pub fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.query_text_match_result.score as f64)
    }
}
