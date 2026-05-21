use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use bounded_vec_deque::BoundedVecDeque;
use instant::Instant;

use super::time::{Global, Lamport, LamportValue};
use crate::editor::view::{model::LocalSelections, PlainTextEditorViewAction};

/// The maximum time we will batch consecutive edits for the same [`Action`].
/// The "batch" here is not to be confused with the [`Buffer`]'s notion
/// of a "batch". A single undo / redo batched item might consist
/// of multiple buffer batched items.
const UNDO_REDO_BATCH_TIMER: Duration = Duration::from_millis(500);

/// The maximum size of the undo stack.
/// TODO: this could be substantially larger now that we don't store snapshots.
const DEFAULT_UNDO_STACK_CAPACITY: usize = 20;

/// An entry in the [`LocalUndoStack`].
#[derive(Clone)]
struct UndoStackEntry {
    /// The set of operations that would be undone.
    /// These numbers are lamport timestamps corresponding to the edit operations.
    /// We don't need a full [`Lamport`] timestamp because the replica ID would be
    /// redundant (this is only for local edits).
    ops: Vec<LamportValue>,

    /// A snapshot of the selection state after the ops.
    selections: LocalSelections,

    /// The action associated with this entry.
    /// If this is the current item on the stack and the last
    /// change in the current batch was a pure selection change, this will be
    /// [`Action::CursorChanged`].
    action: PlainTextEditorViewAction,
}

/// A stack of batched, local operations that can be undone / redone.
/// The undo stack batches consecutive records within a [`UNDO_REDO_BATCH_TIMER`]
/// period if:
/// - the records are associated to the same action
/// - the associated action is not atomic
/// - there aren't any undos (we're at the top of the stack)
#[derive(Clone)]
pub struct LocalUndoStack {
    /// The stack itself.
    stack: BoundedVecDeque<UndoStackEntry>,

    /// Where we are in the stack currently.
    /// As we undo / redo, the position into the stack
    /// changes but the stack itself stays in tact.
    current_index: usize,

    /// The absolute time when the stack was last modified.
    last_changed_at: Option<Instant>,
}

impl LocalUndoStack {
    pub fn new(init_selections: LocalSelections) -> LocalUndoStack {
        Self::new_with_capacity(init_selections, DEFAULT_UNDO_STACK_CAPACITY)
    }

    fn new_with_capacity(init_selections: LocalSelections, capacity: usize) -> LocalUndoStack {
        let init_entry = UndoStackEntry {
            ops: vec![],
            selections: init_selections,
            action: PlainTextEditorViewAction::ReplaceBuffer,
        };
        Self {
            stack: BoundedVecDeque::from_iter([init_entry], capacity),
            current_index: 0,
            last_changed_at: None,
        }
    }

    /// Records a selection change by updating the current item's selection snapshot.
    pub fn record_selection_change(&mut self, selections: LocalSelections) {
        self.stack[self.current_index].selections = selections;
        self.stack[self.current_index].action = PlainTextEditorViewAction::CursorChanged;
    }

    /// Records an edit and potentially batches it with other edits if possible.
    pub fn record_edit(
        &mut self,
        action: PlainTextEditorViewAction,
        operations: impl Iterator<Item = LamportValue>,
        selections: LocalSelections,
    ) {
        let should_insert_new_entry = self.current_index == 0
            || self.current_index < self.stack.len() - 1
            || action.is_atomic()
            || Some(action) != self.last_action()
            || self
                .last_changed_at
                .is_none_or(|t| t.elapsed() >= UNDO_REDO_BATCH_TIMER);

        // If the index is positioned somewhere other than the last item,
        // this means that the user is making an edit after some undo'ing.
        // Any of the items past the current index are not needed anymore.
        self.stack.truncate(self.current_index + 1);

        if should_insert_new_entry {
            // Since we're using a [`BoundedVecDeque`], pushing to the
            // back might remove from the front if the stack is at capacity.
            self.stack.push_back(UndoStackEntry {
                action,
                ops: operations.collect(),
                selections,
            });
            self.current_index = self.stack.len() - 1;
        } else {
            let back = self.stack.back_mut().unwrap();
            back.ops.extend(operations);
            back.selections = selections;
        }

        self.last_changed_at = Some(Instant::now());
    }

    /// Processes an undo and returns
    /// 1. the set of operations that need to be undone
    /// 2. the set of selections that need to be restored
    pub fn undo(&mut self) -> Option<(Vec<LamportValue>, LocalSelections)> {
        if self.current_index == 0 {
            return None;
        }

        self.last_changed_at = None;

        let ops = self.stack[self.current_index].ops.clone();
        self.current_index -= 1;
        let selections = self.stack[self.current_index].selections.clone();

        Some((ops, selections))
    }

