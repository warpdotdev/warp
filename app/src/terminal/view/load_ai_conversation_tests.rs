use chrono::{Local, TimeZone};
use warp_terminal::model::BlockIndex;

use super::find_block_indices_for_exchange_timestamps;

/// Helper: create a `DateTime<Local>` from a unix timestamp in seconds.
fn ts(secs: i64) -> chrono::DateTime<Local> {
    Local.timestamp_opt(secs, 0).unwrap()
}

fn bi(idx: usize) -> BlockIndex {
    BlockIndex::from(idx)
}

// ── All blocks in increasing timestamp order ──────────────────────────

#[test]
fn sorted_blocks_exchange_before_all_blocks() {
    // Exchange at t=1, blocks at t=10, t=20, t=30.
    // Should find the first block (t=10).
    let blocks = vec![(bi(0), ts(10)), (bi(1), ts(20)), (bi(2), ts(30))];
    let exchanges = vec![ts(1)];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(result, vec![Some(bi(0))]);
}

#[test]
fn sorted_blocks_exchange_between_blocks() {
    // Exchange at t=15, blocks at t=10, t=20, t=30.
    // Should find t=20 (first block >= exchange).
    let blocks = vec![(bi(0), ts(10)), (bi(1), ts(20)), (bi(2), ts(30))];
    let exchanges = vec![ts(15)];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(result, vec![Some(bi(1))]);
}

#[test]
fn sorted_blocks_exchange_equal_to_block() {
    // Exchange at t=20, blocks at t=10, t=20, t=30.
    // Should find t=20 (>= includes equality).
    let blocks = vec![(bi(0), ts(10)), (bi(1), ts(20)), (bi(2), ts(30))];
    let exchanges = vec![ts(20)];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(result, vec![Some(bi(1))]);
}

#[test]
fn sorted_blocks_exchange_after_all_blocks() {
    // Exchange at t=100, blocks at t=10, t=20, t=30.
    // No block >= 100, should be None.
    let blocks = vec![(bi(0), ts(10)), (bi(1), ts(20)), (bi(2), ts(30))];
    let exchanges = vec![ts(100)];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(result, vec![None]);
}

#[test]
fn sorted_blocks_multiple_exchanges() {
    // Blocks: t=10, t=20, t=30, t=40.
    // Exchanges: t=5 (→ bi(0)), t=15 (→ bi(1)), t=25 (→ bi(2)), t=35 (→ bi(3)), t=45 (→ None).
    let blocks = vec![
        (bi(0), ts(10)),
        (bi(1), ts(20)),
        (bi(2), ts(30)),
        (bi(3), ts(40)),
    ];
    let exchanges = vec![ts(5), ts(15), ts(25), ts(35), ts(45)];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(
        result,
        vec![Some(bi(0)), Some(bi(1)), Some(bi(2)), Some(bi(3)), None]
    );
}

// ── Sorted tail appended after prefix ─────────────────────────────────
// Simulate restore_conversation_after_view_creation: the terminal already
// has blocks (the prefix), then insert_restored_block appends the
// conversation's command blocks as a sorted tail.
// Prefix: [t=40 @0, t=50 @1]  (pre-existing terminal blocks)
// Tail:   [t=10 @2, t=30 @3]  (conversation blocks, sorted)
//
// The search should only match against the tail. The backwards iteration
// with break stops at the boundary so prefix blocks are never considered.

#[test]
fn sorted_tail_exchange_before_tail() {
    // Exchange at t=5: all tail blocks are >= 5; smallest is t=10 @2.
    let blocks = vec![
        (bi(0), ts(40)),
        (bi(1), ts(50)),
        (bi(2), ts(10)),
        (bi(3), ts(30)),
    ];
    let result = find_block_indices_for_exchange_timestamps(&blocks, &[ts(5)]);
    assert_eq!(result, vec![Some(bi(2))]);
}

#[test]
fn sorted_tail_exchange_between_tail_blocks() {
    // Exchange at t=15: t=30 @3 >= 15 (best), t=10 @2 < 15 → break.
    let blocks = vec![
        (bi(0), ts(40)),
        (bi(1), ts(50)),
        (bi(2), ts(10)),
        (bi(3), ts(30)),
    ];
    let result = find_block_indices_for_exchange_timestamps(&blocks, &[ts(15)]);
    assert_eq!(result, vec![Some(bi(3))]);
}

#[test]
fn sorted_tail_exchange_after_tail() {
    // Exchange at t=35: t=30 @3 < 35 → break immediately.
    // No tail block matches, so None (AI block appended at end).
    // Prefix blocks t=40, t=50 are intentionally not considered.
    let blocks = vec![
        (bi(0), ts(40)),
        (bi(1), ts(50)),
        (bi(2), ts(10)),
        (bi(3), ts(30)),
    ];
    let result = find_block_indices_for_exchange_timestamps(&blocks, &[ts(35)]);
    assert_eq!(result, vec![None]);
}

#[test]
fn sorted_tail_exchange_equals_tail_block() {
    // Exchange at t=10: t=30 @3 >= 10 (best), t=10 @2 >= 10 (better, 10 < 30) → best = @2.
    // t=50 @1… never reached because next step would be t=50 which is still >= 10,
    // but we already found t=10 which is the exact match.
    let blocks = vec![
        (bi(0), ts(40)),
        (bi(1), ts(50)),
        (bi(2), ts(10)),
        (bi(3), ts(30)),
    ];
    let result = find_block_indices_for_exchange_timestamps(&blocks, &[ts(10)]);
    assert_eq!(result, vec![Some(bi(2))]);
}

#[test]
fn sorted_tail_equal_timestamps_pick_first_inserted_block() {
    // When restored commands fall back to the same exchange/message timestamp,
    // insert the AI block before the first command from that exchange.
    let blocks = vec![
        (bi(0), ts(40)),
        (bi(1), ts(50)),
        (bi(2), ts(10)),
        (bi(3), ts(10)),
    ];
    let result = find_block_indices_for_exchange_timestamps(&blocks, &[ts(10)]);
    assert_eq!(result, vec![Some(bi(2))]);
}

#[test]
fn sorted_tail_multiple_exchanges() {
    // Prefix: [t=40 @0, t=50 @1], Tail: [t=10 @2, t=30 @3]
    let blocks = vec![
        (bi(0), ts(40)),
        (bi(1), ts(50)),
        (bi(2), ts(10)),
        (bi(3), ts(30)),
    ];
    let exchanges = vec![ts(5), ts(15), ts(35), ts(45), ts(55)];
    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(
        result,
        vec![
            Some(bi(2)), // t=10 (first tail block)
            Some(bi(3)), // t=30
            None,        // past tail, appended at end
            None,        // past tail
            None,        // past tail
        ]
    );
}

// ── Edge cases ────────────────────────────────────────────────────────

#[test]
fn empty_blocks_returns_none_for_all_exchanges() {
    let blocks: Vec<(BlockIndex, chrono::DateTime<Local>)> = vec![];
    let exchanges = vec![ts(10), ts(20)];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(result, vec![None, None]);
}

#[test]
fn empty_exchanges_returns_empty() {
    let blocks = vec![(bi(0), ts(10))];
    let exchanges: Vec<chrono::DateTime<Local>> = vec![];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert!(result.is_empty());
}

#[test]
fn single_block_at_same_time_as_exchange() {
    let blocks = vec![(bi(0), ts(42))];
    let exchanges = vec![ts(42)];

    let result = find_block_indices_for_exchange_timestamps(&blocks, &exchanges);

    assert_eq!(result, vec![Some(bi(0))]);
}
