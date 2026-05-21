use itertools::Itertools;
use string_offset::ByteOffset;
use warpui::platform::OperatingSystem;

use crate::{
    completer::suggest::MatchRequirement,
    meta::{HasSpan, Span, Spanned},
};
use crate::{meta::SpannedItem, parsers::simple::command_at_cursor_position};

use super::suggest::{suggestions, CompleterOptions, CompletionsFallbackStrategy, SuggestionType};
use super::{context::CompletionContext, get_path_separators};
use super::{Match, MatchStrategy};

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Description {
    pub token: Spanned<String>,
    pub description_text: Option<String>,
    pub suggestion_type: SuggestionType,
}

impl Description {
    pub fn a11y_text(&self) -> String {
        match &self.description_text {
            Some(description_text) => format!(
                "Command inspector triggered for {}, {}",
                self.token.item, description_text
            ),
            None => format!("Command inspector triggered for {}", self.token.item,),
        }
    }
}

/// Describes the case sensitivity used when parsing a top-level command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopLevelCommandCaseSensitivity {
    CaseSensitive,
    CaseInsensitive,
}

impl TopLevelCommandCaseSensitivity {
    pub fn from_os_category(os_category: &str) -> Self {
        match os_category.to_lowercase().as_str() {
            "windows" | "macos" => Self::CaseInsensitive,
            _ => Self::CaseSensitive,
        }
    }
}

impl From<OperatingSystem> for TopLevelCommandCaseSensitivity {
    fn from(value: OperatingSystem) -> Self {
        match value {
            OperatingSystem::Mac | OperatingSystem::Windows => Self::CaseInsensitive,
            _ => Self::CaseSensitive,
        }
    }
}

