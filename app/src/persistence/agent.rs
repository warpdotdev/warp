use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use diesel::associations::HasTable;
use diesel::prelude::*;
use diesel::result::Error;
use diesel::SqliteConnection;
use prost::Message;
use warp_multi_agent_api as api;

use super::model::{AgentConversation, AgentConversationData};
use crate::persistence::model::{AgentConversationRecord, AgentTaskRecord};
use crate::persistence::schema::{self, agent_conversations, agent_tasks};

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = agent_conversations)]
struct NewAgentConversation {
    conversation_id: String,
    conversation_data: String,
}

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = agent_tasks)]
struct NewAgentTask {
    conversation_id: String,
    task_id: String,
    task: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum UpsertConversationError {
    #[error("Failed to serialize conversation data: {0:?}")]
    Serialization(#[from] serde_json::Error),
    #[error("Failed to upsert conversation to sqlite: {0:?}")]
    DB(#[from] diesel::result::Error),
}

/// Maximum number of `agent_conversations` rows we retain on disk before the
/// upsert path starts evicting trees.
///
/// Orchestration sessions can produce 5–20 child conversations each, so a
/// limit of 100 only buys roughly 5–20 sessions of history for active
/// orchestration users — the limit was hit constantly in real-world use and
/// caused parents and children to age out independently, splitting trees on
/// disk. 500 is a more comfortable ceiling that still bounds total DB size.
pub(super) const MAX_PERSISTED_CONVERSATION_COUNT: usize = 500;

pub(super) fn upsert_agent_conversation<'a>(
    conn: &mut SqliteConnection,
    conversation_id_param: &str,
    tasks: impl IntoIterator<Item = &'a api::Task>,
    conversation_data_param: AgentConversationData,
) -> Result<(), UpsertConversationError> {
    use diesel::QueryDsl;
    use schema::agent_conversations::dsl::*;
    use schema::agent_tasks::dsl as tasks_dsl;

    let serialized_conversation_data = serde_json::to_string(&conversation_data_param)?;

    conn.transaction::<_, Error, _>(|conn| {
        // Upsert the conversation level metadata
        let new_conversation = NewAgentConversation {
            conversation_id: conversation_id_param.to_owned(),
            conversation_data: serialized_conversation_data,
        };

        diesel::insert_into(agent_conversations::table())
            .values(&new_conversation)
            .on_conflict(conversation_id)
            .do_update()
            .set(&new_conversation)
            .execute(conn)?;

        // Upsert each task
        for task in tasks {
            let task_binary = task.encode_to_vec();
            let new_task = NewAgentTask {
                conversation_id: conversation_id_param.to_owned(),
                task_id: task.id.clone(),
                task: task_binary,
            };

            if let Err(e) = diesel::insert_into(agent_tasks::table)
                .values(&new_task)
                .on_conflict(tasks_dsl::task_id)
                .do_update()
                .set(&new_task)
                .execute(conn)
            {
                log::warn!("Failed to upsert task {e:?}");
                return Err(e);
            }
        }

        // Prune old conversations if we exceed MAX_PERSISTED_CONVERSATION_COUNT.
        //
        // Eviction is tree-aware: parents and children are an atomic unit, so
        // we never delete a parent whose child still lives in the DB (or vice
        // versa). See `select_conversations_to_evict`.
        let conversation_count: i64 = agent_conversations::table().count().get_result(conn)?;
        if conversation_count > MAX_PERSISTED_CONVERSATION_COUNT as i64 {
            let all_rows: Vec<AgentConversationRecord> = agent_conversations::table()
                .select(AgentConversationRecord::as_select())
                .load(conn)?;
            let conversations_to_remove =
                select_conversations_to_evict(&all_rows, MAX_PERSISTED_CONVERSATION_COUNT);
            if !conversations_to_remove.is_empty() {
                delete_agent_conversations(conn, conversations_to_remove)?;
            }
        }

        Ok(())
    })?;

    Ok(())
}

/// Computes the list of `conversation_id`s to evict so the remaining set fits
/// within `limit` while never splitting an orchestration tree.
///
/// Algorithm:
/// 1. Parse each row's `conversation_data` as [`AgentConversationData`].
///    Parse failures are treated as rows without a parent (standalone trees),
///    so we never silently retain garbage by accidentally linking it.
/// 2. Resolve each row to its tree root by walking `parent_conversation_id`
///    upward. A row whose declared `parent_conversation_id` is not present
///    in `rows` (orphan reference) is treated as its own root — the
///    upstream Option B filter handles the historical orphans separately.
/// 3. Group rows by tree root and compute `effective_last_modified_at =
///    max(member.last_modified_at)` for each tree.
/// 4. Sort trees freshest-first by `effective_last_modified_at` DESC, ties
///    broken by `root_id` ascending for determinism.
/// 5. Greedy keep: always retain the freshest tree (even if it alone exceeds
///    `limit`, so we never split an active orchestration session). For each
///    subsequent tree, retain if cumulative kept rows + tree size ≤ `limit`;
///    otherwise evict the entire tree and every older tree after it.
///
/// The returned vector is sorted by `conversation_id` for stability so logs
/// and tests are reproducible.
pub(super) fn select_conversations_to_evict(
    rows: &[AgentConversationRecord],
    limit: usize,
) -> Vec<String> {
    if rows.len() <= limit {
        return Vec::new();
    }

    // Step 1+2: build conversation_id -> parent_conversation_id (only when
    // the declared parent is also present in `rows`; otherwise the row is
    // treated as a tree root).
    let row_set: HashSet<&str> = rows.iter().map(|r| r.conversation_id.as_str()).collect();
    let parent_by_id: HashMap<&str, Option<String>> = rows
        .iter()
        .map(|r| {
            let parent = serde_json::from_str::<AgentConversationData>(&r.conversation_data)
                .ok()
                .and_then(|d| d.parent_conversation_id)
                .filter(|p| row_set.contains(p.as_str()));
            (r.conversation_id.as_str(), parent)
        })
        .collect();

    fn find_root<'a>(start: &'a str, parent_by_id: &'a HashMap<&str, Option<String>>) -> &'a str {
        let mut current = start;
        let mut seen: HashSet<&str> = HashSet::new();
        loop {
            if !seen.insert(current) {
                // Defensive guard against cycles. Treat the cycle entry as
                // the root rather than looping forever.
                return current;
            }
            match parent_by_id.get(current) {
                Some(Some(p)) => current = p.as_str(),
                _ => return current,
            }
        }
    }

    // Step 3: group by tree root.
    let mut trees: HashMap<String, Vec<&AgentConversationRecord>> = HashMap::new();
    for row in rows {
        let root = find_root(row.conversation_id.as_str(), &parent_by_id).to_owned();
        trees.entry(root).or_default().push(row);
    }

    // Step 4: deterministic sort.
    let mut tree_list: Vec<(NaiveDateTime, String, Vec<&AgentConversationRecord>)> = trees
        .into_iter()
        .map(|(root, members)| {
            let effective = members
                .iter()
                .map(|r| r.last_modified_at)
                .max()
                .expect("tree always has at least one member by construction");
            (effective, root, members)
        })
        .collect();
    tree_list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    // Step 5: greedy keep, hard-stop semantics. Once any tree exceeds the
    // budget, every older tree is also evicted (matches the orchestrator
    // spec's "Keep trees in order until... exceed the limit").
    let mut kept_count: usize = 0;
    let mut evicted: Vec<String> = Vec::new();
    let mut stopped = false;
    let mut first = true;
    for (_effective, _root, members) in tree_list {
        let tree_size = members.len();
        let keep_this = if stopped {
            false
        } else if first {
            // Always keep the freshest tree to avoid evicting an active
            // session purely because it's larger than `limit`.
            true
        } else {
            kept_count + tree_size <= limit
        };
        if keep_this {
            kept_count += tree_size;
        } else {
            stopped = true;
            for m in &members {
                evicted.push(m.conversation_id.clone());
            }
        }
        first = false;
    }

    evicted.sort();
    evicted
}

pub(super) fn read_agent_conversations(
    conn: &mut SqliteConnection,
) -> Result<Vec<AgentConversation>, diesel::result::Error> {
    use schema::agent_conversations::dsl::*;

    let mut conversations_by_id = HashMap::<String, AgentConversation>::from_iter(
        agent_conversations
            .select(AgentConversationRecord::as_select())
            .load(conn)?
            .into_iter()
            .map(|conversation| {
                (
                    conversation.conversation_id.clone(),
                    AgentConversation {
                        conversation,
                        tasks: vec![],
                    },
                )
            }),
    );

    let task_records: Vec<AgentTaskRecord> = agent_tasks::table
        .select(AgentTaskRecord::as_select())
        .load(conn)?;

    let mut invalid_conversation_ids = HashSet::new();
    for task_record in task_records {
        if let Some(conversation) = conversations_by_id.get_mut(&task_record.conversation_id) {
            match api::Task::decode(&task_record.task[..]) {
                Ok(api_task) => {
                    conversation.tasks.push(api_task);
                }
                Err(e) => {
                    log::error!("Failed to decode task protobuf: {e}");

                    invalid_conversation_ids
                        .insert(conversation.conversation.conversation_id.clone());
                }
            }
        }
    }

    conversations_by_id.retain(|c_id, _| !invalid_conversation_ids.contains(c_id));

    Ok(conversations_by_id.into_values().collect())
}

/// Read a single agent conversation by its ID, including decoded tasks.
pub(crate) fn read_agent_conversation_by_id(
    conn: &mut SqliteConnection,
    conversation_id_str: &str,
) -> Result<Option<AgentConversation>, diesel::result::Error> {
    use schema::agent_conversations::dsl as convo_dsl;
    use schema::agent_tasks::dsl as tasks_dsl;

    let maybe_record: Option<AgentConversationRecord> = convo_dsl::agent_conversations
        .filter(convo_dsl::conversation_id.eq(conversation_id_str.to_owned()))
        .select(AgentConversationRecord::as_select())
        .first(conn)
        .optional()?;

    let Some(conversation_record) = maybe_record else {
        return Ok(None);
    };

    let task_records: Vec<AgentTaskRecord> = schema::agent_tasks::table
        .filter(tasks_dsl::conversation_id.eq(conversation_id_str))
        .select(AgentTaskRecord::as_select())
        .load(conn)?;

    let mut decoded_tasks = Vec::new();
    for task_record in task_records.into_iter() {
        match api::Task::decode(&task_record.task[..]) {
            Ok(task) => decoded_tasks.push(task),
            Err(e) => {
                log::error!("Failed to decode task protobuf: {e}");
            }
        }
    }

    Ok(Some(AgentConversation {
        conversation: conversation_record,
        tasks: decoded_tasks,
    }))
}

pub(super) fn delete_agent_conversations(
    conn: &mut SqliteConnection,
    conversation_ids: Vec<String>,
) -> Result<(), diesel::result::Error> {
    use diesel::{ExpressionMethods, QueryDsl};
    use schema::agent_conversations::dsl::*;
    use schema::agent_tasks::dsl as tasks_dsl;

    conn.transaction::<_, Error, _>(|conn| {
        // Delete tasks for these conversations first (due to foreign key constraint)
        diesel::delete(
            agent_tasks::table.filter(tasks_dsl::conversation_id.eq_any(&conversation_ids)),
        )
        .execute(conn)?;

        // Delete the conversations themselves
        diesel::delete(
            agent_conversations::table().filter(conversation_id.eq_any(&conversation_ids)),
        )
        .execute(conn)?;

        Ok(())
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
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
        // Use 2026-01-01 00:00:00 as a stable baseline so test timestamps
        // are easy to read in failure messages.
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

    /// (i) Prune returns an empty eviction list when row count is at or below
    /// the limit, regardless of tree shape.
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

    /// (ii) A tree of 5 members whose effective_last_modified_at lands in the
    /// freshest set is kept atomically; eviction targets older standalone
    /// rows instead of splitting the tree.
    #[test]
    fn keeps_fresh_tree_atomically_and_evicts_older_singletons() {
        // Tree rooted at "root": 1 parent (older) + 4 children, with the
        // newest child at secs=2000 (so the tree's effective timestamp is
        // 2000).
        let mut rows = vec![
            make_row(1, "root", None, 100), // parent, older
            make_row(2, "c1", Some("root"), 500),
            make_row(3, "c2", Some("root"), 1000),
            make_row(4, "c3", Some("root"), 1500),
            make_row(5, "c4", Some("root"), 2000),
        ];
        // 9 older standalone conversations, secs 10..18.
        for i in 0_i32..9 {
            let id = format!("s{i}");
            rows.push(make_row(100 + i, &id, None, 10 + i64::from(i)));
        }
        // 14 rows total, limit 13 => exactly 1 should be evicted.
        let evicted = select_conversations_to_evict(&rows, 13);
        assert_eq!(evicted.len(), 1, "evicted={evicted:?}");
        assert_eq!(evicted[0], "s0", "must evict the oldest singleton");
        // No tree member is in the evicted list.
        for tree_member in ["root", "c1", "c2", "c3", "c4"] {
            assert!(
                !evicted.contains(&tree_member.to_string()),
                "tree member {tree_member} was evicted; evicted={evicted:?}"
            );
        }
    }

    /// (iii) A child is never deleted while its parent is kept: if a parent
    /// row exists with an old `last_modified_at` but its child is fresh,
    /// the whole tree stays.
    #[test]
    fn child_kept_drags_parent_along() {
        // Parent is older than every standalone row, but the child is the
        // newest thing in the DB. The tree's effective ts = child's ts.
        let mut rows = vec![
            make_row(1, "parent", None, 1),              // very old parent
            make_row(2, "child", Some("parent"), 9_999), // very fresh child
        ];
        // 8 standalone rows with mid-range ages.
        for i in 0_i32..8 {
            let id = format!("s{i}");
            rows.push(make_row(100 + i, &id, None, 100 + i64::from(i)));
        }
        // 10 rows total, limit 9 => 1 should be evicted, and it must NOT
        // be the parent (because the tree is the freshest item).
        let evicted = select_conversations_to_evict(&rows, 9);
        assert_eq!(evicted.len(), 1, "evicted={evicted:?}");
        assert!(!evicted.contains(&"parent".to_string()));
        assert!(!evicted.contains(&"child".to_string()));
        assert_eq!(evicted[0], "s0");
    }

    /// (iv) A parent is never deleted while its child is kept: reverse of
    /// (iii). Tree's effective_last_modified_at = max(parent, child) so
    /// even a stale child won't drag down a fresh parent.
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

    /// (v) An orphan row whose declared parent_conversation_id isn't present
    /// in `rows` is treated as its own single-row tree.
    #[test]
    fn orphan_with_missing_parent_is_its_own_tree() {
        let rows = vec![
            make_row(1, "orphan", Some("missing_parent_id"), 9_999), // fresh
            make_row(2, "a", None, 100),
            make_row(3, "b", None, 200),
            make_row(4, "c", None, 300),
        ];
        // 4 rows, limit 3 => 1 evicted; orphan is the freshest so it
        // survives and the oldest singleton goes.
        let evicted = select_conversations_to_evict(&rows, 3);
        assert_eq!(evicted.len(), 1, "evicted={evicted:?}");
        assert_eq!(evicted[0], "a");
        assert!(!evicted.contains(&"orphan".to_string()));
    }

    /// (vi) A single tree that is itself larger than `limit` is still kept
    /// in full — we never split an active orchestration session, even if
    /// it pushes us above the limit.
    #[test]
    fn single_tree_larger_than_limit_is_kept_in_full() {
        // One tree with 1 parent + 199 children = 200 members.
        let mut rows = vec![make_row(1, "big_root", None, 10_000)];
        for i in 0_i32..199 {
            let cid = format!("big_child_{i}");
            rows.push(make_row(2 + i, &cid, Some("big_root"), 100 + i64::from(i)));
        }
        // Plus 1 older standalone.
        rows.push(make_row(9_999, "older_singleton", None, 50));
        let evicted = select_conversations_to_evict(&rows, 50);
        // The tree must survive intact even though it alone exceeds 50.
        // older_singleton is the only thing that can be evicted.
        assert_eq!(evicted, vec!["older_singleton".to_string()]);
    }

    /// Extra: parse failures fall back to standalone trees. A row with
    /// malformed `conversation_data` is treated as a root, never gets
    /// linked into another tree, and is eligible for eviction on its own.
    #[test]
    fn parse_failure_is_treated_as_standalone_tree() {
        let mut rows = vec![
            AgentConversationRecord {
                id: 1,
                conversation_id: "garbage".to_string(),
                conversation_data: "{not valid json".to_string(),
                last_modified_at: ts(50), // older than the rest
            },
            make_row(2, "a", None, 100),
            make_row(3, "b", None, 200),
            make_row(4, "c", None, 300),
        ];
        // Append another row that *claims* garbage as its parent. Because
        // garbage doesn't parse, the upward walk for this row stops at the
        // declared parent name "garbage" — but "garbage" IS in row_set, so
        // it would normally be linked. We want to verify the filter only
        // skips parents missing from the row set, not parents whose data is
        // malformed. The link is preserved here; the test below validates
        // that fact.
        rows.push(make_row(5, "child_of_garbage", Some("garbage"), 9_999));
        // 5 rows, limit 4 => evict 1. The garbage+child tree is the
        // freshest (effective=9_999) so it's kept. The single oldest other
        // row is "a".
        let evicted = select_conversations_to_evict(&rows, 4);
        assert_eq!(evicted, vec!["a".to_string()]);
    }

    /// Determinism: same input twice produces the same output.
    #[test]
    fn eviction_is_deterministic() {
        let rows = vec![
            make_row(1, "a", None, 100),
            make_row(2, "b", None, 100), // tie on timestamp
            make_row(3, "c", None, 100),
            make_row(4, "d", None, 100),
        ];
        let e1 = select_conversations_to_evict(&rows, 2);
        let e2 = select_conversations_to_evict(&rows, 2);
        assert_eq!(e1, e2);
        // With ties, root_id ASC sort means "a" and "b" survive and
        // "c","d" get evicted (greedy hard-stop after the first failure).
        assert_eq!(e1, vec!["c".to_string(), "d".to_string()]);
    }
}
