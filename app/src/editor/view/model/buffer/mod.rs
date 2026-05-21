mod anchor;
mod deferred_ops;
mod subword_boundaries;
#[cfg(test)]
mod test;
mod text;
mod time;
mod undo;

/// The public interfaces that we expose to the model.
/// This should be a very limited set of APIs and should
/// not expose the internal details of the buffer.
pub use {
    anchor::{Anchor, AnchorBias, AnchorRangeExt},
    deferred_ops::DeferredOperations,
    subword_boundaries::SubwordBoundaries,
    text::{Text, TextSummary},
    time::{LamportValue, ReplicaId},
};

use super::selections::{
    AsSelection, MarkedTextState, RemoteSelection, RemoteSelections, Selection,
};
use super::EditorSnapshot;
use super::{selections::LocalSelections, LocalSelection};
use crate::editor::{CursorColors, PlainTextEditorViewAction};
use anyhow::{anyhow, Result};
use itertools::Itertools;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::{
    cmp::{self},
    collections::HashMap,
    iter::{self, Iterator},
    ops::{AddAssign, Range},
    str,
};
use string_offset::{ByteOffset, CharOffset};
use sum_tree::{self, Cursor, FilterCursor, SeekBias, SumTree};
use time::{Global, Lamport};
use undo::{LocalUndoStack, UndoHistory};
use vec1::{vec1, Vec1};
use warpui::color::ColorU;
use warpui::text::{point::Point, words::is_default_word_boundary, BufferIndex, TextBuffer};
use warpui::text_layout::TextStyle;
use warpui::{Entity, ModelContext};

#[cfg(test)]
use rand::prelude::*;

#[cfg_attr(test, derive(Clone))]
pub struct Buffer {
    /// The latest observed lamport timestamp.
    /// When we mutate the buffer ourselves (edit / undo / selection change),
    /// we tick the clock.
    ///
    /// The replica ID of the clock is the replica ID of this peer.
    lamport_clock: Lamport,

    /// The set of edits we've observed from all peers in the system.
    ///
    /// Selection changes are not observed in the version vector;
    /// while selections depend on edits, the converse is not true.
    versions: Global,

    /// The main data structure of the buffer.
    /// Responsible for storing the edit history
    /// in a tree for fast lookup and insertion.
    fragments: SumTree<Fragment>,

    /// A mapping between an insertion ID and the set of
    /// edits that were made relative to that ID.
    insertion_splits: HashMap<Lamport, SumTree<InsertionSplit>>,

    /// The set of operations that we've deferred for now.
    /// We defer an operation when it cannot yet be applied
    /// (i.e. it was based on edits we haven't observed yet).
    deferred_ops: DeferredOperations,

    /// The current batch of changes, if any.
    /// All CRDT-compliant buffer changes must be batched.
    /// There can only be one batch active at a time.
    ///
    /// Today, edits to the buffer are _not_ sent in a batched form (e.g.
    /// 1 CRDT operation for N edits) to peers.
    batch_state: BatchState,

    /// The selection state for this peer. This is more enriched
    /// than remote selection state because local selection state
    /// can be changed by this peer (whereas remote selection state
    /// cannot be).
    local_selections: LocalSelections,

    /// Selection state per peer.
    ///
    /// We optimistically merge our view of a peer's remote
    /// selections when we receive selection updates or when text is edited
    /// (locally or remotely).
    remote_selections: HashMap<ReplicaId, RemoteSelections>,

    /// The undo stack for local edits.
    local_undo_stack: LocalUndoStack,

    /// The undo history for local and remote edits.
    undo_history: UndoHistory,

    /// The remote replicas that have explicitly been registered
    /// (via [`Self::register_replica`]).
    ///
    /// Only operations from known peers can be applied to the buffer.
    registered_peers: HashMap<ReplicaId, Peer>,
}

#[derive(Clone)]
/// Wrapper struct that holds fields needed to render
/// a peer's cursors.
pub struct PeerSelectionData {
    /// The colors with which this replica's
    /// cursors / selections should be rendered in.
    pub colors: CursorColors,
    /// Whether or not the peer's cursors are drawn.
    pub display_name: String,
    pub image_url: Option<String>,
    pub should_draw_cursors: bool,
}

#[derive(Clone)]
pub struct Peer {
    pub selection_data: PeerSelectionData,
}

/// Whether changes to the buffer are being batched
/// and the nature of the batch (if any).
#[derive(Clone)]
enum BatchState {
    /// There is no on-going batch.
    Inactive,
    /// There is an active batch but it only supports selection changes.
    SelectionChangesOnly {
        /// The set of selections before the batch started.
        selections_before_batch: LocalSelections,
    },
    /// There is an active batch and it supports both edits and selection changes.
    EditsAndSelectionChanges {
        /// The origin of the batched edit.
        edit_origin: EditOrigin,
        /// The action associated to the batched edit.
        action: PlainTextEditorViewAction,
        /// The set of local edit operations for the current batch.
        edits: Vec<EditOperation>,
        /// The number of [`Self::edits`] already recorded on the undo stack.
        /// This relies on [`edits`] being append-only.
        /// We support an API ([`Buffer::record_edits`]) to record the intermediate edits
        /// to the undo stack so we keep track of which are already recorded to avoid double-recording.
        num_edits_already_recorded: usize,
        /// The set of selections before the batch started.
        selections_before_batch: LocalSelections,
        /// The version vector before the batch started.
        since: Global,
        /// Whether or not we should skip recording the edits on the
        /// undo stack when finishing the batch.
        skip_undo: bool,
    },
    /// There is an active batch for an undo / redo operation.
    UndoRedo {
        /// The version vector before the batch started.
        since: Global,
        undo_op: Option<UndoOperation>,
        selections_before_batch: LocalSelections,
    },
}

impl BatchState {
    fn is_batching(&self) -> bool {
        !matches!(self, Self::Inactive)
    }

    fn extend_edits(&mut self, new_edits: Vec<EditOperation>) {
        debug_assert!(matches!(self, Self::EditsAndSelectionChanges { .. }));

        match self {
            Self::Inactive => {
                log::warn!("Tried to extend batch when we weren't already batching");
            }
            Self::SelectionChangesOnly { .. } => {
                log::warn!("Tried to add edits to selection-only batch");
            }
            Self::UndoRedo { .. } => {
                log::warn!("Tried to add edits to undo/redo batch");
            }
            Self::EditsAndSelectionChanges { edits, .. } => {
                edits.extend(new_edits);
            }
        }
    }

    /// Ensures that we are in an appropriate batching state when attempting
    /// to make a selection change to the buffer.
    ///
    /// Panics in debug mode, if not.
    fn attempt_selection_change(&self) -> Result<()> {
        if !self.is_batching() {
            debug_assert!(false, "Cannot change selections outside of a batch");
            anyhow::bail!("Cannot change selections outside of a batch");
        }
        Ok(())
    }

    /// Ensures that we are in an appropriate batching state when attempting
    /// to make an edit to the buffer.
    ///
    /// Panics in debug mode, if not.
    fn attempt_edit(&self) -> Result<()> {
        let res = match self {
            Self::Inactive => Err(anyhow!("Cannot edit outside of a batch")),
            Self::SelectionChangesOnly { .. } | Self::UndoRedo { .. } => {
                Err(anyhow!("Batch does not support edits"))
            }
            Self::EditsAndSelectionChanges { .. } => Ok(()),
        };
        if let Err(e) = &res {
            debug_assert!(false, "{e}");
        }
        res
    }

    /// Ensures that we are in an appropriate batching state when attempting
    /// to perform an undo / redo.
    ///
    /// Panics in debug mode, if not.
    fn attempt_undo(&self) -> Result<()> {
        let res = match self {
            Self::Inactive => Err(anyhow!("Cannot undo outside of a batch")),
            Self::SelectionChangesOnly { .. } | Self::EditsAndSelectionChanges { .. } => {
                Err(anyhow!("Batch does not support undo / redo"))
            }
            Self::UndoRedo { .. } => Ok(()),
        };
        if let Err(e) = &res {
            debug_assert!(false, "{e}");
        }
        res
    }

    fn set_undo_op(&mut self, op: UndoOperation) {
        match self {
            Self::Inactive => {
                log::warn!("Tried to add undo op when we weren't already batching");
            }
            Self::SelectionChangesOnly { .. } => {
                log::warn!("Tried to add undo op during selections-only batch");
            }
            Self::UndoRedo { undo_op, .. } => {
                *undo_op = Some(op);
            }
            Self::EditsAndSelectionChanges { .. } => {
                log::warn!("Tried to add undo op during edits batch");
            }
        }
    }
}

/// What initiated the change in the buffer.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum EditOrigin {
    /// The user typed the change by entering characters.
    UserTyped,

    /// The user didn't type a character but the user did initiate the change (e.g. backspace, paste, etc.).
    UserInitiated,

    /// The user didn't initiate this change. For example, an unsolicited update
    /// from the server replaces some client-side buffer.
    SystemEdit,

    /// The change is caused by a change in a synced terminal input
    SyncedTerminalInput,

    /// This edit came from a peer.
    /// Used for collaborative editors.
    RemoteEdit,
}

impl EditOrigin {
    pub fn is_user(&self) -> bool {
        matches!(self, EditOrigin::UserTyped) || matches!(self, EditOrigin::UserInitiated)
    }
}

#[derive(Clone)]
pub struct CharsWithStyle<'a> {
    fragments_cursor: Cursor<'a, Fragment, CharOffset, CharOffset>,
    fragment_chars: str::Chars<'a>,
    fragment_text_style: TextStyle,
    fragment_offset: CharOffset,
    reversed: bool,
    undo_history: &'a UndoHistory,
}

pub struct Chars<'a>(CharsWithStyle<'a>);

/// StyleOperation is used for storing data pertaining to a single operation
/// that can be performed on a particular TextStyle property e.g. foreground color.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum StyleOperation {
    // None is the default case where we don't change the existing style,
    // if any exists.
    None,
    // Clear specifies explicitly that we wish to clear any existing style,
    // if it exists.
    Clear,
    // Style specifies that we wish to add/overwrite the existing style
    // with the new style (includes specifying a color).
    Style { color: ColorU },
}

impl StyleOperation {
    /// Get resulting color from applying the current StyleOperation onto
    /// some current color.
    fn get_result_color(self, current_color: Option<ColorU>) -> Option<ColorU> {
        match self {
            Self::Style { color } => Some(color),
            Self::Clear => None,
            Self::None => current_color,
        }
    }
}

/// TextStyleOperation defines a single operation for updating
/// text styles - it can update each property of the TextStyle e.g. foreground
/// color, background color, etc.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct TextStyleOperation {
    foreground_color: StyleOperation,
    syntax_color: StyleOperation,
    background_color: StyleOperation,
    error_underline_color: StyleOperation,
}

impl Default for TextStyleOperation {
    fn default() -> Self {
        TextStyleOperation::new()
    }
}

impl TextStyleOperation {
    pub fn new() -> Self {
        TextStyleOperation {
            foreground_color: StyleOperation::None,
            syntax_color: StyleOperation::None,
            background_color: StyleOperation::None,
            error_underline_color: StyleOperation::None,
        }
    }

    /// Applies a certain TextStyleOperation onto a TextStyle and returns
    /// the resulting new TextStyle.
    ///
    /// # Arguments
    ///
    /// * `text_style` - The TextStyle we want to apply the TextStyleOperation on.
    /// * `text_style_operation` - The TextStyleOperation to apply.
    ///
    /// # Example
    /// ```
    /// use warpui::color::ColorU;
    /// use warpui::text_layout::TextStyle;
    /// use warp::editor::TextStyleOperation;
    /// TextStyleOperation::apply_text_style_operation(
    ///     TextStyle::default(),
    ///     TextStyleOperation::default().set_error_underline_color(ColorU::black()),
    /// );
    /// ```
    ///
    pub fn apply_text_style_operation(
        mut text_style: TextStyle,
        text_style_operation: TextStyleOperation,
    ) -> TextStyle {
        text_style.foreground_color = text_style_operation
            .foreground_color
            .get_result_color(text_style.foreground_color);
        text_style.syntax_color = text_style_operation
            .syntax_color
            .get_result_color(text_style.syntax_color);
        text_style.background_color = text_style_operation
            .background_color
            .get_result_color(text_style.background_color);
        text_style.error_underline_color = text_style_operation
            .error_underline_color
            .get_result_color(text_style.error_underline_color);
        text_style
    }

    pub fn set_foreground_color(mut self, foreground_color: ColorU) -> Self {
        self.foreground_color = StyleOperation::Style {
            color: foreground_color,
        };
        self
    }

    pub fn clear_foreground_color(mut self) -> Self {
        self.foreground_color = StyleOperation::Clear;
        self
    }

    pub fn set_syntax_color(mut self, syntax_color: ColorU) -> Self {
        self.syntax_color = StyleOperation::Style {
            color: syntax_color,
        };
        self
    }

    pub fn clear_syntax_color(mut self) -> Self {
        self.syntax_color = StyleOperation::Clear;
        self
    }

    pub fn set_background_color(mut self, background_color: ColorU) -> Self {
        self.background_color = StyleOperation::Style {
            color: background_color,
        };
        self
    }

    pub fn clear_background_color(mut self) -> Self {
        self.background_color = StyleOperation::Clear;
        self
    }

    pub fn set_error_underline_color(mut self, error_underline_color: ColorU) -> Self {
        self.error_underline_color = StyleOperation::Style {
            color: error_underline_color,
        };
        self
    }

    pub fn clear_error_underline_color(mut self) -> Self {
        self.error_underline_color = StyleOperation::Clear;
        self
    }

    pub fn clear_all() -> Self {
        Self {
            foreground_color: StyleOperation::Clear,
            syntax_color: StyleOperation::Clear,
            background_color: StyleOperation::Clear,
            error_underline_color: StyleOperation::Clear,
        }
    }

    /// Set the text style to have no decorations (underlines or syntax color).
    pub fn clear_decorations(self) -> Self {
        self.clear_error_underline_color().clear_syntax_color()
    }
}

pub struct TextStyleRuns<'a> {
    fragments_cursor: Cursor<'a, Fragment, ByteOffset, ByteOffset>,
    byte_index_start: ByteOffset,
    byte_index_end: ByteOffset,
    undo_history: &'a UndoHistory,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextRun {
    text: String,
    text_style: TextStyle,
    // Range of the text in the overall buffer.
    range: Range<ByteOffset>,
}

impl TextRun {
    pub fn new(text: String, text_style: TextStyle, range: Range<ByteOffset>) -> Self {
        TextRun {
            text,
            text_style,
            range,
        }
    }

    pub fn text_from_text_runs(text_runs: &[TextRun]) -> String {
        text_runs
            .iter()
            .map(|text_run| text_run.text().to_string())
            .collect::<Vec<String>>()
            .join("")
    }
}

impl TextRun {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn text_style(&self) -> TextStyle {
        self.text_style
    }

    pub fn byte_range(&self) -> &Range<ByteOffset> {
        &self.range
    }
}

#[derive(Clone)]
pub struct StylizedChar {
    char: char,
    style: TextStyle,
}

impl StylizedChar {
    pub fn new(char: char, style: TextStyle) -> Self {
        StylizedChar { char, style }
    }

    pub fn char(&self) -> char {
        self.char
    }

    pub fn style(&self) -> TextStyle {
        self.style
    }
}

impl From<StylizedChar> for char {
    fn from(stylized_char: StylizedChar) -> Self {
        stylized_char.char
    }
}

