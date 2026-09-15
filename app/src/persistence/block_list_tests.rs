//! Unit tests for the `ai_queries` and restored-block persistence layer in [`super`].
//!
//! Covers the FIFO eviction cap added to [`super::upsert_ai_query`], the empty-input filter
//! that drives the persistence skip in `handle_ai_history_event`, and the SQL per-pane cap in
//! [`super::get_all_restored_blocks`].

use std::sync::Arc;

use chrono::{DateTime, Duration, Local, NaiveDateTime};
use diesel::connection::SimpleConnection;
use diesel::sqlite::SqliteConnection;
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl};
use diesel_migrations::MigrationHarness;

use super::{
    MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION, get_all_restored_blocks,
    process_ai_queries_for_nld_history_match, process_ai_queries_for_uparrow_prompt,
    read_recent_ai_queries, upsert_ai_query_with_limit,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentExchangeId, AIAgentInput, UserQueryMode};
use crate::ai::blocklist::{
    AIQueryHistoryOutputStatus, PersistedAIInput, PersistedAIInputType, SerializedBlockListItem,
};
use crate::ai::llms::LLMId;
use crate::app_state::PaneUuid;
use crate::persistence::model::{NewBlock, NewTerminalPane};
use crate::persistence::schema;

/// Builds an in-memory SQLite database with all migrations applied.
fn test_connection() -> SqliteConnection {
    let mut conn =
        SqliteConnection::establish(":memory:").expect("in-memory sqlite connection should open");
    conn.run_pending_migrations(::persistence::MIGRATIONS)
        .expect("migrations should run");
    conn
}

/// Builds a query-bearing [`PersistedAIInput`] with a fresh, unique `exchange_id`.
fn make_query(text: &str) -> Arc<PersistedAIInput> {
    Arc::new(PersistedAIInput {
        exchange_id: AIAgentExchangeId::new(),
        conversation_id: AIConversationId::new(),
        start_ts: Local::now(),
        inputs: vec![PersistedAIInputType::Query {
            text: text.to_string(),
            context: Default::default(),
            referenced_attachments: Default::default(),
        }],
        output_status: AIQueryHistoryOutputStatus::Completed,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        coding_model_id: LLMId::from("test-coding-model"),
    })
}

/// Clones `query` with an explicit `start_ts` so ordering-sensitive tests are deterministic
/// (the NLD reader orders by `start_ts`, which `make_query`'s `Local::now()` cannot guarantee
/// across rapid inserts).
fn with_start_ts(query: Arc<PersistedAIInput>, start_ts: DateTime<Local>) -> Arc<PersistedAIInput> {
    Arc::new(PersistedAIInput {
        start_ts,
        ..(*query).clone()
    })
}

fn ai_query_count(conn: &mut SqliteConnection) -> i64 {
    use crate::persistence::schema::ai_queries::dsl::ai_queries;
    ai_queries
        .count()
        .first(conn)
        .expect("count query should succeed")
}

/// Returns the persisted `exchange_id`s ordered by `id` ascending (i.e. insertion / FIFO order).
fn remaining_exchange_ids(conn: &mut SqliteConnection) -> Vec<String> {
    use crate::persistence::schema::ai_queries::dsl::{ai_queries, exchange_id, id};
    ai_queries
        .select(exchange_id)
        .order(id.asc())
        .load::<String>(conn)
        .expect("load query should succeed")
}

fn input_json_for_exchange(conn: &mut SqliteConnection, exchange: &str) -> String {
    use crate::persistence::schema::ai_queries::dsl::{ai_queries, exchange_id, input};
    ai_queries
        .filter(exchange_id.eq(exchange))
        .select(input)
        .first::<String>(conn)
        .expect("row for exchange should exist")
}

/// Returns the text of the first query input on a [`PersistedAIInput`].
fn first_query_text(query: &PersistedAIInput) -> &str {
    match query.inputs.first().expect("query should have an input") {
        PersistedAIInputType::Query { text, .. } => text,
    }
}

