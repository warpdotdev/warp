use chrono::Local;
use std::collections::HashSet;
use warp_core::ui::appearance::Appearance;
use warpui::{platform::WindowStyle, App};

use crate::ai::blocklist::AIQueryHistory;
use crate::input_suggestions::{filter_tab_suggestions, HistoryOrder};
use crate::terminal::model::session::SessionId;
use crate::terminal::HistoryEntry;
use warp_completer::completer::{
    EngineFileType, Match, MatchStrategy, MatchedSuggestion, Priority, Suggestion,
    SuggestionResults, SuggestionType, TopLevelCommandCaseSensitivity,
};
use warp_completer::meta::Span;

use super::{HistoryInputSuggestion, InputSuggestions, TabCompletionsPreselectOption};

fn prefix_matched_suggestion(name: &str) -> MatchedSuggestion {
    let suggestion = Suggestion::with_same_display_and_replacement(
        name,
        None,
        SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseInsensitive),
        Priority::default(),
    );
    MatchedSuggestion::new(
        suggestion,
        Match::Prefix {
            is_case_sensitive: true,
        },
    )
}

#[test]
fn test_basic_tab_prefix() {
    let suggestions = vec![
        prefix_matched_suggestion("stash"),
        prefix_matched_suggestion("status"),
        prefix_matched_suggestion("stats/")
            .with_file_type(warp_completer::completer::EngineFileType::Directory),
    ];

    let suggestion_results = SuggestionResults {
        replacement_span: Span::default(),
        suggestions,
        match_strategy: MatchStrategy::CaseSensitive,
    };
    let matches = filter_tab_suggestions(&suggestion_results, "st", &['/'])
        .into_iter()
        .map(|item| item.matches)
        .collect::<Vec<_>>();

    assert_eq!(
        matches,
        vec![
            Some((0..2).collect()),
            Some((0..2).collect()),
            Some((0..2).collect()),
        ]
    )
}

#[test]
fn test_basic_tab_fuzzy() {
    let suggestions = vec![
        prefix_matched_suggestion("fast"),
        prefix_matched_suggestion("salt"),
        prefix_matched_suggestion("must").with_file_type(EngineFileType::File),
        prefix_matched_suggestion("first/").with_file_type(EngineFileType::Directory),
    ];

    let suggestion_results = SuggestionResults {
        replacement_span: Span::default(),
        suggestions,
        match_strategy: MatchStrategy::Fuzzy,
    };
    let matches = filter_tab_suggestions(&suggestion_results, "st", &['/'])
        .into_iter()
        .map(|item| (item.display, item.matches))
        .collect::<Vec<_>>();

    assert_eq!(
        matches,
        vec![
            (Some("salt".to_owned()), Some(vec![0, 3])),
            (Some("first/".to_owned()), Some(vec![3, 4])),
            (Some("must".to_owned()), Some(vec![2, 3])),
            (Some("fast".to_owned()), Some(vec![2, 3])),
        ]
    );
}

#[test]
fn test_case_insensitive_filter() {
    let suggestions = vec![
        prefix_matched_suggestion("/Users/"),
        prefix_matched_suggestion("/usr/"),
        prefix_matched_suggestion("/uninstalled/"),
        prefix_matched_suggestion("first/"),
    ];

    let suggestion_results = SuggestionResults {
        replacement_span: Span::default(),
        suggestions,
        match_strategy: MatchStrategy::CaseInsensitive,
    };
    let matches = filter_tab_suggestions(&suggestion_results, "/us", &['/']);

    assert_eq!(matches.len(), 2);
}

#[test]
// TODO This delta exists right now for filepath completions as a quirk
// of how our tab completions work. In the future, we won't be
// including the entire filepath in the replacement and this code
// will have to change accordingly.
fn test_display_replacement_prefix() {
    let suggestions = vec![
        prefix_matched_suggestion("rest/")
            .with_replacement("src/rest/")
            .with_file_type(EngineFileType::Directory),
        prefix_matched_suggestion("rust/")
            .with_replacement("src/rust/")
            .with_file_type(EngineFileType::Directory),
    ];

    let suggestion_results = SuggestionResults {
        replacement_span: Span::default(),
        suggestions,
        match_strategy: MatchStrategy::CaseSensitive,
    };
    let matches = filter_tab_suggestions(&suggestion_results, "src/r", &['/'])
        .into_iter()
        .map(|item| item.matches)
        .collect::<Vec<_>>();

    assert_eq!(matches, vec![Some(vec![0]), Some(vec![0]),])
}