struct Edits<'a, F: Fn(&FragmentSummary) -> bool> {
    cursor: FilterCursor<'a, F, Fragment, CharOffset>,
    since: Global,
    delta: isize,
    undo_history: &'a UndoHistory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub old_range: Range<CharOffset>,
    pub new_range: Range<CharOffset>,
}

impl Edit {
    pub fn delta(&self) -> isize {
        (self.new_range.end.as_usize() - self.new_range.start.as_usize()) as isize
            - (self.old_range.end.as_usize() - self.old_range.start.as_usize()) as isize
    }

    pub fn old_extent(&self) -> CharOffset {
        self.old_range.end - self.old_range.start
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Insertion {
    id: Lamport,
    parent_id: Lamport,
    offset_in_parent: CharOffset,
    text: Text,
}

#[derive(Eq, PartialEq, Clone, Debug)]
struct Fragment {
    /// The id of the fragment. Not to be confused
    /// with the associated edit ID.
    id: FragmentId,
    insertion: Insertion,
    text: Text,

    /// The edit IDs of deletions that affect
    /// the text in this fragment. Instead of mutating
    /// this directly, use [`Self::mark_deletion`]
    /// to update the set of deletions.
    ///
    /// We use a [`BTreeSet`] to efficiently compute intersections
    /// with other sets (e.g. undo map).
    deletions: BTreeSet<Lamport>,

    /// Needed to ensure that text styles from a deleted fragment
    /// are only inherited one time. For example, if we delete all the way
    /// to the end of a style and then start typing, we want to first look to the
    /// most recently deleted style and inherit it once. This works to "save"
    /// what we were typing up to the deletion.
    text_style_inherited_after_deletion: bool,

    /// Whether or not the fragment is visible.
    ///
    /// This is cached on the fragment and must
    /// be recomputed when either
    /// 1. marking a deletion, or
    /// 2. undo'ing / redo'ing the edit or any of its deletions
    is_visible: bool,

    /// The maximum undo timestamp that affects
    /// this fragment (either the edit itself
    /// or any of its deletions), for each peer.
    max_undo_ts: Global,
}

#[derive(Eq, PartialEq, Clone, Debug)]
pub struct FragmentSummary {
    text_summary: TextSummary,
    max_fragment_id: FragmentId,
    max_version: Global,
}

#[derive(Eq, PartialEq, Clone, Debug)]
struct InsertionSplit {
    extent: CharOffset,
    fragment_id: FragmentId,
}

#[derive(Eq, PartialEq, Clone, Debug, Default)]
struct InsertionSplitSummary {
    extent: CharOffset,
}

/// A CRDT payload for an edit operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EditOperation {
    /// The lamport timestamp of this edit.
    lamport_timestamp: Lamport,

    /// The edits that had been observed when this edit was made.
    /// This intentionally excludes this edit (identified by [`Self::lamport_timestamp`]) itself.
    versions: Global,

    /// The edit ID that the start of this edit is relative to.
    /// The (start_id, start_character_offset) identify the start of
    /// the range that is being edited.
    start_id: Lamport,

    /// The offset from the edit identified by [`Self::start_id`].
    start_character_offset: CharOffset,

    /// The edit ID that the end of this edit is relative to.
    end_id: Lamport,

    /// The offset from the edit identified by [`Self::end_id`].
    /// The (end_id, end_character_offset) identify the end of
    /// the range that is being edited.
    end_character_offset: CharOffset,

    /// The replacement text.
    /// Note that this is a plain-old string;
    /// we do not support styling via CRDT operations.
    new_text: String,
}

/// A CRDT payload for an undo operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UndoOperation {
    /// The lamport timestamp of the undo operation.
    /// Used to uniquely identify undo operations.
    lamport_timestamp: Lamport,

    /// The version vector at the time of the undo.
    versions: Global,

    /// The lamport timestamps of the operations being undone. A replica can only
    /// undo its own operation so the replica ID of each of these operations is lamport.replica_id.
    operations: Vec<LamportValue>,
}

/// A CRDT payload for updating selections.
/// A selection update is often sent alongside
/// an [`EditOperation`] or [`UndoOperation`] as
/// selection state usually changes as a result of such operations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpdateSelectionsOperation {
    /// The lamport timestamp of the selections update.
    /// We only apply selection updates that are newer than ones we've seen before.
    lamport_timestamp: Lamport,

    /// The actual selection ranges.
    /// There must be at least 1 selection.
    selections: Vec1<RemoteSelection>,
}

/// The set of possible CRDT operations supported by the [`Buffer`].
///
/// TODO (suraj): instead of making this type serializable / deserializable,
/// we should have a dedicated type that converts to and from this type
/// and is intended to be serialized / deserialized (e.g. a protobuf).
/// Otherwise, we run the risk of easily introducing breaking changes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    Edit(EditOperation),
    Undo(UndoOperation),
    UpdateSelections(UpdateSelectionsOperation),
}

struct OperationEditArgs {
    pub start_id: Option<Lamport>,
    pub start_character_offset: Option<CharOffset>,
    pub end_id: Option<Lamport>,
    pub end_character_offset: Option<CharOffset>,
    pub version_in_range: Global,
    new_text: String,
    pub lamport_timestamp: Lamport,
}

impl OperationEditArgs {
    pub fn new(
        new_text: Option<Text>,
        lamport_timestamp: Lamport,
        version_in_range: Global,
    ) -> Self {
        let new_text = if let Some(new_text) = new_text {
            new_text.as_str().to_string()
        } else {
            String::from("")
        };

        Self {
            start_id: None,
            start_character_offset: None,
            end_id: None,
            end_character_offset: None,
            version_in_range,
            new_text,
            lamport_timestamp,
        }
    }

    pub fn update_end(&mut self, end_id: Lamport, end_offset: CharOffset) {
        self.end_id = Some(end_id);
        self.end_character_offset = Some(end_offset);
    }

    pub fn update_start(&mut self, start_id: Lamport, start_offset: CharOffset) {
        self.start_id = Some(start_id);
        self.start_character_offset = Some(start_offset);
    }
}

impl TryFrom<OperationEditArgs> for EditOperation {
    type Error = anyhow::Error;

    fn try_from(edit_args: OperationEditArgs) -> Result<Self> {
        let Some(start_id) = edit_args.start_id else {
            anyhow::bail!("Tried to build EditOperation without start_id")
        };

        let Some(end_id) = edit_args.end_id else {
            anyhow::bail!("Tried to build EditOperation without end_id")
        };

        let Some(start_character_offset) = edit_args.start_character_offset else {
            anyhow::bail!("Tried to build EditOperation without start_character_offset")
        };

        let Some(end_character_offset) = edit_args.end_character_offset else {
            anyhow::bail!("Tried to build EditOperation without end_character_offset")
        };

        Ok(EditOperation {
            start_id,
            start_character_offset,
            end_id,
            end_character_offset,
            versions: edit_args.version_in_range,
            new_text: edit_args.new_text,
            lamport_timestamp: edit_args.lamport_timestamp,
        })
    }
}

struct SplicedFragment {
    chars_to_fragment_start: CharOffset,
    fragment: Fragment,
    insertion_split: InsertionSplit,
}

impl Buffer {
    pub(super) fn new_with_replica_id<T: Into<Text>>(replica_id: ReplicaId, base_text: T) -> Self {
        let mut insertion_splits = HashMap::new();
        let mut fragments = SumTree::new();

        // We use a dedicated, sentinel replica ID for the base edit so that all
        // peers in the system can apply it consistently.
        let base_replica_id = ReplicaId::base_replica_id();
        let base_insertion = Insertion {
            id: Lamport::new(base_replica_id.clone()),
            parent_id: Lamport::new(base_replica_id),
            offset_in_parent: 0.into(),
            text: base_text.into(),
        };

        insertion_splits.insert(
            base_insertion.id.clone(),
            SumTree::from_item(InsertionSplit {
                fragment_id: FragmentId::min_value().clone(),
                extent: 0.into(),
            }),
        );
        fragments.push(Fragment {
            id: FragmentId::min_value().clone(),
            insertion: base_insertion.clone(),
            text: base_insertion
                .text
                .slice(CharOffset::from(0)..CharOffset::from(0)),
            deletions: BTreeSet::new(),
            text_style_inherited_after_deletion: false,
            is_visible: true,
            max_undo_ts: Global::new(),
        });

        if !base_insertion.text.is_empty() {
            let base_fragment_id =
                FragmentId::between(FragmentId::min_value(), FragmentId::max_value());

            insertion_splits
                .get_mut(&base_insertion.id)
                .unwrap()
                .push(InsertionSplit {
                    fragment_id: base_fragment_id.clone(),
                    extent: base_insertion.text.len(),
                });
            fragments.push(Fragment {
                id: base_fragment_id,
                text: base_insertion.text.clone(),
                insertion: base_insertion,
                deletions: BTreeSet::new(),
                text_style_inherited_after_deletion: false,
                is_visible: true,
                max_undo_ts: Global::new(),
            });
        }

        // TODO: should the first cursor actually be at the end
        // of the buffer rather than at the start (in case there's
        // base text)?
        let root_selection = LocalSelection {
            selection: Selection {
                start: Anchor::Start,
                end: Anchor::Start,
                reversed: false,
            },
            clamp_direction: Default::default(),
            goal_start_column: None,
            goal_end_column: None,
        };
        let root_selections: LocalSelections = vec1![root_selection.clone()].into();

        Self {
            fragments,
            insertion_splits,
            versions: Global::new(),
            deferred_ops: DeferredOperations::new(),
            lamport_clock: Lamport::new(replica_id),
            batch_state: BatchState::Inactive,
            local_selections: root_selections.clone(),
            remote_selections: HashMap::new(),
            local_undo_stack: LocalUndoStack::new(root_selections),
            undo_history: UndoHistory::new(),
            registered_peers: HashMap::new(),
        }
    }

    pub(super) fn new<T: Into<Text>>(base_text: T) -> Self {
        Self::new_with_replica_id(ReplicaId::random(), base_text)
    }

    /// Recreates the buffer, but leaving the registered peers in tact.
    pub fn recreate<T: Into<Text>>(&mut self, replica_id: ReplicaId, base_text: T) {
        let peers = std::mem::take(&mut self.registered_peers);
        *self = Self::new_with_replica_id(replica_id, base_text);
        self.registered_peers = peers;
    }

    pub fn replica_id(&self) -> ReplicaId {
        self.lamport_clock.replica_id()
    }

    pub fn local_selections(&self) -> &LocalSelections {
        &self.local_selections
    }

    /// Returns an iterator over each remote replicas selection set and id.
    pub fn remote_selections(&self) -> impl Iterator<Item = (&ReplicaId, &RemoteSelections)> {
        self.remote_selections
            .iter()
            .filter_map(|(replica_id, remote_selections)| {
                self.registered_peers
                    .get(replica_id)
                    .map(|_| (replica_id, remote_selections))
            })
    }

    /// Initializes a batch for edits and selection changes;
    /// all further edits and selection changes will be batched.
    /// The batch must be completed via [`Self::end_batch`].
    ///
    /// TODO (suraj): investigate if we can use RAII to enforce the batch is ended
    /// when a guard (that would be returned by this fn) is dropped.
    /// The tricky part is how we would execute [`Self::on_batch_finished`] in
    /// a destructor.
    pub(super) fn start_edits_and_selection_changes_batch(
        &mut self,
        edit_origin: EditOrigin,
        action: PlainTextEditorViewAction,
        skip_undo: bool,
    ) {
        debug_assert!(!self.batch_state.is_batching());
        self.batch_state = BatchState::EditsAndSelectionChanges {
            edit_origin,
            action,
            since: self.versions.clone(),
            selections_before_batch: self.local_selections.clone(),
            edits: vec![],
            num_edits_already_recorded: 0,
            skip_undo,
        };
    }

    pub(super) fn refresh_version_on_edits_and_selection_changes_batch(&mut self) {
        debug_assert!(self.batch_state.is_batching());
        if let BatchState::EditsAndSelectionChanges { since, .. } = &mut self.batch_state {
            *since = self.versions.clone();
        }
    }

    /// Initializes a batch for selection-only changes;
    /// all further selection changes will be batched.
    /// The batch must be completed via [`Self::end_batch`].
    pub(super) fn start_selection_changes_only_batch(&mut self) {
        debug_assert!(!self.batch_state.is_batching());
        self.batch_state = BatchState::SelectionChangesOnly {
            selections_before_batch: self.local_selections.clone(),
        };
    }

    fn start_undo_redo_batch(&mut self) {
        debug_assert!(!self.batch_state.is_batching());
        self.batch_state = BatchState::UndoRedo {
            since: self.versions.clone(),
            undo_op: None,
            selections_before_batch: self.local_selections.clone(),
        };
    }

    /// Completes the on-going batch.
    pub(super) fn end_batch(&mut self, ctx: &mut ModelContext<Self>) {
        debug_assert!(self.batch_state.is_batching());
        let batch_state = std::mem::replace(&mut self.batch_state, BatchState::Inactive);
        let mut operations = vec![];

        let selections_before_batch = match batch_state {
            BatchState::Inactive => {
                log::warn!("Tried to end batch when a batch wasn't ongoing");
                return;
            }
            BatchState::SelectionChangesOnly {
                selections_before_batch,
            } => {
                if selections_before_batch != self.local_selections {
                    self.local_undo_stack
                        .record_selection_change(self.local_selections.clone());
                    ctx.emit(Event::SelectionsChanged);
                }
                selections_before_batch
            }
            BatchState::EditsAndSelectionChanges {
                edits,
                edit_origin,
                action,
                since,
                num_edits_already_recorded,
                skip_undo,
                selections_before_batch,
                ..
            } => {
                if !edits.is_empty() {
                    if !skip_undo {
                        let edit_lamport_values = edits
                            .iter()
                            .skip(num_edits_already_recorded)
                            .map(|o: &EditOperation| o.lamport_timestamp.value);

                        self.local_undo_stack.record_edit(
                            action,
                            edit_lamport_values,
                            self.local_selections.clone(),
                        );
                    }

                    let edit_ranges = self.edits_since(since).collect::<Vec<_>>();
                    ctx.emit(Event::Edited {
                        edits: edit_ranges,
                        edit_origin,
                    });
                } else if selections_before_batch != self.local_selections {
                    // We only emit this event if we're not already emitting
                    // an [`Event::Edited`] event.
                    ctx.emit(Event::SelectionsChanged);
                }

                operations.extend(edits.into_iter().map(Operation::Edit));
                selections_before_batch
            }
            BatchState::UndoRedo {
                since,
                undo_op,
                selections_before_batch,
            } => {
                if let Some(undo_op) = undo_op {
                    operations.push(Operation::Undo(undo_op));

                    let edit_ranges = self.edits_since(since).collect::<Vec<_>>();
                    ctx.emit(Event::Edited {
                        edits: edit_ranges,
                        edit_origin: EditOrigin::UserInitiated,
                    });
                }
                selections_before_batch
            }
        };

        if selections_before_batch != self.local_selections {
            operations.push(self.record_selections_update());
        }

        // In practice, we only need to emit this event if there're operations to fan out.
        // In tests, we expect this event regardless.
        if cfg!(test) || !operations.is_empty() {
            ctx.emit(Event::UpdatePeers {
                operations: Rc::new(operations),
            });
        }
    }

    pub(super) fn registered_peers(&self) -> HashMap<ReplicaId, Peer> {
        self.registered_peers.clone()
    }

