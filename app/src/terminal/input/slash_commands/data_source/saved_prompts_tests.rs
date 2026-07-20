use std::sync::Arc;

use ordered_float::OrderedFloat;

use super::{SavedPromptCandidate, SavedPromptsSnapshot, fuzzy_match_saved_prompts};
use crate::server::ids::{ClientId, SyncId};
use crate::workflows::CloudWorkflowModel;
use crate::workflows::workflow::Workflow;

const TEST_FONT_FAMILY: warpui::fonts::FamilyId = warpui::fonts::FamilyId(0);

fn test_candidate(name: &str, query: &str) -> SavedPromptCandidate {
    SavedPromptCandidate {
        id: SyncId::ClientId(ClientId::new()),
        model: Arc::new(CloudWorkflowModel {
            data: Workflow::AgentMode {
                name: name.to_owned(),
                query: query.to_owned(),
                description: None,
                arguments: vec![],
            },
        }),
        breadcrumbs: "Personal".to_owned(),
    }
}

fn run_match(
    query_text: &str,
    candidates: Vec<SavedPromptCandidate>,
) -> Vec<crate::search::data_source::QueryResult<super::AcceptSlashCommandOrSavedPrompt>> {
    run_match_with_ai(query_text, candidates, true)
}

fn run_match_with_ai(
    query_text: &str,
    candidates: Vec<SavedPromptCandidate>,
    ai_enabled: bool,
) -> Vec<crate::search::data_source::QueryResult<super::AcceptSlashCommandOrSavedPrompt>> {
    futures_lite::future::block_on(fuzzy_match_saved_prompts(SavedPromptsSnapshot {
        candidates,
        query_text: query_text.to_owned(),
        font_family: TEST_FONT_FAMILY,
        ai_enabled,
    }))
    .unwrap()
}

#[test]
fn test_empty_query_returns_no_results() {
    let candidates = vec![test_candidate("My Prompt", "do something")];
    let results = run_match("", candidates);
    assert!(results.is_empty());
}

#[test]
fn test_matching_query_returns_results() {
    let candidates = vec![test_candidate(
        "Refactor Code",
        "refactor the selected code",
    )];
    let results = run_match("refactor", candidates);
    assert_eq!(results.len(), 1);
    assert!(results[0].score() > OrderedFloat(0.0));
}

#[test]
fn test_non_matching_query_returns_no_results() {
    let candidates = vec![test_candidate(
        "Refactor Code",
        "refactor the selected code",
    )];
    let results = run_match("zzzzzzzzz", candidates);
    assert!(results.is_empty());
}

#[test]
fn test_weak_matches_filtered_when_query_len_over_one() {
    // A two-character query that barely matches should be filtered by the score > 25 threshold.
    let candidates = vec![test_candidate(
        "A very long prompt name",
        "some content here",
    )];
    let results = run_match("zx", candidates);
    assert!(results.is_empty());
}

#[test]
fn test_single_char_query_uses_prefix_match() {
    // Single-character queries use prefix matching on the name.
    let candidates = vec![test_candidate("Abc", "do something")];
    let results = run_match("a", candidates);
    assert!(!results.is_empty());
}

#[test]
fn test_single_char_query_no_prefix_match_returns_empty() {
    // Single-character queries that don't prefix-match the name should return no results,
    // even if the character appears elsewhere in the name or content.
    let candidates = vec![test_candidate("Refactor Code", "apply changes")];
    let results = run_match("a", candidates);
    assert!(results.is_empty());
}

#[test]
fn test_single_char_query_matches_multiple_by_prefix() {
    let candidates = vec![
        test_candidate("Deploy to Production", "deploy app"),
        test_candidate("Debug Tests", "run debugger"),
        test_candidate("Run Tests", "cargo test"),
    ];
    let results = run_match("d", candidates);
    assert_eq!(results.len(), 2);
}

#[test]
fn test_multiple_workflows_returns_matching_subset() {
    let candidates = vec![
        test_candidate("Deploy to Production", "deploy app to prod"),
        test_candidate("Run Tests", "cargo test"),
        test_candidate("Deploy Staging", "deploy app to staging"),
    ];
    let results = run_match("deploy", candidates);
    assert_eq!(results.len(), 2);
}

#[test]
fn test_ai_disabled_returns_no_results_even_for_matching_query() {
    // Saved prompts are AI-destined, so when AI is globally disabled they should be hidden
    // regardless of whether the query would otherwise match.
    let candidates = vec![test_candidate(
        "Refactor Code",
        "refactor the selected code",
    )];
    let results = run_match_with_ai("refactor", candidates, false);
    assert!(results.is_empty());
}
