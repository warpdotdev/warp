pub mod alias;
#[cfg_attr(feature = "v2", path = "v2.rs")]
#[cfg_attr(not(feature = "v2"), path = "legacy.rs")]
mod imp;
mod priority;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};

use alias::{AliasExpansionResult, expand_command_aliases};
use async_recursion::async_recursion;
use imp::*;
use itertools::Itertools;
pub use priority::Priority;
use smol_str::SmolStr;
use warp_command_signatures::IconType;
use warp_core::ui::theme::AnsiColorIdentifier;

use super::EngineFileType;
use super::coalesce::coalesce_completion_results;
use super::context::CompletionContext;
use super::engine::{self, completion_location};
use super::matchers::{Match, MatchStrategy, MatchType};
use crate::completer::TopLevelCommandCaseSensitivity;
use crate::completer::describe::OptionCaseSensitivity;
use crate::meta::Span;
use crate::parsers::classify_command;
use crate::parsers::simple::parse_for_completions;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Suggestion {
    // We store `display` and `replacement` as SmolStr for a few reasons:
    // 1. They will often be short enough that a heap allocation is not needed,
    // 2. They get cloned frequently, and clone is O(1) for SmolStr.
    pub display: SmolStr,
    pub replacement: SmolStr,

    pub description: Option<String>,
    pub suggestion_type: SuggestionType,
    pub override_icon: Option<IconType>,
    pub priority: Priority,
    pub is_hidden: bool,
    /// If Some(), this suggestion is a file/directory.
    pub file_type: Option<EngineFileType>,
    /// This field helps us properly describe abbreviations. Normally commands
    /// are described by matching the replacement string to the token. But
    /// abbreviations are unique as the replacement string is the expanded form
    /// of the command, so we need to differentiate them.
    pub is_abbreviation: bool,
}

impl Suggestion {
    pub fn new(
        display_text: impl Into<SmolStr>,
        replacement_text: impl Into<SmolStr>,
        description: Option<String>,
        suggestion_type: SuggestionType,
        priority: Priority,
    ) -> Self {
        Self {
            display: display_text.into(),
            replacement: replacement_text.into(),
            description,
            suggestion_type,
            priority,
            override_icon: None,
            is_hidden: false,
            file_type: None,
            is_abbreviation: false,
        }
    }

    /// Creates a `Suggestion` where the display and replacement are the same.
    pub fn with_same_display_and_replacement(
        name: impl Into<SmolStr>,
        description: Option<String>,
        suggestion_type: SuggestionType,
        priority: Priority,
    ) -> Self {
        let name = name.into();
        Self {
            display: name.clone(),
            replacement: name,
            description,
            suggestion_type,
            priority,
            override_icon: None,
            is_hidden: false,
            file_type: None,
            is_abbreviation: false,
        }
    }

    pub fn new_for_abbreviation(
        display_text: impl Into<SmolStr>,
        replacement_text: impl Into<SmolStr>,
        priority: Priority,
    ) -> Self {
        let replacement_text = replacement_text.into();
        let description = format!("Abbreviation for \"{replacement_text}\"");
        Self {
            display: display_text.into(),
            replacement: replacement_text,
            description: Some(description),
            suggestion_type: SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
            priority,
            override_icon: None,
            is_hidden: false,
            file_type: None,
            is_abbreviation: true,
        }
    }

    pub fn cmp_by_display(&self, other: &Self) -> Ordering {
        let a = self.display.trim_end_matches(std::path::MAIN_SEPARATOR);
        let b = other.display.trim_end_matches(std::path::MAIN_SEPARATOR);
        a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b))
    }

    /// Note: the ordering here is unconventional. Suggestions with greater
    /// priorities have a Less Ordering so that when we sort with this fn,
    /// more important suggestions appear first.
    pub fn cmp_by_reversed_priority_and_display(&self, other: &Self) -> Ordering {
        let priority_cmp = self.priority.cmp(&other.priority).reverse();

        // Using then_with here to preempt expensive string comparisons
        priority_cmp.then_with(|| self.cmp_by_display(other))
    }
}

