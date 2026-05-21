use std::{collections::VecDeque, time::Duration};

use instant::Instant;
use warp_util::content_version::ContentVersion;

use crate::render::model::RenderedSelectionSet;

use super::core::{CoreEditorAction, ReplacementRange};

/// Threshold to separate two non-atomic undo items.
const UNDO_REDO_TIMER: Duration = Duration::from_millis(500);

/// Represents one high-level editor action that should be
/// undo/redo in one step. The high-level action could be broken
/// down into a set of smaller core editor actions.
///
/// Note that we also record the selection and replacement range state
/// before and after the editor action since they don't need to be
/// recalculated.
#[derive(Debug, Clone)]
pub struct ReversibleEditorActions {
    pub actions: Vec<ReversibleEditorAction>,
    pub selections: ReversibleSelectionState,
    pub replacement_range: ReplacementRange,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UndoActionType {
    // The action is atomic and should not be combined with any other action
    // for a batched undo/redo.
    Atomic,
    // The action is non-atomic and could be undo/redo in a batch.
    NonAtomic(NonAtomicType),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NonAtomicType {
    Insert,
    Backspace,
}

impl NonAtomicType {
    // Whether the change adds or removes content.
    fn is_addition(&self) -> bool {
        match self {
            NonAtomicType::Insert => true,
            NonAtomicType::Backspace => false,
        }
    }
}

/// A collection of editor actions that should be undo/redo together.
struct UndoStackItem {
    items: VecDeque<Vec<ReversibleEditorAction>>,
    selections: ReversibleSelectionState,
    replacement_range: ReplacementRange,
    // The version of the content after the original actions were applied.
    version: ContentVersion,
}

impl UndoStackItem {
    /// Create a new undo stack item.
    /// The version of the buffer after the actions were applied is also recorded.
    fn new(item: ReversibleEditorActions, version: ContentVersion) -> Self {
        let mut new_items = VecDeque::new();
        new_items.push_back(item.actions);

        Self {
            items: new_items,
            selections: item.selections,
            replacement_range: item.replacement_range,
            version,
        }
    }

    fn reverse(&mut self) {
        let mut new_items = VecDeque::new();

        // We are reversing three things here:
        // 1. The batched edit actions' order
        // 2. Within each edit action, its core edit actions' order
        // 3. Each core edit action
        for mut item in self.items.drain(..) {
            item.reverse();

            for action in &mut item {
                std::mem::swap(&mut action.next, &mut action.reverse);
            }

            new_items.push_front(item);
        }
        self.items = new_items;

        std::mem::swap(&mut self.selections.next, &mut self.selections.reverse);
        std::mem::swap(
            &mut self.replacement_range.new_range,
            &mut self.replacement_range.old_range,
        );
    }

    // Pushing another edit action to the batch.
    fn add(
        &mut self,
        item: ReversibleEditorActions,
        action: NonAtomicType,
        version: ContentVersion,
    ) {
        let is_addition = action.is_addition();
        self.items.push_front(item.actions);
        self.selections.reverse = item.selections.reverse;
        self.version = version;

        // This might be a bit confusing. For batched additions, the new range after the undo
        // will always be the first edit actions' new range. The old range needs to be
        // extended because we are inserting content into the buffer in each action.
        //
        // For batched deletions, the new ranges' start needs to be moved back with each edit because
        // we are removing content from the buffer. The old range will be the range of the last edit
        // actions' old range.
        if is_addition {
            self.replacement_range.old_range.end = self
                .replacement_range
                .old_range
                .end
                .max(item.replacement_range.old_range.end);
        } else {
            self.replacement_range.old_range = item.replacement_range.old_range;
            self.replacement_range.new_range.start = self
                .replacement_range
                .new_range
                .start
                .min(item.replacement_range.new_range.start);
        }
    }

    pub fn actions(&self) -> Vec<Vec<CoreEditorAction>> {
        let mut result = Vec::new();
        for item in &self.items {
            result.push(item.iter().map(|action| action.next.clone()).collect());
        }
        result
    }

    pub fn selection(&self) -> RenderedSelectionSet {
        self.selections.next.clone()
    }
}

#[derive(Debug, Clone)]
pub struct ReversibleEditorAction {
    pub next: CoreEditorAction,
    pub reverse: CoreEditorAction,
}

#[derive(Debug, Clone)]
pub struct ReversibleSelectionState {
    pub next: RenderedSelectionSet,
    pub reverse: RenderedSelectionSet,
}

/// Changes need to be applied to the buffer model with a triggered
/// undo/redo action.
#[derive(Debug)]
pub struct UndoResult {
    pub actions: Vec<Vec<CoreEditorAction>>,
    pub selection: RenderedSelectionSet,
    pub replacement_range: ReplacementRange,
}

#[derive(Debug, Clone)]
pub(super) struct UndoArg {
    pub actions: Vec<ReversibleEditorAction>,
    pub replacement_range: ReplacementRange,
}

pub(super) struct UndoStack {
    stack: VecDeque<UndoStackItem>,
    current_index: usize,
    capacity: usize,
    previous_action_type: Option<NonAtomicType>,
    last_edit: Option<Instant>,
    // The buffer version after everything is undone.
    initial_version: ContentVersion,
}

impl UndoStack {
    pub fn new(capacity: usize, initial_version: ContentVersion) -> UndoStack {
        Self {
            stack: Default::default(),
            current_index: 0,
            capacity,
            previous_action_type: None,
            last_edit: None,
            initial_version,
        }
    }

    pub fn clear_previous_non_atomic_type(&mut self) {
        self.previous_action_type = None;
        self.last_edit = None;
    }

    fn push_undo_item_to_stack(&mut self, item: ReversibleEditorActions, version: ContentVersion) {
        let mut stack_size = self.stack.len();

        if self.current_index < stack_size {
            // We need to clear the redo stack if the user has undone already and then make edits.
            self.stack.truncate(self.current_index);
            stack_size = self.current_index;
        }

        if stack_size == self.capacity {
            self.initial_version = self
                .stack
                .pop_front()
                .expect("Undo stack is empty after checking size")
                .version;
            self.current_index -= 1;
        }

        self.stack.push_back(UndoStackItem::new(item, version));
        self.current_index += 1;
    }

    #[cfg(test)]
    pub(super) fn push_non_atomic_edit(
        &mut self,
        item: ReversibleEditorActions,
        action: NonAtomicType,
        version: ContentVersion,
    ) {
        if self.previous_action_type == Some(action) && self.current_index > 0 {
            self.stack[self.current_index - 1].add(item, action, version);
            return;
        }
        self.previous_action_type = Some(action);
        self.push_undo_item_to_stack(item, version);
    }

    pub fn push_new_edit(
        &mut self,
        item: ReversibleEditorActions,
        action: UndoActionType,
        version: ContentVersion,
    ) {
        // An item should be pushed into the last batch if:
        // 1. Action is non-atomic
        // 2. Action matches the previous batch's non-atomic type
        // 3. The gap between two edits hasn't exceeded threshold
        if let UndoActionType::NonAtomic(action) = action {
            let elapsed_time_within_limit = match self.last_edit {
                Some(timer) => timer.elapsed() < UNDO_REDO_TIMER,
                None => false,
            };

            if self.previous_action_type == Some(action)
                && self.current_index > 0
                && elapsed_time_within_limit
            {
                self.stack[self.current_index - 1].add(item, action, version);
                return;
            }

            self.previous_action_type = Some(action);
            self.last_edit = Some(Instant::now());
        } else {
            self.previous_action_type = None;
            self.last_edit = None;
        }

        self.push_undo_item_to_stack(item, version);
    }

    pub fn undo(&mut self) -> Option<UndoResult> {
        if self.stack.is_empty() || self.current_index == 0 {
            return None;
        }

        self.previous_action_type = None;
        self.current_index -= 1;
        let undo_item = &self.stack[self.current_index];

        let undo_result = UndoResult {
            actions: undo_item.actions(),
            selection: undo_item.selection(),
            replacement_range: undo_item.replacement_range.clone(),
        };

        self.stack[self.current_index].reverse();
        Some(undo_result)
    }

    pub fn redo(&mut self) -> Option<UndoResult> {
        if self.current_index >= self.stack.len() {
            return None;
        }
        let redo_item = &self.stack[self.current_index];

        let redo_result = UndoResult {
            actions: redo_item.actions(),
            selection: redo_item.selection(),
            replacement_range: redo_item.replacement_range.clone(),
        };

        self.stack[self.current_index].reverse();
        self.current_index += 1;
        Some(redo_result)
    }

    pub fn reset(&mut self, version: ContentVersion) {
        self.stack.clear();
        self.current_index = 0;
        self.clear_previous_non_atomic_type();
        self.initial_version = version;
    }

    /// Check if the given version matches the current version as reflected in the undo stack.
    /// If at the bottom of the stack, check if it's equal to the earliest version of the content
    /// that the undo stack was aware of.
    /// If the stack is not empty, check if the version matches with the last completed item in the stack.
    /// Due to undo/redo moving the `current_index` pointer in the stack, the last completed item is not
    /// always the last item on the stack.
    pub fn version_match(&self, version: &ContentVersion) -> bool {
        if self.current_index == 0 {
            return *version == self.initial_version;
        }

        self.stack[self.current_index - 1].version == *version
    }

    pub(super) fn set_initial_version(&mut self, version: ContentVersion) {
        self.initial_version = version;
    }

    pub(super) fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

#[cfg(test)]
#[path = "undo_tests.rs"]
pub mod tests;
