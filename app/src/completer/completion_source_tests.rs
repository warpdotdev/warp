use std::future;

use warp_completer::completer::{
    CompletionsFallbackStrategy, Match, MatchStrategy, MatchedSuggestion, Priority, Suggestion,
    SuggestionResults, SuggestionType,
};
use warp_completer::meta::Span;
use warpui::r#async::block_on;

use super::{CompletionSourcePolicy, resolve_completion_results};
use crate::terminal::model::completions::ShellCompletion;

fn suggestion_results(name: &str) -> SuggestionResults {
    SuggestionResults {
        replacement_span: Span::new(0, 1),
        suggestions: vec![MatchedSuggestion::new(
            Suggestion::with_same_display_and_replacement(
                name,
                None,
                SuggestionType::Argument,
                Priority::default(),
            ),
            Match::Prefix {
                is_case_sensitive: true,
            },
        )],
        match_strategy: MatchStrategy::Fuzzy,
    }
}

#[test]
fn policy_requires_enablement_shell_support_and_single_line_input() {
    let disabled = CompletionSourcePolicy::from_inputs(false, false, true, false);
    assert!(!disabled.should_request_native_shell_completions());
    assert!(matches!(
        disabled.fallback_strategy(CompletionsFallbackStrategy::FilePaths),
        CompletionsFallbackStrategy::FilePaths
    ));

    let forced = CompletionSourcePolicy::from_inputs(false, true, true, false);
    assert!(forced.should_request_native_shell_completions());
    assert!(matches!(
        forced.fallback_strategy(CompletionsFallbackStrategy::FilePaths),
        CompletionsFallbackStrategy::None
    ));

    let unsupported = CompletionSourcePolicy::from_inputs(true, false, false, false);
    assert!(!unsupported.should_request_native_shell_completions());

    let multiline = CompletionSourcePolicy::from_inputs(true, false, true, true);
    assert!(!multiline.should_request_native_shell_completions());
}

#[test]
fn nonempty_warp_results_win_without_force() {
    let results = block_on(resolve_completion_results(
        Some(suggestion_results("warp")),
        future::ready(Some(vec![ShellCompletion::new("native".to_owned())])),
        "w",
        false,
    ))
    .expect("Warp results should be retained");

    assert_eq!(results.suggestions[0].replacement(), "warp");
}

#[test]
fn native_results_replace_empty_or_forced_warp_results() {
    let empty_warp_results = SuggestionResults {
        replacement_span: Span::new(0, 0),
        suggestions: vec![],
        match_strategy: MatchStrategy::Fuzzy,
    };
    let native_after_empty = block_on(resolve_completion_results(
        Some(empty_warp_results),
        future::ready(Some(vec![ShellCompletion::new("native".to_owned())])),
        "command na",
        false,
    ))
    .expect("native results should replace empty Warp results");
    assert_eq!(native_after_empty.suggestions[0].replacement(), "native");
    assert_eq!(native_after_empty.replacement_span, Span::new(8, 10));

    let native_after_force = block_on(resolve_completion_results(
        Some(suggestion_results("warp")),
        future::ready(Some(vec![ShellCompletion::new("native".to_owned())])),
        "λ native",
        true,
    ))
    .expect("forced native results should replace Warp results");
    assert_eq!(native_after_force.suggestions[0].replacement(), "native");
    assert_eq!(native_after_force.replacement_span, Span::new(3, 9));
}