#[allow(clippy::derived_hash_with_manual_eq)]
impl Hash for Suggestion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.display.hash(state);
        self.replacement.hash(state)
    }
}

/// A matched suggestion is what we use to represent a suggestion
/// that has been compared against a query. We use this to filter down
/// the set of suggestions that the user should see.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MatchedSuggestion {
    pub suggestion: Suggestion,
    pub match_type: Match,
}

/// Wrapper implementation around Suggestion.
impl MatchedSuggestion {
    pub fn new(suggestion: impl Into<Suggestion>, match_type: Match) -> Self {
        Self {
            suggestion: suggestion.into(),
            match_type,
        }
    }

    pub fn display(&self) -> &str {
        self.suggestion.display.as_str()
    }

    pub fn replacement(&self) -> &str {
        self.suggestion.replacement.as_str()
    }

    pub fn description(&self) -> Option<String> {
        self.suggestion.description.clone()
    }

    pub fn suggestion_type(&self) -> SuggestionType {
        self.suggestion.suggestion_type
    }

    pub fn priority(&self) -> Priority {
        self.suggestion.priority
    }

    pub fn is_abbreviation(&self) -> bool {
        self.suggestion.is_abbreviation
    }

    /// Helper methods to call into Suggestion comparisons
    pub fn cmp_by_display(&self, other: &Self) -> Ordering {
        Suggestion::cmp_by_display(&self.suggestion, &other.suggestion)
    }

    pub fn cmp_by_reversed_priority_and_display(&self, other: &Self) -> Ordering {
        Suggestion::cmp_by_reversed_priority_and_display(&self.suggestion, &other.suggestion)
    }
}

/// While commands in the POSIX world require their option names to be spelled out in full,
/// PowerShell cmdlets do not require this. This enum indicates these behaviors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchRequirement {
    /// For an option to be recognized, its whole name must be spelled out.
    EntireName,
    /// This variant signifies [`warp_command_signatures::ParserDirectives::flags_match_unique_prefix`]
    /// being `true`. Only a prefix which is long enough to make the intended option unambiguous is
    /// needed.
    UniquePrefixOnly,
}

/// Variants point 1:1 to the variants of [`SuggestionType`]. Used for hashing/sorting suggestion
/// types.
#[derive(Clone, Copy, Debug, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub enum SuggestionTypeName {
    Command = 1,
    Variable = 2,
    Argument = 3,
    Subcommand = 4,
    Option = 5,
}

impl Display for SuggestionTypeName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Command => "Command",
                Self::Subcommand => "Subcommand",
                Self::Argument => "Argument",
                Self::Option => "Option",
                Self::Variable => "Variable",
            },
        )
    }
}

/// Used for the syntax highlighting use-case. This maps different command parts
/// to different colors, as appropriate.
impl From<SuggestionTypeName> for AnsiColorIdentifier {
    fn from(suggestion: SuggestionTypeName) -> Self {
        match suggestion {
            SuggestionTypeName::Command => Self::Green,
            SuggestionTypeName::Subcommand => Self::Blue,
            SuggestionTypeName::Variable => Self::Magenta,
            SuggestionTypeName::Argument => Self::Cyan,
            SuggestionTypeName::Option => Self::Yellow,
        }
    }
}

/// Differentiates the types of suggestions and contains any data specific to that type. For
/// example, [`MatchRequirement`] only applies to [`SuggestionType::Option`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionType {
    Command(TopLevelCommandCaseSensitivity),
    Variable,
    Argument,
    Subcommand,
    Option(MatchRequirement, OptionCaseSensitivity),
}

