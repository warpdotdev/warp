use std::collections::HashMap;

use super::{descendants_from_parent_map, proven_kill_pgid};

#[test]
fn prefers_a_foreground_pgid_that_is_in_the_shell_tree() {
    assert_eq!(proven_kill_pgid(Some(400), 100, 1, &[100, 400]), Some(400));
}

#[test]
fn rejects_the_drivers_own_process_group_even_if_it_is_in_the_tree() {
    assert_eq!(proven_kill_pgid(Some(7), 100, 7, &[7, 100]), Some(100));
}

#[test]
fn rejects_a_foreground_pgid_that_is_not_in_the_shell_tree() {
    assert_eq!(proven_kill_pgid(Some(999), 100, 1, &[100]), Some(100));
}

#[test]
fn skips_the_kill_when_no_target_can_be_proved() {
    assert_eq!(proven_kill_pgid(Some(999), 100, 1, &[]), None);
    assert_eq!(proven_kill_pgid(None, 100, 100, &[100]), None);
}

#[test]
fn falls_back_to_the_shell_group_when_it_is_still_in_the_tree() {
    assert_eq!(proven_kill_pgid(None, 100, 1, &[100]), Some(100));
}

#[test]
fn descendants_walk_children_once_and_skip_the_root() {
    let children = HashMap::from([(1, vec![2, 3]), (2, vec![4]), (3, vec![])]);
    let mut got: Vec<_> = descendants_from_parent_map(&children, 1)
        .into_iter()
        .collect();
    got.sort();
    assert_eq!(got, vec![2, 3, 4]);
}

#[test]
fn descendants_walk_skips_cycles_and_duplicate_edges() {
    let children = HashMap::from([(1, vec![2, 2]), (2, vec![1, 3])]);
    let mut got: Vec<_> = descendants_from_parent_map(&children, 1)
        .into_iter()
        .collect();
    got.sort();
    assert_eq!(got, vec![1, 2, 3]);
}
