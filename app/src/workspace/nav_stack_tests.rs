use warpui::navigation::MAX_STACK_SIZE;
use warpui::units::Lines;

use super::*;
use crate::terminal::block_list_viewport::ScrollLines;

fn make_entry(tab_index: usize) -> NavigationEntry {
    NavigationEntry {
        window_id: WindowId::from_usize(1),
        tab_index,
        pane_id: PaneId::dummy_pane_id(),
        scroll_snapshot: None,
    }
}

fn new_stack() -> NavigationStack {
    NavigationStack::default()
}

#[test]
fn test_push_and_go_back() {
    let mut stack = new_stack();

    stack.push(make_entry(0));

    assert!(stack.can_go_back());
    assert!(!stack.can_go_forward());

    let entry = stack.go_back(make_entry(1)).expect("should go back");
    assert_eq!(entry.tab_index, 0);
    assert!(stack.can_go_forward());
}

#[test]
fn test_go_forward() {
    let mut stack = new_stack();

    stack.push(make_entry(0));

    let _back = stack.go_back(make_entry(1));

    let entry = stack.go_forward(make_entry(0)).expect("should go forward");
    assert_eq!(entry.tab_index, 1);
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

    assert_eq!(stack.entry_count(), MAX_STACK_SIZE);
}

#[test]
fn test_push_deduplicates_consecutive_identical_entries() {
    let mut stack = new_stack();
    let pane = PaneId::dummy_pane_id();

    let entry_a = NavigationEntry {
        window_id: WindowId::from_usize(1),
        tab_index: 0,
        pane_id: pane,
        scroll_snapshot: None,
    };
    let entry_b = NavigationEntry {
        window_id: WindowId::from_usize(1),
        tab_index: 1,
        pane_id: pane,
        scroll_snapshot: None,
    };

    stack.push(entry_a.clone());
    stack.push(entry_a.clone());
    assert_eq!(stack.entry_count(), 1);

    stack.push(entry_b.clone());
    stack.push(entry_b.clone());
    assert_eq!(stack.entry_count(), 2);

    stack.push(entry_a.clone());
    assert_eq!(stack.entry_count(), 3);
}

#[test]
fn test_push_skips_near_duplicate_terminal_scroll_entries() {
    let mut stack = new_stack();
    let pane = PaneId::dummy_pane_id();

    let entry_at = |scroll_top: f64| NavigationEntry {
        window_id: WindowId::from_usize(1),
        tab_index: 0,
        pane_id: pane,
        scroll_snapshot: Some(ScrollSnapshot::Terminal(ScrollPosition::FixedAtPosition {
            scroll_lines: ScrollLines::ScrollTop(Lines::new(scroll_top)),
        })),
    };

    stack.push(entry_at(100.0));
    stack.push(entry_at(106.0));
    assert_eq!(stack.entry_count(), 1);

    stack.push(entry_at(150.0));
    assert_eq!(stack.entry_count(), 2);
}

#[test]
fn test_multiple_back_forward() {
    let mut stack = new_stack();

    stack.push(make_entry(0));
    stack.push(make_entry(1));
    stack.push(make_entry(2));

    let entry = stack.go_back(make_entry(3)).expect("back from 3");
    assert_eq!(entry.tab_index, 2);

    let entry = stack.go_back(make_entry(2)).expect("back from 2");
    assert_eq!(entry.tab_index, 1);

    let entry = stack.go_back(make_entry(1)).expect("back from 1");
    assert_eq!(entry.tab_index, 0);

    assert!(!stack.can_go_back());
    assert!(stack.can_go_forward());

    let entry = stack.go_forward(make_entry(0)).expect("forward from 0");
    assert_eq!(entry.tab_index, 1);

    let entry = stack.go_forward(make_entry(1)).expect("forward from 1");
    assert_eq!(entry.tab_index, 2);
}