    pub(super) fn register_peer(
        &mut self,
        replica_id: ReplicaId,
        selection_data: PeerSelectionData,
    ) {
        self.registered_peers
            .insert(replica_id, Peer { selection_data });
    }

    pub(super) fn unregister_peer(&mut self, replica_id: &ReplicaId) {
        self.registered_peers.remove(replica_id);
    }

    pub(super) fn unregister_all_peers(&mut self) {
        self.registered_peers = HashMap::new();
    }

    pub(super) fn set_peer_selection_data(
        &mut self,
        replica_id: &ReplicaId,
        selection_data: PeerSelectionData,
    ) {
        let Some(peer) = self.registered_peers.get_mut(replica_id) else {
            log::warn!("Tried to update selection data of non-existent peer");
            return;
        };
        peer.selection_data = selection_data;
    }

    pub fn snapshot(&self) -> EditorSnapshot {
        let selections = self
            .local_selections
            .selections
            .mapped_ref(|selection| selection.to_offset(self));
        let buffer_text_runs = self.text_style_runs().collect();
        EditorSnapshot::new(selections, buffer_text_runs)
    }

    pub fn text_summary(&self) -> TextSummary {
        self.fragments.extent::<TextSummary>()
    }

    pub fn text_summary_for_range(&self, range: Range<CharOffset>) -> TextSummary {
        let mut summary = TextSummary::default();

        let mut cursor = self.fragments.cursor::<CharOffset, CharOffset>();
        cursor.seek(&range.start, SeekBias::Right);

        if let Some(fragment) = cursor.item() {
            let summary_start = cmp::max(*cursor.start(), range.start) - *cursor.start();
            let summary_end = cmp::min(range.end - *cursor.start(), fragment.len());
            summary += &fragment.text.slice(summary_start..summary_end).summary();
            cursor.next();
        }

        if range.end > *cursor.start() {
            summary += &cursor.summary::<TextSummary>(&range.end, SeekBias::Right);

            if let Some(fragment) = cursor.item() {
                let summary_start = cmp::max(*cursor.start(), range.start) - *cursor.start();
                let summary_end = cmp::min(range.end - *cursor.start(), fragment.len());
                summary += &fragment.text.slice(summary_start..summary_end).summary();
            }
        }

        summary
    }

    pub fn len(&self) -> CharOffset {
        self.fragments.extent::<CharOffset>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0.into()
    }

    pub fn line_len(&self, row: u32) -> Result<u32> {
        let row_start_offset = <Point as ToCharOffset>::to_char_offset(&Point::new(row, 0), self)?;
        let row_end_offset = if row >= self.max_point().row {
            self.len()
        } else {
            <Point as ToCharOffset>::to_char_offset(&Point::new(row + 1, 0), self)? - 1
        };

        Ok((row_end_offset.as_usize() - row_start_offset.as_usize()) as u32)
    }

    pub fn max_point(&self) -> Point {
        self.fragments.extent()
    }

    pub fn line(&self, row: u32) -> Result<String> {
        Ok(self
            .chars_at(Point::new(row, 0))?
            .take_while(|c| *c != '\n')
            .collect())
    }

    pub fn text(&self) -> String {
        self.chars().collect()
    }

    pub fn text_for_range<T: ToCharOffset>(&self, range: Range<T>) -> Result<String> {
        Ok(self.chars_for_range(range)?.collect())
    }

    /// Iterator of text style runs of the buffer. Each style run is a string of text that
    /// shares the same text style.
    pub fn text_style_runs(&self) -> impl '_ + Iterator<Item = TextRun> {
        let mut fragments_cursor = self.fragments.cursor::<ByteOffset, ByteOffset>();
        let start = *fragments_cursor.start();
        fragments_cursor.seek(&start, SeekBias::Left);

