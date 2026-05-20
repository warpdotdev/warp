use crate::render::model::RenderedSelection;

use super::*;

#[test]
fn test_version_match_initial() {
    let version = ContentVersion::new();
    let stack = UndoStack::new(5, version);

    assert!(stack.version_match(&version));
}

fn fake_reversible_actions() -> ReversibleEditorActions {
    ReversibleEditorActions {
        actions: vec![],
        selections: ReversibleSelectionState {
            next: RenderedSelectionSet::new(RenderedSelection::new(0.into(), 0.into())),
            reverse: RenderedSelectionSet::new(RenderedSelection::new(0.into(), 0.into())),
        },
        replacement_range: ReplacementRange {
            new_range: 0.into()..0.into(),
            old_range: 0.into()..0.into(),
        },
    }
}

#[test]
fn test_version_match_after_edit() {
    let version = ContentVersion::new();
    let mut stack = UndoStack::new(5, version);

    assert!(stack.version_match(&version));

    let new_version = ContentVersion::new();
    stack.push_undo_item_to_stack(fake_reversible_actions(), new_version);

    assert!(stack.version_match(&new_version));
    assert!(!stack.version_match(&version));
}

#[test]
fn test_version_match_after_undo() {
    let version = ContentVersion::new();
    let mut stack = UndoStack::new(5, version);

    assert!(stack.version_match(&version));

    let new_version = ContentVersion::new();
    stack.push_undo_item_to_stack(fake_reversible_actions(), new_version);
    stack.undo();

    assert!(!stack.version_match(&new_version));
    assert!(stack.version_match(&version));
}

#[test]
fn test_version_match_after_redo() {
    let version = ContentVersion::new();
    let mut stack = UndoStack::new(5, version);

    assert!(stack.version_match(&version));

    let new_version = ContentVersion::new();
    stack.push_undo_item_to_stack(fake_reversible_actions(), new_version);
    stack.undo();
    stack.redo();

    assert!(stack.version_match(&new_version));
    assert!(!stack.version_match(&version));
}

#[test]
fn test_version_match_after_stack_overflow() {
    let version = ContentVersion::new();
    let mut stack = UndoStack::new(3, version);

    assert!(stack.version_match(&version));

    let next_version = ContentVersion::new();
    stack.push_undo_item_to_stack(fake_reversible_actions(), next_version);
    stack.push_undo_item_to_stack(fake_reversible_actions(), ContentVersion::new());
    stack.push_undo_item_to_stack(fake_reversible_actions(), ContentVersion::new());
    stack.push_undo_item_to_stack(fake_reversible_actions(), ContentVersion::new());

    stack.undo();
    stack.undo();
    stack.undo();
    stack.undo();

    assert!(stack.version_match(&next_version));
}

#[test]
fn test_version_match_after_nonatomic_undo_redo() {
    // Make sure that after a series of nonatomic operations, the undo stack takes the newest version.
    let version1 = ContentVersion::new();
    let mut stack = UndoStack::new(5, version1);

    assert!(stack.version_match(&version1));

    let version2 = ContentVersion::new();
    stack.push_non_atomic_edit(fake_reversible_actions(), NonAtomicType::Insert, version2);
    let version3 = ContentVersion::new();
    stack.push_non_atomic_edit(fake_reversible_actions(), NonAtomicType::Insert, version3);

    assert!(stack.version_match(&version3));

    stack.undo();

    assert!(stack.version_match(&version1));

    stack.redo();

    assert!(stack.version_match(&version3));
}

#[test]
fn test_full_rollback() {
    // Test that filling the capacity of the undo stack, undoing everything, and then making a new edit works.
    // This was causing a panic.

    let version1 = ContentVersion::new();
    let mut stack = UndoStack::new(5, version1);

    for _ in 0..5 {
        stack.push_undo_item_to_stack(fake_reversible_actions(), ContentVersion::new());
    }

    assert_eq!(stack.capacity, stack.current_index);

    for _ in 0..5 {
        stack.undo();
    }

    assert_eq!(0, stack.current_index);

    stack.push_undo_item_to_stack(fake_reversible_actions(), ContentVersion::new());
}