#[test]
fn test_display_replacement_fuzzy() {
    // It doesn't actually matter that we use `prefix_matched_suggestion`s
    // here because we recompute the match type in filter by query anyways.
    let suggestions = vec![
        prefix_matched_suggestion("stutter/")
            .with_replacement("src/stutter/")
            .with_file_type(EngineFileType::Directory),
        prefix_matched_suggestion("rust/")
            .with_replacement("src/rust/")
            .with_file_type(EngineFileType::Directory),
    ];

    let suggestion_results = SuggestionResults {
        replacement_span: Span::default(),
        suggestions,
        match_strategy: MatchStrategy::Fuzzy,
    };
    let matches = filter_tab_suggestions(&suggestion_results, "src/ut", &['/'])
        .into_iter()
        .map(|item| (item.display, item.matches))
        .collect::<Vec<_>>();

    assert_eq!(
        matches,
        vec![
            (Some("stutter/".to_owned()), Some(vec![2, 3])),
            (Some("rust/".to_owned()), Some(vec![1, 3])),
        ]
    );
}

#[test]
fn test_fuzzy_substring_search_with_whitespace() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, suggestions) = app.add_window(WindowStyle::NotStealFocus, InputSuggestions::new);

        let options = vec!["axbycz".to_string(), "   axbycz".to_string()];
        suggestions.update(&mut app, |input_suggestions, ctx_input_suggestions| {
            input_suggestions.fuzzy_substring_search(
                "abc".to_string(),
                options,
                ctx_input_suggestions,
            );
        });
        suggestions.read(&app, |suggestions, _| {
            let highlights = suggestions
                .items()
                .iter()
                .map(|item| item.matches())
                .collect::<Vec<_>>();
            assert_eq!(highlights, [Some(&vec![0, 2, 4]), Some(&vec![0, 2, 4])]);
        });
    });
}

#[test]
fn test_preselect_first_item() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, suggestions) = app.add_window(WindowStyle::NotStealFocus, InputSuggestions::new);

        let options = vec![
            prefix_matched_suggestion("a1"),
            prefix_matched_suggestion("a2"),
        ];
        let suggestion_results = SuggestionResults {
            replacement_span: Span::default(),
            suggestions: options,
            match_strategy: MatchStrategy::CaseSensitive,
        };

        suggestions.update(&mut app, |input_suggestions, ctx| {
            input_suggestions.prefix_search_for_tab_completion(
                "a",
                &suggestion_results,
                TabCompletionsPreselectOption::First,
                ctx,
            );
        });

        suggestions.read(&app, |suggestions, _| {
            assert_eq!(suggestions.get_selected_item().unwrap().text.as_str(), "a1");
        });
    });
}

#[test]
fn test_unselected_state() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, suggestions) = app.add_window(WindowStyle::NotStealFocus, InputSuggestions::new);

        let options = vec![
            prefix_matched_suggestion("a1"),
            prefix_matched_suggestion("a2"),
        ];
        let suggestion_results = SuggestionResults {
            replacement_span: Span::default(),
            suggestions: options,
            match_strategy: MatchStrategy::CaseSensitive,
        };

        suggestions.update(&mut app, |input_suggestions, ctx| {
            input_suggestions.prefix_search_for_tab_completion(
                "a",
                &suggestion_results,
                TabCompletionsPreselectOption::Unselected,
                ctx,
            );
        });

        suggestions.read(&app, |suggestions, _| {
            assert!(suggestions.get_selected_item().is_none());
        });

        suggestions.update(&mut app, |input_suggestions, ctx| {
            input_suggestions.select_next(ctx);
        });

        suggestions.read(&app, |suggestions, _| {
            assert_eq!(suggestions.get_selected_item().unwrap().text.as_str(), "a1");
        });
    });
}