        TextStyleRuns {
            fragments_cursor,
            byte_index_start: 0.into(),
            byte_index_end: 0.into(),
            undo_history: &self.undo_history,
        }
    }

    pub fn chars_for_range<T: ToCharOffset>(
        &self,
        range: Range<T>,
    ) -> Result<impl '_ + Iterator<Item = char>> {
        let start = range.start.to_char_offset(self)?;
        let end = range.end.to_char_offset(self)?;
        Ok(self
            .chars_at(start)?
            .take(end.as_usize() - start.as_usize()))
    }

    pub fn chars(&self) -> Chars<'_> {
        self.chars_at(CharOffset::from(0)).unwrap()
    }

    pub fn stylized_chars(&self) -> CharsWithStyle<'_> {
        self.stylized_chars_at(CharOffset::from(0))
            .expect("offset 0 should exist")
    }

    pub fn chars_at<T: ToCharOffset>(&self, position: T) -> Result<Chars<'_>> {
        Ok(Chars(self.stylized_chars_at(position)?))
    }

    pub fn stylized_chars_at<T: ToCharOffset>(&self, position: T) -> Result<CharsWithStyle<'_>> {
        let offset = position.to_char_offset(self)?;

        let mut fragments_cursor = self.fragments.cursor::<CharOffset, CharOffset>();
        fragments_cursor.seek(&offset, SeekBias::Right);

        let fragment_offset = offset - *fragments_cursor.start();
        let (fragment_chars, fragment_text_style) =
            fragments_cursor
                .item()
                .map_or(("".chars(), TextStyle::default()), |fragment| {
                    (
                        fragment.text[fragment_offset..].chars(),
                        fragment.text.text_style().unwrap_or_default(),
                    )
                });

        Ok(CharsWithStyle {
            fragments_cursor,
            fragment_chars,
            fragment_offset,
            fragment_text_style,
            reversed: false,
            undo_history: &self.undo_history,
        })
    }

    /// Get an iterator of subword starting points forward from the given offset
    pub fn subword_starts_from_offset<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<SubwordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(SubwordBoundaries::forward_subword_starts(
            position.to_char_offset(self)?,
            self.chars_at(offset)?,
            self,
        ))
    }

    /// Get an iterator of subword ending points forward from the given offset,
    /// excluding the current location if it is a word boundary.
    pub fn subword_ends_from_offset_exclusive<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<SubwordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(SubwordBoundaries::forward_subword_ends_exclusive(
            position.to_char_offset(self)?,
            self.chars_at(offset)?,
            self,
        ))
    }

    /// Get an iterator of word starting points backwards from the given offset,
    /// excluding the current location if it is a word boundary.
    pub fn subword_backward_starts_from_offset_exclusive<T: BufferIndex>(
        &self,
        position: T,
    ) -> Result<SubwordBoundaries<'_, Self>> {
        let offset = position.to_char_offset(self)?;
        Ok(SubwordBoundaries::backward_subword_starts_exclusive(
            position.to_char_offset(self)?,
            self.chars_rev_at(offset)?,
            self,
        ))
    }

    pub fn edits_since(&self, since: Global) -> impl '_ + Iterator<Item = Edit> {
        let since_2 = since.clone();
        let cursor = self
            .fragments
            .filter(move |summary| summary.max_version.changed_since(&since_2));

        Edits {
            cursor,
            since,
            delta: 0,
            undo_history: &self.undo_history,
        }
    }

    pub(super) fn indent(
        &mut self,
        row_range: Range<u32>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let max_row = self.max_point().row;
        let offset_ranges = row_range
            .map(|row| {
                if row > max_row {
                    return Err(anyhow!("invalid row: {}", row));
                }
                let line_start = ToCharOffset::to_char_offset(&Point::new(row, 0), self)?;
                Ok(line_start..line_start)
            })
            .collect::<Result<Vec<Range<CharOffset>>>>()?;
        self.edit(offset_ranges, "    ", ctx)
    }

    pub(super) fn unindent(
        &mut self,
        row_ranges: Vec<Range<u32>>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let max_row = self.max_point().row;
        let offset_ranges = row_ranges
            .into_iter()
            .flat_map(|row_range| {
                row_range.map(|row| {
                    if row > max_row {
                        return Err(anyhow!("invalid row: {}", row));
                    }
                    let tab_start = ToCharOffset::to_char_offset(&Point::new(row, 0), self)?;
                    let tab_end = tab_start
                        + self
                            .line(row)
                            .unwrap()
                            .chars()
                            .take(4)
                            .take_while(|c| c.is_whitespace())
                            .count();
                    Ok(tab_start..tab_end)
                })
            })
            .collect::<Result<Vec<Range<CharOffset>>>>()?;
        self.edit(offset_ranges, "", ctx)
    }

    /// Ticks the lamport timestamp and observes the time
    /// in the version vector.
    ///
    /// Returns the version vector _before_ the tick.
    fn tick(&mut self) -> Global {
        let version_before_tick = self.versions.clone();
        self.lamport_clock.tick();
        self.versions.observe(&self.lamport_clock);
        version_before_tick
    }

    pub(super) fn last_action(&self) -> Option<PlainTextEditorViewAction> {
        self.local_undo_stack.last_action()
    }

    pub(super) fn undo(&mut self, ctx: &mut ModelContext<Self>) {
        self.start_undo_redo_batch();

        if let Some((ops, selections)) = self.local_undo_stack.undo() {
            if let Err(e) = self.local_undo_or_redo(ops, selections) {
                log::warn!("Failed to perform local undo: {e}");
            }
        }

        self.end_batch(ctx);
    }

    pub(super) fn redo(&mut self, ctx: &mut ModelContext<Self>) {
        self.start_undo_redo_batch();

        if let Some((ops, selections)) = self.local_undo_stack.redo() {
            if let Err(e) = self.local_undo_or_redo(ops, selections) {
                log::warn!("Failed to perform local redo: {e}");
            }
        }

        self.end_batch(ctx);
    }

    pub(super) fn record_edits(
        &mut self,
        action: PlainTextEditorViewAction,
        _ctx: &mut ModelContext<Self>,
    ) {
        if let BatchState::EditsAndSelectionChanges {
            edits,
            num_edits_already_recorded,
            ..
        } = &mut self.batch_state
        {
            let lamport_values = edits
                .iter()
                .skip(*num_edits_already_recorded)
                .map(|o| o.lamport_timestamp.value);
            self.local_undo_stack.record_edit(
                action,
                lamport_values,
                self.local_selections.clone(),
            );
            *num_edits_already_recorded = edits.len();
        } else {
            debug_assert!(
                false,
                "Tried to record edits in an insufficient batch state"
            );
        }
    }

    pub(super) fn reset_undo_stack(&mut self) {
        self.local_undo_stack.reset(self.local_selections.clone());
    }

    /// Undoes or redoes the `edits` (based on what each edit's current
    /// state is; see [`Self::undo_or_redo`]) and restores the selection
    /// state to `selections`.
    fn local_undo_or_redo(
        &mut self,
        edits: Vec<LamportValue>,
        selections: LocalSelections,
    ) -> Result<()> {
        self.batch_state.attempt_undo()?;

        let since = self.tick();
        self.undo_or_redo(self.lamport_clock.clone(), edits.clone())?;
        self.change_selections(selections)?;

        self.batch_state.set_undo_op(UndoOperation {
            lamport_timestamp: self.lamport_clock.clone(),
            operations: edits,
            versions: since.clone(),
        });
        Ok(())
    }

    /// Undoes or redoes the `edits` with the given `undo_ts``.
    ///
    /// For each edit:
    /// - if it is undone, then we would redo it.
    /// - if it is redone (or was never undone), we would undo it.
    fn undo_or_redo(&mut self, undo_ts: Lamport, edits: Vec<LamportValue>) -> Result<()> {
        // Register all of the edits that should be undone.
        let mut undone_edits = BTreeSet::new();
        for edit in edits {
            let l = Lamport {
                replica_id: undo_ts.replica_id(),
                value: edit,
            };
            undone_edits.insert(l.clone());
            self.undo_history.undo(l, undo_ts.value);
        }

        // TODO: rather than iterating over each fragment and checking
        // if that fragment needs to be updated, we should just search
        // for all the relevant fragments directly.
        let old_fragments = self.fragments.clone();
        let mut new_fragments = SumTree::new();
        let mut cursor = old_fragments.cursor::<FragmentIdRef, ()>();
        cursor.seek(&FragmentIdRef::new(FragmentId::min_value()), SeekBias::Left);

        while let Some(mut fragment) = cursor.item().cloned() {
            // If the edit associated to this fragment or
            // any of its deletions were undone, we should
            // 1. recompute whether this fragment is visible
            //    because whether a fragment is visible depends
            //    on if the fragment or its deletions were undone
            //    (see [`Fragment::is_visible`]) and we just marked
            //    edits as undone above
            // 2. reflect the undo in the fragment's max undo ts
            if undone_edits.contains(&fragment.insertion.id)
                || !fragment.deletions.is_disjoint(&undone_edits)
            {
                fragment.recompute_visibility(&self.undo_history);
                fragment.max_undo_ts.observe(&undo_ts);
            }

            new_fragments.push(fragment);
            cursor.next();
        }
        self.fragments = new_fragments;

        // After apply an undo / redo operation, we need to merge selection state
        // in case there are overlapping ranges.
        self.merge_all_selections()?;

        Ok(())
    }

    pub fn versions(&self) -> Global {
        self.versions.clone()
    }

    /// Changes the set of local selections to the ones provided,
    /// and post-processes them to ensure that any contiguous selections are merged.
    /// Selections can only be changed while batching (see [`Self::start_batch`]).
    ///
    /// TODO: ideally, this method can take in a callback that operates over
    /// a mutable [`LocalSelections`] to avoid requiring the caller to allocate a new
    /// set of [`LocalSelections`]. But this leads to borrow errors because a lot of
    /// selection changes require querying the [`Buffer`].
    pub(super) fn change_selections(&mut self, new_selections: LocalSelections) -> Result<()> {
        // Make sure this change is part of a batch.
        self.batch_state.attempt_selection_change()?;

        self.local_selections = new_selections;
        self.merge_local_selections()?;
        Ok(())
    }

    /// Replaces the old ranges with the new_text. Note that old_ranges MUST be sorted and have
    /// unique values, otherwise the call will panick.
    pub(super) fn edit<I, S, T>(
        &mut self,
        old_ranges: I,
        new_text: T,
        _ctx: &mut ModelContext<Self>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = Range<S>>,
        S: ToCharOffset,
        T: Into<Text>,
    {
        // Make sure this edit is part of a batch.
        self.batch_state.attempt_edit()?;

        let new_text = new_text.into();
        let new_text = if !new_text.is_empty() {
            Some(new_text)
        } else {
            None
        };

        let old_ranges = old_ranges
            .into_iter()
            .map(|range| Ok(range.start.to_char_offset(self)?..range.end.to_char_offset(self)?))
            .collect::<Result<Vec<Range<CharOffset>>>>()?;

        let ops = self.update_fragments_for_edit(old_ranges, new_text.clone())?;

        if let Some(op) = ops.last() {
            self.versions.observe(&op.lamport_timestamp);
        }

        self.batch_state.extend_edits(ops);

        // After editing the buffer, we need to merge selection state
        // in case there are overlapping ranges.
        self.merge_all_selections()?;

        Ok(())
    }

    #[cfg(test)]
    pub(super) fn all_selections(
        &self,
        include_self: bool,
    ) -> HashMap<ReplicaId, Vec<Range<CharOffset>>> {
        let mut all = HashMap::new();

        for id in self.remote_selections.keys() {
            all.insert(id.clone(), self.selections_for_replica(id.clone()));
        }

        if include_self {
            all.insert(
                self.replica_id(),
                self.selections_for_replica(self.replica_id()),
            );
        }

        all
    }

    #[cfg(test)]
    pub(super) fn selections_for_replica(&self, replica: ReplicaId) -> Vec<Range<CharOffset>> {
        use crate::editor::RangeExt;
        use itertools::Either;

        let selections = if replica == self.replica_id() {
            Either::Left(
                self.local_selections
                    .selections
                    .clone()
                    .into_iter()
                    .map(|selection| selection.selection),
            )
        } else {
            Either::Right(
                self.remote_selections
                    .get(&replica)
                    .cloned()
                    .into_iter()
                    .flat_map(|selections| selections.selections.into_iter())
                    .map(|selection| selection.selection),
            )
        };

        selections
            .map(|s| {
                // TODO (suraj): ideally, `to_offset` already returns the range in a sorted fashion.
                let sorted = s.to_offset(self).sorted();
                sorted.0..sorted.1
            })
            .collect()
    }

    /// Equivalent to [`Buffer::edit`] but does it in a self-started batch.
    #[cfg(test)]
    pub(super) fn edit_for_test<I, S, T>(
        &mut self,
        old_ranges: I,
        new_text: T,
        edit_origin: EditOrigin,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = Range<S>>,
        S: ToCharOffset,
        T: Into<Text>,
    {
        self.start_edits_and_selection_changes_batch(
            edit_origin,
            PlainTextEditorViewAction::ReplaceBuffer,
            false,
        );
        self.edit(old_ranges, new_text, ctx)?;
        self.end_batch(ctx);
        Ok(())
    }

    /// Changes local selections to the ranges provided in a self-started batch.
    /// Returns true if the selection state was actually changed (i.e. old selections
    /// not equal to new selections).
    #[cfg(test)]
    pub(super) fn change_selections_for_test<I, S>(
        &mut self,
        new_ranges: I,
        ctx: &mut ModelContext<Self>,
    ) -> Result<bool>
    where
        I: IntoIterator<Item = Range<S>>,
        S: ToCharOffset,
    {
        let old_selections = self.local_selections.clone();

        self.start_selection_changes_only_batch();
        let new_selections = new_ranges
            .into_iter()
            .map(|r| {
                LocalSelection::new_for_test(
                    self.anchor_before(r.start)
                        .expect("start of range should be valid"),
                    self.anchor_before(r.end)
                        .expect("end of range should be valid"),
                )
            })
            .collect();
        self.change_selections(Vec1::try_from_vec(new_selections)?.into())?;
        self.end_batch(ctx);

        Ok(old_selections != self.local_selections)
    }

    /// Updates the text styles for text within a given buffer
    /// in-place (by updating the underlying Fragments). Note that the caller
    /// can pass in multiple ranges at once if they'd like to apply the same
    /// style to each of the ranges.
    ///
    /// # Arguments
    ///
    /// * `old_ranges` - An object that can be iterated over, containing Ranges
    ///   defined by CharOffsets.
    /// * `text_style_operation` - The TextStyleOperation to apply to the given text ranges.
    /// * `ctx` - ModelContext for the buffer being updated.
    ///
    /// # Example
    /// ```ignore
    /// use warpui::{color::ColorU, App, ModelHandle};
    /// use warp::Assets;
    /// use warp::editor::model::buffer::{Buffer, TextStyleOperation, EditOrigin};
    /// use string_offset::CharOffset;
    /// App::test((), |mut app| async move {
    ///     let buffer_model: &mut ModelHandle<Buffer> =
    ///     &mut app.add_model(|_| Buffer::new(0, "abc"));
    ///     buffer_model.update(&mut app, |buffer, ctx| {
    ///         buffer.update_styles(
    ///             vec![CharOffset::from(0)..CharOffset::from(2)],
    ///             TextStyleOperation::default().set_error_underline_color(ColorU::black()),
    ///             EditOrigin::UserInitiated,
    ///             ctx,
    ///         );
    ///     });
    /// });
    /// ```
    ///
    pub(super) fn update_styles<I, S>(
        &mut self,
        old_ranges: I,
        text_style_operation: TextStyleOperation,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = Range<S>>,
        S: ToCharOffset,
    {
        // We don't need to be in a txn when updating styles because
        // 1. it doesn't actually change the contents of the buffer (including selection state)
        // 2. it is not a CRDT-compliant change
        let old_ranges = old_ranges
            .into_iter()
            .map(|range| Ok(range.start.to_char_offset(self)?..range.end.to_char_offset(self)?))
            .collect::<Result<Vec<Range<CharOffset>>>>()?;

        self.update_fragment_styles(
            old_ranges
                .into_iter()
                .filter(|old_range| old_range.end > old_range.start),
            text_style_operation,
        )?;

        ctx.emit(Event::StylesUpdated);
        Ok(())
    }

    pub(super) fn apply_ops<I: IntoIterator<Item = Operation>>(
        &mut self,
        ops: I,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let since = self.versions.clone();

        let mut deferred_ops = Vec::new();
        for op in ops {
            if self.can_apply_op(&op) {
                self.apply_op(op)?;
            } else {
                deferred_ops.push(op);
            }
        }
        self.deferred_ops.extend(deferred_ops);
        self.flush_deferred_ops()?;

        let edits_since = self.edits_since(since).collect();
        ctx.emit(Event::Edited {
            edits: edits_since,
            edit_origin: EditOrigin::RemoteEdit,
        });

        Ok(())
    }

    fn apply_op(&mut self, op: Operation) -> Result<()> {
        // Apply the operation based on its type.
        match op {
            Operation::Edit(EditOperation {
                start_id,
                start_character_offset: start_offset,
                end_id,
                end_character_offset: end_offset,
                new_text,
                versions: version_in_range,
                lamport_timestamp,
            }) => {
                // If this edit was already applied, ignore it.
                if self.versions.observed(&lamport_timestamp) {
                    return Ok(());
                }

                self.apply_edit(
                    start_id,
                    start_offset,
                    end_id,
                    end_offset,
                    new_text,
                    &version_in_range,
                    &lamport_timestamp,
                )?;

                // Acknowledge that this edit was applied.
                self.lamport_clock.observe(&lamport_timestamp);
                self.versions.observe(&lamport_timestamp);
            }
            Operation::Undo(UndoOperation {
                lamport_timestamp,
                operations,
                ..
            }) => {
                // If this undo was already applied, ignore it.
                if self.versions.observed(&lamport_timestamp) {
                    return Ok(());
                }

                self.undo_or_redo(lamport_timestamp.clone(), operations)?;

                // Acknowledge that this undo was applied.
                self.lamport_clock.observe(&lamport_timestamp);
                self.versions.observe(&lamport_timestamp);
            }
            Operation::UpdateSelections(UpdateSelectionsOperation {
                lamport_timestamp,
                selections,
                ..
            }) => {
                // If we have newer selections for this peer, ignore this update.
                if self
                    .remote_selections
                    .get(&lamport_timestamp.replica_id)
                    .map(|existing| existing.lamport >= lamport_timestamp.value)
                    .unwrap_or(false)
                {
                    return Ok(());
                }

                self.remote_selections.insert(
                    lamport_timestamp.replica_id(),
                    RemoteSelections {
                        selections: selections.clone(),
                        lamport: lamport_timestamp.value,
                    },
                );
                self.merge_remote_selections_for_peer(lamport_timestamp.replica_id())?;

                // Acknowledge that we saw this event.
                // We intentionally do not observe the operation's lamport timestamp
                // in the version vector; otherwise, it's possible that we reject lagging edits.
                self.lamport_clock.observe(&lamport_timestamp);
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_edit(
        &mut self,
        start_id: Lamport,
        start_character_offset: CharOffset,
        end_id: Lamport,
        end_character_offset: CharOffset,
        new_text: String,
        version_in_range: &Global,
        lamport_timestamp: &Lamport,
    ) -> Result<()> {
        let mut new_text = if new_text.is_empty() {
            None
        } else {
            Some(new_text.into())
        };

        let start_fragment_id = self.resolve_fragment_id(start_id, start_character_offset)?;
        let end_fragment_id = self.resolve_fragment_id(end_id, end_character_offset)?;

        let old_fragments = self.fragments.clone();
        let last_id = old_fragments.extent::<FragmentIdRef>().0.unwrap();
        let last_id_ref = FragmentIdRef::new(last_id);

        let mut cursor = old_fragments.cursor::<FragmentIdRef, ()>();
        let mut new_fragments =
            cursor.slice(&FragmentIdRef::new(&start_fragment_id), SeekBias::Left);

        if start_character_offset == cursor.item().unwrap().end_character_offset() {
            new_fragments.push(cursor.item().unwrap().clone());
            cursor.next();
        }

        while let Some(fragment) = cursor.item() {
            if new_text.is_none() && fragment.id > end_fragment_id {
                break;
            }

            let mut fragment = fragment.clone();

            if fragment.id == start_fragment_id || fragment.id == end_fragment_id {
                let split_start = if start_fragment_id == fragment.id {
                    start_character_offset
                } else {
                    fragment.start_character_offset()
                };
                let split_end = if end_fragment_id == fragment.id {
                    end_character_offset
                } else {
                    fragment.end_character_offset()
                };
                let (before_range, within_range, after_range) = self.split_fragment(
                    cursor.prev_item().as_ref().unwrap(),
                    &fragment,
                    split_start..split_end,
                );
                let insertion = new_text.take().map(|new_text| {
                    self.build_fragment_to_insert(
                        before_range
                            .as_ref()
                            .or_else(|| cursor.prev_item())
                            .unwrap(),
                        within_range.as_ref().or(after_range.as_ref()),
                        new_text,
                        lamport_timestamp.clone(),
                    )
                });
                if let Some(fragment) = before_range {
                    new_fragments.push(fragment);
                }
                if let Some(fragment) = insertion {
                    new_fragments.push(fragment);
                }
                if let Some(mut fragment) = within_range {
                    if version_in_range.observed(&fragment.insertion.id) {
                        fragment.mark_deletion(lamport_timestamp.clone(), &self.undo_history);
                    }
                    new_fragments.push(fragment);
                }
                if let Some(fragment) = after_range {
                    new_fragments.push(fragment);
                }
            } else {
                if new_text.is_some() && lamport_timestamp > &fragment.insertion.id {
                    new_fragments.push(self.build_fragment_to_insert(
                        cursor.prev_item().as_ref().unwrap(),
                        Some(&fragment),
                        new_text.take().unwrap(),
                        lamport_timestamp.clone(),
                    ));
                }

                if fragment.id < end_fragment_id
                    && version_in_range.observed(&fragment.insertion.id)
                {
                    fragment.mark_deletion(lamport_timestamp.clone(), &self.undo_history);
                }
                new_fragments.push(fragment);
            }

            cursor.next();
        }

        if let Some(new_text) = new_text {
            new_fragments.push(self.build_fragment_to_insert(
                cursor.prev_item().as_ref().unwrap(),
                None,
                new_text,
                lamport_timestamp.clone(),
            ));
        }

        new_fragments.push_tree(cursor.slice(&last_id_ref, SeekBias::Right));
        self.fragments = new_fragments;

        // After applying a remote edit, we need to merge selection state
        // in case there are overlapping ranges.
        self.merge_all_selections()?;

        Ok(())
    }

    fn flush_deferred_ops(&mut self) -> Result<()> {
        let mut deferred_ops = Vec::new();
        for op in self.deferred_ops.drain() {
            if self.can_apply_op(&op) {
                self.apply_op(op)?;
            } else {
                deferred_ops.push(op);
            }
        }
        self.deferred_ops.extend(deferred_ops);
        Ok(())
    }

    fn can_apply_op(&self, op: &Operation) -> bool {
        if self.deferred_ops.replica_deferred(op.replica_id()) {
            return false;
        }

        match op {
            Operation::Edit(EditOperation {
                start_id,
                end_id,
                versions: version_in_range,
                ..
            }) => {
                self.versions.observed(start_id)
                    && self.versions.observed(end_id)
                    && *version_in_range <= self.versions
            }
            Operation::Undo(UndoOperation { versions, .. }) => *versions <= self.versions,
            Operation::UpdateSelections(UpdateSelectionsOperation { selections, .. }) => {
                selections.iter().all(|selection| selection.observed(self))
            }
        }
    }

    fn resolve_fragment_id(
        &self,
        edit_id: Lamport,
        character_offset: CharOffset,
    ) -> Result<FragmentId> {
        let split_tree = self
            .insertion_splits
            .get(&edit_id)
            .ok_or_else(|| anyhow!("failed to find edit id {edit_id:?} in insertion splits map"))?;
        let mut cursor = split_tree.cursor::<CharOffset, ()>();
        cursor.seek(&character_offset, SeekBias::Left);
        Ok(cursor
            .item()
            .ok_or_else(|| anyhow!("failed to find edit id {edit_id:?} in insertion splits tree"))?
            .fragment_id
            .clone())
    }

    /// Splices `current_fragment` into a new spliced fragment at `char_offset`, updating `fragment`
    /// to start at `char_offset`. Note, both `char_offset` and `chars_to_fragment_start` are
    /// character offsets from the _start_ of the buffer. For example, if `current_fragment` goes
    /// from character offset 10 to 20 and `char_offset` is 15, `current_fragment` would now start
    /// at 15 and a new spliced fragment would be returned that goes from 10 -> 15. Note it is up to
    /// the caller to ensure that `char_offset` is contained within `fragment`.
    fn splice_fragment_at_char_offset(
        current_fragment: &mut Fragment,
        last_fragment: &Fragment,
        chars_to_fragment_start: CharOffset,
        char_offset: CharOffset,
    ) -> SplicedFragment {
        let mut prefix = current_fragment.clone();

        prefix.set_end_character_offset(
            prefix.start_character_offset() + (char_offset - chars_to_fragment_start),
        );
        prefix.id = FragmentId::between(&last_fragment.id, &current_fragment.id);
        current_fragment.set_start_offset(prefix.end_character_offset());

        let insertion_split = InsertionSplit {
            extent: prefix.end_character_offset() - prefix.start_character_offset(),
            fragment_id: prefix.id.clone(),
        };

        SplicedFragment {
            chars_to_fragment_start: char_offset,
            fragment: prefix,
            insertion_split,
        }
    }

    /// Inserts `new_text` as a new fragment into the buffer, splicing fragments as necessary and
    /// returns a Vector of `Operations` of the edits that were made.
    ///
    /// The buffer is composed is a series of `Fragments` (essentially a range of text, and a
    /// timestamp to indicate when/if the fragment was deleted). At a high level, this function
    /// builds up a new set of fragments that make up the model after the edit. All fragments before
    /// the first item in the range or after any of the ranges are considered to be untouched. If a
    /// range starts or ends within a fragment, that fragment is sliced into 2 fragments (one for the
    /// piece up to the range that is now considered deleted), and one for the piece of the fragment
    /// that is not covered by the range. Any fragments that are fully contained within a range are
    /// considered to be fully deleted.
    ///
    /// Note it is up to the caller to ensure that `old_ranges` is sorted by the start of the range
    /// ranges are mutually exclusive and that the start point is <= the end point for each range.
    fn update_fragments_for_edit(
        &mut self,
        old_ranges: Vec<Range<CharOffset>>,
        new_text: Option<Text>,
    ) -> Result<Vec<EditOperation>> {
        debug_assert!(
            old_ranges.iter().all(|r| r.start <= r.end)
                && old_ranges
                    .windows(2)
                    .all(|w| w[0].start < w[1].start && w[0].end < w[1].start)
        );

        let mut old_ranges = old_ranges.into_iter().filter(|range| {
            // When new_text is some, that means we're inserting text or replacing text,
            // so any given range could be empty or non-empty.
            // When removing text, the range should be non-empty (removing nothing is not useful).
            if new_text.is_some() {
                range.start <= range.end
            } else {
                range.start < range.end
            }
        });

        let cur_range = match old_ranges.next() {
            None => return Ok(Vec::new()),
            Some(range) => range,
        };
        let mut ops = Vec::with_capacity(old_ranges.size_hint().0);

        let old_fragments = self.fragments.clone();
        let mut cursor = old_fragments.cursor::<CharOffset, CharOffset>();
        let mut new_fragments = SumTree::new();

        // Push all parts of the existing fragments SumTree that occur _before_ the start of the
        // current range since these fragments are unaffected by the current edit.
        new_fragments.push_tree(cursor.slice(&cur_range.start, SeekBias::Right));

        // Since ranges is non-empty, we know we're going to produce at least one edit.
        // Note that the version vector that we use for the edit payload _does_ not include
        // this edit, while the lamport clock _does_ reflect this edit. This is also true
        // for other edit ops we might end up producing as part of this function.
        let mut version = self.tick();

        let mut operation_edit_args = OperationEditArgs::new(
            new_text.clone(),
            self.lamport_clock.clone(),
            version.clone(),
        );

        let mut cur_range = Some(cur_range);

        // We loop through all fragments that are either fully within the current range
        // or intersect with the current range. Note that we're using a 2 pointers approach
        // with the current range and current fragment (we move our current fragment until
        // we're outside of the current range and then we move onto the next range).
        while let Some(mut current_fragment) = cur_range
            .is_some()
            .then(|| cursor.item().cloned())
            .flatten()
        {
            // Number of characters the start of the fragment is offset from the beginning of the
            // buffer.
            let mut chars_to_fragment_start = *cursor.start();

            // Number of characters the end of the fragment is offset from the start of the
            // buffer.
            let mut chars_to_fragment_end =
                chars_to_fragment_start + current_fragment.visible_len(&self.undo_history);

            // For both cases (fully contained fragment or intersecting), we delete the original fragment insertion.
            // In the case of intersecting fragments, we will split this fragment into 2 parts.
            // In the case of fully contained fragments, we will entirely be replacing the fragment with our new range.
            let old_split_tree = self
                .insertion_splits
                .remove(&current_fragment.insertion.id)
                .ok_or_else(|| anyhow!("Current fragment does not exist in insertion_splits"))?;

            let mut splits_cursor = old_split_tree.cursor::<CharOffset, ()>();
            // Initialize a new SumTree for the current fragment which we are splitting.
            let mut new_split_tree =
                splits_cursor.slice(&current_fragment.start_character_offset(), SeekBias::Right);

            // Find all splices that start or end within the current fragment. Then, split the
            // fragment and reassemble it in both trees accounting for the deleted and the newly
            // inserted text.
            while let Some(range) = cur_range
                .as_ref()
                .filter(|range| range.start < chars_to_fragment_end)
            {
                Self::maybe_splice_fragment(
                    range,
                    &mut current_fragment,
                    &mut new_split_tree,
                    &mut new_fragments,
                    &mut chars_to_fragment_start,
                )?;

                // If the end of the current range is equal to the start of the current fragment, our
                // edit operation's end should be the end of the last fragment.
                // Otherwise, if the end of the current range is equal to the end of the current fragment,
                // our edit operation's end should be the end of the current fragment.
                if range.end == chars_to_fragment_start {
                    let last_fragment = new_fragments.last().ok_or_else(|| {
                        anyhow!("New fragment tree should have at least one item")
                    })?;
                    operation_edit_args.update_end(
                        last_fragment.insertion.id.clone(),
                        last_fragment.end_character_offset(),
                    );
                } else if range.end == chars_to_fragment_end {
                    operation_edit_args.update_end(
                        current_fragment.insertion.id.clone(),
                        current_fragment.end_character_offset(),
                    );
                }

                // The range is at a fragment boundary. Insert the text as a new fragment before the
                // current fragment.
                if range.start == chars_to_fragment_start {
                    let last_fragment = new_fragments.last().ok_or_else(|| {
                        anyhow!("New fragment tree should have at least one item")
                    })?;
                    // The start point for our edit operation is the end of the last fragment.
                    operation_edit_args.update_start(
                        last_fragment.insertion.id.clone(),
                        last_fragment.end_character_offset(),
                    );

                    if let Some(mut new_text) = new_text.clone() {
                        // If the new fragment is going to be inserted at a fragment boundary, then look
                        // at the left most visible or recently deleted fragment. Otherwise, we want to inherit
                        // the style from the current fragment since we are splitting it.
                        new_text.fallback_text_style_with(|| {
                            // Note this means chars_to_fragment_start == range.start == range.end
                            // i.e. this range is a single insertion point. Hence, we try to inherit the style (left/recently deleted).
                            // Example: "foobar" where "foo" is highlighted. The user starts typing right after "foo" - we would expect to retain the
                            // highlight. In that example, the range is 3..3
                            if chars_to_fragment_start == range.end {
                                self.style_from_left_visible_or_recently_deleted_fragment(
                                    &mut new_fragments,
                                )
                            } else {
                                // Otherwise, in this case, we're splitting the current fragment so adopt it's style.
                                // Example: "foobar" where "bar" is highlighted. The user then selects "b" and pastes in "xyz".
                                // At this point, we expect "fooxyzar" where "xyzar" is highlighted.
                                current_fragment
                                    .text
                                    .text_style
                                    .map(TextStyle::filter_inheritable_styles)
                            }
                        });
                        let new_fragment = self.build_fragment_to_insert(
                            new_fragments.last().ok_or_else(|| {
                                anyhow!("New fragment tree should have at least one item")
                            })?,
                            Some(&current_fragment),
                            new_text,
                            self.lamport_clock.clone(),
                        );
                        new_fragments.push(new_fragment);
                    }
                }

                // Check if the current fragment can be fully deleted or if it has to be spliced.
                if let Some(spliced_fragment) = self.maybe_mark_fragment_as_deleted(
                    &mut current_fragment,
                    chars_to_fragment_start,
                    chars_to_fragment_end,
                    new_fragments.last().ok_or_else(|| {
                        anyhow!("New fragment tree should have at least one item")
                    })?,
                    range,
                    &mut operation_edit_args,
                ) {
                    // If we had to delete a spliced fragment, we update our current position
                    // (note that this will actually be range.end).
                    chars_to_fragment_start = spliced_fragment.chars_to_fragment_start;
                    new_split_tree.push(spliced_fragment.insertion_split);
                    new_fragments.push(spliced_fragment.fragment)
                }

                // If the current range ends inside this fragment, we can advance to the next range
                // and check if it also intersects the current fragment. Otherwise we break out of
                // the loop and find the first fragment that the range does not contain fully.
                if range.end <= chars_to_fragment_end {
                    ops.push(operation_edit_args.try_into()?);
                    // We move onto the next range.
                    cur_range = old_ranges.next();
                    // For more precise timestamps on edits (since this loop will take some
                    // time to execute).
                    if cur_range.is_some() {
                        version = self.tick();
                    }
                    // We initialize our new edit operation for the next range.
                    operation_edit_args = OperationEditArgs::new(
                        new_text.clone(),
                        self.lamport_clock.clone(),
                        version.clone(),
                    );
                } else {
                    break;
                }
            }

            // We're done with the current fragment - save relevant data to structs.
            new_split_tree.push(InsertionSplit {
                extent: current_fragment.end_character_offset()
                    - current_fragment.start_character_offset(),
                fragment_id: current_fragment.id.clone(),
            });
            splits_cursor.next(); // Move onto the next insertion split.
            new_split_tree.push_tree(
                splits_cursor.slice(&old_split_tree.extent::<CharOffset>(), SeekBias::Right),
            );
            self.insertion_splits
                .insert(current_fragment.insertion.id.clone(), new_split_tree);
            new_fragments.push(current_fragment);

            // Continue scanning forward until we find a fragment that is not fully contained by the
            // current splice. Mark each intermediate fragment as deleted.
            cursor.next();
            if let Some(range) = cur_range.clone() {
                let range_ends_at_fragment_boundary = self.mark_fragments_as_deleted(
                    &mut cursor,
                    &mut new_fragments,
                    &mut operation_edit_args,
                    &mut chars_to_fragment_start,
                    &mut chars_to_fragment_end,
                    range,
                );

                // If the range ends at a fragment boundary, we've marked all necessary fragments as
                // deleted and we can continue updating the fragments for the next range. If the
                // range does NOT end at a fragment boundary, we continue the current while loop so
                // the last fragment can be spliced appropriately.
                if range_ends_at_fragment_boundary {
                    ops.push(operation_edit_args.try_into()?);

                    // Move onto the next range + start a new edit operation.
                    cur_range = old_ranges.next();
                    if cur_range.is_some() {
                        version = self.tick();
                    }

                    operation_edit_args = OperationEditArgs::new(
                        new_text.clone(),
                        self.lamport_clock.clone(),
                        version.clone(),
                    );
                }

                // If the range we are currently evaluating starts after the end of the fragment
                // that the cursor is parked at, we should seek to the next splice's start range
                // and push all the fragments in between into the new tree (they are untouched).
                if let Some(cur_range) = cur_range.as_ref() {
                    if cur_range.start > chars_to_fragment_end {
                        new_fragments.push_tree(cursor.slice(&cur_range.start, SeekBias::Right));
                    }
                }
            }
        }

        // Handle range that is at the end of the buffer if it exists. There should never be
        // multiple because ranges must be disjoint.
        // Example: we have a buffer of "foo" and cur_range is 3..3 - we need to add
        // a last fragment to the end.
        if cur_range.is_some() {
            debug_assert_eq!(old_ranges.next(), None);
            let last_fragment = new_fragments
                .last()
                .ok_or_else(|| anyhow!("New fragment tree should have at least one item"))?;
            operation_edit_args = OperationEditArgs::new(
                new_text.clone(),
                self.lamport_clock.clone(),
                version.clone(),
            );
            // Our edit operation's offsets are equal (single point edit).
            operation_edit_args.update_start(
                last_fragment.insertion.id.clone(),
                last_fragment.end_character_offset(),
            );
            operation_edit_args.update_end(
                last_fragment.insertion.id.clone(),
                last_fragment.end_character_offset(),
            );
            ops.push(operation_edit_args.try_into()?);

            if let Some(mut new_text) = new_text {
                new_text.fallback_text_style_with(|| {
                    // In this case, there's only the one possibility since we're at the end of the
                    // buffer - hence we inherit styles from left/recently deleted (no current
                    // fragment that we're splitting).
                    self.style_from_left_visible_or_recently_deleted_fragment(&mut new_fragments)
                });
                let last_fragment = new_fragments
                    .last()
                    .ok_or_else(|| anyhow!("New fragment tree should have at least one item"))?;
                let new_fragment = self.build_fragment_to_insert(
                    last_fragment,
                    None,
                    new_text,
                    self.lamport_clock.clone(),
                );
                new_fragments.push(new_fragment);
            }
        } else {
            // Capture all remaining untouched fragments and add them into our new SumTree.
            new_fragments
                .push_tree(cursor.slice(&old_fragments.extent::<CharOffset>(), SeekBias::Right));
        }

        self.fragments = new_fragments;
        Ok(ops)
    }

    /// Updates the desired ranges by applying `text_style_operation`, splicing fragments as necessary.
    ///
    /// This function is similar to `update_fragments_for_edit` above, except it focuses on
    /// updating the styles for these ranges in-place and does not produce Edit Operations.
    ///
    /// At a high level, this function builds up a new set of fragments that make up the model after the edit.
    /// All fragments before the first item in the range or after any of the ranges are considered to be untouched.
    /// If a range starts or ends within a fragment, that fragment is sliced into 2 fragments (one for the
    /// piece up to the range that is to be styled), and one for the piece of the fragment
    /// that is not covered by the range. Any fragments that are fully contained within a range get their
    /// styles updated appropriately.
    ///
    /// Note it is up to the caller to ensure that `old_ranges` is sorted by the start of the range.
    fn update_fragment_styles<I>(
        &mut self,
        mut old_ranges: I,
        text_style_operation: TextStyleOperation,
    ) -> Result<()>
    where
        I: Iterator<Item = Range<CharOffset>>,
    {
        let cur_range = match old_ranges.next() {
            None => return Ok(()),
            Some(range) => range,
        };

        let old_fragments = self.fragments.clone();

        let mut cursor = old_fragments.cursor::<CharOffset, CharOffset>();
        let mut new_fragments = SumTree::new();

        // Push all parts of the existing fragments SumTree that occur _before_ the start of the
        // current range since these fragments are unaffected by the current edit.
        new_fragments.push_tree(cursor.slice(&cur_range.start, SeekBias::Right));

        let mut cur_range = Some(cur_range);

        // We loop through all fragments that are either fully within the current range
        // or intersect with the current range.
        while let Some(mut current_fragment) = cur_range
            .is_some()
            .then(|| cursor.item().cloned())
            .flatten()
        {
            // Number of characters the start of the current fragment is offset from the beginning of the
            // buffer.
            let mut chars_to_fragment_start = *cursor.start();

            // Number of characters the end of the current fragment is offset from the start of the
            // buffer.
            let mut chars_to_fragment_end =
                chars_to_fragment_start + current_fragment.visible_len(&self.undo_history);

            let old_split_tree = self
                .insertion_splits
                .remove(&current_fragment.insertion.id)
                .ok_or_else(|| anyhow!("Current fragment does not exist in insertion_splits"))?;

            let mut splits_cursor = old_split_tree.cursor::<CharOffset, ()>();
            let mut new_split_tree =
                splits_cursor.slice(&current_fragment.start_character_offset(), SeekBias::Right);

            // Find all splices that start or end within the current fragment. Then, split the
            // fragment and reassemble it in both trees accounting for the styled and unstyled text.
            while let Some(range) = cur_range
                .as_ref()
                .filter(|range| range.start < chars_to_fragment_end)
            {
                Self::maybe_splice_fragment(
                    range,
                    &mut current_fragment,
                    &mut new_split_tree,
                    &mut new_fragments,
                    &mut chars_to_fragment_start,
                )?;

                // Covers the case of range covering the entire current fragment or partial (range ends inside fragment)
                // At this point, we know that chars_to_fragment_start is the start of the range and start of the current fragment
                // (potentially spliced from before) - we style the relevant portion of the fragment.
                if let Ok(Some(spliced_fragment)) = self.maybe_style_fragment(
                    &mut current_fragment,
                    &mut new_fragments,
                    chars_to_fragment_start,
                    chars_to_fragment_end,
                    range,
                    text_style_operation,
                ) {
                    new_split_tree.push(spliced_fragment.insertion_split);
                    new_fragments.push(spliced_fragment.fragment);
                    // If spliced, we move our start pointer over
                    chars_to_fragment_start = spliced_fragment.chars_to_fragment_start;
                }

                // If the current range ends inside this fragment, we can advance to the next range
                // and check if it also intersects the current fragment. Otherwise we break out of
                // the loop and find the first fragment that the range does not contain fully.
                if range.end <= chars_to_fragment_end {
                    cur_range = old_ranges.next();
                } else {
                    break;
                }
            }

            new_split_tree.push(InsertionSplit {
                extent: current_fragment.end_character_offset()
                    - current_fragment.start_character_offset(),
                fragment_id: current_fragment.id.clone(),
            });
            splits_cursor.next();
            new_split_tree.push_tree(
                splits_cursor.slice(&old_split_tree.extent::<CharOffset>(), SeekBias::Right),
            );
            self.insertion_splits
                .insert(current_fragment.insertion.id.clone(), new_split_tree);
            // We add in the current fragment (along with insertion split above).
            new_fragments.push(current_fragment);

            // Continue scanning forward until we find a fragment that is not fully contained by the
            // current splice. We style each intermediate fragment (with `text_style_operation`).
            cursor.next();
            if let Some(range) = cur_range.clone() {
                let range_ends_at_fragment_boundary = self.style_intermediate_fragments(
                    &mut cursor,
                    &mut new_fragments,
                    &mut chars_to_fragment_start,
                    &mut chars_to_fragment_end,
                    range,
                    text_style_operation,
                );

                // If the range ends at a fragment boundary, we've marked all necessary fragments as
                // deleted and we can continue updating the fragments for the next range. If the
                // range does NOT end at a fragment boundary, we continue the current while loop so
                // the last fragment can be spliced appropriately.
                if range_ends_at_fragment_boundary {
                    cur_range = old_ranges.next();
                }

                // If the range we are currently evaluating starts after the end of the fragment
                // that the cursor is parked at, we should seek to the next splice's start range
                // and push all the fragments in between into the new tree.
                if let Some(cur_range) = cur_range.as_ref() {
                    if cur_range.start > chars_to_fragment_end {
                        new_fragments.push_tree(cursor.slice(&cur_range.start, SeekBias::Right));
                    }
                }
            }
        }

        // Add remaining fragments (note range at end of buffer is no-op for update styles).
        new_fragments
            .push_tree(cursor.slice(&old_fragments.extent::<CharOffset>(), SeekBias::Right));

        self.fragments = new_fragments;
        Ok(())
    }

    fn maybe_splice_fragment(
        range: &Range<CharOffset>,
        fragment: &mut Fragment,
        new_split_tree: &mut SumTree<InsertionSplit>,
        new_fragments: &mut SumTree<Fragment>,
        chars_to_fragment_start: &mut CharOffset,
    ) -> Result<()> {
        // The range intersects a fragment -- this means we need to splice the fragment at
        // the correct offset. For example a fragment with the text "foo" starting at 0
        // would be spliced into "fo" if the start of the range was 2.
        if range.start > *chars_to_fragment_start {
            // Note that the current_fragment is overwritten to be the latter part of the splice
            // whereas the spliced_fragment contains the earlier part of the splice.
            // The spliced fragment should not be styled.
            let spliced_fragment = Self::splice_fragment_at_char_offset(
                fragment,
                new_fragments
                    .last()
                    .ok_or_else(|| anyhow!("New fragment tree should have at least one item"))?,
                *chars_to_fragment_start,
                range.start,
            );
            *chars_to_fragment_start = spliced_fragment.chars_to_fragment_start;
            new_split_tree.push(spliced_fragment.insertion_split);
            new_fragments.push(spliced_fragment.fragment);
            return Ok(());
        }
        Ok(())
    }

    /// Marks `fragment` as deleted if the `range` exceeds past the Fragment. If the range
    /// ends within the fragment, the fragment is spliced and only the part that overlaps is marked
    /// as deleted.
    fn maybe_mark_fragment_as_deleted(
        &self,
        fragment: &mut Fragment,
        chars_to_fragment_start: CharOffset,
        chars_to_fragment_end: CharOffset,
        last_fragment: &Fragment,
        range: &Range<CharOffset>,
        operation_edit_args: &mut OperationEditArgs,
    ) -> Option<SplicedFragment> {
        if range.end < chars_to_fragment_end {
            if range.end > chars_to_fragment_start {
                // The range ends somewhere within the current fragment. Slice out the part
                // of the fragment before the end of the range as deleted.

                let mut spliced_fragment = Self::splice_fragment_at_char_offset(
                    fragment,
                    last_fragment,
                    chars_to_fragment_start,
                    range.end,
                );

                // Mark the spliced fragment as deleted.
                spliced_fragment
                    .fragment
                    .mark_deletion(self.lamport_clock.clone(), &self.undo_history);
                operation_edit_args.update_end(
                    fragment.insertion.id.clone(),
                    fragment.start_character_offset(),
                );
                operation_edit_args
                    .version_in_range
                    .observe(&fragment.insertion.id);

                return Some(spliced_fragment);
            }
            // If range.end < chars_to_fragment_start, we shouldn't be trying to delete/splice anyhow (fragment is untouched).
            // If range.end == chars_to_fragment_start, we apply edit operations with the last fragment's end character (see main body of caller function).
        } else {
            // The range ends beyond the current fragment, this means we can mark the entire
            // fragment as deleted and move on to find the fragment where the range ends.
            operation_edit_args
                .version_in_range
                .observe(&fragment.insertion.id);
            fragment.mark_deletion(self.lamport_clock.clone(), &self.undo_history);
        }

        None
    }

    /// Styles `fragment` entirely if the `range` exceeds past the Fragment. If the range
    /// ends within the fragment, the fragment is spliced and only the part that overlaps is
    /// styled.
    ///
    /// We have the following possibilities for return values (of type Result<Option<SplicedFragment>>):
    /// 1. Some(SplicedFragment) - the range splices the current fragment
    ///    i.e. the end of the range falls within the current fragment. We return
    ///    the new spliced fragment which is the earlier part (we set the appropriate
    ///    new text style).
    /// 2. The function returns an error - this only happens if the range splices
    ///    the current fragment and new_fragments has 0 items, which should never occur
    ///    (in a practical sense).
    /// 3. None - the range entirely covers the current fragment meaning we can
    ///    style the current fragment in-place (note that the caller already
    ///    has a reference to the current fragment hence we don't return it).
    fn maybe_style_fragment(
        &mut self,
        fragment: &mut Fragment,
        new_fragments: &mut SumTree<Fragment>,
        chars_to_fragment_start: CharOffset,
        chars_to_fragment_end: CharOffset,
        range: &Range<CharOffset>,
        text_style_operation: TextStyleOperation,
    ) -> Result<Option<SplicedFragment>> {
        if range.end < chars_to_fragment_end {
            if range.end > chars_to_fragment_start {
                // The range ends somewhere within the current fragment. Slice out the part
                // of the fragment before the end of the range and style it appropriately.
                let mut spliced_fragment = Self::splice_fragment_at_char_offset(
                    fragment,
                    new_fragments.last().ok_or_else(|| {
                        anyhow!("New fragment tree should have at least one item")
                    })?,
                    chars_to_fragment_start,
                    range.end,
                );
                spliced_fragment.fragment.set_text_style(
                    TextStyleOperation::apply_text_style_operation(
                        spliced_fragment
                            .fragment
                            .text
                            .text_style
                            .unwrap_or_default(),
                        text_style_operation,
                    ),
                );
                return Ok(Some(spliced_fragment));
            }
        } else {
            // Mutate the current fragment to update the styles for the text.
            let cur_text = fragment.text.clone();
            fragment.set_text_style(TextStyleOperation::apply_text_style_operation(
                cur_text.text_style.unwrap_or_default(),
                text_style_operation,
            ));
        }
        Ok(None)
    }

    /// Mark all fragments as deleted that are completely covered by the current `range`. Returns
    /// whether the range ends exactly at a fragment boundary.
    fn mark_fragments_as_deleted(
        &mut self,
        cursor: &mut Cursor<Fragment, CharOffset, CharOffset>,
        new_fragments: &mut SumTree<Fragment>,
        operation_edit_args: &mut OperationEditArgs,
        fragment_start: &mut CharOffset,
        fragment_end: &mut CharOffset,
        range: Range<CharOffset>,
    ) -> bool {
        while let Some(fragment) = cursor.item() {
            *fragment_start = *cursor.start();
            *fragment_end = *fragment_start + fragment.visible_len(&self.undo_history);

            // Check if range fully contains fragment.
            if range.start < *fragment_start && range.end >= *fragment_end {
                let mut new_fragment = fragment.clone();
                new_fragment.mark_deletion(
                    operation_edit_args.lamport_timestamp.clone(),
                    &self.undo_history,
                );
                // Mark down the timestamp in our global map (used for operations such as
                // undos later).
                operation_edit_args
                    .version_in_range
                    .observe(&new_fragment.insertion.id);
                // We still want to keep track of the fully deleted fragment in our
                // new SumTree.
                new_fragments.push(new_fragment);
                // Move onto the next fragment.
                cursor.next();
                // If the range's end lines up with the current fragment's end,
                // we're done looping and can update our edit operation.
                if range.end == *fragment_end {
                    operation_edit_args.update_end(
                        fragment.insertion.id.clone(),
                        fragment.end_character_offset(),
                    );
                    // Indicates we're at a fragment boundary.
                    return true;
                }
            } else {
                // Indicates we're not at a fragment boundary.
                return false;
            }
        }
        false
    }

    /// Style all fragments that are completely covered by the current `range`. Returns
    /// whether the range ends exactly at a fragment boundary.
    fn style_intermediate_fragments(
        &mut self,
        cursor: &mut Cursor<Fragment, CharOffset, CharOffset>,
        new_fragments: &mut SumTree<Fragment>,
        fragment_start: &mut CharOffset,
        fragment_end: &mut CharOffset,
        range: Range<CharOffset>,
        text_style_operation: TextStyleOperation,
    ) -> bool {
        while let Some(fragment) = cursor.item() {
            *fragment_start = *cursor.start();
            *fragment_end = *fragment_start + fragment.visible_len(&self.undo_history);

            if range.start < *fragment_start && range.end >= *fragment_end {
                let cur_text = fragment.text.clone();

                let mut new_fragment = fragment.clone();
                new_fragment.set_text_style(TextStyleOperation::apply_text_style_operation(
                    cur_text.text_style.unwrap_or_default(),
                    text_style_operation,
                ));
                new_fragments.push(new_fragment.clone());
                cursor.next();

                if range.end == *fragment_end {
                    return true;
                }
            } else {
                return false;
            }
        }
        false
    }

    /// Searches through all the fragments on the buffer end, where the current
    /// fragment is to be inserted. If the fragments immediately to the on the buffer
    /// are deleted, this function returns the most recently deleted of this string
    /// of deleted fragments.
    fn last_deleted_fragment_at_buffer_end<'a>(
        &self,
        new_fragments: &'a SumTree<Fragment>,
    ) -> Option<&'a Fragment> {
        let summary = new_fragments.summary().text_summary.chars;

        let mut cursor = new_fragments.cursor::<CharOffset, CharOffset>();
        cursor.seek(&summary, SeekBias::Left);

        if cursor
            .item()
            .is_some_and(|f| f.is_visible(&self.undo_history))
        {
            cursor.next();
        }
        let mut last_deleted_fragment_timestamp = None;
        let mut last_deleted_fragment = None;
        // Iterate rightwards to find the most recently deleted fragment
        for item in cursor {
            if item.is_visible(&self.undo_history) {
                break;
            }
            let max_timestamp = item.deletions.iter().max();
            if max_timestamp > last_deleted_fragment_timestamp {
                last_deleted_fragment_timestamp = max_timestamp;
                last_deleted_fragment = Some(item);
            }
        }

        last_deleted_fragment
    }

    /// Searches left of where the new fragment will be inserted in order to find
    /// the most immediate left visible fragment. Will return None if none exist.
    fn previous_visible_fragment_style(
        &self,
        new_fragments: &SumTree<Fragment>,
    ) -> Option<TextStyle> {
        let summary = new_fragments.summary().text_summary.chars;
        let mut cursor = new_fragments.cursor::<CharOffset, CharOffset>();

        cursor.seek(&summary, SeekBias::Left);
        while cursor
            .item()
            .is_some_and(|fragment| !fragment.is_visible(&self.undo_history))
        {
            cursor.prev();
        }

        cursor.item().and_then(|fragment| {
            fragment
                .text
                .text_style
                .map(TextStyle::filter_inheritable_styles)
        })
    }

    /// Gets the text style from either the most recently deleted fragment to
    /// the right of the new fragment in the sum tree or, that fragment doesn't
    /// exist or it has already been inherited from, gets the style from the
    /// left visible fragment.
    fn style_from_left_visible_or_recently_deleted_fragment(
        &self,
        new_fragments: &mut SumTree<Fragment>,
    ) -> Option<TextStyle> {
        let last_deleted_fragment = self
            .last_deleted_fragment_at_buffer_end(new_fragments)
            .filter(|fragment| !fragment.text_style_inherited_after_deletion);

        match last_deleted_fragment {
            None => self.previous_visible_fragment_style(new_fragments),
            Some(last_deleted_fragment) => {
                let (new_fragments_temp, text_style) = {
                    let mut new_last_deleted_fragment = last_deleted_fragment.clone();

                    // Slice up to the new_last_deleted_fragment
                    let mut cursor = new_fragments.cursor::<FragmentIdRef, CharOffset>();
                    let mut new_fragments_temp = cursor.slice(
                        &FragmentIdRef::new(&last_deleted_fragment.id),
                        SeekBias::Left,
                    );

                    let inherited_text_style = new_last_deleted_fragment
                        .text
                        .text_style
                        .map(TextStyle::filter_inheritable_styles);

                    // Mark the new_last_deleted_fragment as already inherited and add it to
                    // the SumTree
                    new_last_deleted_fragment.text_style_inherited_after_deletion = true;
                    new_fragments_temp.push(new_last_deleted_fragment);

                    // Move the cursor past the new_last_deleted_fragment then add the rest of the tree
                    // back after the new_fragments_temp
                    cursor.next();
                    new_fragments_temp.push_tree(
                        cursor.slice(&new_fragments.extent::<FragmentIdRef>(), SeekBias::Right),
                    );
                    (new_fragments_temp, inherited_text_style)
                };
                *new_fragments = new_fragments_temp;
                text_style
            }
        }
    }

    fn split_fragment(
        &mut self,
        prev_fragment: &Fragment,
        fragment: &Fragment,
        range: Range<CharOffset>,
    ) -> (Option<Fragment>, Option<Fragment>, Option<Fragment>) {
        debug_assert!(range.start >= fragment.start_character_offset());
        debug_assert!(range.start <= fragment.end_character_offset());
        debug_assert!(range.end <= fragment.end_character_offset());
        debug_assert!(range.end >= fragment.start_character_offset());

        if range.end == fragment.start_character_offset() {
            (None, None, Some(fragment.clone()))
        } else if range.start == fragment.end_character_offset() {
            (Some(fragment.clone()), None, None)
        } else if range.start == fragment.start_character_offset()
            && range.end == fragment.end_character_offset()
        {
            (None, Some(fragment.clone()), None)
        } else {
            let mut prefix = fragment.clone();

            let after_range = if range.end < fragment.end_character_offset() {
                let mut suffix = prefix.clone();
                suffix.set_start_offset(range.end);
                prefix.set_end_character_offset(range.end);
                prefix.id = FragmentId::between(&prev_fragment.id, &suffix.id);
                Some(suffix)
            } else {
                None
            };

            let within_range = if range.start != range.end {
                let mut suffix = prefix.clone();
                suffix.set_start_offset(range.start);
                prefix.set_end_character_offset(range.start);
                prefix.id = FragmentId::between(&prev_fragment.id, &suffix.id);
                Some(suffix)
            } else {
                None
            };

            let before_range = if range.start > fragment.start_character_offset() {
                Some(prefix)
            } else {
                None
            };

            let old_split_tree = self
                .insertion_splits
                .remove(&fragment.insertion.id)
                .unwrap();
            let mut cursor = old_split_tree.cursor::<CharOffset, ()>();
            let mut new_split_tree =
                cursor.slice(&fragment.start_character_offset(), SeekBias::Right);

            if let Some(ref fragment) = before_range {
                new_split_tree.push(InsertionSplit {
                    extent: range.start - fragment.start_character_offset(),
                    fragment_id: fragment.id.clone(),
                });
            }

            if let Some(ref fragment) = within_range {
                new_split_tree.push(InsertionSplit {
                    extent: range.end - range.start,
                    fragment_id: fragment.id.clone(),
                });
            }

            if let Some(ref fragment) = after_range {
                new_split_tree.push(InsertionSplit {
                    extent: fragment.end_character_offset() - range.end,
                    fragment_id: fragment.id.clone(),
                });
            }

            cursor.next();
            new_split_tree
                .push_tree(cursor.slice(&old_split_tree.extent::<CharOffset>(), SeekBias::Right));

            self.insertion_splits
                .insert(fragment.insertion.id.clone(), new_split_tree);

            (before_range, within_range, after_range)
        }
    }

    fn build_fragment_to_insert(
        &mut self,
        prev_fragment: &Fragment,
        next_fragment: Option<&Fragment>,
        text: Text,
        lamport_timestamp: Lamport,
    ) -> Fragment {
        let next_fragment_id = match next_fragment {
            Some(f) => &f.id,
            None => FragmentId::max_value(),
        };
        let new_fragment_id = FragmentId::between(&prev_fragment.id, next_fragment_id);

        let mut split_tree = SumTree::new();
        split_tree.push(InsertionSplit {
            extent: text.len(),
            fragment_id: new_fragment_id.clone(),
        });
        self.insertion_splits
            .insert(lamport_timestamp.clone(), split_tree);

        Fragment::new(
            new_fragment_id,
            Insertion {
                id: lamport_timestamp,
                parent_id: prev_fragment.insertion.id.clone(),
                offset_in_parent: prev_fragment.end_character_offset(),
                text,
            },
        )
    }

    pub fn anchor_before<T: ToCharOffset>(&self, position: T) -> Result<Anchor> {
        self.anchor_at(position, AnchorBias::Left)
    }

    pub fn anchor_after<T: ToCharOffset>(&self, position: T) -> Result<Anchor> {
        self.anchor_at(position, AnchorBias::Right)
    }

    pub fn anchor_at<T: ToCharOffset>(&self, position: T, bias: AnchorBias) -> Result<Anchor> {
        let offset = position.to_char_offset(self)?;
        let max_offset = self.len();
        if offset > max_offset {
            return Err(anyhow!("offset is out of range"));
        }

        let seek_bias;
        match bias {
            AnchorBias::Left => {
                if offset == 0.into() {
                    return Ok(Anchor::Start);
                } else {
                    seek_bias = SeekBias::Left;
                }
            }
            AnchorBias::Right => {
                if offset == max_offset {
                    return Ok(Anchor::End);
                } else {
                    seek_bias = SeekBias::Right;
                }
            }
        };

        let mut cursor = self.fragments.cursor::<CharOffset, CharOffset>();
        cursor.seek(&offset, seek_bias);
        let fragment = cursor.item().unwrap();
        let offset_in_fragment = offset - *cursor.start();
        let offset_in_insertion = fragment.start_character_offset() + offset_in_fragment;
        let anchor = Anchor::Middle {
            insertion_id: fragment.insertion.id.clone(),
            offset: offset_in_insertion,
            bias,
        };
        Ok(anchor)
    }

    fn fragment_id_for_anchor(&self, anchor: &Anchor) -> Result<&FragmentId> {
        match anchor {
            Anchor::Start => Ok(FragmentId::max_value()),
            Anchor::End => Ok(FragmentId::min_value()),
            Anchor::Middle {
                insertion_id,
                offset,
                bias,
                ..
            } => {
                let seek_bias = match bias {
                    AnchorBias::Left => SeekBias::Left,
                    AnchorBias::Right => SeekBias::Right,
                };

                let splits = self
                    .insertion_splits
                    .get(insertion_id)
                    .ok_or_else(|| anyhow!("split does not exist for insertion id"))?;
                let mut splits_cursor = splits.cursor::<CharOffset, ()>();
                splits_cursor.seek(offset, seek_bias);
                splits_cursor
                    .item()
                    .ok_or_else(|| anyhow!("split offset is out of range"))
                    .map(|split| &split.fragment_id)
            }
        }
    }

    fn summary_for_anchor(&self, anchor: &Anchor) -> Result<TextSummary> {
        match anchor {
            Anchor::Start => Ok(TextSummary::default()),
            Anchor::End => Ok(self.fragments.summary().text_summary),
            Anchor::Middle {
                insertion_id,
                offset,
                bias,
            } => {
                let seek_bias = match bias {
                    AnchorBias::Left => SeekBias::Left,
                    AnchorBias::Right => SeekBias::Right,
                };

                let splits = self
                    .insertion_splits
                    .get(insertion_id)
                    .ok_or_else(|| anyhow!("split does not exist for insertion id"))?;
                let mut splits_cursor = splits.cursor::<CharOffset, ()>();
                splits_cursor.seek(offset, seek_bias);
                let split = splits_cursor
                    .item()
                    .ok_or_else(|| anyhow!("split offset is out of range"))?;

                let mut fragments_cursor = self.fragments.cursor::<FragmentIdRef, TextSummary>();
                fragments_cursor.seek(&FragmentIdRef::new(&split.fragment_id), SeekBias::Left);
                let fragment = fragments_cursor
                    .item()
                    .ok_or_else(|| anyhow!("fragment id does not exist"))?;

                let mut summary = fragments_cursor.start().clone();
                if fragment.is_visible(&self.undo_history) {
                    summary += fragment
                        .text
                        .slice(..*offset - fragment.start_character_offset())
                        .summary();
                }
                Ok(summary)
            }
        }
    }

    pub fn point_for_offset(&self, offset: impl ToCharOffset) -> Result<Point> {
        let offset = offset.to_char_offset(self)?;
        let mut fragments_cursor = self.fragments.cursor::<CharOffset, TextSummary>();
        fragments_cursor.seek(&offset, SeekBias::Left);
        fragments_cursor
            .item()
            .ok_or_else(|| anyhow!("offset is out of range"))
            .map(|fragment| {
                let overshoot = fragment.point_for_offset(offset - fragments_cursor.start().chars);
                fragments_cursor.start().lines + overshoot
            })
    }

    /// Merges local and remote selection sets.
    fn merge_all_selections(&mut self) -> Result<()> {
        self.merge_local_selections()?;
        self.merge_remote_selections()?;
        Ok(())
    }

    /// Merges local selections only.
    fn merge_local_selections(&mut self) -> Result<()> {
        let mut local_selections = self.local_selections.selections.clone();
        self.merge_selections(&mut local_selections)?;
        self.local_selections.selections = local_selections;
        Ok(())
    }

    /// Merges remote selections for all peers.
    fn merge_remote_selections(&mut self) -> Result<()> {
        let replica_ids = self.remote_selections.keys().cloned().collect_vec();
        for replica_id in replica_ids {
            self.merge_remote_selections_for_peer(replica_id)?;
        }
        Ok(())
    }

    /// Merges remote selections for the given peer.
    fn merge_remote_selections_for_peer(&mut self, replica_id: ReplicaId) -> Result<()> {
        let Some(mut new_selections) = self.remote_selections.remove(&replica_id) else {
            return Ok(());
        };
        self.merge_selections(&mut new_selections.selections)?;
        self.remote_selections.insert(replica_id, new_selections);
        Ok(())
    }

    /// Merges any overlapping selections in-place in the given `selections` set.
    fn merge_selections<T>(&self, selections: &mut Vec1<T>) -> Result<()>
    where
        T: AsSelection,
    {
        let mut i = 1;

        while i < selections.len() {
            if selections[i - 1]
                .as_selection()
                .end
                .to_char_offset(self)
                .unwrap()
                >= selections[i]
                    .as_selection()
                    .start
                    .to_char_offset(self)
                    .unwrap()
            {
                let removed = selections.remove(i).unwrap();

                if removed.as_selection().start.to_char_offset(self).unwrap()
                    < selections[i - 1]
                        .as_selection()
                        .start
                        .to_char_offset(self)
                        .unwrap()
                {
                    selections[i - 1].as_mut_selection().start =
                        removed.as_selection().start.clone();
                }

                if removed.as_selection().end.to_char_offset(self).unwrap()
                    > selections[i - 1]
                        .as_selection()
                        .end
                        .to_char_offset(self)
                        .unwrap()
                {
                    selections[i - 1].as_mut_selection().end = removed.as_selection().end.clone();
                }
            } else {
                i += 1;
            }
        }

        Ok(())
    }

    /// Records a local selection update and returns
    /// the corresponding [`Operation`] to notify peers of the update.
    fn record_selections_update(&mut self) -> Operation {
        // We only tick the lamport clock and do not observe it in the version vector
        // because edits are not dependent on selection updates.
        // We still need to tick the lamport clock to distinguish between
        // different events (e.g. different selection updates).
        self.lamport_clock.tick();

        let selections = self.local_selections.selections.clone().mapped(Into::into);
        Operation::UpdateSelections(UpdateSelectionsOperation {
            lamport_timestamp: self.lamport_clock.clone(),
            selections,
        })
    }

    /// This method gets a word "nearest" to a point, where "nearest" means the following:
    /// If the point lands on a word, return that word. If not, move the point forward until word
    /// characters are encountered, skipping over any whitespace or punctuation. Do not check for
    /// word chars behind the point. Do not move passed a newline to look for word chars on the
    /// next line. If, within those constraints, we cannot find word chars, return None. If these
    /// semantics sound strange, it's b/c this is how Vim's "*" and "#" commands work.
    pub fn get_word_nearest_to_point(&self, point: &Point) -> Option<String> {
        let line = self.line(point.row).ok()?;
        // Once we find word-characters, assign this to Some.
        let mut word_found_at: Option<CharOffset> = None;
        // This is the loop counter, which is incremented manually.
        let mut word_char_search_offset = CharOffset::from(point.column as usize);
        for c in line.chars_at(word_char_search_offset).ok()? {
            // If no words exist from this point to the end of the line, then there is nothing to
            // return.
            if c == '\n' {
                return None;
            }
            if !is_default_word_boundary(c) {
                word_found_at = Some(word_char_search_offset);
                break;
            }
            word_char_search_offset += 1;
        }

        let word_start = line
            .word_starts_backward_from_offset_inclusive(word_found_at?)
            .ok()?
            .next()?
            .column as usize;
        let word_end = line
            .word_ends_from_offset_exclusive(word_found_at?)
            .ok()?
            .next()?
            .column as usize;

        Some(
            line.chars()
                .skip(word_start)
                .take(word_end - word_start)
                .collect(),
        )
    }

    pub fn marked_text_state(&self) -> MarkedTextState {
        self.local_selections.marked_text_state()
    }

    pub fn set_marked_text_state(&mut self, marked_text_state: MarkedTextState) {
        self.local_selections
            .set_marked_text_state(marked_text_state);
    }
}

impl TextBuffer for Buffer {
    type Chars<'a>
        = Chars<'a>
    where
        Self: 'a;

    type CharsReverse<'a>
        = Chars<'a>
    where
        Self: 'a;

    fn chars_at(&self, offset: CharOffset) -> Result<Self::Chars<'_>> {
        Buffer::chars_at(self, offset)
    }

    fn chars_rev_at(&self, offset: CharOffset) -> Result<Self::CharsReverse<'_>> {
        Buffer::chars_at(self, offset).map(|chars| chars.rev())
    }

    fn to_point(&self, offset: CharOffset) -> Result<Point> {
        offset.to_point(self)
    }

    fn to_offset(&self, point: Point) -> Result<CharOffset> {
        <Point as ToCharOffset>::to_char_offset(&point, self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// A local edit was made.
    /// There may have also been selection changes.
    Edited {
        /// The edited ranges.
        edits: Vec<Edit>,
        /// The origin of the edit.
        edit_origin: EditOrigin,
    },

    /// Selection state was changed.
    /// If the selection change was associated to
    /// an edit, then we rely on the [`Self::Edited`]
    /// event.
    SelectionsChanged,

    /// There was a buffer change made locally
    /// and peers should be updated.
    UpdatePeers { operations: Rc<Vec<Operation>> },

    /// Styles were updated.
    StylesUpdated,
}

impl Entity for Buffer {
    type Event = Event;
}

impl sum_tree::Dimension<'_, FragmentSummary> for Point {
    fn add_summary(&mut self, summary: &FragmentSummary) {
        *self += summary.text_summary.lines;
    }
}

impl CharsWithStyle<'_> {
    pub fn rev(mut self) -> Self {
        if self.reversed {
            self.reversed = false;
            self.fragment_chars = self.fragments_cursor.item().map_or("".chars(), |fragment| {
                fragment.text[self.fragment_offset..].chars()
            });
        } else {
            self.reversed = true;
            self.fragment_chars = self.fragments_cursor.item().map_or("".chars(), |fragment| {
                fragment.text[..self.fragment_offset].chars()
            });
        }

        self
    }
}

impl Chars<'_> {
    pub fn rev(self) -> Self {
        Self(self.0.rev())
    }
}

