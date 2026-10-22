use mockall::Sequence;
use serde_json::{Value, json};

use super::*;
use crate::server::server_api::ai::{AgentRunsPage, MockAIClient};

fn run(id: u128) -> Value {
    json!({
        "task_id": uuid::Uuid::from_u128(if id == 0 { 9999 } else { id }).to_string(),
        "parent_run_id": uuid::Uuid::from_u128(9999).to_string(),
        "title": "child",
        "state": "SUCCEEDED",
        "prompt": "work",
        "created_at": "2026-10-01T00:00:00Z",
        "updated_at": "2026-10-01T00:00:00Z",
        "execution_location": if id.is_multiple_of(2) { "LOCAL" } else { "REMOTE" },
        "is_sandbox_running": false
    })
}

fn subtree_run(id: u128, parent: u128, location: &str, harness: Option<&str>) -> Value {
    let mut row = run(id);
    row["parent_run_id"] = json!(uuid::Uuid::from_u128(parent));
    row["execution_location"] = json!(location);
    if let Some(harness) = harness {
        row["agent_config"] = json!({"harness": {"type": harness}});
    }
    row
}

#[test]
fn restored_local_child_subtree_discovery() {
    let mut client = MockAIClient::new();
    let mut sequence = Sequence::new();
    for (parent, cursor, rows, next) in [
        (
            9999,
            None,
            vec![
                subtree_run(2, 9999, "LOCAL", None),
                subtree_run(3, 9999, "REMOTE", None),
                subtree_run(4, 9999, "LOCAL", Some("claude")),
                subtree_run(5, 9999, "LOCAL", Some("future")),
            ],
            Some("root-2"),
        ),
        (
            9999,
            Some("root-2"),
            vec![
                subtree_run(2, 9999, "LOCAL", None),
                subtree_run(6, 9999, "LOCAL", Some("oz")),
            ],
            None,
        ),
        (
            2,
            None,
            vec![subtree_run(8, 2, "LOCAL", None)],
            Some("child-2"),
        ),
        (
            2,
            Some("child-2"),
            vec![subtree_run(8, 2, "LOCAL", None)],
            None,
        ),
        (6, None, vec![], None),
        (8, None, vec![], None),
    ] {
        let mut response = Some(page(rows, next.is_some(), next));
        client
            .expect_list_agent_runs_page()
            .withf(move |_, filter| {
                filter.ancestor_run_id.as_deref()
                    == Some(uuid::Uuid::from_u128(parent).to_string().as_str())
                    && filter.cursor.as_deref() == cursor
            })
            .times(1)
            .in_sequence(&mut sequence)
            .returning(move |_, _| Ok(response.take().unwrap()));
    }

    let batches = futures::executor::block_on(list_local_subtree(
        &client,
        uuid::Uuid::from_u128(9999).to_string().parse().unwrap(),
    ))
    .unwrap();

    assert_eq!(
        batches
            .iter()
            .map(|batch| batch.parent_task_id.to_string())
            .collect::<Vec<_>>(),
        [9999, 2, 6, 8].map(|id| uuid::Uuid::from_u128(id).to_string())
    );
    assert_eq!(batches[0].children.len(), 5);
    assert_eq!(
        batches[0]
            .children
            .iter()
            .map(|child| child.task_id.to_string())
            .collect::<Vec<_>>(),
        [2, 3, 4, 5, 6].map(|id| uuid::Uuid::from_u128(id).to_string())
    );
    assert_eq!(batches[1].children.len(), 1);
    assert_eq!(
        batches[1].children[0].parent_run_id,
        Some(uuid::Uuid::from_u128(2).to_string())
    );
    assert_eq!(
        batches[1].children[0].task_id.to_string(),
        uuid::Uuid::from_u128(8).to_string()
    );
}

#[test]
fn restored_local_child_subtree_incomplete() {
    for failure in ["later-page", "conflict", "root-cycle"] {
        let mut client = MockAIClient::new();
        let mut sequence = Sequence::new();
        client
            .expect_list_agent_runs_page()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| {
                Ok(page(
                    vec![
                        subtree_run(2, 9999, "LOCAL", None),
                        subtree_run(6, 9999, "LOCAL", None),
                    ],
                    false,
                    None,
                ))
            });
        client
            .expect_list_agent_runs_page()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(move |_, _| {
                Ok(page(
                    vec![subtree_run(
                        match failure {
                            "conflict" => 6,
                            "root-cycle" => 9999,
                            _ => 8,
                        },
                        2,
                        "LOCAL",
                        None,
                    )],
                    failure == "later-page",
                    Some("next"),
                ))
            });
        if failure == "later-page" {
            client
                .expect_list_agent_runs_page()
                .times(1)
                .in_sequence(&mut sequence)
                .returning(|_, _| Err(anyhow::anyhow!("child list unavailable")));
        }

        let result = futures::executor::block_on(list_local_subtree(
            &client,
            uuid::Uuid::from_u128(9999).to_string().parse().unwrap(),
        ));
        assert!(result.is_err(), "{failure} exposed a partial subtree");
    }
}

