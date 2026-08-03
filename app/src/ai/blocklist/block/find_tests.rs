use std::collections::HashMap;
use std::sync::Arc;

use super::compute_find_matches;
use crate::ai::agent::{AIAgentInput, UserQueryMode};
use crate::terminal::find::FindOptions;

fn user_query_input(query: &str, mode: UserQueryMode) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_string(),
        context: Arc::new([]),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: mode,
        running_command: None,
        intended_agent: None,
    }
}

// Regression test for https://github.com/warpdotdev/warp/issues/11697.
// Find match highlights on the initial agent query must be computed against the displayed
// query (prefixed with "/agent "), not the stored query, so the ranges line up with the
// rendered text.
#[test]
fn find_matches_align_with_agent_prefixed_initial_query() {
    let query = "list files".to_string();
    let input = user_query_input(&query, UserQueryMode::Normal);

    let displayed = input
        .display_user_query(Some(&query))
        .expect("user query should produce displayed text");
    assert_eq!(displayed, "/agent list files");

    let options = FindOptions::default().with_query(Some("list".to_string()));

    // "list" starts after the "/agent " prefix (7 chars), so the match must be 7..11 — not
    // 0..4, which is what matching against the un-prefixed stored query would produce.
    assert_eq!(compute_find_matches(&displayed, &options), vec![7..11]);
}

// A "/plan" query is stored with its prefix already attached, so its highlights have always
// lined up. Lock that in alongside the agent-prefix fix.
#[test]
fn find_matches_align_with_plan_prefixed_query() {
    let input = user_query_input("write tests", UserQueryMode::Plan);

    let displayed = input
        .display_user_query(None)
        .expect("user query should produce displayed text");
    assert_eq!(displayed, "/plan write tests");

    let options = FindOptions::default().with_query(Some("tests".to_string()));
    assert_eq!(compute_find_matches(&displayed, &options), vec![12..17]);
}
