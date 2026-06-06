//! Contains the v2 implementation of flag suggestion generation that depends on the JS-compatible
//! command signature struct (`crate::signatures::CommandSignature`).
use std::cmp::Ordering;
use std::iter;

use itertools::Itertools;

use crate::completer::describe::OptionCaseSensitivity;
use crate::completer::suggest::MatchRequirement;
use crate::completer::{
    LocationType, Match, MatchStrategy, MatchedSuggestion, Suggestion, SuggestionType,
};
use crate::meta::Spanned;
use crate::signatures::Command;

pub fn complete(
    matcher: MatchStrategy,
    location: &Spanned<LocationType>,
    found_signature: Option<&Command>,
) -> Vec<MatchedSuggestion> {
    let (
        Some(found_signature),
        Spanned {
            item:
                LocationType::Flag {
                    flag_name: partial_flag_name,
                    ..
                },
            ..
        },
    ) = (found_signature, location)
    else {
        return Default::default();
    };

    let suggestions = short_hand_flag_suggestions(
        found_signature,
        partial_flag_name
            .as_ref()
            .map(|name| name.item.as_str())
            .unwrap_or(""),
        matcher,
    );

    let should_order_long_hand_before_short_hand = partial_flag_name.is_none();
    let sort_flag_suggestions = |a: &MatchedSuggestion, b: &MatchedSuggestion| -> Ordering {
        match (
            is_short_hand_flag_name(a.suggestion.display.as_str()),
            is_short_hand_flag_name(b.suggestion.display.as_str()),
        ) {
            (true, false) => {
                if should_order_long_hand_before_short_hand {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (false, true) => {
                if should_order_long_hand_before_short_hand {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (_, _) => a.cmp_by_display(b),
        }
    };

    suggestions.sorted_by(sort_flag_suggestions).collect()
}

/// Returns suggestions for short hand flags (i.e. flags that are of the form '-X').
///
/// Short-hand flags included in `input_token` are not included in returned suggestions since
/// they've already been specified, except for the last specified flag (if any), since the
/// completion engine always returns the suggestion for the 'current token' if there is an exact
/// match, (which in that case would be the last short-hand flag in the token).
fn short_hand_flag_suggestions(
    command_signature: &Command,
    input_token: &str,
    matcher: MatchStrategy,
) -> Box<dyn Iterator<Item = MatchedSuggestion>> {
    let (prefix, partial_flag_name) =
        if let Some(partial_flag_name) = input_token.strip_prefix("--") {
            ("--", partial_flag_name)
        } else if let Some(partial_flag_name) = input_token.strip_prefix('-') {
            ("-", partial_flag_name)
        } else if input_token.is_empty() {
            ("", "")
        } else {
            return Box::new(iter::empty());
        };

    Box::new(
        command_signature
            .options
            .iter()
            .flat_map(|option| option.name.iter().map(move |name| (option, name)))
            .filter(|(_, name)| name.starts_with(prefix))
            .filter_map(|(option, name)| {
                if is_short_hand_flag_name(name) {
                    // Multiple short-hand flags can be specified in a single token with a leading
                    // '-' (e.g. ssh -Xv). Do not suggest a shorthand flag if it has already been
                    // specified unless it is the last flag in the current token; the completion engine
                    // always returns a suggestion if it exactly matches the current token.
                    let name_without_prefix = name.trim_start_matches('-');
                    let is_flag_already_included = partial_flag_name.contains(name_without_prefix);
                    let is_current_flag = partial_flag_name
                        .chars()
                        .last()
                        .is_some_and(|c| c.to_string() == name_without_prefix);
                    (!is_flag_already_included || is_current_flag).then(|| {
                        let replacement_text = if is_current_flag {
                            // The replacement text should be exactly the existing flags.
                            format!("-{partial_flag_name}")
                        } else {
                            // The replacement text should be the existing flags followed by the new one.
                            format!("-{partial_flag_name}{name_without_prefix}")
                        };
                        MatchedSuggestion {
                            suggestion: Suggestion::new(
                                name,
                                replacement_text,
                                option.description.clone(),
                                // TODO(CORE-2795)
                                SuggestionType::Option(
                                    MatchRequirement::EntireName,
                                    OptionCaseSensitivity::CaseSensitive,
                                ),
                                option.priority.into(),
                            ),
                            match_type: Match::Prefix {
                                is_case_sensitive: true,
                            },
                        }
                    })
                } else if is_long_hand_flag_name(name) {
                    matcher.get_match_type(input_token, name).map(|match_type| {
                        let suggestion = Suggestion::with_same_display_and_replacement(
                            name,
                            option.description.clone(),
                            // TODO(CORE-2795)
                            SuggestionType::Option(
                                MatchRequirement::EntireName,
                                OptionCaseSensitivity::CaseSensitive,
                            ),
                            option.priority.into(),
                        );
                        MatchedSuggestion::new(suggestion, match_type)
                    })
                } else {
                    None
                }
            })
            .sorted_by(MatchedSuggestion::cmp_by_display),
    )
}

/// Heuristic to determine if a flag name is a short-hand flag or not.
///
/// * A single dash followed by a single character (`-h`, `-v`, etc.) is short-hand, unless the second character is also a dash.
/// * A single dash followed by multiple characters (`-version`) is long-hand
/// * Two dashes followed by 0 or more characters is long-hand
fn is_short_hand_flag_name(name: &str) -> bool {
    name.len() == 2 && name.starts_with('-') && name != "--"
}

/// Tests if `name` is a long-hand flag name. A long-hand flag is a string
/// starting with `-` that is not a short-hand flag.
fn is_long_hand_flag_name(name: &str) -> bool {
    name.starts_with('-') && !is_short_hand_flag_name(name)
}
