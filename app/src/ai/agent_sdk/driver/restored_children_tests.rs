use std::cell::RefCell;

use futures::channel::oneshot;
use futures::executor::block_on;
use futures::{pin_mut, poll};
use serde_json::json;

use super::*;
use crate::ai::ambient_agents::ExecutionLocation;

fn run_id(id: u128) -> AmbientAgentTaskId {
    Uuid::from_u128(id).to_string().parse().unwrap()
}

fn child(id: u128, parent: u128) -> AmbientAgentTask {
    serde_json::from_value(json!({
        "task_id": Uuid::from_u128(id),
        "parent_run_id": Uuid::from_u128(parent),
        "title": "child",
        "state": "SUCCEEDED",
        "prompt": "work",
        "created_at": "2026-10-01T00:00:00Z",
        "updated_at": "2026-10-01T00:00:00Z",
        "execution_location": "LOCAL",
        "is_sandbox_running": false
    }))
    .unwrap()
}

#[test]
fn local_child_setup_bounds_admission_and_waits_for_every_sibling() {
    block_on(async {
        let root_id = AIConversationId::new();
        let first_id = AIConversationId::new();
        let second_id = AIConversationId::new();
        let third_id = AIConversationId::new();
        let fourth_id = AIConversationId::new();
        let fifth_id = AIConversationId::new();
        let batches = [ChildRunBatch {
            parent_task_id: run_id(99),
            children: vec![
                child(1, 99),
                child(2, 99),
                child(3, 99),
                child(4, 99),
                child(5, 99),
            ],
        }];
        let (first, first_ready) = oneshot::channel();
        let (second, second_ready) = oneshot::channel();
        let (third, third_ready) = oneshot::channel();
        let (fourth, fourth_ready) = oneshot::channel();
        let (fifth, fifth_ready) = oneshot::channel();
        let mut readiness = HashMap::from([
            (run_id(1), first_ready),
            (run_id(2), second_ready),
            (run_id(3), third_ready),
            (run_id(4), fourth_ready),
            (run_id(5), fifth_ready),
        ]);
        let started = RefCell::new(Vec::new());
        let setup = prepare_local_subtree_conversations(
            run_id(99),
            root_id,
            &batches,
            |parent, parent_conversation, task| {
                assert_eq!(parent, run_id(99));
                assert_eq!(parent_conversation, root_id);
                started.borrow_mut().push(task.task_id);
                let ready = readiness.remove(&task.task_id).unwrap();
                async move { ready.await.unwrap() }
            },
        );
        pin_mut!(setup);

        assert!(poll!(setup.as_mut()).is_pending());
        assert_eq!(
            *started.borrow(),
            [run_id(1), run_id(2), run_id(3), run_id(4)]
        );

        third.send(Ok(third_id)).unwrap();
        assert!(poll!(setup.as_mut()).is_pending());
        assert_eq!(
            *started.borrow(),
            [run_id(1), run_id(2), run_id(3), run_id(4), run_id(5)]
        );

        first.send(Ok(first_id)).unwrap();
        second.send(Ok(second_id)).unwrap();
        fourth.send(Ok(fourth_id)).unwrap();
        assert!(poll!(setup.as_mut()).is_pending());
        fifth.send(Ok(fifth_id)).unwrap();

        let conversations = setup.await.unwrap();
        assert_eq!(
            conversations,
            HashMap::from([
                (run_id(99), root_id),
                (run_id(1), first_id),
                (run_id(2), second_id),
                (run_id(3), third_id),
                (run_id(4), fourth_id),
                (run_id(5), fifth_id),
            ])
        );
    });
}

#[test]
fn local_child_setup_finishes_parent_batch_before_creating_descendants() {
    block_on(async {
        let root_id = AIConversationId::new();
        let parent_id = AIConversationId::new();
        let sibling_id = AIConversationId::new();
        let grandchild_id = AIConversationId::new();
        let batches = [
            ChildRunBatch {
                parent_task_id: run_id(99),
                children: vec![child(1, 99), child(2, 99)],
            },
            ChildRunBatch {
                parent_task_id: run_id(1),
                children: vec![child(3, 1)],
            },
            ChildRunBatch {
                parent_task_id: run_id(2),
                children: vec![],
            },
        ];
        let (parent, parent_ready) = oneshot::channel();
        let (sibling, sibling_ready) = oneshot::channel();
        let (grandchild, grandchild_ready) = oneshot::channel();
        let mut readiness = HashMap::from([
            (run_id(1), parent_ready),
            (run_id(2), sibling_ready),
            (run_id(3), grandchild_ready),
        ]);
        let started = RefCell::new(Vec::new());
        let setup = prepare_local_subtree_conversations(
            run_id(99),
            root_id,
            &batches,
            |parent, parent_conversation, task| {
                started
                    .borrow_mut()
                    .push((parent, parent_conversation, task.task_id));
                let ready = readiness.remove(&task.task_id).unwrap();
                async move { ready.await.unwrap() }
            },
        );
        pin_mut!(setup);

        assert!(poll!(setup.as_mut()).is_pending());
        parent.send(Ok(parent_id)).unwrap();
        assert!(poll!(setup.as_mut()).is_pending());
        assert_eq!(
            *started.borrow(),
            [
                (run_id(99), root_id, run_id(1)),
                (run_id(99), root_id, run_id(2))
            ]
        );

        sibling.send(Ok(sibling_id)).unwrap();
        assert!(poll!(setup.as_mut()).is_pending());
        assert_eq!(
            *started.borrow(),
            [
                (run_id(99), root_id, run_id(1)),
                (run_id(99), root_id, run_id(2)),
                (run_id(1), parent_id, run_id(3)),
            ]
        );
        grandchild.send(Ok(grandchild_id)).unwrap();
        assert_eq!(
            setup.await.unwrap(),
            HashMap::from([
                (run_id(99), root_id),
                (run_id(1), parent_id),
                (run_id(2), sibling_id),
                (run_id(3), grandchild_id),
            ])
        );
    });
}

#[test]
fn local_child_setup_ignores_passive_children_and_empty_batches() {
    let root_id = AIConversationId::new();
    let mut remote = child(1, 99);
    remote.execution_location = Some(ExecutionLocation::Remote);
    let batches = [ChildRunBatch {
        parent_task_id: run_id(99),
        children: vec![remote],
    }];
    let result = block_on(prepare_local_subtree_conversations(
        run_id(99),
        root_id,
        &batches,
        |_, _, _| async {
            panic!("passive children must not get a local owner");
        },
    ))
    .unwrap();
    assert_eq!(result, HashMap::from([(run_id(99), root_id)]));

    let result = block_on(prepare_local_subtree_conversations(
        run_id(99),
        root_id,
        &[],
        |_, _, _| async {
            panic!("an empty subtree must not prepare children");
        },
    ))
    .unwrap();
    assert_eq!(result, HashMap::from([(run_id(99), root_id)]));
}