#[test]
fn upsert_ai_query_caps_table_and_evicts_oldest_first() {
    let mut conn = test_connection();
    let limit = 3;

    // Insert five distinct exchanges into a table capped at three.
    let queries: Vec<Arc<PersistedAIInput>> =
        (0..5).map(|i| make_query(&format!("q{i}"))).collect();
    let exchange_ids: Vec<String> = queries.iter().map(|q| q.exchange_id.to_string()).collect();

    for query in &queries {
        upsert_ai_query_with_limit(&mut conn, query.clone(), limit).expect("upsert should succeed");
    }

    // The table never exceeds the limit.
    assert_eq!(ai_query_count(&mut conn), limit);

    // The two oldest (q0, q1) are evicted; the three newest remain in insertion order.
    assert_eq!(
        remaining_exchange_ids(&mut conn),
        exchange_ids[2..].to_vec()
    );
}

#[test]
fn upsert_ai_query_stays_below_limit_without_evicting() {
    let mut conn = test_connection();
    let limit = 3;

    // Filling exactly up to the limit should not evict anything.
    let queries: Vec<Arc<PersistedAIInput>> =
        (0..3).map(|i| make_query(&format!("q{i}"))).collect();
    let exchange_ids: Vec<String> = queries.iter().map(|q| q.exchange_id.to_string()).collect();

    for query in &queries {
        upsert_ai_query_with_limit(&mut conn, query.clone(), limit).expect("upsert should succeed");
    }

    assert_eq!(ai_query_count(&mut conn), limit);
    assert_eq!(remaining_exchange_ids(&mut conn), exchange_ids);
}

#[test]
fn upsert_ai_query_updates_existing_exchange_without_evicting() {
    let mut conn = test_connection();
    let limit = 2;

    // Fill the table to its limit with two distinct exchanges.
    let first = make_query("first");
    let second = make_query("second");
    upsert_ai_query_with_limit(&mut conn, first.clone(), limit).expect("upsert should succeed");
    upsert_ai_query_with_limit(&mut conn, second.clone(), limit).expect("upsert should succeed");
    assert_eq!(ai_query_count(&mut conn), limit);

    // Re-upsert the oldest exchange (same `exchange_id`) repeatedly. Because this is an update of
    // an existing exchange rather than a new one, it must update in place and never evict.
    let updated_first = Arc::new(PersistedAIInput {
        inputs: vec![PersistedAIInputType::Query {
            text: "first-updated".to_string(),
            context: Default::default(),
            referenced_attachments: Default::default(),
        }],
        ..(*first).clone()
    });
    for _ in 0..5 {
        upsert_ai_query_with_limit(&mut conn, updated_first.clone(), limit)
            .expect("upsert should succeed");
    }

    // Still exactly two rows, and both original exchanges survive (the oldest was not evicted).
    assert_eq!(ai_query_count(&mut conn), limit);
    assert_eq!(
        remaining_exchange_ids(&mut conn),
        vec![
            first.exchange_id.to_string(),
            second.exchange_id.to_string()
        ]
    );

    // The in-place update took effect.
    let input_json = input_json_for_exchange(&mut conn, &first.exchange_id.to_string());
    assert!(
        input_json.contains("first-updated"),
        "existing row should have been updated in place, got: {input_json}"
    );
}

/// Builds a [`PersistedAIInput`] whose inputs serialize to `[]`, mirroring legacy rows
/// written before empty inputs were skipped at write time.
fn make_empty_input_query() -> Arc<PersistedAIInput> {
    Arc::new(PersistedAIInput {
        inputs: vec![],
        ..(*make_query("unused")).clone()
    })
}

#[test]
fn process_ai_queries_for_nld_history_match_filters_empty_and_whitespace_inputs_oldest_first() {
    let mut conn = test_connection();

    // Explicit, strictly increasing timestamps keep the `start_ts`-ordered read deterministic.
    let t0 = Local::now();
    for query in [
        with_start_ts(make_query("older prompt"), t0),
        with_start_ts(make_query("   "), t0 + Duration::seconds(1)),
        with_start_ts(make_empty_input_query(), t0 + Duration::seconds(2)),
        with_start_ts(make_query("newer prompt"), t0 + Duration::seconds(3)),
    ] {
        upsert_ai_query_with_limit(&mut conn, query, 10).expect("upsert should succeed");
    }

    let recent_ai_queries = read_recent_ai_queries(&mut conn).expect("read should succeed");
    let prompts = process_ai_queries_for_nld_history_match(&recent_ai_queries);
    let texts: Vec<&str> = prompts.iter().map(|(text, _)| text.as_str()).collect();
    // `[]` and whitespace-only rows are dropped; the rest come back oldest-first.
    assert_eq!(texts, vec!["older prompt", "newer prompt"]);
}