#[test]
fn test_unchanged_preselect_option() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, suggestions) = app.add_window(WindowStyle::NotStealFocus, InputSuggestions::new);

        let options = vec![
            prefix_matched_suggestion("a1"),
            prefix_matched_suggestion("a2"),
        ];
        let suggestion_results = SuggestionResults {
            replacement_span: Span::default(),
            suggestions: options,
            match_strategy: MatchStrategy::CaseSensitive,
        };

        suggestions.update(&mut app, |input_suggestions, ctx| {
            input_suggestions.prefix_search_for_tab_completion(
                "a",
                &suggestion_results,
                TabCompletionsPreselectOption::First,
                ctx,
            );
        });

        suggestions.update(&mut app, |input_suggestions, ctx| {
            input_suggestions.select_next(ctx);
        });

        suggestions.read(&app, |suggestions, _| {
            assert_eq!(suggestions.get_selected_item().unwrap().text.as_str(), "a2");
        });

        suggestions.update(&mut app, |input_suggestions, ctx| {
            input_suggestions.exit(true, ctx);
        });

        suggestions.update(&mut app, |input_suggestions, ctx| {
            input_suggestions.prefix_search_for_tab_completion(
                "a",
                &suggestion_results,
                TabCompletionsPreselectOption::Unchanged,
                ctx,
            );
        });

        suggestions.read(&app, |suggestions, _| {
            assert_eq!(suggestions.get_selected_item().unwrap().text.as_str(), "a2");
        });
    });
}

#[test]
fn test_history_order() {
    let mut live_sessions = HashSet::new();
    let current_session_id = SessionId::from(3);
    live_sessions.insert(SessionId::from(1));
    live_sessions.insert(SessionId::from(2));
    live_sessions.insert(current_session_id);
    let now = Local::now();

    // Commands in current session
    let current_session_cmd = HistoryInputSuggestion::Command {
        entry: &HistoryEntry::command_at_time(
            "echo current session".to_string(),
            now,
            Some(current_session_id),
            false,
        ),
    };
    assert_eq!(
        current_session_cmd.history_order(Some(current_session_id), &live_sessions,),
        HistoryOrder::CurrentSession
    );

    // Commands in different live session
    let different_session_cmd = HistoryInputSuggestion::Command {
        entry: &HistoryEntry::command_at_time(
            "echo different session".to_string(),
            now,
            Some(SessionId::from(1)),
            false,
        ),
    };
    assert_eq!(
        different_session_cmd.history_order(Some(current_session_id), &live_sessions,),
        HistoryOrder::DifferentSession
    );

    // Restored commands in current session are treated as CurrentSession
    let restored_cmd = HistoryInputSuggestion::Command {
        entry: &HistoryEntry::command_at_time("echo restored".to_string(), now, None, true),
    };
    assert_eq!(
        restored_cmd.history_order(Some(current_session_id), &live_sessions,),
        HistoryOrder::CurrentSession
    );

    // Commands with no session are treated as DifferentSession
    let no_session_cmd = HistoryInputSuggestion::Command {
        entry: &HistoryEntry::command_at_time(
            "echo no session".to_string(),
            now - chrono::Duration::seconds(10),
            None,
            false,
        ),
    };
    assert_eq!(
        no_session_cmd.history_order(Some(current_session_id), &live_sessions,),
        HistoryOrder::DifferentSession
    );

    // AI queries from current session
    let ai_query_current = HistoryInputSuggestion::AIQuery {
        entry: AIQueryHistory::new_for_test(
            "ai query current session",
            now,
            HistoryOrder::CurrentSession,
        ),
    };
    assert_eq!(
        ai_query_current.history_order(Some(current_session_id), &live_sessions,),
        HistoryOrder::CurrentSession
    );

    // AI queries from different session
    let ai_query_different = HistoryInputSuggestion::AIQuery {
        entry: AIQueryHistory::new_for_test(
            "ai query different session",
            now,
            HistoryOrder::DifferentSession,
        ),
    };
    assert_eq!(
        ai_query_different.history_order(Some(current_session_id), &live_sessions,),
        HistoryOrder::DifferentSession
    );
}
