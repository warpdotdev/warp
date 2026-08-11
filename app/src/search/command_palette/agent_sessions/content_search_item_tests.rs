use super::*;
use crate::terminal::CLIAgent;

const SESSION_ID: &str = "0198c0de-0000-4000-8000-000000000001";

fn item(partial: bool) -> ContentSearchItem {
    ContentSearchItem::new(
        ContentHit {
            agent: CLIAgent::Claude,
            session_id: SESSION_ID.to_owned(),
            project_name: "warp".to_owned(),
            task_name: "Rename the project rail".to_owned(),
            cwd: "/repos/warp".to_owned(),
            snippet: "…the POA-2236 regression…".to_owned(),
            snippet_match_indices: vec![4, 5, 6],
            partial,
        },
        0.5,
    )
}

#[test]
fn a_content_row_sits_in_the_content_tier() {
    assert_eq!(item(false).priority_tier(), CONTENT_ROW_TIER);
    assert_eq!(CONTENT_ROW_TIER, 2, "the content tier is the plan's tier 2");
}

#[test]
fn accepting_a_row_resumes_that_exact_session() {
    match item(false).accept_result() {
        CommandPaletteItemAction::ResumeAgentSession { agent, session_id } => {
            assert_eq!(agent, CLIAgent::Claude);
            assert_eq!(session_id, SESSION_ID);
        }
        other => panic!("expected ResumeAgentSession, got {other:?}"),
    }
}

#[test]
fn a_partial_scan_says_so() {
    // What `partial` changes is the meaning of a row's *absence*: the
    // transcript was too large to digest whole, so "not in the list" stops
    // meaning "not in the conversation". That has to reach the user.
    let help = item(true).accessibility_help_message().unwrap();
    assert!(help.contains("Only part of this conversation was searched."));

    let complete = item(false).accessibility_help_message().unwrap();
    assert!(!complete.contains("Only part"));
}