#[test]
fn process_ai_queries_for_uparrow_prompt_keeps_newest_capped_oldest_first() {
    // Build 150 oldest-first queries; only the newest 100 should survive, order preserved.
    let queries: Vec<PersistedAIInput> = (0..150)
        .map(|i| (*make_query(&format!("q{i}"))).clone())
        .collect();

    let kept = process_ai_queries_for_uparrow_prompt(queries);

    assert_eq!(kept.len(), 100);
    // The newest 100 (q50..=q149) survive, still oldest-first.
    assert_eq!(first_query_text(&kept[0]), "q50");
    assert_eq!(first_query_text(&kept[99]), "q149");
}

#[test]
fn process_ai_queries_for_uparrow_prompt_keeps_all_when_under_cap() {
    // Fewer than the cap: everything is kept, order preserved.
    let queries: Vec<PersistedAIInput> = (0..3)
        .map(|i| (*make_query(&format!("q{i}"))).clone())
        .collect();

    let kept = process_ai_queries_for_uparrow_prompt(queries);

    let texts: Vec<&str> = kept.iter().map(first_query_text).collect();
    assert_eq!(texts, vec!["q0", "q1", "q2"]);
}

#[test]
fn empty_input_skip_filters_out_non_query_inputs() {
    // Mirrors the filter in `handle_ai_history_event`: only query-bearing inputs are persisted.
    // An exchange whose inputs are all non-query types collapses to an empty `inputs` vec, which
    // is the exact condition that skips persistence.
    let user_query = AIAgentInput::UserQuery {
        query: "hello".to_string(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: UserQueryMode::default(),
        running_command: None,
        intended_agent: None,
    };
    let non_query = AIAgentInput::ResumeConversation {
        context: Default::default(),
    };

    // A query input is persistable; a non-query input is not.
    assert!(PersistedAIInputType::try_from(&user_query).is_ok());
    assert!(PersistedAIInputType::try_from(&non_query).is_err());

    // An exchange carrying only non-query inputs collapses to empty -> skipped.
    let only_non_query = [non_query];
    let persisted: Vec<_> = only_non_query
        .iter()
        .filter_map(|input| PersistedAIInputType::try_from(input).ok())
        .collect();
    assert!(persisted.is_empty());

    // An exchange carrying a query input is persisted.
    let with_query = [user_query];
    let persisted: Vec<_> = with_query
        .iter()
        .filter_map(|input| PersistedAIInputType::try_from(input).ok())
        .collect();
    assert_eq!(persisted.len(), 1);
}

const RESTORE_OVER_CAP_COUNT: usize = MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize + 1;

fn restore_test_connection() -> SqliteConnection {
    let mut conn = test_connection();
    conn.batch_execute("PRAGMA foreign_keys = OFF")
        .expect("foreign keys should disable for restore fixtures");
    conn
}

fn insert_pane(conn: &mut SqliteConnection, id: i32, uuid: &[u8]) {
    diesel::insert_into(schema::terminal_panes::table)
        .values(NewTerminalPane {
            id,
            uuid: uuid.to_vec(),
            cwd: None,
            is_active: false,
            shell_launch_data: None,
            input_config: None,
            llm_model_override: None,
            active_profile_id: None,
            conversation_ids: None,
            active_conversation_id: None,
        })
        .execute(conn)
        .expect("insert pane should succeed");
}

fn insert_block(
    conn: &mut SqliteConnection,
    pane_uuid: &[u8],
    block_id: &str,
    start_ts: Option<NaiveDateTime>,
) {
    let empty = Vec::new();
    diesel::insert_into(schema::blocks::table)
        .values(NewBlock {
            block_id,
            pane_leaf_uuid: pane_uuid.to_vec(),
            stylized_command: &empty,
            stylized_output: &empty,
            pwd: None,
            git_branch: None,
            git_branch_name: None,
            virtual_env: None,
            conda_env: None,
            exit_code: 0,
            did_execute: true,
            is_background: false,
            completed_ts: None,
            start_ts,
            ps1: None,
            rprompt: None,
            honor_ps1: false,
            shell: None,
            user: None,
            host: None,
            prompt_snapshot: None,
            ai_metadata: None,
            is_local: Some(true),
            agent_view_visibility: None,
        })
        .execute(conn)
        .expect("insert block should succeed");
}

fn insert_blocks_oldest_first(
    conn: &mut SqliteConnection,
    pane_uuid: &[u8],
    prefix: &str,
    count: usize,
    first_ts: NaiveDateTime,
) {
    for i in 0..count {
        insert_block(
            conn,
            pane_uuid,
            &format!("{prefix}{i:03}"),
            Some(first_ts + Duration::seconds(i as i64)),
        );
    }
}

fn insert_blocks_newest_first(
    conn: &mut SqliteConnection,
    pane_uuid: &[u8],
    prefix: &str,
    count: usize,
    first_ts: NaiveDateTime,
) {
    for i in (0..count).rev() {
        insert_block(
            conn,
            pane_uuid,
            &format!("{prefix}{i:03}"),
            Some(first_ts + Duration::seconds(i as i64)),
        );
    }
}

fn insert_blocks_same_start_ts(
    conn: &mut SqliteConnection,
    pane_uuid: &[u8],
    prefix: &str,
    count: usize,
    start_ts: NaiveDateTime,
) {
    for i in 0..count {
        insert_block(conn, pane_uuid, &format!("{prefix}{i:03}"), Some(start_ts));
    }
}

fn insert_unreadable_block(
    conn: &mut SqliteConnection,
    pane_uuid: &[u8],
    block_id: &str,
    start_ts: NaiveDateTime,
) {
    diesel::sql_query(
        "INSERT INTO blocks (
            pane_leaf_uuid, stylized_command, stylized_output, exit_code, did_execute,
            honor_ps1, is_background, block_id, start_ts
         ) VALUES (?, x'', x'', 'not-an-int', 1, 0, 0, ?, ?)",
    )
    .bind::<diesel::sql_types::Binary, _>(pane_uuid)
    .bind::<diesel::sql_types::Text, _>(block_id)
    .bind::<diesel::sql_types::Timestamp, _>(start_ts)
    .execute(conn)
    .expect("insert unreadable block should succeed");
}

