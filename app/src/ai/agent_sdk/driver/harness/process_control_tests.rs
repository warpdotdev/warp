use super::proven_kill_pgid;

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
fn rejects_pgids_below_two() {
    assert_eq!(proven_kill_pgid(Some(0), 100, 1, &[0, 100]), Some(100));
    assert_eq!(proven_kill_pgid(Some(1), 100, 1, &[1, 100]), Some(100));
}

#[test]
fn skips_the_kill_when_no_target_can_be_proved() {
    assert_eq!(proven_kill_pgid(Some(999), 100, 1, &[]), None);
    assert_eq!(proven_kill_pgid(None, 100, 100, &[100]), None);
    assert_eq!(proven_kill_pgid(None, 1, 7, &[1]), None);
}

#[test]
fn falls_back_to_the_shell_group_when_it_is_still_in_the_tree() {
    assert_eq!(proven_kill_pgid(None, 100, 1, &[100]), Some(100));
}