fn page(runs: Vec<Value>, has_next_page: bool, cursor: Option<&str>) -> AgentRunsPage {
    serde_json::from_value(json!({
        "runs": runs,
        "page_info": { "has_next_page": has_next_page, "next_cursor": cursor }
    }))
    .unwrap()
}

#[test]
fn restored_local_child_discovery_pages() {
    let mut client = MockAIClient::new();
    let mut sequence = Sequence::new();
    client
        .expect_list_agent_runs_page()
        .withf(|limit, filter| {
            *limit == 100
                && filter.ancestor_run_id.as_deref()
                    == Some(uuid::Uuid::from_u128(9999).to_string().as_str())
                && filter.execution_location.is_none()
                && filter.sort_by == Some(RunSortBy::CreatedAt)
                && filter.sort_order == Some(RunSortOrder::Asc)
                && filter.cursor.is_none()
        })
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _| Ok(page((0..100).map(run).collect(), true, Some("page-2"))));
    client
        .expect_list_agent_runs_page()
        .withf(|_, filter| filter.cursor.as_deref() == Some("page-2"))
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _| Ok(page(vec![run(99), run(100), run(101)], false, None)));
    let children = futures::executor::block_on(list_direct_children(
        &client,
        uuid::Uuid::from_u128(9999).to_string().parse().unwrap(),
    ))
    .unwrap();
    assert_eq!(children.len(), 101);
    assert_eq!(
        children.first().unwrap().task_id.to_string(),
        uuid::Uuid::from_u128(1).to_string()
    );
    assert_eq!(
        children.last().unwrap().task_id.to_string(),
        uuid::Uuid::from_u128(101).to_string()
    );
}

#[test]
fn restored_local_child_discovery_rejects_incomplete_traversal() {
    let cases = [
        (vec![run(1)], true, None),
        (vec![run(1)], true, Some(" ")),
        (vec![run(1)], true, Some("page-2")),
        (
            vec![{
                let mut row = run(1);
                row["execution_location"] = Value::Null;
                row
            }],
            false,
            None,
        ),
        (
            vec![{
                let mut row = run(1);
                row["parent_run_id"] = json!(uuid::Uuid::from_u128(42));
                row
            }],
            false,
            None,
        ),
        (
            vec![run(1), {
                let mut row = run(1);
                row["execution_location"] = json!("LOCAL");
                row
            }],
            false,
            None,
        ),
        (
            vec![run(1), {
                let mut row = run(1);
                row["agent_config_snapshot"] = json!({"harness": {"type": "claude"}});
                row
            }],
            false,
            None,
        ),
        (
            ["future-a", "future-b"]
                .into_iter()
                .map(|harness| {
                    let mut row = run(1);
                    row["agent_config"] = json!({"harness": {"type": harness}});
                    row
                })
                .collect(),
            false,
            None,
        ),
    ];
    for (runs, has_next, cursor) in cases {
        let mut client = MockAIClient::new();
        let mut first = Some(page(runs, has_next, cursor));
        client.expect_list_agent_runs_page().returning(move |_, _| {
            Ok(first
                .take()
                .unwrap_or_else(|| page(vec![run(2)], true, Some("page-2"))))
        });
        assert!(
            futures::executor::block_on(list_direct_children(
                &client,
                uuid::Uuid::from_u128(9999).to_string().parse().unwrap(),
            ))
            .is_err()
        );
    }
}

#[test]
fn restored_local_child_strict_page_decoder() {
    for response in [
        json!({"runs": [run(1), {"task_id": "invalid"}], "page_info": {"has_next_page": false}}),
        json!({"runs": [run(1)]}),
        json!({"runs": [], "page_info": {"next_cursor": "next"}}),
    ] {
        assert!(serde_json::from_value::<AgentRunsPage>(response).is_err());
    }
}
