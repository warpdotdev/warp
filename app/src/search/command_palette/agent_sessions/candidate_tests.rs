use chrono::{DateTime, TimeZone, Utc};

use super::*;

fn candidate(
    session_id: &str,
    origin: CandidateOrigin,
    task_name: &str,
    last_active: Option<DateTime<Utc>>,
) -> AgentSessionCandidate {
    AgentSessionCandidate {
        agent: CLIAgent::Claude,
        session_id: session_id.to_owned(),
        project_name: "warp".to_owned(),
        task_name: task_name.to_owned(),
        cwd: "/repos/warp".to_owned(),
        origin,
        last_active,
    }
}

fn at(minute: u32) -> Option<DateTime<Utc>> {
    Some(Utc.with_ymd_and_hms(2026, 8, 3, 12, minute, 0).unwrap())
}

#[test]
fn merge_prefers_live_then_handle_then_scanned_for_the_same_session() {
    let merged = merge(
        vec![candidate("abc", CandidateOrigin::Live, "live name", None)],
        vec![candidate(
            "abc",
            CandidateOrigin::Handle,
            "handle name",
            at(1),
        )],
        vec![candidate(
            "abc",
            CandidateOrigin::Scanned,
            "scanned name",
            at(2),
        )],
    );

    assert_eq!(merged.len(), 1, "one session must yield exactly one row");
    assert_eq!(merged[0].origin, CandidateOrigin::Live);
    assert_eq!(merged[0].task_name, "live name");
}

#[test]
fn merge_prefers_the_handle_when_the_session_is_not_live() {
    let merged = merge(
        vec![],
        vec![candidate(
            "abc",
            CandidateOrigin::Handle,
            "handle name",
            at(1),
        )],
        vec![candidate(
            "abc",
            CandidateOrigin::Scanned,
            "scanned name",
            at(2),
        )],
    );

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].origin, CandidateOrigin::Handle);
}

#[test]
fn merge_keeps_distinct_sessions_and_orders_by_authority_then_recency() {
    let merged = merge(
        vec![candidate("live", CandidateOrigin::Live, "live", None)],
        vec![
            candidate("handle-old", CandidateOrigin::Handle, "old", at(1)),
            candidate("handle-new", CandidateOrigin::Handle, "new", at(20)),
        ],
        vec![candidate(
            "scanned-newest",
            CandidateOrigin::Scanned,
            "newest",
            at(30),
        )],
    );

    let ids: Vec<&str> = merged
        .iter()
        .map(|candidate| candidate.session_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["live", "handle-new", "handle-old", "scanned-newest"],
        "sources form bands (their timestamps are not comparable), and rows \
         sort newest-first inside each band"
    );
}

#[test]
fn merge_treats_the_same_id_under_different_agents_as_different_sessions() {
    let mut codex = candidate("abc", CandidateOrigin::Handle, "codex task", at(1));
    codex.agent = CLIAgent::Codex;

    let merged = merge(
        vec![],
        vec![
            candidate("abc", CandidateOrigin::Handle, "claude task", at(2)),
            codex,
        ],
        vec![],
    );

    assert_eq!(merged.len(), 2, "the dedupe key is (agent, session id)");
}