impl Iterator for Chars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Into::into)
    }
}

impl Iterator for TextStyleRuns<'_> {
    type Item = TextRun;

    fn next(&mut self) -> Option<Self::Item> {
        let mut current_string = String::new();
        let mut current_text_style = None;

        while let Some(fragment) = self.fragments_cursor.item() {
            if fragment.is_visible(self.undo_history) && !fragment.text.as_str().is_empty() {
                match current_text_style {
                    None => {
                        current_text_style = Some(fragment.text.text_style().unwrap_or_default());
                        current_string.push_str(fragment.text.as_str());
                        self.byte_index_start = *self.fragments_cursor.start();
                        self.byte_index_end =
                            *self.fragments_cursor.start() + fragment.text.byte_len();
                    }
                    Some(text_style) => {
                        let fragment_text_style = fragment.text.text_style().unwrap_or_default();

                        // If the text style is the same as the last fragment, merge the strings
                        // together.
                        if text_style == fragment_text_style {
                            current_string.push_str(fragment.text.as_str());
                            self.byte_index_end += fragment.text.as_str().len();
                        } else {
                            return Some(TextRun::new(
                                current_string,
                                text_style,
                                self.byte_index_start..self.byte_index_end,
                            ));
                        }
                    }
                }
            }
            self.fragments_cursor.next();
        }

        current_text_style.map(|text_style| {
            TextRun::new(
                current_string,
                text_style,
                self.byte_index_start..self.byte_index_end,
            )
        })
    }
}