fn ts(stamp: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").expect("timestamp should parse")
}

fn command_ids(items: &[SerializedBlockListItem]) -> Vec<&str> {
    items
        .iter()
        .map(|item| match item {
            SerializedBlockListItem::Command { block } => block.id.as_str(),
        })
        .collect()
}

#[test]
fn get_all_restored_blocks_caps_each_pane_independently() {
    let mut conn = restore_test_connection();
    let pane_a = b"pane-a".as_slice();
    let pane_b = b"pane-b".as_slice();
    insert_pane(&mut conn, 1, pane_a);
    insert_pane(&mut conn, 2, pane_b);
    let first_ts = ts("2024-01-01 00:00:00");
    insert_blocks_oldest_first(&mut conn, pane_a, "a", RESTORE_OVER_CAP_COUNT, first_ts);
    insert_blocks_oldest_first(&mut conn, pane_b, "b", RESTORE_OVER_CAP_COUNT, first_ts);

    let restored = get_all_restored_blocks(&mut conn).expect("restore should succeed");
    let pane_a_ids = command_ids(&restored[&PaneUuid(pane_a.to_vec())]);
    let pane_b_ids = command_ids(&restored[&PaneUuid(pane_b.to_vec())]);

    assert_eq!(
        pane_a_ids.len(),
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize
    );
    assert_eq!(pane_a_ids.first().copied(), Some("a001"));
    assert_eq!(pane_a_ids.last().copied(), Some("a100"));
    assert_eq!(
        pane_b_ids.len(),
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize
    );
    assert_eq!(pane_b_ids.first().copied(), Some("b001"));
    assert_eq!(pane_b_ids.last().copied(), Some("b100"));
}

#[test]
fn get_all_restored_blocks_keeps_newest_by_start_ts_not_insertion_order() {
    let mut conn = restore_test_connection();
    let pane = b"pane-a".as_slice();
    insert_pane(&mut conn, 1, pane);
    insert_blocks_newest_first(
        &mut conn,
        pane,
        "b",
        RESTORE_OVER_CAP_COUNT,
        ts("2024-01-01 00:00:00"),
    );

    let restored = get_all_restored_blocks(&mut conn).expect("restore should succeed");
    let ids = command_ids(&restored[&PaneUuid(pane.to_vec())]);

    assert_eq!(
        ids.len(),
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize
    );
    assert_eq!(ids.first().copied(), Some("b001"));
    assert_eq!(ids.last().copied(), Some("b100"));
}

