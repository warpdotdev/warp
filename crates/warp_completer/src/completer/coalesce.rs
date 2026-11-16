//! This module contains logic to sort the final vector of suggestions based on priority.
use std::collections::HashMap;

use itertools::Itertools;

use super::suggest::{MatchedSuggestion, Priority};
use crate::completer::suggest::SuggestionTypeName;

/// Given a map of computed, unordered suggestion vectors keyed by `SuggestionType`, returns a
/// single vector of suggestions in order.
/// Suggestions are returned in the following order:
/// 1. "High" priority (greater than default) suggestions, by descending priority value using
///    lexicographic order to break ties.
/// 2. Default priority suggestions, in the same order they were returned by the engine.
/// 3. "Low" priority (less than default) suggestions, by descending priority value using
///    lexicographic order to break ties.
pub(super) fn coalesce_completion_results(
    completion_results_by_type: HashMap<SuggestionTypeName, Vec<MatchedSuggestion>>,
) -> Vec<MatchedSuggestion> {
    let mut high_priority_suggestions = vec![];
    let mut default_priority_suggestions = vec![];
    let mut low_priority_suggestions = vec![];

    // We process suggestions by suggestion type to keep the default priority suggestions ordered by
    // SuggestionType maintaining the order produced by the completion engine. This isn't necessary
    // for low and high priority suggestions because those get sorted by priority and lexicogrpahic
    // order later.
    for (_, suggestions) in completion_results_by_type
        .into_iter()
        .sorted_by_key(|(suggestion_type, _)| *suggestion_type)
    {
        let mut default_priority_suggestions_for_type = vec![];

        for suggestion in suggestions.into_iter() {
            match suggestion.priority().cmp(&Priority::default()) {
                std::cmp::Ordering::Less => {
                    low_priority_suggestions.push(suggestion);
                }
                std::cmp::Ordering::Equal => {
                    default_priority_suggestions_for_type.push(suggestion);
                }
                std::cmp::Ordering::Greater => {
                    high_priority_suggestions.push(suggestion);
                }
            }
        }

        default_priority_suggestions.extend(default_priority_suggestions_for_type);
    }

    let sorted_high_priority_suggestsions = high_priority_suggestions
        .into_iter()
        .sorted_by(MatchedSuggestion::cmp_by_reversed_priority_and_display);

    let sorted_low_priority_suggestions = low_priority_suggestions
        .into_iter()
        .sorted_by(MatchedSuggestion::cmp_by_reversed_priority_and_display);

    sorted_high_priority_suggestsions
        .into_iter()
        // We don't sort default priority suggestions to preserve the ordering produced by the
        // completion engine.
        .chain(default_priority_suggestions)
        .chain(sorted_low_priority_suggestions)
        .unique_by(|suggestion| suggestion.suggestion.display.clone())
        .collect::<Vec<MatchedSuggestion>>()
}
