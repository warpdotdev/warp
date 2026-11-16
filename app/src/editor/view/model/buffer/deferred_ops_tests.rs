use itertools::Itertools;
use string_offset::CharOffset;

use super::super::time::{Global, Lamport};
use super::{DeferredOperations, Operation};
use crate::editor::view::model::buffer::EditOperation;
use crate::editor::view::model::buffer::time::ReplicaId;

fn edit_operation(lamport: Lamport) -> Operation {
    Operation::Edit(EditOperation {
        lamport_timestamp: lamport.clone(),
        versions: Global::new(),
        start_id: lamport.clone(),
        start_character_offset: CharOffset::from(0),
        end_id: lamport,
        end_character_offset: CharOffset::from(0),
        new_text: String::from(""),
    })
}

#[test]
fn test_ordering() {
    let mut ops = DeferredOperations::new();
    let edits = vec![
        edit_operation(Lamport {
            replica_id: ReplicaId::new(1),
            value: 10.into(),
        }),
        edit_operation(Lamport {
            replica_id: ReplicaId::new(5),
            value: 2.into(),
        }),
        edit_operation(Lamport {
            replica_id: ReplicaId::new(3),
            value: 20.into(),
        }),
        edit_operation(Lamport {
            replica_id: ReplicaId::new(9),
            value: 30.into(),
        }),
        edit_operation(Lamport {
            replica_id: ReplicaId::new(2),
            value: 1.into(),
        }),
        edit_operation(Lamport {
            replica_id: ReplicaId::new(1),
            value: 2.into(),
        }),
    ];
    ops.extend(edits.clone());

    let drained = ops.drain();
    let expected = edits
        .into_iter()
        .sorted_by_key(|k| k.lamport_timestamp().clone())
        .collect_vec();
    assert_eq!(drained, expected);
}

#[test]
fn test_replica_deferred() {
    let mut ops = DeferredOperations::new();
    let replica_id = ReplicaId::new(1);
    assert!(!ops.replica_deferred(&replica_id));

    ops.extend(vec![edit_operation(Lamport {
        replica_id: replica_id.clone(),
        value: 0.into(),
    })]);

    assert!(ops.replica_deferred(&replica_id));
    assert!(!ops.replica_deferred(&ReplicaId::new(2)));
}
