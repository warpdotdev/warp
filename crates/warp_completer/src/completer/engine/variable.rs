use std::collections::HashSet;

use itertools::Itertools;
use smol_str::SmolStr;

use crate::completer::matchers::MatchStrategy;
use crate::completer::suggest::{MatchedSuggestion, Priority, Suggestion, SuggestionType};
use crate::parsers::ParsedToken;

pub fn suggestions(
    matcher: MatchStrategy,
    env_vars: &HashSet<SmolStr>,
    parsed_token: &ParsedToken,
) -> Vec<MatchedSuggestion> {
    let var_to_complete = parsed_token;

    if !var_to_complete.as_str().starts_with('$') {
        return Default::default();
    }

    let var_to_complete = &var_to_complete.as_str()[1..];

    env_vars
        .iter()
        .filter_map(|name| {
            matcher
                .get_match_type(var_to_complete, name)
                .map(|match_type| {
                    let suggestion_text = format!("${name}");
                    let suggestion = Suggestion::with_same_display_and_replacement(
                        suggestion_text,
                        None,
                        SuggestionType::Variable,
                        Priority::default(),
                    );
                    MatchedSuggestion::new(suggestion, match_type)
                })
        })
        .sorted_by(MatchedSuggestion::cmp_by_display)
        .collect()
}