#[test]
fn get_all_restored_blocks_breaks_equal_start_ts_ties_by_higher_id() {
    let mut conn = restore_test_connection();
    let pane = b"pane-a".as_slice();
    insert_pane(&mut conn, 1, pane);
    insert_blocks_same_start_ts(
        &mut conn,
        pane,
        "b",
        RESTORE_OVER_CAP_COUNT,
        ts("2024-01-01 00:00:01"),
    );

    let restored = get_all_restored_blocks(&mut conn).expect("restore should succeed");
    let ids = command_ids(&restored[&PaneUuid(pane.to_vec())]);

    assert_eq!(
        ids.len(),
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize
    );
    assert_eq!(ids.first().copied(), Some("b001"));
    assert_eq!(ids.last().copied(), Some("b100"));
}

#[test]
fn get_all_restored_blocks_treats_null_start_ts_as_oldest() {
    let mut conn = restore_test_connection();
    let pane = b"pane-a".as_slice();
    insert_pane(&mut conn, 1, pane);
    insert_block(&mut conn, pane, "null-a", None);
    insert_block(&mut conn, pane, "null-b", None);
    insert_blocks_oldest_first(
        &mut conn,
        pane,
        "t",
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize,
        ts("2024-01-01 00:00:01"),
    );

    let restored = get_all_restored_blocks(&mut conn).expect("restore should succeed");
    let ids = command_ids(&restored[&PaneUuid(pane.to_vec())]);

    assert_eq!(
        ids.len(),
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize
    );
    assert_eq!(ids.first().copied(), Some("t000"));
    assert_eq!(ids.last().copied(), Some("t099"));
}

#[test]
fn get_all_restored_blocks_skips_unreadable_rows_outside_the_cap() {
    let mut conn = restore_test_connection();
    let pane = b"pane-a".as_slice();
    insert_pane(&mut conn, 1, pane);
    insert_unreadable_block(&mut conn, pane, "unreadable", ts("2024-01-01 00:00:00"));
    insert_blocks_oldest_first(
        &mut conn,
        pane,
        "b",
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize,
        ts("2024-01-01 00:00:01"),
    );

    let restored = get_all_restored_blocks(&mut conn).expect("restore should succeed");
    let ids = command_ids(&restored[&PaneUuid(pane.to_vec())]);

    assert_eq!(
        ids.len(),
        MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION as usize
    );
    assert_eq!(ids.first().copied(), Some("b000"));
    assert_eq!(ids.last().copied(), Some("b099"));
}

#[test]
fn get_all_restored_blocks_includes_empty_panes_and_ignores_orphans() {
    let mut conn = restore_test_connection();
    let pane_with_blocks = b"pane-a".as_slice();
    let empty_pane = b"pane-empty".as_slice();
    let orphan_pane = b"pane-orphan".as_slice();
    insert_pane(&mut conn, 1, pane_with_blocks);
    insert_pane(&mut conn, 2, empty_pane);
    insert_block(
        &mut conn,
        pane_with_blocks,
        "kept",
        Some(ts("2024-01-01 00:00:01")),
    );
    insert_block(
        &mut conn,
        orphan_pane,
        "orphan",
        Some(ts("2024-01-01 00:00:02")),
    );

    let restored = get_all_restored_blocks(&mut conn).expect("restore should succeed");

    assert_eq!(
        command_ids(&restored[&PaneUuid(pane_with_blocks.to_vec())]),
        vec!["kept"]
    );
    assert!(restored[&PaneUuid(empty_pane.to_vec())].is_empty());
    assert!(!restored.contains_key(&PaneUuid(orphan_pane.to_vec())));
}

#[test]
fn get_all_restored_blocks_keeps_all_blocks_when_under_cap() {
    let mut conn = restore_test_connection();
    let pane = b"pane-a".as_slice();
    insert_pane(&mut conn, 1, pane);
    insert_block(&mut conn, pane, "first", Some(ts("2024-01-01 00:00:01")));
    insert_block(&mut conn, pane, "second", Some(ts("2024-01-01 00:00:02")));

    let restored = get_all_restored_blocks(&mut conn).expect("restore should succeed");

    assert_eq!(
        command_ids(&restored[&PaneUuid(pane.to_vec())]),
        vec!["first", "second"]
    );
}
