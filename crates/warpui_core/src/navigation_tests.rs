use std::thread;

use super::*;

#[derive(Clone, Debug, PartialEq)]
struct TestEntry {
    id: usize,
}

impl NavigationEntry for TestEntry {}

fn make_entry(id: usize) -> TestEntry {
    TestEntry { id }
}

fn new_stack() -> NavigationStack<TestEntry> {
    NavigationStack::default()
}

#[test]
fn test_push_and_go_back() {
    let mut stack = new_stack();

    stack.push(make_entry(0));

    assert!(stack.can_go_back());
    assert!(!stack.can_go_forward());

    let entry = stack.go_back(make_entry(1)).expect("should go back");
    assert_eq!(entry.id, 0);
    assert!(stack.can_go_forward());
}

#[test]
fn test_go_forward() {
    let mut stack = new_stack();

    stack.push(make_entry(0));

    let _back = stack.go_back(make_entry(1));

    let entry = stack.go_forward(make_entry(0)).expect("should go forward");
    assert_eq!(entry.id, 1);
    assert!(!stack.can_go_forward());
}

#[test]
fn test_push_truncates_forward_history() {
    let mut stack = new_stack();

    stack.push(make_entry(0));

    let _back = stack.go_back(make_entry(1));
    assert!(stack.can_go_forward());

    stack.push(make_entry(2));

    assert!(!stack.can_go_forward());
}

#[test]
fn test_navigating_guard_prevents_push() {
    let mut stack = new_stack();

    stack.set_navigating(true);
    stack.push(make_entry(0));
    assert!(!stack.can_go_back());
    stack.set_navigating(false);
}

#[test]
fn test_empty_stack() {
    let stack = new_stack();

    assert!(!stack.can_go_back());
    assert!(!stack.can_go_forward());
}

#[test]
fn test_max_stack_size() {
    let mut stack = new_stack();

    for i in 0..150 {
        stack.push(make_entry(i));
    }

    assert_eq!(stack.back_len(), MAX_STACK_SIZE);
}

#[test]
fn test_multiple_back_forward() {
    let mut stack = new_stack();

    stack.push(make_entry(0));
    stack.push(make_entry(1));
    stack.push(make_entry(2));

    let entry = stack.go_back(make_entry(3)).expect("back from 3");
    assert_eq!(entry.id, 2);

    let entry = stack.go_back(make_entry(2)).expect("back from 2");
    assert_eq!(entry.id, 1);

    let entry = stack.go_back(make_entry(1)).expect("back from 1");
    assert_eq!(entry.id, 0);

    assert!(!stack.can_go_back());
    assert!(stack.can_go_forward());

    let entry = stack.go_forward(make_entry(0)).expect("forward from 0");
    assert_eq!(entry.id, 1);

    let entry = stack.go_forward(make_entry(1)).expect("forward from 1");
    assert_eq!(entry.id, 2);
}

#[test]
fn test_entry_count() {
    let mut stack = new_stack();

    assert_eq!(stack.entry_count(), 0);

    stack.push(make_entry(0));
    stack.push(make_entry(1));
    assert_eq!(stack.entry_count(), 2);

    let _back = stack.go_back(make_entry(2));
    assert_eq!(stack.entry_count(), 2);
}

#[test]
fn test_push_debounced_captures_first_entry() {
    let mut stack = new_stack();

    stack.store_pending(make_entry(0));
    stack.store_pending(make_entry(1));
    stack.store_pending(make_entry(2));

    assert!(!stack.can_go_back());

    stack.flush();

    assert!(stack.can_go_back());
    let entry = stack.go_back(make_entry(99)).expect("should go back");
    assert_eq!(entry.id, 0);
}

#[test]
fn test_flush_on_empty_pending_is_noop() {
    let mut stack = new_stack();

    stack.push(make_entry(0));
    stack.flush();

    assert_eq!(stack.back_len(), 1);
}

#[test]
fn test_flush_if_expired_before_duration() {
    let mut stack = new_stack();
    stack.set_debounce_duration(Duration::from_millis(200));

    stack.store_pending(make_entry(0));
    stack.flush_if_expired();

    assert!(!stack.can_go_back());
}

#[test]
fn test_flush_if_expired_after_duration() {
    let mut stack = new_stack();
    stack.set_debounce_duration(Duration::from_millis(10));

    stack.store_pending(make_entry(0));
    thread::sleep(Duration::from_millis(15));
    stack.flush_if_expired();

    assert!(stack.can_go_back());
}

#[test]
fn test_expected_focus_loss_consumed_once_and_cleared_by_push() {
    let mut stack = new_stack();
    let window = crate::WindowId::new();
    let other_window = crate::WindowId::new();

    assert!(!stack.take_expected_focus_loss(window));

    stack.expect_focus_loss(window);
    assert!(!stack.take_expected_focus_loss(other_window));
    assert!(stack.take_expected_focus_loss(window));
    assert!(!stack.take_expected_focus_loss(window));

    stack.expect_focus_loss(window);
    stack.push(make_entry(0));
    assert!(!stack.take_expected_focus_loss(window));

    stack.expect_focus_loss(window);
    stack.clear();
    assert!(!stack.take_expected_focus_loss(window));
}

#[test]
fn test_push_debounced_during_navigation_is_ignored() {
    let mut stack = new_stack();

    stack.set_navigating(true);
    stack.store_pending(make_entry(0));
    stack.set_navigating(false);
    stack.flush();

    assert!(!stack.can_go_back());
}

#[test]
fn test_flush_before_go_back() {
    let mut stack = new_stack();

    stack.push(make_entry(0));
    stack.store_pending(make_entry(1));
    stack.flush();

    assert_eq!(stack.back_len(), 2);

    let entry = stack.go_back(make_entry(99)).expect("should go back");
    assert_eq!(entry.id, 1);
}

#[test]
fn test_peek_and_discard_back() {
    let mut stack = new_stack();

    stack.push(make_entry(0));
    stack.push(make_entry(1));

    let entry = stack.peek_back().expect("should peek back");
    assert_eq!(entry.id, 1);

    let entry = stack.discard_back().expect("should discard back");
    assert_eq!(entry.id, 1);

    let entry = stack.peek_back().expect("should still have a back entry");
    assert_eq!(entry.id, 0);
}

#[test]
fn test_peek_and_discard_forward() {
    let mut stack = new_stack();

    stack.push(make_entry(0));
    let _ = stack.go_back(make_entry(1));

    let entry = stack.peek_forward().expect("should peek forward");
    assert_eq!(entry.id, 1);

    let entry = stack.discard_forward().expect("should discard forward");
    assert_eq!(entry.id, 1);
    assert!(!stack.can_go_forward());
}

#[test]
fn test_retain_prunes_back_forward_and_pending() {
    let mut stack = new_stack();

    stack.push(make_entry(0));
    stack.push(make_entry(1));
    let _ = stack.go_back(make_entry(2));
    stack.store_pending(make_entry(3));

    stack.retain(|entry| entry.id == 0);

    let entry = stack.peek_back().expect("should keep matching back entry");
    assert_eq!(entry.id, 0);
    assert!(!stack.can_go_forward());

    stack.flush();
    assert_eq!(stack.back_len(), 1);
}
