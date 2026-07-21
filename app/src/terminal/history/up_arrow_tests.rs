//! Tests for the shared [`prompt_history_for_terminal_view`] getter used by the
//! GUI and TUI up-arrow prompt-history menus.
use warpui::{App, EntityId};

use super::prompt_history_for_terminal_view;
use crate::ai::blocklist::history_model::mock_history_model_with_prompts;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};

/// Asserts that querying a history seeded with `prompts` (oldest-first) yields
/// exactly `expected`.
fn assert_prompt_history(prompts: &[&str], expected: &[&str]) {
    let prompts: Vec<String> = prompts.iter().map(|prompt| (*prompt).to_owned()).collect();
    let expected: Vec<String> = expected.iter().map(|entry| (*entry).to_owned()).collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| mock_history_model_with_prompts(prompts));
        app.read(|ctx| {
            let texts: Vec<String> = prompt_history_for_terminal_view(terminal_surface_id, ctx)
                .into_iter()
                .map(|entry| entry.query_text)
                .collect();
            assert_eq!(texts, expected);
        });
    });
}

#[test]
fn prompt_history_dedupes_orders_and_excludes_whitespace() {
    // Oldest-first submission order. "deploy the app" appears twice; the newer
    // occurrence wins and the older is dropped. The whitespace-only prompt must
    // never appear.
    assert_prompt_history(
        &[
            "deploy the app",
            "delete the cache",
            "deploy the app",
            "   ",
            "build the project",
        ],
        &["delete the cache", "deploy the app", "build the project"],
    );
}

#[test]
fn prompt_history_excludes_ignored_prompts() {
    let prompts: Vec<String> = ["deploy the app", "delete the cache", "build the project"]
        .iter()
        .map(|prompt| (*prompt).to_owned())
        .collect();
    App::test((), |app| async move {
        let terminal_surface_id = EntityId::new();
        app.add_singleton_model(move |_| mock_history_model_with_prompts(prompts));
        app.add_singleton_model(|_| {
            IgnoredSuggestionsModel::new(vec![(
                "delete the cache".to_owned(),
                SuggestionType::AIQuery,
            )])
        });
        app.read(|ctx| {
            let texts: Vec<String> = prompt_history_for_terminal_view(terminal_surface_id, ctx)
                .into_iter()
                .map(|entry| entry.query_text)
                .collect();
            // The ignored prompt is excluded; the rest remain in order.
            assert_eq!(
                texts,
                vec!["deploy the app".to_owned(), "build the project".to_owned()]
            );
        });
    });
}