impl SuggestionType {
    pub fn to_name(&self) -> SuggestionTypeName {
        match self {
            Self::Command(_) => SuggestionTypeName::Command,
            Self::Variable => SuggestionTypeName::Variable,
            Self::Argument => SuggestionTypeName::Argument,
            Self::Subcommand => SuggestionTypeName::Subcommand,
            Self::Option(..) => SuggestionTypeName::Option,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuggestionResults {
    pub replacement_span: Span,
    pub suggestions: Vec<MatchedSuggestion>,
    pub match_strategy: MatchStrategy,
}

pub struct FilteredSuggestion<'a> {
    pub suggestion: &'a Suggestion,
    pub match_type: MatchType,
    /// The indices of the matching characters between suggestion.display and
    /// the query that this FilteredSuggestion is derived from.
    pub matching_indices: Vec<usize>,
}

impl SuggestionResults {
    /// Orders the suggestions in the following order:
    /// 1. A suggestion that matches the query exactly (if any)
    /// 2. Prefix suggestions (with the intelligent ordering preserved)
    /// 3. Fuzzy suggestions (which are not a prefix match, ordered by fuzzy score)
    pub fn filter_by_query(
        &self,
        query: &str,
        path_separators: &[char],
    ) -> impl Iterator<Item = FilteredSuggestion<'_>> + '_ + use<'_> {
        // We build up the suggestions to avoid having to iterate over the
        // same set of suggestions multiple times. This is performance-sensitive code.
        // Note that the suggestions in these sets are mutually exclusive.
        let mut exact_match_suggestion: Option<(Match, &Suggestion)> = None;
        let mut case_insensitive_exact_match_suggestion: Option<(Match, &Suggestion)> = None;
        let mut prefix_suggestions = vec![];
        let mut fuzzy_suggestions = vec![];

        // TODO: In the future, we won't be including the entire filepath
        // in the replacement and this code will have to change accordingly.
        // This is to figure out if the query matches the replacement, and if
        // not, only take the part after the last slash. We are doing this here
        // to avoid calling count() on each suggestion, which is linear time.
        // e.g. If query is "a/b/c" then query_len will be 1 since we only take
        // "c" because it's after the last slash and it is 1 character long.
        let original_query_len = query.chars().count();
        let file_query = query
            .rsplit_once(path_separators)
            .map_or(query, |(_, after_last_slash)| after_last_slash);
        let file_query_len = file_query.chars().count();

        for suggestion in self.suggestions.iter() {
            // This is a very ad-hoc way of overcoming the problem of the query containing
            // the entire filepath (i.e. `app/src/platform.rs` as opposed to just the final `platform.rs`).
            // The reason we need to change the query is that we still want to use `suggestion.display()`
            // since that will yield the best fuzzy scores (vs. a large prefix matching). However,
            // we still compare the whole query to the replacement to see if there is a match at all to begin with.
            let query_for_suggestion = if suggestion.suggestion.file_type.is_some() {
                if self
                    .match_strategy
                    .get_match_type(query, suggestion.replacement())
                    .is_none()
                {
                    continue;
                }
                file_query
            } else {
                query
            };

            if let Some(match_type) = self
                .match_strategy
                .get_match_type(query_for_suggestion, suggestion.display())
            {
                // If the suggestion is hidden, we should only show it if it's an exact match.
                if suggestion.suggestion.is_hidden && !matches!(match_type, Match::Exact { .. }) {
                    continue;
                }

                match match_type {
                    Match::Exact {
                        is_case_sensitive: true,
                    } if exact_match_suggestion.is_none() => {
                        // If the suggestion matches the query exactly, we treat it specially
                        // since we want to order this suggestion first. There should only be one such suggestion.
                        exact_match_suggestion = Some((
                            Match::Exact {
                                is_case_sensitive: true,
                            },
                            &suggestion.suggestion,
                        ));
                    }
                    Match::Exact {
                        is_case_sensitive: false,
                    } if case_insensitive_exact_match_suggestion.is_none() => {
                        case_insensitive_exact_match_suggestion = Some((
                            Match::Exact {
                                is_case_sensitive: false,
                            },
                            &suggestion.suggestion,
                        ));
                    }
                    Match::Prefix { is_case_sensitive } => prefix_suggestions
                        .push((Match::Prefix { is_case_sensitive }, &suggestion.suggestion)),
                    Match::Fuzzy { ref match_result } => {
                        // Note that if display and replacement differ, then this could
                        // produce the wrong set of matching indices depending on query.
                        // An example of this is filepaths, which we special-case above.
                        let score = match_result.score;
                        fuzzy_suggestions.push((match_type, &suggestion.suggestion, score));
                    }
                    _ => {}
                }
            }
        }

        exact_match_suggestion
            .into_iter()
            .chain(case_insensitive_exact_match_suggestion)
            .chain(prefix_suggestions)
            .chain(
                fuzzy_suggestions
                    .into_iter()
                    .sorted_by_key(|(_, _, score)| *score)
                    .rev()
                    .map(|(match_type, matched_suggestion, _)| (match_type, matched_suggestion)),
            )
            .map(move |(match_type, suggestion)| {
                let telemetry_match_type = match_type.clone().into();
                match match_type {
                    Match::Prefix { .. } | Match::Exact { .. } => {
                        // Similar to above, we should use the appropriate length
                        // depending on which query we used (which is dependent on
                        // whether the suggestion is a file path or not).
                        let len = if suggestion.file_type.is_some() {
                            file_query_len
                        } else {
                            original_query_len
                        };
                        let matching_indices = (0..len).collect();
                        FilteredSuggestion {
                            suggestion,
                            matching_indices,
                            match_type: telemetry_match_type,
                        }
                    }
                    Match::Fuzzy { match_result } => FilteredSuggestion {
                        suggestion,
                        matching_indices: match_result.matched_indices,
                        match_type: telemetry_match_type,
                    },
                }
            })
    }

    /// Returns a `MatchedSuggestion` if there is a _single_ prefix suggestion, otherwise returns
    /// `None`.
    pub fn single_prefix_suggestion(&self) -> Option<&MatchedSuggestion> {
        let (
            num_prefix_suggestions,
            num_case_insensitive_prefix_suggestions,
            last_prefix_suggestion,
            last_case_insensitive_prefix_suggestion,
        ) = self.suggestions.iter().fold(
            (0, 0, None, None),
            |(num_items, num_case_insensitive_items, suggestion, case_insensitive_suggestion),
             item| {
                match item.match_type {
                    // We don't care about distinguishing proper prefixes here.
                    Match::Prefix {
                        is_case_sensitive: true,
                    }
                    | Match::Exact {
                        is_case_sensitive: true,
                    } => (
                        num_items + 1,
                        num_case_insensitive_items,
                        Some(item),
                        case_insensitive_suggestion,
                    ),
                    Match::Prefix {
                        is_case_sensitive: false,
                    }
                    | Match::Exact {
                        is_case_sensitive: false,
                    } => (
                        num_items,
                        num_case_insensitive_items + 1,
                        suggestion,
                        Some(item),
                    ),
                    _ => (
                        num_items,
                        num_case_insensitive_items,
                        suggestion,
                        case_insensitive_suggestion,
                    ),
                }
            },
        );

        if num_prefix_suggestions == 1 {
            last_prefix_suggestion
        } else if num_prefix_suggestions == 0 && num_case_insensitive_prefix_suggestions == 1 {
            last_case_insensitive_prefix_suggestion
        } else {
            None
        }
    }
}

/// In the cases where we don't have completions to show, we can potentially
/// fallback to one of these types.
#[derive(Debug, Copy, Clone)]
pub enum CompletionsFallbackStrategy {
    FilePaths,
    None,
}

/// Options struct passed to public completer APIs to configure completions logic.
#[derive(Debug, Copy, Clone)]
pub struct CompleterOptions {
    /// The match strategy that should be used to filter suggestions.
    pub match_strategy: MatchStrategy,

