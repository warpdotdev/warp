use super::*;
use crate::search::command_palette::agent_sessions::candidate::AgentSessionCandidate;
use crate::search::command_palette::agent_sessions::search::match_agent_session;
use crate::terminal::CLIAgent;

const SESSION_ID: &str = "0198c0de-0000-4000-8000-000000000000";

fn item(recency_bonus: f64) -> AgentSessionSearchItem {
    let candidate = AgentSessionCandidate {
        agent: CLIAgent::Claude,
        session_id: SESSION_ID.to_owned(),
        project_name: "warp".to_owned(),
        task_name: "Rename the project rail".to_owned(),
        cwd: "/repos/warp".to_owned(),
        origin: CandidateOrigin::Handle,
        last_active: None,
    };
    let matched = match_agent_session(&candidate, "").expect("empty query matches everything");
    AgentSessionSearchItem::new(matched, recency_bonus)
}

#[test]
fn a_session_row_sits_in_the_name_tier() {
    assert_eq!(item(0.5).priority_tier(), NAME_ROW_TIER);
    assert_eq!(NAME_ROW_TIER, 4, "the name tier is the plan's tier 4");
}

#[test]
fn accepting_a_row_resumes_that_exact_session() {
    match item(0.5).accept_result() {
        CommandPaletteItemAction::ResumeAgentSession { agent, session_id } => {
            assert_eq!(agent, CLIAgent::Claude);
            assert_eq!(session_id, SESSION_ID);
        }
        other => panic!("expected ResumeAgentSession, got {other:?}"),
    }
}

#[test]
fn recency_only_breaks_ties_between_equal_matches() {
    // Both rows scored 0 (the empty query), so the recency bonus alone decides
    // — and it can never exceed 1, so it can never outrank a better match.
    let newest = item(0.9);
    let oldest = item(0.1);

    assert!(newest.score() > oldest.score());
    assert!(newest.score() < OrderedFloat(1.0));
}
