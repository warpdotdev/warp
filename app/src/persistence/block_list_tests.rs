//! Unit tests for the `ai_queries` persistence layer in [`super`].
//!
//! Covers the FIFO eviction cap added to [`super::upsert_ai_query`] and the empty-input filter
//! that drives the persistence skip in `handle_ai_history_event`.

use std::sync::Arc;

use chrono::Local;
use diesel::sqlite::SqliteConnection;
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl};
use diesel_migrations::MigrationHarness;

use super::{read_nld_prompts_with_limit, upsert_ai_query_with_limit};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentExchangeId, AIAgentInput, UserQueryMode};
use crate::ai::blocklist::{AIQueryHistoryOutputStatus, PersistedAIInput, PersistedAIInputType};
use crate::ai::llms::LLMId;

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
fn read_nld_prompts_filters_empty_and_whitespace_inputs_newest_first() {
    let mut conn = test_connection();

    for query in [
        make_query("older prompt"),
        make_query("   "),
        make_empty_input_query(),
        make_query("newer prompt"),
    ] {
        upsert_ai_query_with_limit(&mut conn, query, 10).expect("upsert should succeed");
    }

    let prompts = read_nld_prompts_with_limit(&mut conn, 10).expect("read should succeed");
    let texts: Vec<&str> = prompts.iter().map(|(text, _)| text.as_str()).collect();
    // `[]` and whitespace-only rows are dropped; the rest come back newest-first.
    assert_eq!(texts, vec!["newer prompt", "older prompt"]);
}

#[test]
fn read_nld_prompts_filters_empty_inputs_before_applying_limit() {
    let mut conn = test_connection();

    // The newest row has an empty input. If the limit were applied before the filter, the
    // two-row window would be (empty, q1) and only "q1" would survive; filtering in SQL
    // before the limit returns the two newest query-bearing rows instead.
    for query in [make_query("q0"), make_query("q1"), make_empty_input_query()] {
        upsert_ai_query_with_limit(&mut conn, query, 10).expect("upsert should succeed");
    }

    let prompts = read_nld_prompts_with_limit(&mut conn, 2).expect("read should succeed");
    let texts: Vec<&str> = prompts.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(texts, vec!["q1", "q0"]);
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
