//! Contains the implementation of internal suggestion generation logic that's coupled to the
//! legacy command signature struct (`crate::signatures::CommandSignature`).
use std::collections::HashMap;

use crate::completer::engine::{self, CompletionLocation};
use crate::completer::suggest::SuggestionTypeName;
use crate::completer::{CompleterOptions, CompletionContext, LocationType, MatchedSuggestion};
use crate::parsers::{ClassifiedCommand, SignatureAtTokenIndex};

/// Returns a map of `SuggestionType` to vectors of `MatchedSuggestion`s for `line`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn completion_results_from_locations(
    line: &str,
    classified_command: Option<ClassifiedCommand>,
    tokens_from_command: &[&str],
    found_signature: Option<SignatureAtTokenIndex<'_>>,
    locations: Vec<CompletionLocation>,
    session_env_vars: Option<&HashMap<String, String>>,
    options: &CompleterOptions,
    context: &dyn CompletionContext,
) -> HashMap<SuggestionTypeName, Vec<MatchedSuggestion>> {
    let mut completion_results_by_type =
        HashMap::<SuggestionTypeName, Vec<MatchedSuggestion>>::new();

    // For each location type, build up a single completion result for the
    // corresponding suggestion type.
    for location in locations {
        match location.item {
            LocationType::Command { parsed_token, .. } => {
                let results =
                    engine::command_suggestions(context, options.match_strategy, &parsed_token)
                        .await;
                completion_results_by_type
                    .entry(SuggestionTypeName::Command)
                    .or_default()
                    .extend(results);
            }
            LocationType::Variable { parsed_token } => {
                if let Some(env_vars) = context.environment_variable_names() {
                    let results = engine::variable_suggestions(
                        options.match_strategy,
                        env_vars,
                        &parsed_token,
                    );
                    completion_results_by_type
                        .entry(SuggestionTypeName::Variable)
                        .or_default()
                        .extend(results);
                }
            }
            LocationType::Flag { .. } => {
                let results =
                    engine::flag_suggestions(options.match_strategy, &location, found_signature);
                completion_results_by_type
                    .entry(SuggestionTypeName::Option)
                    .or_default()
                    .extend(results);
            }
            LocationType::Argument {
                ref parsed_token, ..
            } => {
                if let Some(classified_command) = classified_command.as_ref() {
                    let results = engine::argument_suggestions(
                        line,
                        tokens_from_command,
                        classified_command.clone(),
                        found_signature,
                        &location,
                        parsed_token,
                        session_env_vars,
                        options,
                        context,
                    )
                    .await;
                    completion_results_by_type
                        .entry(SuggestionTypeName::Argument)
                        .or_default()
                        .extend(results);
                }
            }
        }
    }

    completion_results_by_type
}
