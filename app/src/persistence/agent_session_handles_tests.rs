use diesel::SqliteConnection;
use diesel::prelude::*;
use diesel_migrations::MigrationHarness;

use super::*;
use crate::persistence::schema::agent_session_handles::dsl;

/// Builds an in-memory SQLite database with all migrations applied.
fn test_connection() -> SqliteConnection {
    let mut conn =
        SqliteConnection::establish(":memory:").expect("in-memory sqlite connection should open");
    conn.run_pending_migrations(::persistence::MIGRATIONS)
        .expect("migrations should run");
    conn
}

fn all_rows(conn: &mut SqliteConnection) -> Vec<AgentSessionHandleRecord> {
    load_all(conn).expect("load_all should succeed")
}

const PANE_A: &[u8] = b"pane-aaaaaaaaaaa";
const PANE_B: &[u8] = b"pane-bbbbbbbbbbb";
const CWD: &str = "/Users/example/dev/project";

#[test]
fn inflight_insert_then_identify_promotes_the_same_row() {
    let mut conn = test_connection();
    insert_inflight(&mut conn, "Claude", PANE_A, CWD).unwrap();

    let rows = all_rows(&mut conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, None);

    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();
    let rows = all_rows(&mut conn);
    assert_eq!(rows.len(), 1, "identify must promote, not add");
    assert_eq!(rows[0].session_id.as_deref(), Some("11111111-aaaa"));
}

#[test]
fn second_session_start_on_same_pane_keeps_one_inflight_row() {
    let mut conn = test_connection();
    insert_inflight(&mut conn, "Claude", PANE_A, CWD).unwrap();
    insert_inflight(&mut conn, "Claude", PANE_A, CWD).unwrap();

    let inflight: i64 = dsl::agent_session_handles
        .filter(dsl::session_id.is_null())
        .count()
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(
        inflight, 1,
        "a pane has at most one in-flight launch per agent"
    );
}

#[test]
fn same_session_id_from_another_pane_merges_not_duplicates() {
    let mut conn = test_connection();
    insert_inflight(&mut conn, "Claude", PANE_A, CWD).unwrap();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();

    // Resume the same upstream session from a different pane.
    insert_inflight(&mut conn, "Claude", PANE_B, CWD).unwrap();
    identify(&mut conn, "Claude", PANE_B, CWD, "11111111-aaaa").unwrap();

    let rows = all_rows(&mut conn);
    assert_eq!(rows.len(), 1, "one task row per upstream session");
    assert_eq!(
        rows[0].pane_uuid, PANE_B,
        "provenance moves to the resuming pane"
    );
}

#[test]
fn two_distinct_sessions_on_one_pane_are_two_rows() {
    let mut conn = test_connection();
    insert_inflight(&mut conn, "Claude", PANE_A, CWD).unwrap();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();
    insert_inflight(&mut conn, "Claude", PANE_A, CWD).unwrap();
    identify(&mut conn, "Claude", PANE_A, CWD, "22222222-bbbb").unwrap();

    assert_eq!(
        all_rows(&mut conn).len(),
        2,
        "full history: earlier session survives"
    );
}

#[test]
fn identify_without_prior_session_start_creates_the_task_row() {
    let mut conn = test_connection();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();

    let rows = all_rows(&mut conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id.as_deref(), Some("11111111-aaaa"));
}

#[test]
fn different_agents_keep_separate_inflight_slots_on_one_pane() {
    let mut conn = test_connection();
    insert_inflight(&mut conn, "Claude", PANE_A, CWD).unwrap();
    insert_inflight(&mut conn, "Codex", PANE_A, CWD).unwrap();

    assert_eq!(all_rows(&mut conn).len(), 2);
}

#[test]
fn touch_updates_last_seen_at() {
    let mut conn = test_connection();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();
    let before = all_rows(&mut conn)[0].last_seen_at;

    // NaiveDateTime has sub-second precision; force an observable delta.
    diesel::update(dsl::agent_session_handles)
        .set(dsl::last_seen_at.eq(before - chrono::Duration::hours(1)))
        .execute(&mut conn)
        .unwrap();

    touch(&mut conn, "Claude", "11111111-aaaa").unwrap();
    assert!(all_rows(&mut conn)[0].last_seen_at > before - chrono::Duration::hours(1));
}

#[test]
fn set_title_caches_the_label() {
    let mut conn = test_connection();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();
    set_title(&mut conn, "Claude", "11111111-aaaa", "Fix retry backoff").unwrap();

    assert_eq!(
        all_rows(&mut conn)[0].title.as_deref(),
        Some("Fix retry backoff")
    );
}

#[test]
fn set_read_state_round_trips_both_bits() {
    let mut conn = test_connection();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();

    let row = all_rows(&mut conn).remove(0);
    assert!(
        !row.success_seen && !row.marked_unread,
        "defaults are unread-neutral"
    );

    set_read_state(&mut conn, "Claude", "11111111-aaaa", false, true).unwrap();
    let row = all_rows(&mut conn).remove(0);
    assert!(!row.success_seen && row.marked_unread);

    set_read_state(&mut conn, "Claude", "11111111-aaaa", true, false).unwrap();
    let row = all_rows(&mut conn).remove(0);
    assert!(row.success_seen && !row.marked_unread);
}

#[test]
fn forget_removes_exactly_the_named_task() {
    let mut conn = test_connection();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();
    identify(&mut conn, "Claude", PANE_B, CWD, "22222222-bbbb").unwrap();

    forget(&mut conn, "Claude", "11111111-aaaa").unwrap();
    let rows = all_rows(&mut conn);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id.as_deref(), Some("22222222-bbbb"));
}

#[test]
fn gc_ages_out_old_handles_on_write() {
    let mut conn = test_connection();
    identify(&mut conn, "Claude", PANE_A, CWD, "11111111-aaaa").unwrap();

    // Backdate the handle past the age window, then trigger a write.
    let ancient = chrono::Utc::now().naive_utc() - chrono::Duration::days(31);
    diesel::update(dsl::agent_session_handles)
        .set(dsl::last_seen_at.eq(ancient))
        .execute(&mut conn)
        .unwrap();

    identify(&mut conn, "Claude", PANE_B, CWD, "22222222-bbbb").unwrap();
    let rows = all_rows(&mut conn);
    assert_eq!(rows.len(), 1, "aged-out handle is pruned by the next write");
    assert_eq!(rows[0].session_id.as_deref(), Some("22222222-bbbb"));
}

#[test]
fn gc_caps_handles_per_cwd_keeping_most_recent() {
    let mut conn = test_connection();
    for i in 0..25 {
        let pane = format!("pane-{i:>11}");
        identify(
            &mut conn,
            "Claude",
            pane.as_bytes(),
            CWD,
            &format!("session-{i:04}"),
        )
        .unwrap();
    }

    let rows = all_rows(&mut conn);
    assert_eq!(rows.len(), 20, "per-cwd cap holds");

    // Another cwd is unaffected by this cwd's cap.
    identify(
        &mut conn,
        "Claude",
        PANE_B,
        "/Users/example/dev/other",
        "other-1",
    )
    .unwrap();
    assert_eq!(all_rows(&mut conn).len(), 21);
}
