use fuzzy_match::{FuzzyMatchResult, match_indices_case_insensitive};

/// Result of fuzzy matching a slash command.
#[derive(Clone, Debug)]
pub struct SlashCommandFuzzyMatchResult {
    pub name_match_result: Option<FuzzyMatchResult>,
    pub description_match_result: Option<FuzzyMatchResult>,
}

impl SlashCommandFuzzyMatchResult {
    pub fn try_match(
        query: &str,
        name: &str,
        description: Option<&str>,
    ) -> Option<SlashCommandFuzzyMatchResult> {
        let name_match_result = match_indices_case_insensitive(name, query);
        let description_match_result =
            description.and_then(|desc| match_indices_case_insensitive(desc, query));

        match (&name_match_result, &description_match_result) {
            (None, None) => None,
            _ => Some(SlashCommandFuzzyMatchResult {
                name_match_result,
                description_match_result,
            }),
        }
    }

    /// Name matches are weighted higher (80%) than description matches (20%).
    pub fn score(&self) -> f64 {
        let name_score = self
            .name_match_result
            .as_ref()
            .map(|result| (result.score as f64) * 0.8) // Name has higher weight
            .unwrap_or(0.0);

        let description_score = self
            .description_match_result
            .as_ref()
            .map(|result| (result.score as f64) * 0.2) // Description has lower weight
            .unwrap_or(0.0);

        name_score + description_score
    }
}