impl Iterator for CharsWithStyle<'_> {
    type Item = StylizedChar;

    fn next(&mut self) -> Option<Self::Item> {
        if self.reversed {
            if let Some(char) = self.fragment_chars.next_back() {
                self.fragment_offset -= 1;
                Some(StylizedChar::new(char, self.fragment_text_style))
            } else {
                loop {
                    self.fragments_cursor.prev();
                    if let Some(fragment) = self.fragments_cursor.item() {
                        if fragment.is_visible(self.undo_history) {
                            self.fragment_offset = fragment.len();
                            self.fragment_chars = fragment.text.as_str().chars();
                            self.fragment_text_style =
                                fragment.text.text_style().unwrap_or_default();
                            return self
                                .fragment_chars
                                .next_back()
                                .map(|char| StylizedChar::new(char, self.fragment_text_style));
                        }
                    } else {
                        return None;
                    }
                }
            }
        } else if let Some(char) = self.fragment_chars.next() {
            self.fragment_offset += 1;
            Some(StylizedChar::new(char, self.fragment_text_style))
        } else {
            loop {
                self.fragments_cursor.next();
                if let Some(fragment) = self.fragments_cursor.item() {
                    if fragment.is_visible(self.undo_history) {
                        self.fragment_chars = fragment.text.as_str().chars();
                        self.fragment_text_style = fragment.text.text_style().unwrap_or_default();
                        self.fragment_offset = 0.into();
                        return self
                            .fragment_chars
                            .next()
                            .map(|char| StylizedChar::new(char, self.fragment_text_style));
                    }
                } else {
                    return None;
                }
            }
        }
    }
}