    /// Processes a redo and returns
    /// 1. the set of operations that need to be redone
    /// 2. the set of selections that need to be restored
    pub fn redo(&mut self) -> Option<(Vec<LamportValue>, LocalSelections)> {
        if self.current_index == self.stack.len() - 1 {
            return None;
        }

        self.last_changed_at = None;

        self.current_index += 1;
        let UndoStackEntry {
            ops, selections, ..
        } = self.stack[self.current_index].clone();

        Some((ops, selections))
    }

    /// Resets the undo stack to a single item with the given selections.
    pub fn reset(&mut self, selections: LocalSelections) {
        let init_entry = UndoStackEntry {
            ops: vec![],
            selections,
            action: PlainTextEditorViewAction::ReplaceBuffer,
        };
        self.stack = BoundedVecDeque::from_iter([init_entry], DEFAULT_UNDO_STACK_CAPACITY);
        self.last_changed_at = None;
        self.current_index = 0;
    }

    /// Returns the action associated to the last item on the undo stack.
    pub fn last_action(&self) -> Option<PlainTextEditorViewAction> {
        if self.stack.len() == 1 {
            return None;
        };
        self.stack.back().map(|e| e.action)
    }
}

/// A history of local and remote undos.
#[derive(Clone)]
pub struct UndoHistory {
    /// A mapping from an edit ID to the lamport timestamps of undo operations for that edit.
    /// The values are bare lamport times because the replica ID of an undo operation will
    /// always be the replica ID for the edit itself (one can only undo their own edits).
    ///
    /// Suppose an undo operation of an edit `e` is defined as `undo(e)`.
    /// Then, a redo of an edit operation `e` can be thought of as `undo(undo(e))`.
    /// Using that line of logic, an edit is considered undone iff the number of undos are odd.
    ///
    /// We use a [`BTreeSet`] to quickly query the max undo ts for a given edit
    /// and to efficiently compute intersections with other sets (e.g. deletions).
    map: HashMap<Lamport, BTreeSet<LamportValue>>,
}

impl UndoHistory {
    pub fn new() -> Self {
        UndoHistory {
            map: HashMap::new(),
        }
    }

    /// Returns the largest known undo timestamp for `edit`.
    fn max_undo_ts(&self, edit_id: &Lamport) -> LamportValue {
        self.map
            .get(edit_id)
            .and_then(|v| v.last().copied())
            .unwrap_or_default()
    }

    /// Registers an undo for `edit`
    pub fn undo(&mut self, edit_id: Lamport, undo_ts: LamportValue) {
        // The undo timestamp should be larger than all undo timestamps
        // so far for this edit and must be after the edit itself.
        debug_assert!(undo_ts > self.max_undo_ts(&edit_id) && undo_ts > edit_id.value);
        self.map.entry(edit_id).or_default().insert(undo_ts);
    }

    /// Returns true iff `edit` is considered undone.
    /// An edit is considered undone iff it has an odd number of undos.
    /// In other words, there exists some undo that wasn't redone.
    pub fn is_edit_undone(&self, edit_id: &Lamport) -> bool {
        self.map.get(edit_id).map_or(0, |v| v.len()) % 2 == 1
    }

    /// Returns true iff `edit` was undone when the version vector was `version`.
    pub fn was_edit_undone(&self, edit_id: &Lamport, version: &Global) -> bool {
        let num_undos = self
            .map
            .get(edit_id)
            .iter()
            .flat_map(|v| v.iter())
            .take_while(|undo| {
                version.observed(&Lamport {
                    replica_id: edit_id.replica_id(),
                    value: **undo,
                })
            })
            .count();
        num_undos % 2 == 1
    }
}

impl PlainTextEditorViewAction {
    /// Whether the action is atomic. An atomic action is one where a single event
    /// is independently undoable. This differs from non-atomic actions (such as `INSERT_TEXT`)
    /// where multiple events are coalesced and undone together if they occur within [`UNDO_REDO_BATCH_TIMER`] of each other.
    pub fn is_atomic(&self) -> bool {
        matches!(
            self,
            PlainTextEditorViewAction::Yank
                | PlainTextEditorViewAction::AcceptCompletionSuggestion
                | PlainTextEditorViewAction::NewLine
                | PlainTextEditorViewAction::DeleteWordLeft
                | PlainTextEditorViewAction::DeleteWordRight
                | PlainTextEditorViewAction::AutoSuggestion
                | PlainTextEditorViewAction::ReplaceBuffer
                | PlainTextEditorViewAction::Paste
                | PlainTextEditorViewAction::ClearLines
                | PlainTextEditorViewAction::ClearAndCopyLines
                | PlainTextEditorViewAction::DeleteAll
                | PlainTextEditorViewAction::CutAll
                | PlainTextEditorViewAction::InsertSelectedText
                | PlainTextEditorViewAction::CutWordRight
                | PlainTextEditorViewAction::SystemInsert
                | PlainTextEditorViewAction::ExpandAlias
                | PlainTextEditorViewAction::CycleCompletionSuggestion
        )
    }
}

#[cfg(test)]
#[path = "undo_tests.rs"]
mod tests;
