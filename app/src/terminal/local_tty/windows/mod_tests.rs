use std::collections::HashMap;

use super::*;

#[test]
fn collect_descendant_process_ids_walks_full_process_tree_without_root() {
    let child_processes_by_parent = HashMap::from([
        (10, vec![11, 12]),
        (11, vec![13]),
        (12, vec![14, 15]),
        (15, vec![16]),
        (99, vec![100]),
    ]);

    let mut descendants = collect_descendant_process_ids(10, &child_processes_by_parent);
    descendants.sort_unstable();

    assert_eq!(descendants, vec![11, 12, 13, 14, 15, 16]);
}

#[test]
fn collect_descendant_process_ids_handles_cycles_defensively() {
    let child_processes_by_parent = HashMap::from([(10, vec![11]), (11, vec![12]), (12, vec![10])]);

    let mut descendants = collect_descendant_process_ids(10, &child_processes_by_parent);
    descendants.sort_unstable();

    assert_eq!(descendants, vec![11, 12]);
}