impl<F: Fn(&FragmentSummary) -> bool> Iterator for Edits<'_, F> {
    type Item = Edit;

    fn next(&mut self) -> Option<Self::Item> {
        let mut change: Option<Edit> = None;

        while let Some(fragment) = self.cursor.item() {
            let new_offset = *self.cursor.start();
            let old_offset =
                CharOffset::from((new_offset.as_usize() as isize - self.delta) as usize);

            if !fragment.was_visible(&self.since, self.undo_history)
                && fragment.is_visible(self.undo_history)
            {
                if let Some(ref mut change) = change {
                    if change.new_range.end == new_offset {
                        change.new_range.end += fragment.len();
                        self.delta += fragment.len().as_usize() as isize;
                    } else {
                        break;
                    }
                } else {
                    change = Some(Edit {
                        old_range: old_offset..old_offset,
                        new_range: new_offset..new_offset + fragment.len(),
                    });
                    self.delta += fragment.len().as_usize() as isize;
                }
            } else if fragment.was_visible(&self.since, self.undo_history)
                && !fragment.is_visible(self.undo_history)
            {
                if let Some(ref mut change) = change {
                    if change.new_range.end == new_offset {
                        change.old_range.end += fragment.len();
                        self.delta -= fragment.len().as_usize() as isize;
                    } else {
                        break;
                    }
                } else {
                    change = Some(Edit {
                        old_range: old_offset..old_offset + fragment.len(),
                        new_range: new_offset..new_offset,
                    });
                    self.delta -= fragment.len().as_usize() as isize;
                }
            }

            self.cursor.next();
        }

        change
    }
}

