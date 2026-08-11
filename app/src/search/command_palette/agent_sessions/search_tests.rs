use super::*;
use crate::search::command_palette::agent_sessions::candidate::CandidateOrigin;
use crate::terminal::CLIAgent;

fn candidate(task_name: &str, project_name: &str, cwd: &str) -> AgentSessionCandidate {
    AgentSessionCandidate {
        agent: CLIAgent::Claude,
        session_id: "0198c0de-0000-4000-8000-000000000000".to_owned(),
        project_name: project_name.to_owned(),
        task_name: task_name.to_owned(),
        cwd: cwd.to_owned(),
        origin: CandidateOrigin::Handle,
        last_active: None,
    }
}

#[test]
fn matches_the_task_name() {
    let candidate = candidate("Rename the project rail", "warp", "/repos/warp");
    let matched = match_agent_session(&candidate, "rail").expect("task name should match");

    assert!(matched.score() > 0);
    assert!(!matched.highlight_indices().task_indices().is_empty());
    assert!(matched.highlight_indices().project_indices().is_empty());
}

#[test]
fn matches_the_project_name() {
    let candidate = candidate("Rename the rail", "zuellig-market", "/repos/market");
    let matched = match_agent_session(&candidate, "zuellig").expect("project name should match");

    assert!(!matched.highlight_indices().project_indices().is_empty());
    assert!(matched.highlight_indices().task_indices().is_empty());
}

#[test]
fn matches_the_working_directory() {
    let candidate = candidate("Rename the rail", "warp", "/repos/poa-agent");
    let matched = match_agent_session(&candidate, "poa").expect("cwd should match");

    assert!(!matched.highlight_indices().cwd_indices().is_empty());
    assert!(matched.highlight_indices().task_indices().is_empty());
}

#[test]
fn drops_a_candidate_that_matches_no_field() {
    assert!(
        match_agent_session(
            &candidate("Rename the rail", "warp", "/repos/warp"),
            "kubernetes"
        )
        .is_none()
    );
}

#[test]
fn an_empty_query_matches_every_candidate_with_no_score() {
    let candidates = vec![
        candidate("first", "warp", "/repos/warp"),
        candidate("second", "warp", "/repos/warp"),
    ];
    let matched: Vec<_> = filter_agent_sessions(&candidates, "").collect();

    assert_eq!(matched.len(), 2);
    assert!(matched.iter().all(|matched| matched.score() == 0));
}

#[test]
fn highlight_indices_are_char_indices_not_byte_indices() {
    // "Café" is 5 bytes and 4 chars, so a match on the following word lands on
    // a different number depending on which the matcher counts in. The text
    // elements highlight by char, so a byte index would underline the wrong
    // letters on any non-ASCII label.
    let candidate = candidate("Café rail", "warp", "/repos/warp");
    let matched = match_agent_session(&candidate, "rail").expect("task name should match");

    assert_eq!(
        matched.highlight_indices().task_indices(),
        &vec![5, 6, 7, 8],
        "'rail' starts at char 5 of \"Café rail\" (byte 6)"
    );
}

#[test]
fn matching_is_case_insensitive() {
    let candidate = candidate("Rename the project RAIL", "warp", "/repos/warp");
    assert!(match_agent_session(&candidate, "rail").is_some());
}