    /// The fallback strategy used to generate completion suggestions when no `CommandSignature`
    /// exists for the command.
    pub fallback_strategy: CompletionsFallbackStrategy,

    /// If true, we suggest file paths and nothing else.
    pub suggest_file_path_completions_only: bool,

    /// If true, we treat quotes as plain literals. Otherwise contents between opening and closing quotes
    /// will be considered as a single token.
    pub parse_quotes_as_literals: bool,
}

impl Default for CompleterOptions {
    fn default() -> Self {
        Self {
            match_strategy: MatchStrategy::Fuzzy,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        }
    }
}

/// This is the public API for using Warp's completion engine. Note that
/// the completion engines could end up performing I/O (e.g. calling generators,
/// interacting with the file system, etc.), so you should ensure that you
/// are on a background thread when using this API.
pub async fn suggestions<T: CompletionContext>(
    line: &str,
    pos: usize,
    session_env_vars: Option<&HashMap<String, String>>,
    options: CompleterOptions,
    ctx: &T,
) -> Option<SuggestionResults> {
    let line = &line[0..pos];
    if line.trim().is_empty() {
        return None;
    }
    suggestions_internal(line, session_env_vars, &options, ctx).await
}

/// Produces `SuggestionResults` with the `replacement_span` if specified. If not specified,
/// the `replacement_span` is the value directly from the completer.
#[async_recursion]
async fn suggestions_internal<'a>(
    line: &str,
    session_env_vars: Option<&'a HashMap<String, String>>,
    options: &CompleterOptions,
    ctx: &'a dyn CompletionContext,
) -> Option<SuggestionResults> {
    // Lite command we are completing upon. Note that this includes the full command including
    // parts like environment variable assignment.
    let command_to_complete =
        parse_for_completions(line, ctx.escape_char(), options.parse_quotes_as_literals)
            .unwrap_or_default();

    // The vector of tokens in the command. Note that the tokens are modified later to remove
    // any environment variable assignment token for completion generation.
    // TODO(kevin): We are using a mutable vector here so we don't need to allocate
    // multiple times. But this makes the code harder to read. We should think about
    // a better way to represent it.
    let mut tokens_from_command = command_to_complete
        .parts
        .iter()
        .map(|s| s.as_str())
        .collect_vec();

    let classified_command = classify_command(
        command_to_complete.clone(),
        &mut tokens_from_command,
        ctx.command_registry(),
        ctx.command_case_sensitivity(),
    );

    let locations = completion_location(ctx, line, classified_command.as_ref());

    // If there are no completion locations, short-circuit.
    let replacement_span = locations.last()?.span;

    if options.suggest_file_path_completions_only {
        let path_completion_context = ctx.path_completion_context()?;
        let classified_command = &classified_command?;
        let path_completions = engine::path::sorted_paths_relative_to(
            &classified_command.command.last_token(),
            options.match_strategy,
            path_completion_context,
        )
        .await;
        if path_completions.is_empty() {
            return None;
        }
        return Some(SuggestionResults {
            suggestions: path_completions,
            replacement_span,
            match_strategy: options.match_strategy,
        });
    }

    // Expand the line using any top level aliases or command-specific aliases.
    let AliasExpansionResult {
        expanded_command_line,
        signature_for_completions,
        tokens_from_command,
        classified_command,
    } = expand_command_aliases(line, options.parse_quotes_as_literals, ctx).await;
    // After expanding the line, reparse the expanded command.
    // We had to parse before alias expansion in order to get the correct replacement span.

    let locations = completion_location(ctx, &expanded_command_line, classified_command.as_ref());

    // Get a single completion result for each corresponding suggestion type.
    let completion_results_by_type = completion_results_from_locations(
        &expanded_command_line,
        classified_command,
        &tokens_from_command
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>(),
        signature_for_completions,
        locations,
        session_env_vars,
        options,
        ctx,
    )
    .await;

    let suggestions = coalesce_completion_results(completion_results_by_type);

    // Although we don't order the exact match and fuzzy completions here,
    // we do order them in filter_by_query which we run right after running the
    // completer and when the user types to filter.
    // TODO: perform the ordering here and add a check in filter_by_query
    // to prevent re-ordering again if the query hasn't changed.
    Some(SuggestionResults {
        suggestions,
        replacement_span,
        match_strategy: options.match_strategy,
    })
}

#[cfg(test)]
#[path = "test.rs"]
mod tests;