#[derive(Ord, PartialOrd, Eq, PartialEq, Clone, Debug)]
struct FragmentId(Box<[u16]>);

lazy_static! {
    static ref FRAGMENT_ID_EMPTY: FragmentId = FragmentId(Box::new([]));
    static ref FRAGMENT_ID_MIN_VALUE: FragmentId = FragmentId(Box::new([0]));
    static ref FRAGMENT_ID_MAX_VALUE: FragmentId = FragmentId(Box::new([u16::MAX]));
}

impl Default for FragmentId {
    fn default() -> Self {
        FRAGMENT_ID_EMPTY.clone()
    }
}

impl FragmentId {
    fn min_value() -> &'static Self {
        &FRAGMENT_ID_MIN_VALUE
    }

    fn max_value() -> &'static Self {
        &FRAGMENT_ID_MAX_VALUE
    }

    fn between(left: &Self, right: &Self) -> Self {
        Self::between_with_max(left, right, u16::MAX)
    }

    fn between_with_max(left: &Self, right: &Self, max_value: u16) -> Self {
        let mut new_entries = Vec::new();

        let left_entries = left.0.iter().cloned().chain(iter::repeat(0));
        let right_entries = right.0.iter().cloned().chain(iter::repeat(max_value));
        for (l, r) in left_entries.zip(right_entries) {
            let interval = r - l;
            if interval > 1 {
                new_entries.push(l + (interval / 2).clamp(1, 8));
                break;
            } else {
                new_entries.push(l);
            }
        }

        FragmentId(new_entries.into_boxed_slice())
    }
}

#[derive(Ord, PartialOrd, Eq, PartialEq, Clone, Debug, Default)]
struct FragmentIdRef<'a>(Option<&'a FragmentId>);

impl<'a> FragmentIdRef<'a> {
    fn new(id: &'a FragmentId) -> Self {
        Self(Some(id))
    }
}

impl<'a> sum_tree::Dimension<'a, FragmentSummary> for FragmentIdRef<'a> {
    fn add_summary(&mut self, summary: &'a FragmentSummary) {
        self.0 = Some(&summary.max_fragment_id)
    }
}

impl Fragment {
    fn new(id: FragmentId, insertion: Insertion) -> Self {
        Self {
            id,
            text: insertion.text.clone(),
            insertion,
            deletions: BTreeSet::new(),
            text_style_inherited_after_deletion: false,
            is_visible: true,
            max_undo_ts: Global::new(),
        }
    }

    fn mark_deletion(&mut self, deletion: Lamport, undo_history: &UndoHistory) {
        self.deletions.insert(deletion);
        self.recompute_visibility(undo_history);
    }

    fn set_text_style(&mut self, text_style: TextStyle) {
        self.text.text_style = Some(text_style);
    }

    /// The number of characters within the underlying `Text` the fragment starts.
    fn start_character_offset(&self) -> CharOffset {
        self.text.range().start
    }

    fn set_start_offset(&mut self, offset: CharOffset) {
        self.text = self
            .insertion
            .text
            .slice(offset..self.end_character_offset())
            .with_text_style(self.text.text_style());
    }

    /// The number of characters within the underlying `Text` the fragment ends.
    fn end_character_offset(&self) -> CharOffset {
        self.text.range().end
    }

    fn set_end_character_offset(&mut self, offset: CharOffset) {
        self.text = self
            .insertion
            .text
            .slice(self.start_character_offset()..offset)
            .with_text_style(self.text.text_style());
    }

    fn visible_len(&self, undo_history: &UndoHistory) -> CharOffset {
        if self.is_visible(undo_history) {
            self.len()
        } else {
            0.into()
        }
    }

    fn len(&self) -> CharOffset {
        self.text.len()
    }

    fn recompute_visibility(&mut self, undo_history: &UndoHistory) {
        self.is_visible = self.is_visible(undo_history);
    }

    /// Returns true iff the fragment's edit should be incorporated
    /// into the final buffer.
    ///
    /// Prefer using the [`Self::is_visible`] function over
    /// the `is_visible` attribute to query the freshest state.
    fn is_visible(&self, undo_history: &UndoHistory) -> bool {
        !undo_history.is_edit_undone(&self.insertion.id)
            && self
                .deletions
                .iter()
                .all(|deletion| undo_history.is_edit_undone(deletion))
    }

    /// Returns true iff the fragment's edit should have been incorporated
    /// when the buffer's version vector was `version`.
    fn was_visible(&self, version: &Global, undo_history: &UndoHistory) -> bool {
        version.observed(&self.insertion.id)
            && !undo_history.was_edit_undone(&self.insertion.id, version)
            && self
                .deletions
                .iter()
                .all(|d| !version.observed(d) || undo_history.was_edit_undone(d, version))
    }

    fn point_for_offset(&self, offset: CharOffset) -> Point {
        self.text.point_for_offset(offset)
    }

    fn offset_for_point(&self, point: Point) -> CharOffset {
        self.text.offset_for_point(point)
    }

    fn byte_offset_for_point(&self, point: Point) -> ByteOffset {
        self.text.byte_offset_for_point(point)
    }

    /// Converts a `ByteOffset` from the start of the fragment to its corresponding `CharOffset`.
    fn char_offset_for_byte_offset(&self, byte_offset: ByteOffset) -> CharOffset {
        self.text.char_offset_for_byte_offset(byte_offset)
    }
}

impl sum_tree::Item for Fragment {
    type Summary = FragmentSummary;

    fn summary(&self) -> Self::Summary {
        let mut max_version = Global::new();
        max_version.observe(&self.insertion.id);
        for deletion in &self.deletions {
            max_version.observe(deletion);
        }
        max_version.observe_all(&self.max_undo_ts);

        if self.is_visible {
            FragmentSummary {
                text_summary: self.text.summary(),
                max_fragment_id: self.id.clone(),
                max_version,
            }
        } else {
            FragmentSummary {
                text_summary: TextSummary::default(),
                max_fragment_id: self.id.clone(),
                max_version,
            }
        }
    }
}

impl AddAssign<&FragmentSummary> for FragmentSummary {
    fn add_assign(&mut self, other: &Self) {
        self.text_summary += &other.text_summary;
        debug_assert!(self.max_fragment_id <= other.max_fragment_id);
        self.max_fragment_id = other.max_fragment_id.clone();
        self.max_version.observe_all(&other.max_version);
    }
}

impl Default for FragmentSummary {
    fn default() -> Self {
        FragmentSummary {
            text_summary: TextSummary::default(),
            max_fragment_id: FragmentId::min_value().clone(),
            max_version: Global::new(),
        }
    }
}

impl sum_tree::Dimension<'_, FragmentSummary> for TextSummary {
    fn add_summary(&mut self, summary: &FragmentSummary) {
        *self += &summary.text_summary;
    }
}

impl sum_tree::Dimension<'_, FragmentSummary> for CharOffset {
    fn add_summary(&mut self, summary: &FragmentSummary) {
        *self += summary.text_summary.chars;
    }
}

impl sum_tree::Dimension<'_, FragmentSummary> for ByteOffset {
    fn add_summary(&mut self, summary: &FragmentSummary) {
        *self += summary.text_summary.bytes;
    }
}

impl sum_tree::Item for InsertionSplit {
    type Summary = InsertionSplitSummary;

    fn summary(&self) -> Self::Summary {
        InsertionSplitSummary {
            extent: self.extent,
        }
    }
}

impl AddAssign<&InsertionSplitSummary> for InsertionSplitSummary {
    fn add_assign(&mut self, other: &Self) {
        self.extent += other.extent;
    }
}

impl sum_tree::Dimension<'_, InsertionSplitSummary> for CharOffset {
    fn add_summary(&mut self, summary: &InsertionSplitSummary) {
        *self += summary.extent;
    }
}

impl Operation {
    fn replica_id(&self) -> &ReplicaId {
        &self.lamport_timestamp().replica_id
    }

    fn lamport_timestamp(&self) -> &Lamport {
        match self {
            Operation::Edit(EditOperation {
                lamport_timestamp, ..
            }) => lamport_timestamp,
            Operation::Undo(UndoOperation {
                lamport_timestamp, ..
            }) => lamport_timestamp,
            Operation::UpdateSelections(UpdateSelectionsOperation {
                lamport_timestamp, ..
            }) => lamport_timestamp,
        }
    }
}

pub trait ToCharOffset {
    fn to_char_offset(&self, buffer: &Buffer) -> Result<CharOffset>;
}

pub trait ToBufferOffset {
    fn to_byte_offset(&self, buffer: &Buffer) -> Result<ByteOffset>;
}

impl ToCharOffset for Point {
    fn to_char_offset(&self, buffer: &Buffer) -> Result<CharOffset> {
        let mut fragments_cursor = buffer.fragments.cursor::<Point, TextSummary>();
        fragments_cursor.seek(self, SeekBias::Left);
        fragments_cursor
            .item()
            .ok_or_else(|| anyhow!("point is out of range"))
            .map(|fragment| {
                let overshoot = fragment.offset_for_point(*self - fragments_cursor.start().lines);
                fragments_cursor.start().chars + overshoot
            })
    }
}

impl ToCharOffset for CharOffset {
    fn to_char_offset(&self, _: &Buffer) -> Result<CharOffset> {
        Ok(*self)
    }
}

impl ToCharOffset for Anchor {
    fn to_char_offset(&self, buffer: &Buffer) -> Result<CharOffset> {
        Ok(buffer.summary_for_anchor(self)?.chars)
    }
}

impl ToCharOffset for &Anchor {
    fn to_char_offset(&self, buffer: &Buffer) -> Result<CharOffset> {
        Ok(buffer.summary_for_anchor(self)?.chars)
    }
}

impl ToCharOffset for ByteOffset {
    fn to_char_offset(&self, buffer: &Buffer) -> Result<CharOffset> {
        let mut fragments_cursor = buffer.fragments.cursor::<ByteOffset, TextSummary>();
        fragments_cursor.seek(self, SeekBias::Left);
        fragments_cursor
            .item()
            .ok_or_else(|| anyhow!("byte index is out of range"))
            .map(|fragment| {
                let chars_to_start_of_fragment = fragments_cursor.start().chars;

                // Compute the number of bytes past the start of the fragment, and call into the
                // fragment to convert the number of bytes into a character offset.
                let num_bytes_overshot = *self - fragments_cursor.start().bytes;
                chars_to_start_of_fragment
                    + fragment.char_offset_for_byte_offset(num_bytes_overshot)
            })
    }
}

impl ToBufferOffset for Point {
    fn to_byte_offset(&self, buffer: &Buffer) -> Result<ByteOffset> {
        let mut fragments_cursor = buffer.fragments.cursor::<Point, TextSummary>();
        fragments_cursor.seek(self, SeekBias::Left);
        fragments_cursor
            .item()
            .ok_or_else(|| anyhow!("point is out of range"))
            .map(|fragment| {
                let overshoot =
                    fragment.byte_offset_for_point(*self - fragments_cursor.start().lines);
                fragments_cursor.start().bytes + overshoot
            })
    }
}

impl ToBufferOffset for ByteOffset {
    fn to_byte_offset(&self, _: &Buffer) -> Result<ByteOffset> {
        Ok(*self)
    }
}

impl ToBufferOffset for Anchor {
    fn to_byte_offset(&self, buffer: &Buffer) -> Result<ByteOffset> {
        Ok(buffer.summary_for_anchor(self)?.bytes)
    }
}

impl ToBufferOffset for &Anchor {
    fn to_byte_offset(&self, buffer: &Buffer) -> Result<ByteOffset> {
        Ok(buffer.summary_for_anchor(self)?.bytes)
    }
}

impl ToBufferOffset for CharOffset {
    fn to_byte_offset(&self, buffer: &Buffer) -> Result<ByteOffset> {
        let point = buffer.point_for_offset(*self)?;
        point.to_byte_offset(buffer)
    }
}

pub trait ToPoint {
    fn to_point(&self, buffer: &Buffer) -> Result<Point>;
}

impl ToPoint for Anchor {
    fn to_point(&self, buffer: &Buffer) -> Result<Point> {
        Ok(buffer.summary_for_anchor(self)?.lines)
    }
}

impl ToPoint for CharOffset {
    fn to_point(&self, buffer: &Buffer) -> Result<Point> {
        let mut fragments_cursor = buffer.fragments.cursor::<CharOffset, TextSummary>();
        fragments_cursor.seek(self, SeekBias::Left);
        fragments_cursor
            .item()
            .ok_or_else(|| anyhow!("offset is out of range"))
            .map(|fragment| {
                let overshoot = fragment.point_for_offset(*self - fragments_cursor.start().chars);
                fragments_cursor.start().lines + overshoot
            })
    }
}

#[cfg(test)]
#[derive(Debug)]
/// What ranges to use when editing in tests.
pub enum RangesWhenEditing {
    /// Use the existing selections as the ranges.
    UseExistingSelections,

    /// Use a random set of ranges, up to `num_ranges`.
    UseRandomRanges { num_ranges: usize },
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