impl From<TopLevelCommandCaseSensitivity> for MatchStrategy {
    fn from(value: TopLevelCommandCaseSensitivity) -> Self {
        match value {
            TopLevelCommandCaseSensitivity::CaseSensitive => Self::CaseSensitive,
            TopLevelCommandCaseSensitivity::CaseInsensitive => Self::CaseInsensitive,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionCaseSensitivity {
    CaseSensitive,
    CaseInsensitive,
}

impl From<OptionCaseSensitivity> for MatchStrategy {
    fn from(value: OptionCaseSensitivity) -> Self {
        match value {
            OptionCaseSensitivity::CaseSensitive => Self::CaseSensitive,
            OptionCaseSensitivity::CaseInsensitive => Self::CaseInsensitive,
        }
    }
}

/// Describes the part of the command at the given cursor location, by using the completion results of the
/// command at that location and then matching those results with the token to extract the desired description.
///
/// For example, with the line `git commit` and pos 1, the function would return a Description
/// struct for the command `git`
pub async fn describe<T: CompletionContext>(
    line: &str,
    pos: ByteOffset,
    context: &T,
) -> Option<Description> {
    let command_to_describe = command_at_cursor_position(line, context.escape_char(), pos);
    match command_to_describe {
        Some(command) => {
            let command_span = command
                .parts
                .iter()
                .find(|token| {
                    token.span.start() <= pos.as_usize() && token.span.end() >= pos.as_usize()
                })
                .cloned();

            match command_span {
                Some(token) => {
                    // For --flag=value tokens, split based on cursor position so hovering the flag
                    // part describes the flag and hovering the value part describes the value.
                    let token = split_flag_eq_token_at_cursor(token, pos);
                    describe_given_token(line, &command.span(), token, context).await
                }
                None => None,
            }
        }
        None => None,
    }
}

/// Returns a completion description for the given token in the command.
/// line is the entire input.
/// command_span is the span for the command we're describing.
/// token is the specific span of the word we're describing.
/// e.g. line = "git status && cd dir", command_span = "git status", token = "status"
pub async fn describe_given_token<T: CompletionContext>(
    line: &str,
    command_span: &Span,
    token: Spanned<String>,
    context: &T,
) -> Option<Description> {
    let path_separators = get_path_separators(context).all;

    // If the filepath ends with a separator, we need to run the completer with the
    // separator trimmed. Otherwise, the completer won't return suggestions for the
    // current directory. For example, if we have `cd foo`, the completer will return
    // a suggestion for `foo` so we can properly describe it. With `cd foo/`, the
    // completer would only return suggestions for subdirectories of `foo`, so we
    // wouldn't be able to describe it.
    let token_end = if token.item.ends_with(path_separators) {
        token.span().end() - 1
    } else {
        token.span().end()
    };
    let start_raw_byte_index = command_span.start();
    let end_raw_byte_index = start_raw_byte_index.max(token_end);
    let start = floor_char_boundary(line, start_raw_byte_index);
    let end = floor_char_boundary(line, end_raw_byte_index);
    let complete_on = &line[start..end];

    let results = suggestions(
        complete_on,
        complete_on.len(),
        None,
        CompleterOptions {
            // Use a case-insensitive matcher here since we need case-insensitive matches for command
            // suggestion types. We do a final match based on the correct command type below.
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        context,
    )
    .await;

    let trimmed_token_item = token.item.trim_end_matches(path_separators);

    results.and_then(|results| {
        let mut prefix_matches = vec![];
        for suggestion in results.suggestions {
            let matching_suggestion_token = if suggestion.is_abbreviation() {
                // For abbreviations, we match the token with the display
                // text of the suggestion because the replacement text is
                // the expanded form of the abbreviation.
                suggestion.display()
            } else {
                // TODO: add a property on the Suggestion type so we don't have to keep recomputing this
                suggestion.replacement().trim_end_matches(path_separators)
            };

            let matcher = match suggestion.suggestion_type() {
                SuggestionType::Command(case_sensitivity) => case_sensitivity.into(),
                SuggestionType::Option(_, case_sensitivity) => case_sensitivity.into(),
                _ => MatchStrategy::CaseSensitive,
            };

            match (
                matcher.get_match_type(trimmed_token_item, matching_suggestion_token),
                suggestion.suggestion_type(),
            ) {
                (Some(Match::Exact { .. }), _) => {
                    return Some(Description {
                        token: match suggestion.suggestion_type() {
                            // For top-level commands `suggestion.display()` contains the preferred
                            // stylization, e.g. "Get-Help" instead of "get-help".
                            SuggestionType::Command(_) => {
                                suggestion.display().to_owned().spanned(token.span())
                            }
                            // For all other suggestion types, show it exactly as the user typed
                            // it.
                            _ => token.clone(),
                        },
                        description_text: suggestion.description(),
                        suggestion_type: suggestion.suggestion_type(),
                    });
                }
                (
                    Some(Match::Prefix { .. }),
                    SuggestionType::Option(MatchRequirement::UniquePrefixOnly, _),
                ) => {
                    prefix_matches.push(Description {
                        token: suggestion.display().to_owned().spanned(token.span()),
                        description_text: suggestion.description(),
                        suggestion_type: suggestion.suggestion_type(),
                    });
                }
                _ => (),
            };
        }
        prefix_matches.into_iter().exactly_one().ok()
    })
}

/// If the token is a `--flag=value` token, returns a sub-token for the part the
/// cursor is on: the flag name part if the cursor is on or before the `=`, or the
/// value part if the cursor is after the `=`.
fn split_flag_eq_token_at_cursor(token: Spanned<String>, pos: ByteOffset) -> Spanned<String> {
    if !token.item.starts_with('-') {
        return token;
    }
    let Some(eq_pos) = token.item.find('=') else {
        return token;
    };
    let eq_byte_pos = token.span.start() + eq_pos;
    if pos.as_usize() <= eq_byte_pos {
        // Cursor is on the flag name part (including '=').
        token.item[..eq_pos]
            .to_string()
            .spanned(Span::new(token.span.start(), eq_byte_pos))
    } else {
        // Cursor is on the value part.
        token.item[eq_pos + 1..]
            .to_string()
            .spanned(Span::new(eq_byte_pos + 1, token.span.end()))
    }
}

/// TODO: replace with str::floor_char_boundary once it's not on nightly anymore.
fn floor_char_boundary(original_string: &str, idx: usize) -> usize {
    if idx >= original_string.len() {
        original_string.len()
    } else {
        let mut curr = idx;
        // Stop at zero since it's always a char boundary.
        while curr > 0 && !original_string.is_char_boundary(curr) {
            curr -= 1;
        }
        curr
    }
}

#[cfg(all(test, not(feature = "v2")))]
#[path = "describe_tests.rs"]
mod tests;
