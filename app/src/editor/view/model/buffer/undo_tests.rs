use super::{LocalUndoStack, UndoHistory};
use crate::editor::{
    view::model::{
        buffer::{
            time::{Global, Lamport, LamportValue},
            undo::UNDO_REDO_BATCH_TIMER,
            ReplicaId,
        },
        Anchor, LocalSelection, LocalSelections,
    },
    PlainTextEditorViewAction,
};
use vec1::vec1;

fn local_selections(start: Anchor, end: Anchor) -> LocalSelections {
    LocalSelections {
        pending: None,
        selections: vec1![LocalSelection::new_for_test(start, end)],
        marked_text_state: Default::default(),
    }
}

fn lamport(value: impl Into<LamportValue>) -> Lamport {
    Lamport {
        replica_id: ReplicaId::new(1),
        value: value.into(),
    }
}

#[test]
fn test_undo_stack_initialization() {
    let mut undo_stack = LocalUndoStack::new(local_selections(Anchor::Start, Anchor::Start));
    assert!(undo_stack.undo().is_none());
    assert!(undo_stack.redo().is_none());
}

#[test]
fn test_undo_stack_undo_redo() {
    let mut undo_stack = LocalUndoStack::new(local_selections(Anchor::Start, Anchor::Start));

    // [`Action::ReplaceBuffer`] is an atomic action so these edits won't be coalesced.
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [0.into()].into_iter(),
        local_selections(Anchor::Start, Anchor::End),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [1.into()].into_iter(),
        local_selections(Anchor::End, Anchor::Start),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [2.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, [2.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::Start));

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, [1.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, [0.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::Start));

    assert!(undo_stack.undo().is_none());

    let (ops, selections) = undo_stack.redo().unwrap();
    assert_eq!(ops, [0.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    let (ops, selections) = undo_stack.redo().unwrap();
    assert_eq!(ops, [1.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::Start));

    let (ops, selections) = undo_stack.redo().unwrap();
    assert_eq!(ops, [2.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::End));

    assert!(undo_stack.redo().is_none());
}

#[test]
fn test_undo_stack_selections_changed() {
    let mut undo_stack = LocalUndoStack::new(local_selections(Anchor::Start, Anchor::Start));

    undo_stack.record_selection_change(local_selections(Anchor::Start, Anchor::End));
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [0.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );
    undo_stack.record_selection_change(local_selections(Anchor::End, Anchor::Start));

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![0.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    let (ops, selections) = undo_stack.redo().unwrap();
    assert_eq!(ops, vec![0.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::Start));
}

#[test]
fn test_undo_stack_coalescing() {
    let mut undo_stack = LocalUndoStack::new(local_selections(Anchor::Start, Anchor::Start));

    // [`Action::InsertChar`] is not an atomic action so these edits should be coalesced.
    undo_stack.record_edit(
        PlainTextEditorViewAction::InsertChar,
        [0.into()].into_iter(),
        local_selections(Anchor::Start, Anchor::End),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::InsertChar,
        [1.into()].into_iter(),
        local_selections(Anchor::End, Anchor::Start),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::InsertChar,
        [2.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![0.into(), 1.into(), 2.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::Start));

    assert!(undo_stack.undo().is_none());
}

#[test]
fn test_undo_stack_expired_timer() {
    let mut undo_stack = LocalUndoStack::new(local_selections(Anchor::Start, Anchor::Start));

    // [`Action::InsertChar`] is not an atomic action so these edits should normally be coalesced
    // but can't be in this case because the timer expires.
    undo_stack.record_edit(
        PlainTextEditorViewAction::InsertChar,
        [0.into()].into_iter(),
        local_selections(Anchor::Start, Anchor::End),
    );
    std::thread::sleep(UNDO_REDO_BATCH_TIMER + std::time::Duration::from_millis(10));
    undo_stack.record_edit(
        PlainTextEditorViewAction::InsertChar,
        [1.into()].into_iter(),
        local_selections(Anchor::End, Anchor::Start),
    );

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![1.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![0.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::Start));

    assert!(undo_stack.undo().is_none());
}

#[test]
fn test_undo_stack_undo_fork() {
    let mut undo_stack = LocalUndoStack::new(local_selections(Anchor::Start, Anchor::Start));

    // [`Action::ReplaceBuffer`] is an atomic action so these edits won't be coalesced.
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [0.into()].into_iter(),
        local_selections(Anchor::Start, Anchor::End),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [1.into()].into_iter(),
        local_selections(Anchor::End, Anchor::Start),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [2.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![2.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::Start));

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![1.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [4.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );

    assert!(undo_stack.redo().is_none());

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![4.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    let (ops, selections) = undo_stack.redo().unwrap();
    assert_eq!(ops, vec![4.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::End));

    assert!(undo_stack.redo().is_none());
}

#[test]
fn test_undo_stack_capacity() {
    let mut undo_stack =
        LocalUndoStack::new_with_capacity(local_selections(Anchor::Start, Anchor::Start), 3);

    // [`Action::ReplaceBuffer`] is an atomic action so these edits won't be coalesced.
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [0.into()].into_iter(),
        local_selections(Anchor::Start, Anchor::End),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [1.into()].into_iter(),
        local_selections(Anchor::End, Anchor::Start),
    );
    undo_stack.record_edit(
        PlainTextEditorViewAction::ReplaceBuffer,
        [2.into()].into_iter(),
        local_selections(Anchor::End, Anchor::End),
    );

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![2.into()]);
    assert_eq!(selections, local_selections(Anchor::End, Anchor::Start));

    let (ops, selections) = undo_stack.undo().unwrap();
    assert_eq!(ops, vec![1.into()]);
    assert_eq!(selections, local_selections(Anchor::Start, Anchor::End));

    // Since the capacity of the stack is 3, it can only keep track of the latest
    // 2 edits (because it also tracks the base entry).
    assert!(undo_stack.undo().is_none());
}

#[test]
fn test_max_undo_ts() {
    let mut undo_history = UndoHistory::new();

    undo_history.undo(lamport(1), 2.into());
    undo_history.undo(lamport(0), 3.into());
    undo_history.undo(lamport(1), 5.into());
    undo_history.undo(lamport(0), 7.into());

    assert_eq!(undo_history.max_undo_ts(&lamport(0)), 7.into());
    assert_eq!(undo_history.max_undo_ts(&lamport(1)), 5.into());
    assert_eq!(undo_history.max_undo_ts(&lamport(2)), 0.into());
}

#[test]
#[should_panic]
fn test_undo_history_grow_only() {
    let mut undo_history = UndoHistory::new();
    undo_history.undo(lamport(1), 3.into());
    undo_history.undo(lamport(1), 2.into());
}

#[test]
fn test_is_edit_undone() {
    let mut undo_history = UndoHistory::new();

    undo_history.undo(lamport(1), 2.into());
    undo_history.undo(lamport(0), 3.into());
    undo_history.undo(lamport(1), 5.into());

    assert!(undo_history.is_edit_undone(&lamport(0)));
    assert!(!undo_history.is_edit_undone(&lamport(1)));
    assert!(!undo_history.is_edit_undone(&lamport(2)));
}

#[test]
fn test_was_edit_undone() {
    let mut undo_history = UndoHistory::new();
    let mut versions = Global::new();

    undo_history.undo(lamport(1), 2.into());
    undo_history.undo(lamport(1), 3.into());
    undo_history.undo(lamport(1), 5.into());

    assert!(!undo_history.was_edit_undone(&lamport(1), &versions));

    versions.observe(&lamport(2));
    assert!(undo_history.was_edit_undone(&lamport(1), &versions));

    versions.observe(&lamport(3));
    assert!(!undo_history.was_edit_undone(&lamport(1), &versions));

    versions.observe(&lamport(4));
    assert!(!undo_history.was_edit_undone(&lamport(1), &versions));

    versions.observe(&lamport(5));
    assert!(undo_history.was_edit_undone(&lamport(1), &versions));
}
