use chrono::NaiveDate;

use super::*;

fn data_with_parent(parent: Option<&str>) -> String {
    match parent {
        Some(p) => {
            format!(r#"{{"server_conversation_token":null,"parent_conversation_id":"{p}"}}"#)
        }
        None => r#"{"server_conversation_token":null}"#.to_string(),
    }
}

fn ts(secs_from_epoch: i64) -> NaiveDateTime {
    // 2026-01-01 baseline keeps failure messages readable.
    NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        + chrono::Duration::seconds(secs_from_epoch)
}

fn make_row(
    id: i32,
    conversation_id: &str,
    parent: Option<&str>,
    secs: i64,
) -> AgentConversationRecord {
    AgentConversationRecord {
        id,
        conversation_id: conversation_id.to_string(),
        conversation_data: data_with_parent(parent),
        last_modified_at: ts(secs),
    }
}

/// Row count ≤ limit ⇒ no eviction, regardless of tree shape.
#[test]
fn prune_is_no_op_when_under_limit() {
    let rows = vec![
        make_row(1, "a", None, 100),
        make_row(2, "b", None, 200),
        make_row(3, "c", None, 300),
    ];
    assert!(select_conversations_to_evict(&rows, 3).is_empty());
    assert!(select_conversations_to_evict(&rows, 100).is_empty());
}

/// A tree's effective timestamp is the max of its members; older standalone
/// rows get evicted instead of splitting a fresh tree.
#[test]
fn keeps_fresh_tree_atomically_and_evicts_older_singletons() {
    let mut rows = vec![
        make_row(1, "root", None, 100), // parent, older
        make_row(2, "c1", Some("root"), 500),
        make_row(3, "c2", Some("root"), 1000),
        make_row(4, "c3", Some("root"), 1500),
        make_row(5, "c4", Some("root"), 2000),
    ];
    for i in 0_i32..9 {
        let id = format!("s{i}");
        rows.push(make_row(100 + i, &id, None, 10 + i64::from(i)));
    }
    let evicted = select_conversations_to_evict(&rows, 13);
    assert_eq!(evicted.len(), 1, "evicted={evicted:?}");
    assert_eq!(evicted[0], "s0", "must evict the oldest singleton");
    for tree_member in ["root", "c1", "c2", "c3", "c4"] {
        assert!(
            !evicted.contains(&tree_member.to_string()),
            "tree member {tree_member} was evicted; evicted={evicted:?}"
        );
    }
}

/// A stale parent is kept when its child is fresh: tree ts = max(members).
#[test]
fn child_kept_drags_parent_along() {
    let mut rows = vec![
        make_row(1, "parent", None, 1),              // very old parent
        make_row(2, "child", Some("parent"), 9_999), // very fresh child
    ];
    for i in 0_i32..8 {
        let id = format!("s{i}");
        rows.push(make_row(100 + i, &id, None, 100 + i64::from(i)));
    }
    let evicted = select_conversations_to_evict(&rows, 9);
    assert_eq!(evicted.len(), 1, "evicted={evicted:?}");
    assert!(!evicted.contains(&"parent".to_string()));
    assert!(!evicted.contains(&"child".to_string()));
    assert_eq!(evicted[0], "s0");
}

/// Reverse of the previous case: a stale child is kept when its parent is
/// fresh.
#[test]
fn parent_kept_drags_child_along() {
    let mut rows = vec![
        make_row(1, "parent", None, 9_999),      // very fresh parent
        make_row(2, "child", Some("parent"), 1), // very old child
    ];
    for i in 0_i32..8 {
        let id = format!("s{i}");
        rows.push(make_row(100 + i, &id, None, 100 + i64::from(i)));
    }
    let evicted = select_conversations_to_evict(&rows, 9);
    assert_eq!(evicted.len(), 1, "evicted={evicted:?}");
    assert!(!evicted.contains(&"parent".to_string()));
    assert!(
        !evicted.contains(&"child".to_string()),
        "stale child must not be evicted while its parent is kept; evicted={evicted:?}"
    );
    assert_eq!(evicted[0], "s0");
}

/// Orphans (declared parent missing from row set) are their own root.
#[test]
fn orphan_with_missing_parent_is_its_own_tree() {
    let rows = vec![
        make_row(1, "orphan", Some("missing_parent_id"), 9_999), // fresh
        make_row(2, "a", None, 100),
        make_row(3, "b", None, 200),
        make_row(4, "c", None, 300),
    ];
    let evicted = select_conversations_to_evict(&rows, 3);
    assert_eq!(evicted.len(), 1, "evicted={evicted:?}");
    assert_eq!(evicted[0], "a");
    assert!(!evicted.contains(&"orphan".to_string()));
}

/// The freshest tree is retained even when it alone exceeds the cap, so we
/// never split an active orchestration session.
#[test]
fn single_tree_larger_than_limit_is_kept_in_full() {
    let mut rows = vec![make_row(1, "big_root", None, 10_000)];
    for i in 0_i32..199 {
        let cid = format!("big_child_{i}");
        rows.push(make_row(2 + i, &cid, Some("big_root"), 100 + i64::from(i)));
    }
    rows.push(make_row(9_999, "older_singleton", None, 50));
    let evicted = select_conversations_to_evict(&rows, 50);
    assert_eq!(evicted, vec!["older_singleton".to_string()]);
}

/// A parse-failure row is still a valid parent reference: it just becomes
/// its own root rather than getting quarantined out of the parent index.
#[test]
fn parse_failure_row_is_treated_as_root_and_can_be_referenced_by_others() {
    let mut rows = vec![
        AgentConversationRecord {
            id: 1,
            conversation_id: "garbage".to_string(),
            conversation_data: "{not valid json".to_string(),
            last_modified_at: ts(50),
        },
        make_row(2, "a", None, 100),
        make_row(3, "b", None, 200),
        make_row(4, "c", None, 300),
    ];
    rows.push(make_row(5, "child_of_garbage", Some("garbage"), 9_999));
    let evicted = select_conversations_to_evict(&rows, 4);
    assert_eq!(evicted, vec!["a".to_string()]);
}

/// Same input twice produces the same output. Tie-broken by root_id ASC.
#[test]
fn eviction_is_deterministic() {
    let rows = vec![
        make_row(1, "a", None, 100),
        make_row(2, "b", None, 100),
        make_row(3, "c", None, 100),
        make_row(4, "d", None, 100),
    ];
    let e1 = select_conversations_to_evict(&rows, 2);
    let e2 = select_conversations_to_evict(&rows, 2);
    assert_eq!(e1, e2);
    assert_eq!(e1, vec!["c".to_string(), "d".to_string()]);
}
