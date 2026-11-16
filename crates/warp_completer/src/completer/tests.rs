use std::sync::Arc;

use itertools::Itertools;

use crate::completer::{Priority, Suggestion, SuggestionType};

/// Ordering tests for Suggestions
#[test]
fn test_suggestions_cmp_display() {
    let display_names = [
        "dir2/".to_owned(),
        "abc".to_owned(),
        "file".to_owned(),
        "abc2".to_owned(),
        "dir1".to_owned(),
    ];

    let suggestions: Vec<Suggestion> = display_names
        .iter()
        .map(|display| Suggestion {
            display: Arc::new(display.to_owned()),
            replacement: "dummy".to_owned(),
            description: None,
            suggestion_type: SuggestionType::Argument,
            priority: Priority::default(),
            override_icon: None,
            is_hidden: false,
            file_type: None,
            is_abbreviation: false,
        })
        .sorted_by(Suggestion::cmp_by_display)
        .collect();

    assert_eq!(suggestions[0].display.as_str(), display_names[1]);
    assert_eq!(suggestions[1].display.as_str(), display_names[3]);
    assert_eq!(suggestions[2].display.as_str(), display_names[4]);
    assert_eq!(suggestions[3].display.as_str(), display_names[0]);
    assert_eq!(suggestions[4].display.as_str(), display_names[2]);
}

#[test]
fn test_suggestions_cmp_by_reversed_priority_and_display() {
    let important_globally = Priority::max();
    let default = Priority::default();
    let not_important_globally = Priority::new(-40);
    let priorities = [default, important_globally, not_important_globally, default];

    let suggestions: Vec<Suggestion> = priorities
        .into_iter()
        .enumerate()
        .map(|(idx, priority)| Suggestion {
            display: Arc::new(format!("status_{}", priorities.len() - idx)),
            replacement: "status".to_owned(),
            description: None,
            suggestion_type: SuggestionType::Argument,
            priority,
            override_icon: None,
            is_hidden: false,
            file_type: None,
            is_abbreviation: false,
        })
        .sorted_by(Suggestion::cmp_by_reversed_priority_and_display)
        .collect();

    assert_eq!(suggestions[0].priority, important_globally);
    assert_eq!(suggestions[1].priority, default);
    assert_eq!(suggestions[2].priority, default);
    assert_eq!(suggestions[3].priority, not_important_globally);

    // For the suggestions that tie in priority, we should lexicographically compare their display
    // names, with lower lexicographic order being higher priority.
    assert!(suggestions[1].display < suggestions[2].display);
}
