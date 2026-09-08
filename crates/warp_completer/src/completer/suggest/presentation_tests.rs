use super::{
    ExplicitTabCompletion, MatchedSuggestion, Priority, Suggestion, SuggestionResults,
    SuggestionType,
};
use crate::completer::{MatchStrategy, MatchType, TopLevelCommandCaseSensitivity};
use crate::meta::Span;

const PATH_SEPARATORS: &[char] = &['/'];

fn matched_suggestion(display: &str, query: &str) -> MatchedSuggestion {
    let suggestion = Suggestion::with_same_display_and_replacement(
        display,
        None,
        SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
        Priority::default(),
    );
    let match_type = MatchStrategy::Fuzzy
        .get_match_type(query, display)
        .expect("test suggestion should match its query");
    MatchedSuggestion::new(suggestion, match_type)
}

fn results(query: &str, displays: &[&str]) -> SuggestionResults {
    SuggestionResults {
        replacement_span: Span::new(0, query.len()),
        suggestions: displays
            .iter()
            .map(|display| matched_suggestion(display, query))
            .collect(),
        match_strategy: MatchStrategy::Fuzzy,
    }
}

#[test]
fn prepared_suggestions_follow_presentation_order() {
    let results = results("git", &["graft-it", "git-status", "GIT", "git"]);

    let prepared = results.prepare_for_query("git", PATH_SEPARATORS);
    let displays = prepared
        .iter()
        .map(|suggestion| suggestion.suggestion.display.as_str())
        .collect::<Vec<_>>();

    assert_eq!(displays, ["git", "GIT", "git-status", "graft-it"]);
    assert!(matches!(
        prepared[0].match_type,
        MatchType::Exact {
            is_case_sensitive: true
        }
    ));
    assert_eq!(prepared[0].matching_indices, [0, 1, 2]);
    assert!(matches!(prepared[3].match_type, MatchType::Fuzzy));
}

#[test]
fn explicit_tab_inserts_the_single_prefix_suggestion() {
    let results = results("st", &["sitar", "status"]);

    let ExplicitTabCompletion::InsertSingle {
        suggestion,
        replacement_span,
    } = results.explicit_tab_completion("st", PATH_SEPARATORS)
    else {
        panic!("one prefix suggestion should be inserted");
    };

    assert_eq!(suggestion.suggestion.replacement, "status");
    assert_eq!(replacement_span, Span::new(0, 2));
}

#[test]
fn explicit_tab_opens_an_ordered_menu_with_a_common_prefix() {
    let results = results("s", &["stork", "stash", "status"]);

    let ExplicitTabCompletion::InsertCommonPrefixAndOpen {
        common_prefix,
        suggestions,
        replacement_span,
    } = results.explicit_tab_completion("s", PATH_SEPARATORS)
    else {
        panic!("multiple prefix suggestions should open the menu");
    };
    assert_eq!(common_prefix, "st");
    assert_eq!(replacement_span, Span::new(0, 1));
    assert_eq!(
        suggestions
            .iter()
            .map(|suggestion| suggestion.suggestion.display.as_str())
            .collect::<Vec<_>>(),
        ["stork", "stash", "status"]
    );
}

#[test]
fn explicit_tab_has_no_action_when_the_query_filters_every_candidate() {
    let results = results("a", &["alpha"]);

    assert!(matches!(
        results.explicit_tab_completion("z", PATH_SEPARATORS),
        ExplicitTabCompletion::NoAction
    ));
}

#[test]
fn explicit_tab_does_not_insert_a_case_insensitive_common_prefix() {
    let results = results("ab", &["Abacus", "Abandon"]);

    let ExplicitTabCompletion::Open { suggestions, .. } =
        results.explicit_tab_completion("ab", PATH_SEPARATORS)
    else {
        panic!("case-insensitive prefixes should only open the menu");
    };
    assert_eq!(suggestions.len(), 2);
}

#[test]
fn explicit_tab_computes_common_prefixes_on_utf8_boundaries() {
    let results = results("", &["éclair", "école"]);

    let ExplicitTabCompletion::InsertCommonPrefixAndOpen { common_prefix, .. } =
        results.explicit_tab_completion("", PATH_SEPARATORS)
    else {
        panic!("the shared Unicode prefix should be inserted");
    };
    assert_eq!(common_prefix, "éc");
}
