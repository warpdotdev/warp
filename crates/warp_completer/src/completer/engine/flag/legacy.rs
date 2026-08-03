//! Contains the legacy implementation of flag suggestion generation that depends on the legacy
//! command signature struct (`warp_command_signatures::Signature`).
use itertools::Itertools;
use warp_command_signatures::{FlagStyle, Signature as SpecSignature};

use crate::completer::describe::OptionCaseSensitivity;
use crate::completer::engine::LocationType;
use crate::completer::matchers::{Match, MatchStrategy};
use crate::completer::suggest::{MatchRequirement, MatchedSuggestion, Suggestion, SuggestionType};
use crate::meta::Spanned;
use crate::parsers::SignatureAtTokenIndex;

/// Returns suggestions for short hand flags (i.e. flags that are of the form '-X').
/// We currently omit surfacing flags that are already in partial_without_dashes, except
/// for the most recent one (i.e. with `-abc`, we would omit `-a` and `-b` but include
/// `-c`), which is used for the Describe API.
fn short_hand_flag_suggestions(
    signature: &SpecSignature,
    partial_without_dashes: &str,
) -> impl Iterator<Item = MatchedSuggestion> + use<> {
    signature
        .short_hand_flags()
        .filter_map(|flag| {
            // Since short hand flags can be written one after the other
            // (e.g. ssh -Xv), we can't just prefix match the flag against partial.
            // Instead, the logic below assumes that a short hand flag can only be used once.
            // Note that this is not however true in the real world (e.g. ssh -vv
            // is valid). We should eventually use  the completions spec to figure out
            // how many times an option can be repeated in a given command.
            let is_flag_already_included = partial_without_dashes.contains(flag.name);
            // We want to include the flag if it is the most recent one the user has
            // typed, so we can provide a suggestion for the current short hand flags.
            // This suggestion is used so we have an entry for the current flags in the
            // completions menu, and also in our Describe API.
            let is_current_flag = partial_without_dashes
                .chars()
                .last()
                .is_some_and(|c| c.to_string() == flag.name);
            let should_include_flag = !is_flag_already_included || is_current_flag;
            should_include_flag.then(|| {
                let replacement_text = if is_current_flag {
                    // The replacement text should be exactly the existing flags.
                    format!("-{partial_without_dashes}")
                } else {
                    // The replacement text should be the existing flags followed by the new one.
                    format!("-{}{}", partial_without_dashes, flag.name)
                };

                let case_sensitivity = if signature.parser_directives.always_case_insensitive {
                    OptionCaseSensitivity::CaseInsensitive
                } else {
                    OptionCaseSensitivity::CaseSensitive
                };

                let suggestion = Suggestion::new(
                    format!("-{}", flag.name),
                    replacement_text,
                    flag.description.map(Into::into),
                    SuggestionType::Option(MatchRequirement::EntireName, case_sensitivity),
                    flag.priority.into(),
                );

                MatchedSuggestion {
                    suggestion,
                    match_type: Match::Prefix {
                        is_case_sensitive: true,
                    },
                }
            })
        })
        .sorted_by(MatchedSuggestion::cmp_by_display)
}

/// Returns suggestions for long hand flags (i.e. flags that are of the form '--flag-name'
/// or -flagname') that begin with '--<partial_without_dashes>'.
///
/// If set, `style` filters long flags by their style - for example,
/// `Some(FlagStyle::SingleDash)` if suggesting for `-<partial_without_dashes>`.
fn long_hand_flag_suggestions(
    signature: &SpecSignature,
    matcher: MatchStrategy,
    partial_without_dashes: &str,
    style: Option<FlagStyle>,
) -> impl Iterator<Item = MatchedSuggestion> + use<> {
    signature
        .long_hand_flags()
        .filter(|flag| style.is_none_or(|style| flag.style == style))
        .filter_map(|flag| {
            matcher
                .get_match_type(partial_without_dashes, flag.name)
                .map(|match_type| {
                    let name = match flag.style {
                        FlagStyle::SingleDash => format!("-{}", flag.name),
                        FlagStyle::DoubleDash => format!("--{}", flag.name),
                    };

                    let match_requirement = if signature.parser_directives.flags_match_unique_prefix
                    {
                        MatchRequirement::UniquePrefixOnly
                    } else {
                        MatchRequirement::EntireName
                    };

                    let case_sensitivity = if signature.parser_directives.always_case_insensitive {
                        OptionCaseSensitivity::CaseInsensitive
                    } else {
                        OptionCaseSensitivity::CaseSensitive
                    };

                    let suggestion = Suggestion::with_same_display_and_replacement(
                        name,
                        flag.description.map(Into::into),
                        SuggestionType::Option(match_requirement, case_sensitivity),
                        flag.priority.into(),
                    );
                    MatchedSuggestion::new(suggestion, match_type)
                })
        })
        .sorted_by(MatchedSuggestion::cmp_by_display)
}

pub fn complete(
    matcher: MatchStrategy,
    location: &Spanned<LocationType>,
    found_signature: Option<SignatureAtTokenIndex>,
) -> Vec<MatchedSuggestion> {
    if let Spanned {
        item: LocationType::Flag {
            flag_name: name, ..
        },
        ..
    } = location
        && let Some(found_signature) = found_signature
    {
        let name = name.as_ref().map(|name| &name.item);
        return match name {
            // Case 1: if we are completing on '--<partial>', we surface long hand flags that begin with partial
            Some(long) if long.starts_with("--") => long_hand_flag_suggestions(
                found_signature.signature,
                matcher,
                &long[2..],
                Some(FlagStyle::DoubleDash),
            )
            .collect(),
            // Case 2: if we are completing on a single '-', we surface all short hand flags followed by all long hand flags
            Some(short) if short == "-" => {
                short_hand_flag_suggestions(found_signature.signature, "")
                    .chain(long_hand_flag_suggestions(
                        found_signature.signature,
                        matcher,
                        "",
                        None,
                    ))
                    .collect()
            }
            // Case 3: if we are completing on '-<partial>', we surface short hand flags that begin with partial,
            // followed by long hand flags that begin with partial.
            Some(short) if short.starts_with('-') => {
                short_hand_flag_suggestions(found_signature.signature, &short[1..])
                    .chain(long_hand_flag_suggestions(
                        found_signature.signature,
                        matcher,
                        &short[1..],
                        Some(FlagStyle::SingleDash),
                    ))
                    .collect()
            }
            // Case 4: if we are completing on whitespace (i.e. no prefix), we surface subcommands,
            // followed by all long hand flags, followed by all short hand flags
            None => long_hand_flag_suggestions(found_signature.signature, matcher, "", None)
                .chain(short_hand_flag_suggestions(found_signature.signature, ""))
                .collect(),
            _ => {
                log::info!("Reached option completion branch that should be unreachable");
                Default::default()
            }
        };
    }

    Default::default()
}
