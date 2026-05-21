mod buffer;
mod display_map;
mod selections;

use self::buffer::Peer;

pub use {
    buffer::{
        Anchor, AnchorBias, Chars, EditOrigin, Operation as CrdtOperation, PeerSelectionData,
        ReplicaId, SubwordBoundaries, TextRun, TextStyleOperation, ToBufferOffset, ToCharOffset,
        ToPoint,
    },
    display_map::{Bias, DisplayMap, DisplayPoint, MovementResult, ToDisplayPoint},
    selections::{
        DrawableSelection, LocalDrawableSelectionData, LocalPendingSelection, LocalSelection,
        LocalSelections, MarkedTextState, RemoteDrawableSelectionData, SelectAction, Selection,
        SelectionMode,
    },
};

use std::{
    cmp::{self},
    collections::{HashMap, HashSet},
    mem,
    ops::Range,
    rc::Rc,
};

use num_traits::SaturatingSub;
use string_offset::{ByteOffset, CharOffset};
use vec1::{vec1, Vec1};
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    text_layout::TextStyle,
    AppContext, Entity, ModelAsRef, ModelContext, ModelHandle,
};
use warpui::{
    text::{point::Point, word_boundaries::WordBoundariesPolicy, TextBuffer},
    SingletonEntity,
};

use crate::{editor::RangeExt, vim_registers::VimRegisters};

use vim::{
    find_next_paragraph_end, find_previous_paragraph_start,
    vim::{
        BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion,
        LineMotion, MotionType, TextObjectInclusion, TextObjectType, VimOperator, WordBound,
        WordMotion,
    },
    vim_a_paragraph, vim_inner_paragraph,
};
use vim::{
    vim_a_block, vim_a_quote, vim_a_word, vim_find_char_on_line, vim_find_matching_bracket,
    vim_inner_block, vim_inner_quote, vim_inner_word, vim_word_iterator_from_offset,
};

use buffer::{Buffer, Text};

use super::{movement, PlainTextEditorViewAction, SelectionInsertion, ValidInputType};

use itertools::{
    FoldWhile::{Continue, Done},
    Itertools,
};
use lazy_static::lazy_static;

lazy_static! {
    static ref AUTOCOMPLETE_SYMBOLS: HashMap<&'static str, &'static str> = HashMap::from([
        ("(", ")"),
        ("[", "]"),
        ("{", "}"),
        ("\'", "\'"),
        ("\"", "\""),
    ]);
    static ref CLOSING_SYMBOLS: HashSet<&'static str> =
        AUTOCOMPLETE_SYMBOLS.values().cloned().collect();
}

/// A snapshot of the editor that does not expose
/// buffer implementation details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSnapshot {
    /// When taking a snapshot, we intentionally use char offsets.
    /// We can't use the [`LocalSelection`] type because those rely
    /// on anchors which are associated to edit IDs. Using char offsets
    /// allows us to ensure we can restore selection state _across_
    /// buffers (e.g. ephemeral -> regular).
    selections: Vec1<Range<CharOffset>>,
    /// Similar to the reason we use offsets instead of anchors
    /// for [`Self::selections`], we use [`TextRun`]s instead
    /// of [`Fragment`]s or similar to describe the buffer text.
    buffer_text_runs: Vec<TextRun>,
}

impl EditorSnapshot {
    fn new(selections: Vec1<Range<CharOffset>>, buffer_text_runs: Vec<TextRun>) -> Self {
        Self {
            selections,
            buffer_text_runs,
        }
    }

    fn buffer_text_runs(&self) -> &Vec<TextRun> {
        &self.buffer_text_runs
    }

    pub fn text(&self) -> String {
        TextRun::text_from_text_runs(&self.buffer_text_runs)
    }

    pub fn selections(&self) -> &Vec1<Range<CharOffset>> {
        &self.selections
    }
}

/// Enum representing editor responses to edit and selection actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionState {
    /// Editor will respond to both edits and selection actions; cursor shown.
    Editable,
    /// Editor was set in an editable state, but has selections that cannot be edited.
    EditableWithInvalidSelection,
    /// Editor will respond to selection actions, and ignore edit actions. No cursor is shown.
    Selectable,
    /// Editor will ignore selection and edit actions. Editor is rendered with dimmed text and no cursor.
    Disabled,
}

type StaticMutableModelCallback = fn(&mut EditorModel, &mut ModelContext<EditorModel>);
type StaticCallback = fn(&EditorModel, &mut ModelContext<EditorModel>);

#[derive(Copy, Clone)]
pub enum UpdateBufferOption {
    /// No special behaviour.
    None,

    /// The edits are not recorded on the undo / redo stack.
    #[allow(dead_code)]
    SkipUndoRedoRecord,

    /// The edits are recorded in a dedicated, ephemeral buffer
    /// so as not to change the real buffer.
    /// Ephemeral edits can be materialized as soon as a non-ephemeral edit
    /// is made. This is done by replacing the real buffer with the ephemeral
    /// buffer before the non-ephemeral edit is applied.
    IsEphemeral,
}

impl UpdateBufferOption {
    fn skip_undo(&self) -> bool {
        matches!(self, Self::SkipUndoRedoRecord | Self::IsEphemeral)
    }
}

/// A helper struct to group together state needed
/// to specify how the buffer should be updated.
struct UpdateBuffer<U>
where
    U: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
{
    action: PlainTextEditorViewAction,
    edit_origin: EditOrigin,
    option: UpdateBufferOption,
    callback: U,
}

impl<U> UpdateBuffer<U>
where
    U: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
{
    fn skip_undo(&self) -> bool {
        self.option.skip_undo()
    }
}

/// Helper struct to change the underlying buffer via the model.
/// By edit, we mean string manipulation of the buffer or selection changes.
///
/// The view does not have access to the buffer so instead we expose
/// this struct for the view to declare how the buffer should be changed
/// while the model is responsible for actually changing the buffer
/// and updating corresponding state.
///
/// There are 4 available, optional callbacks. They are executed in the following order:
/// 1) change_selection_callback
/// 2) before_buffer_edit_callback
/// 3) update_buffer_callback
/// 4) post_buffer_edit_change_selection_callback
///
/// The buffer changes specified by a single `Edits` instance
/// are considered atomic / batched. See [`EditorModel::edit`] for how this struct is used.
///
/// TODO (suraj): technically, there's nothing enforcing that these callbacks
/// only do what they're described to do (e.g. the change_selection callback
/// can edit the buffer). To enforce this, we should only expose what needs
/// to be exposed in the respective callbacks. For example, the selection-related
/// callbacks only need the mutable selection state in the callback (not the
/// whole mutable model state). Similarly, for the edit callback, we only need
/// the mutable Buffer. To keep using the single API, we can have the editor model
/// helper APIs return callbacks that can then be used as the callbacks in `Edits`.
/// The view APIs can use this to compose an edit.
pub struct Edits<C, B, U, G>
where
    C: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    B: FnOnce(&EditorModel, &mut ModelContext<EditorModel>),
    U: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    G: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
{
    change_selections_callback: Option<C>,
    before_buffer_edit_callback: Option<B>,
    update_buffer: Option<UpdateBuffer<U>>,
    post_buffer_edit_change_selections_callback: Option<G>,
}

impl
    Edits<
        StaticMutableModelCallback,
        StaticCallback,
        StaticMutableModelCallback,
        StaticMutableModelCallback,
    >
{
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            change_selections_callback: None,
            before_buffer_edit_callback: None,
            update_buffer: None,
            post_buffer_edit_change_selections_callback: None,
        }
    }
}

impl<C, B, U, G> Edits<C, B, U, G>
where
    C: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    B: FnOnce(&EditorModel, &mut ModelContext<EditorModel>),
    U: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    G: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
{
    /// Allows callers to specify a callback to update the selection state
    /// as part of an edit.
    pub fn with_change_selections<T>(self, change_selection: T) -> Edits<T, B, U, G>
    where
        T: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    {
        Edits {
            change_selections_callback: Some(change_selection),
            before_buffer_edit_callback: self.before_buffer_edit_callback,
            update_buffer: self.update_buffer,
            post_buffer_edit_change_selections_callback: self
                .post_buffer_edit_change_selections_callback,
        }
    }

    /// An intermediate callback between `change_selections` and `update_buffer`.
    /// The model provided by the callback is _not_ mutable.
    pub fn with_before_buffer_edit<T>(self, before_buffer_edit: T) -> Edits<C, T, U, G>
    where
        T: FnOnce(&EditorModel, &mut ModelContext<EditorModel>),
    {
        Edits {
            change_selections_callback: self.change_selections_callback,
            before_buffer_edit_callback: Some(before_buffer_edit),
            update_buffer: self.update_buffer,
            post_buffer_edit_change_selections_callback: self
                .post_buffer_edit_change_selections_callback,
        }
    }

    /// Allows callers to specify a callback to update the buffer
    /// as part of an edit.
    pub fn with_update_buffer<T>(
        self,
        action: PlainTextEditorViewAction,
        edit_origin: EditOrigin,
        callback: T,
    ) -> Edits<C, B, T, G>
    where
        T: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    {
        let update_buffer = UpdateBuffer {
            action,
            edit_origin,
            option: UpdateBufferOption::None,
            callback,
        };
        Edits {
            change_selections_callback: self.change_selections_callback,
            before_buffer_edit_callback: self.before_buffer_edit_callback,
            update_buffer: Some(update_buffer),
            post_buffer_edit_change_selections_callback: self
                .post_buffer_edit_change_selections_callback,
        }
    }

    /// Allows callers to specify a callback to update the buffer
    /// as part of an edit, without adding an entry to the undo / redo stack.
    ///
    /// This should be used very sparingly. Prefer to use
    /// [`Self::with_update_buffer`].
    pub fn with_update_buffer_options<T>(
        self,
        action: PlainTextEditorViewAction,
        edit_origin: EditOrigin,
        option: UpdateBufferOption,
        callback: T,
    ) -> Edits<C, B, T, G>
    where
        T: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    {
        let update_buffer = UpdateBuffer {
            action,
            edit_origin,
            option,
            callback,
        };
        Edits {
            change_selections_callback: self.change_selections_callback,
            before_buffer_edit_callback: self.before_buffer_edit_callback,
            update_buffer: Some(update_buffer),
            post_buffer_edit_change_selections_callback: self
                .post_buffer_edit_change_selections_callback,
        }
    }

    /// Allows callers to specify a callback to update the selection state
    /// after the buffer has been updated.
    pub fn with_post_buffer_edit_change_selections<T>(
        self,
        post_buffer_edit_change_selections: T,
    ) -> Edits<C, B, U, T>
    where
        T: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    {
        Edits {
            change_selections_callback: self.change_selections_callback,
            before_buffer_edit_callback: self.before_buffer_edit_callback,
            update_buffer: self.update_buffer,
            post_buffer_edit_change_selections_callback: Some(post_buffer_edit_change_selections),
        }
    }

    fn is_pure_selection_change(&self) -> bool {
        self.update_buffer.is_none() && self.includes_selection_change()
    }

    fn includes_selection_change(&self) -> bool {
        self.change_selections_callback.is_some()
            || self.post_buffer_edit_change_selections_callback.is_some()
    }

    fn is_ephemeral(&self) -> bool {
        self.update_buffer
            .as_ref()
            .is_some_and(|u| matches!(u.option, UpdateBufferOption::IsEphemeral))
    }
}

struct BufferAndDisplayMaps {
    /// The main buffer and display map.
    regular: (ModelHandle<Buffer>, ModelHandle<DisplayMap>),

    /// A buffer and display map dedicated for ephemeral edits (see [`UpdateBufferOption::IsEphemeral`]).
    /// If [`Some`], then the ephemeral buffer is active.
    ephemeral: Option<(ModelHandle<Buffer>, ModelHandle<DisplayMap>)>,
}

impl BufferAndDisplayMaps {
    /// Returns a [`ModelHandle`] to the active [`Buffer`].
    fn buffer_handle(&self) -> &ModelHandle<Buffer> {
        if let Some((buffer, _)) = self.ephemeral.as_ref() {
            buffer
        } else {
            &self.regular.0
        }
    }

    /// Returns a [`ModelHandle`] to the active [`DisplayMap`].
    fn display_map_handle(&self) -> &ModelHandle<DisplayMap> {
        if let Some((_, display_map)) = self.ephemeral.as_ref() {
            display_map
        } else {
            &self.regular.1
        }
    }

    /// Deactivates any ephemeral state.
    fn deactivate_ephemeral_state(&mut self) {
        self.ephemeral.take();
    }

    /// Activates a new ephemeral state.
    fn activate_new_ephemeral_state(&mut self, ctx: &mut ModelContext<EditorModel>) {
        let tab_size = self.regular.1.as_ref(ctx).tab_size();
        let ephemeral_buffer = ctx.add_model(|_| Buffer::new(""));
        let ephemeral_display_map: ModelHandle<DisplayMap> =
            ctx.add_model(|ctx| DisplayMap::new(ephemeral_buffer.clone(), tab_size, ctx));
        ctx.subscribe_to_model(
            &ephemeral_buffer,
            EditorModel::handle_buffer_event_for_non_collaborative_editor,
        );
        ctx.subscribe_to_model(
            &ephemeral_display_map,
            EditorModel::handle_display_map_event,
        );
        self.ephemeral = Some((ephemeral_buffer, ephemeral_display_map));
    }
}

pub struct EditorModel {
    /// The extent to which the editor is interactable.
    interaction_state: InteractionState,

    /// The [`Buffer`]s and [`DisplayMap`]s used by this editor.
    /// Use [`Self::buffer`] and [`Self::display_map`] to access the
    /// correct buffer and display map, respectively.
    buffer_and_display_map: BufferAndDisplayMaps,

    /// This field stores the editor-specific state needed for Vim's visual mode. Visual mode works
    /// differently from the typical editor selection. The typical editor selection stores two
    /// Anchors in [`Selection::start`] and [`Selection::end`] to delimit the selection range,
    /// either of which may be the current cursor position, or "head". However, the Visual mode
    /// selection range may be different from the cursor position. In linewise visual mode,
    /// selection start and end are at line boundaries, for example. Therefore, visual mode derives
    /// the selection range from the current cursor position, "head", and the "visual tail" which
    /// is the anchor for where the cursor was _when visual mode was entered_. This field stores
    /// where the cursor(s) was at the time visual mode was entered.
    vim_visual_tails: Vec<Anchor>,

    /// Counter for recording whether the last editor action contains an autocompleted symbol.
    /// This increments with each consecutive autocompleted insertion.
    consecutive_autocomplete_insertion_edits_counter: usize,

    /// The maximum buffer length for the editor. Any operations that cause the buffer to exceed
    /// this length will be rejected.
    max_buffer_len: Option<usize>,

    /// The types of inputs that are valid for this editor.
    valid_input_type: ValidInputType,

    /// The buffer text before the last edit / undo / redo.
    last_buffer_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorModelEvent {
    Edited {
        edit_origin: EditOrigin,
    },
    StylesUpdated,
    DisplayMapUpdated,
    SelectionsChanged,
    ShellCut(String),
    UpdatePeers {
        operations: Rc<Vec<CrdtOperation>>,
    },
    /// An edit action has replaced the entire buffer's content.
    BufferReplaced,
}

impl Entity for EditorModel {
    type Event = EditorModelEvent;
}

/// The public interface.
impl EditorModel {
    /// Helper method to emit a11y event when the selections change. For now, only single cursor
    /// selections are supported.
    /// The app will emit values based on the following matrix:
    ///
    /// | Start            | End              | What should be read     |
    /// |--------------    |--------------    |---------------------    |
    /// | No selection     | No selection     | Delta (characters)      |
    /// | No selection     | Selection        | "Selected: {delta}"     |
    /// | Selection        | Selection        | "Selected: {delta}"     |
    /// | Selection        | No selection     | "Unselected"            |
    ///
    pub fn delta_for_a11y(
        &self,
        selection_before: Range<ByteOffset>,
        was_selecting: bool,
        ctx: &mut ModelContext<Self>,
    ) -> AccessibilityContent {
        let selection_after = self.first_selection(ctx).to_byte_offset(self.buffer(ctx));
        let is_selecting = !self.first_selection(ctx).is_cursor_only(self.buffer(ctx));

        let text = self.buffer(ctx).text();

        // Note: is_empty on a range means that start >= end
        // We use it to verify whether the direction of the selection has changed
        let (before_point, after_point) =
            if selection_before.is_empty() == selection_after.is_empty() {
                (selection_before.start, selection_after.start)
            } else {
                // we changed direction when selecting
                (selection_before.end, selection_after.start)
            };
        let (start, end) = if before_point < after_point {
            (before_point, after_point)
        } else {
            (after_point, before_point)
        };
        let delta = &text[start.as_usize()..end.as_usize()];
        match (was_selecting, is_selecting) {
            (false, false) => {
                AccessibilityContent::new_without_help(delta, WarpA11yRole::UserAction)
            }
            (_, true) => {
                // Note that Range is start <= x < end, and in our case, when deciding what was the action
                // (selecting or unselecting) we need to take end element into consideration also,
                // hence the helper `inclusive_contains` method.
                fn inclusive_contains(selection: &Range<ByteOffset>, element: ByteOffset) -> bool {
                    selection.contains(&element) || selection.end == element
                }
                // Since the range here may show as empty, we're swapping it, so that it's (n, m), where
                // n<m
                let selection_after = if selection_after.is_empty() {
                    selection_after.end..selection_after.start
                } else {
                    selection_after
                };

                let action = if inclusive_contains(&selection_after, start)
                    && inclusive_contains(&selection_after, end)
                {
                    "selected"
                } else {
                    "unselected"
                };
                AccessibilityContent::new(delta, format!(", {action}"), WarpA11yRole::UserAction)
            }
            (true, false) => {
                AccessibilityContent::new_without_help("Unselected", WarpA11yRole::UserAction)
            }
        }
    }

    pub fn new(
        base_text: String,
        tab_size: usize,
        max_buffer_len: Option<usize>,
        valid_input_type: ValidInputType,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let regular_buffer = ctx.add_model(|_| Buffer::new(base_text.clone()));
        let regular_display_map =
            ctx.add_model(|ctx| DisplayMap::new(regular_buffer.clone(), tab_size, ctx));
        ctx.subscribe_to_model(&regular_buffer, Self::handle_buffer_event);
        ctx.subscribe_to_model(&regular_display_map, Self::handle_display_map_event);
        let regular = (regular_buffer, regular_display_map);

        Self {
            interaction_state: InteractionState::Editable,
            buffer_and_display_map: BufferAndDisplayMaps {
                regular,
                ephemeral: None,
            },
            vim_visual_tails: vec![],
            consecutive_autocomplete_insertion_edits_counter: 0,
            max_buffer_len,
            valid_input_type,
            last_buffer_text: base_text,
        }
    }

    pub fn replica_id<C: ModelAsRef>(&self, ctx: &C) -> ReplicaId {
        self.collaborative_buffer().as_ref(ctx).replica_id()
    }

    pub fn registered_peers<C: ModelAsRef>(&self, ctx: &C) -> HashMap<ReplicaId, Peer> {
        self.buffer(ctx).registered_peers()
    }

    pub fn register_remote_peer(
        &mut self,
        replica_id: ReplicaId,
        selection_data: PeerSelectionData,
        ctx: &mut ModelContext<Self>,
    ) {
        self.collaborative_buffer().update(ctx, |buffer, _ctx| {
            buffer.register_peer(replica_id, selection_data);
        });
    }

    pub fn unregister_all_remote_peers(&mut self, ctx: &mut ModelContext<Self>) {
        self.collaborative_buffer().update(ctx, |buffer, _ctx| {
            buffer.unregister_all_peers();
        });
    }

    pub fn unregister_remote_peer(&mut self, replica_id: &ReplicaId, ctx: &mut ModelContext<Self>) {
        self.collaborative_buffer().update(ctx, |buffer, _ctx| {
            buffer.unregister_peer(replica_id);
        });
    }

    pub fn set_remote_peer_selection_data(
        &mut self,
        replica_id: &ReplicaId,
        selection_data: PeerSelectionData,
        ctx: &mut ModelContext<Self>,
    ) {
        self.collaborative_buffer().update(ctx, |buffer, _ctx| {
            buffer.set_peer_selection_data(replica_id, selection_data);
        });
    }

    pub fn recreate_buffer(&mut self, replica_id: Option<ReplicaId>, ctx: &mut ModelContext<Self>) {
        let replica_id = replica_id.unwrap_or(self.replica_id(ctx));
        let tab_size = self.buffer_and_display_map.regular.1.as_ref(ctx).tab_size();

        self.collaborative_buffer().update(ctx, |buffer, _| {
            buffer.recreate(replica_id, "");
        });
        self.collaborative_display_map().update(ctx, |map, ctx| {
            map.recreate(tab_size, ctx);
        });

        self.buffer_and_display_map.deactivate_ephemeral_state();
    }

    fn refresh_batch_version(&mut self, ctx: &mut ModelContext<Self>) {
        self.buffer_handle().update(ctx, |buffer, _| {
            buffer.refresh_version_on_edits_and_selection_changes_batch()
        });
    }

    fn start_batch<U>(&mut self, update: Option<&UpdateBuffer<U>>, ctx: &mut ModelContext<Self>)
    where
        U: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    {
        if let Some(update) = update {
            if !self.is_ephemeral() {
                self.last_buffer_text = self.buffer_text(ctx);
            }

            self.buffer_handle().update(ctx, |buffer, _| {
                buffer.start_edits_and_selection_changes_batch(
                    update.edit_origin,
                    update.action,
                    update.skip_undo(),
                );
            });
        } else {
            self.buffer_handle().update(ctx, |buffer, _| {
                buffer.start_selection_changes_only_batch();
            });
        }
    }

    fn end_batch(&mut self, ctx: &mut ModelContext<Self>) {
        self.buffer_handle().update(ctx, |buffer, ctx| {
            buffer.end_batch(ctx);
        });
    }

    pub fn change_selections(
        &mut self,
        new_selections: impl Into<LocalSelections>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.buffer_handle().update(ctx, |buffer, _| {
            if let Err(e) = buffer.change_selections(new_selections.into()) {
                log::warn!("Failed to change selections: {e}");
            }
        });
    }

    /// The main API to update the underlying buffer. View APIs
    /// should never attempt to update the buffer state without going
    /// through this API. There are guards in place to ensure we are using
    /// this API when updating the buffer in a controlled fashion.
    ///
    /// Selection changes are only applied if the editor is in a selectable state.
    /// Buffer changes are only applied if the editor is in an editable state.
    pub fn edit<F, G, H, M>(&mut self, ctx: &mut ModelContext<Self>, edit: Edits<F, G, H, M>)
    where
        F: FnOnce(&mut Self, &mut ModelContext<Self>),
        G: FnOnce(&Self, &mut ModelContext<Self>),
        H: FnOnce(&mut Self, &mut ModelContext<Self>),
        M: FnOnce(&mut Self, &mut ModelContext<Self>),
    {
        let can_select = self.can_select();
        let can_edit = self.can_edit();
        let first_cursor_before = self.first_selection(ctx).to_byte_offset(self.buffer(ctx));
        let was_selecting = !self.first_selection(ctx).is_cursor_only(self.buffer(ctx));
        let is_pure_selection_change = can_select && edit.is_pure_selection_change();

        // If this is an ephemeral edit, then snapshot the current buffer
        // so that the ephemeral edit is applied on the latest buffer state
        // (whether that's the real buffer or the last ephemeral buffer).
        // This needs to be done _before_ we start the batch below so that
        // 1) we take the snapshot of the correct (regular vs. ephemeral) buffer
        // 2) we star the the batch on the correct (regular vs. ephemeral) buffer
        let restore_from_snapshot = if can_edit && edit.is_ephemeral() {
            let snapshot = self.as_snapshot(ctx);
            self.buffer_and_display_map
                .activate_new_ephemeral_state(ctx);
            Some(snapshot)
        } else if can_edit && self.is_ephemeral() && edit.update_buffer.is_some() {
            // We're materializing an ephemeral edit, so snapshot the ephemeral buffer
            // so that we can apply it to the regular buffer.
            let snapshot = self.as_snapshot(ctx);
            self.buffer_and_display_map.deactivate_ephemeral_state();
            Some(snapshot)
        } else {
            None
        };

        // Start the batch after setting the ephemeral state so that
        // the batch is started on the right buffer.
        // Note: now that we've started a batch, it must be ended.
        // We cannot return early in this function.
        self.start_batch(edit.update_buffer.as_ref(), ctx);

        // If we need to restore between regular and ephemeral states,
        // do it now so that any selections / edits beyond are applied to the
        // right buffer state. For example, if we tried to select
        // without having restored the snapshot, we would be selecting
        // on the wrong underlying buffer.
        if let Some(snapshot) = restore_from_snapshot {
            self.restore_from_snapshot(snapshot, ctx);
            // Refresh batch version here as we already recorded edits for the snapshot
            // restoration.
            self.refresh_batch_version(ctx);

            // Do a custom undo / redo record without exhausting the batched edits.
            // This should be fine because there shouldn't have been any other edits so far.
            self.buffer_handle().update(ctx, |buffer, ctx| {
                buffer.record_edits(PlainTextEditorViewAction::ReplaceBuffer, ctx);
            });
        }

        if can_select {
            if let Some(change_selection) = edit.change_selections_callback {
                change_selection(self, ctx);
            }
        }

        if let Some(before_buffer_edit) = edit.before_buffer_edit_callback {
            before_buffer_edit(self, ctx);
        }

        let prev_state = self.consecutive_autocomplete_insertion_edits_counter;
        if can_edit {
            if let Some(update_buffer) = edit.update_buffer {
                (update_buffer.callback)(self, ctx);
            }
        }
        let next_state = self.consecutive_autocomplete_insertion_edits_counter;

        if can_select {
            if let Some(post_buffer_edit_change_selection) =
                edit.post_buffer_edit_change_selections_callback
            {
                post_buffer_edit_change_selection(self, ctx);
            }
        }

        // Only emit a11y content for pure selection changes because edits
        // have their own a11y content already.
        // TODO: technically, a pure selection change can also happen via the `update_buffer` API.
        // We might want to actually do this similar to how we emit selection changes (i.e. in `Buffer::end_batch`).
        if is_pure_selection_change {
            let a11y_content = self.delta_for_a11y(first_cursor_before, was_selecting, ctx);
            ctx.emit_a11y_content(a11y_content);
        }

        // Check if the last edit inserted a new autocomplete symbol.
        if prev_state == next_state {
            self.consecutive_autocomplete_insertion_edits_counter = 0;
        }

        self.end_batch(ctx);
    }

    pub fn apply_remote_operations(
        &mut self,
        operations: Vec<CrdtOperation>,
        ctx: &mut ModelContext<Self>,
    ) {
        // The ephemeral buffer isn't collaborative so these operations are meant for the main buffer.
        self.buffer_and_display_map
            .regular
            .0
            .update(ctx, |buffer, ctx| {
                if let Err(e) = buffer.apply_ops(operations, ctx) {
                    log::warn!("Failed to apply remote edits to buffer: {e}");
                }
            })
    }

    pub fn interaction_state(&self) -> InteractionState {
        self.interaction_state
    }

    pub fn set_interaction_state(&mut self, interaction_state: InteractionState) {
        self.interaction_state = interaction_state;
    }

    pub fn can_edit(&self) -> bool {
        matches!(self.interaction_state, InteractionState::Editable)
    }

    pub fn can_select(&self) -> bool {
        matches!(
            self.interaction_state,
            InteractionState::Selectable
                | InteractionState::Editable
                | InteractionState::EditableWithInvalidSelection
        )
    }

    pub fn vim_visual_tails(&self) -> &Vec<Anchor> {
        &self.vim_visual_tails
    }

    pub fn fold(&mut self, ctx: &mut ModelContext<Self>) {
        if self.can_edit() {
            let mut fold_ranges = Vec::new();

            let map = self.display_map(ctx);
            for selection in self.selections(ctx).iter() {
                let (start, end) = selection.display_range(map, ctx).sorted();
                let buffer_start_row = start.to_buffer_point(map, Bias::Left, ctx).unwrap().row;

                for row in (0..=end.row()).rev() {
                    if self.is_line_foldable(row, ctx) && !map.is_line_folded(row) {
                        let fold_range = self.foldable_range_for_line(row, ctx).unwrap();
                        if fold_range.end.row >= buffer_start_row {
                            fold_ranges.push(fold_range);
                            if row <= start.row() {
                                break;
                            }
                        }
                    }
                }
            }

            if !fold_ranges.is_empty() {
                self.display_map_handle().update(ctx, |map, ctx| {
                    map.fold(fold_ranges, ctx).unwrap();
                });
            }
        }
    }

    pub fn fold_selected_ranges(&mut self, ctx: &mut ModelContext<Self>) {
        if self.can_edit() {
            let buffer = self.buffer(ctx);
            let ranges = self
                .selections(ctx)
                .iter()
                .map(|s| s.range(buffer))
                .collect::<Vec<_>>();

            self.display_map_handle().update(ctx, |map, ctx| {
                map.fold(ranges, ctx).unwrap();
            });
        }
    }

    pub fn unfold(&mut self, ctx: &mut ModelContext<Self>) {
        if self.can_edit() {
            let map = self.display_map(ctx);
            let buffer = self.buffer(ctx);
            let ranges = self
                .selections(ctx)
                .iter()
                .map(|s| {
                    let (start, end) = s.display_range(map, ctx).sorted();
                    let mut start = start.to_buffer_point(map, Bias::Left, ctx).unwrap();
                    let mut end = end.to_buffer_point(map, Bias::Left, ctx).unwrap();
                    start.column = 0;
                    end.column = buffer.line_len(end.row).unwrap();
                    start..end
                })
                .collect::<Vec<_>>();

            self.display_map_handle().update(ctx, |map, ctx| {
                map.unfold(ranges, ctx).unwrap();
            });
        }
    }

    pub fn undo(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.can_edit() {
            return;
        }

        if self.is_ephemeral() {
            self.buffer_and_display_map.deactivate_ephemeral_state();
            ctx.emit(EditorModelEvent::Edited {
                edit_origin: EditOrigin::UserInitiated,
            });
            return;
        }

        self.last_buffer_text = self.buffer_text(ctx);
        self.buffer_handle().update(ctx, |buffer, ctx| {
            buffer.undo(ctx);
            ctx.notify();
        });
    }

    pub fn redo(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.can_edit() {
            return;
        }

        self.last_buffer_text = self.buffer_text(ctx);
        self.buffer_handle().update(ctx, |buffer, ctx| {
            buffer.redo(ctx);
            ctx.notify();
        });
    }

    pub fn reset_undo_redo_stack(&mut self, ctx: &mut ModelContext<Self>) {
        self.buffer_handle().update(ctx, |buffer, _ctx| {
            buffer.reset_undo_stack();
        })
    }

    pub fn consecutive_autocomplete_insertion_edits_counter(&self) -> usize {
        self.consecutive_autocomplete_insertion_edits_counter
    }

    pub fn as_snapshot<C: ModelAsRef>(&self, ctx: &C) -> EditorSnapshot {
        self.buffer(ctx).snapshot()
    }

    pub fn buffer_edit<I, S, T>(
        &mut self,
        old_ranges: I,
        new_text: T,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = Range<S>>,
        S: ToCharOffset,
        T: Into<Text>,
    {
        self.buffer_handle().update(ctx, |buffer, ctx| {
            buffer.edit(old_ranges, new_text, ctx).map(|_| ())
        })
    }

    pub fn selected_text<C: ModelAsRef>(&self, ctx: &C) -> String {
        let buffer = self.buffer(ctx);
        let last_ix = self.selections(ctx).len() - 1;
        self.selections(ctx)
            .iter()
            .enumerate()
            .flat_map(|(ix, selection)| {
                let start = selection.start().to_char_offset(buffer).unwrap();
                let end = selection.end().to_char_offset(buffer).unwrap();
                buffer
                    .chars_at(start)
                    .unwrap()
                    .take(end.as_usize() - start.as_usize())
                    .chain(if ix < last_ix { Some('\n') } else { None })
            })
            .collect()
    }

    pub fn selected_text_strings(&self, ctx: &AppContext) -> Vec<String> {
        let buffer = self.buffer(ctx);
        self.selections(ctx)
            .iter()
            .map(|selection| {
                let start = selection
                    .start()
                    .to_char_offset(buffer)
                    .expect("Start character in selection buffer should exist");
                let end = selection
                    .end()
                    .to_char_offset(buffer)
                    .expect("End character in selection buffer should exist");
                buffer
                    .chars_at(start)
                    .expect("Buffer should contain character at start index of selection")
                    .take(end.as_usize().saturating_sub(start.as_usize()))
                    .collect()
            })
            .collect()
    }

    pub fn indent(&mut self, ctx: &mut ModelContext<Self>) {
        let start = self.first_selection(ctx).start().to_point(self.buffer(ctx));
        let end = self.first_selection(ctx).end().to_point(self.buffer(ctx));
        if let Some((start, end)) = start.ok().zip(end.ok()) {
            self.buffer_handle().update(ctx, |buffer, ctx| {
                if let Err(error) = buffer.indent(start.row..end.row + 1, ctx) {
                    log::error!("error indenting text: {error}");
                }
            });
        }
    }

    pub fn unindent(&mut self, ctx: &mut ModelContext<Self>) {
        self.buffer_handle().update(ctx, |buffer, ctx| {
            // Unlike indent, unindent is applied always: to multiple selections and even single characters/words.
            let row_ranges = buffer
                .local_selections()
                .selections
                .iter()
                .map(|selection| {
                    selection.start().to_point(buffer).unwrap().row
                        ..selection.end().to_point(buffer).unwrap().row + 1
                })
                .dedup()
                .collect::<Vec<_>>();

            if let Err(error) = buffer.unindent(row_ranges, ctx) {
                log::error!("error unindenting text: {error}");
            };
        });
    }

    pub fn insert(
        &mut self,
        text: &str,
        text_style: Option<TextStyle>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.insert_internal(text, text_style, SelectionInsertion::No, ctx);
    }

    pub fn insert_selected_text(&mut self, text: &str, ctx: &mut ModelContext<Self>) {
        self.insert_internal(text, None, SelectionInsertion::Yes, ctx);
    }

    pub fn clear_all_styles(&self, ctx: &mut ModelContext<Self>) {
        let len = self.buffer_text(ctx).len();
        let full_range = ByteOffset::from(0)..ByteOffset::from(len);
        self.update_buffer_styles([full_range], TextStyleOperation::clear_all(), ctx);
    }

    pub fn update_buffer_styles<I, S>(
        &self,
        old_ranges: I,
        text_style_operation: TextStyleOperation,
        ctx: &mut ModelContext<Self>,
    ) where
        I: IntoIterator<Item = Range<S>>,
        S: ToCharOffset,
    {
        self.buffer_handle().update(ctx, |buffer, ctx| {
            if let Err(e) = buffer.update_styles(old_ranges, text_style_operation, ctx) {
                log::error!("Error with updating styles: {e:?}")
            }
        });
    }

    /// Autocomplete user inserted text under the following conditions:
    /// 1) The symbol has a corresponding closing symbol (e.g. ( -> ), " -> ", etc)
    /// 2) All of the current cursors are not immediately next to any alphanumeric character
    ///
    // If there are only selections (no single cursor), autocomplete by wrapping the selection with start and end symbol.
    pub fn insert_and_maybe_autocomplete_symbols(
        &mut self,
        text: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        if CLOSING_SYMBOLS.contains(&text)
            && text.chars().count() == 1
            && self.all_cursors_next_character_matches_char(
                text.chars()
                    .next()
                    .expect("Autocompleted symbol should have at least one character"),
                ctx,
            )
        {
            let map = self.display_map(ctx);

            // Moves cursor to the right.
            let mut new_selections = self.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let end = selection.end().to_display_point(map, ctx).unwrap();
                let cursor = map
                    .anchor_before(
                        movement::right(map, end, ctx, false)
                            .expect("moving right should return a valid DisplayPoint"),
                        Bias::Right,
                        ctx,
                    )
                    .expect("DisplayPoint should convert to an Anchor");

                selection.set_start(cursor.clone());
                selection.set_end(cursor);
                selection.set_reversed(false);
                selection.goal_start_column = None;
                selection.goal_end_column = None;
            }
            self.change_selections(new_selections, ctx);
            return;
        }

        // Autocomplete for symbols like double quotes and brackets.
        if let Some(completion) = AUTOCOMPLETE_SYMBOLS.get(text) {
            if self.no_cursor_only_selections(ctx) {
                self.insert_characters_wrap_around_selection(text, completion, ctx);
                return;
            }

            let buffer = self.buffer(ctx);
            // Check whether all of the current cursors are not immediately next to any alphanumeric characters.
            let not_alphanumeric = self.selections(ctx).iter().all(|selection| {
                let position = selection
                    .start()
                    .to_point(buffer)
                    .expect("Start of selection should exist");

                let right_not_alphanumeric = buffer
                    .chars_at(position)
                    .unwrap()
                    .next()
                    .map(|right_char| !right_char.is_alphanumeric())
                    .unwrap_or(true);
                let left_not_alphanumeric = buffer
                    .chars_at(position)
                    .unwrap()
                    .rev()
                    .next()
                    .map(|left_char| !left_char.is_alphanumeric())
                    .unwrap_or(true);
                right_not_alphanumeric && left_not_alphanumeric
            });

            // Always autocomplete for brackets and parentheses. For quotes, only autocomplete
            // if both sides of the cursor are not alphabetic or numeric characters.
            if (text != "\"" && text != "\'") || not_alphanumeric {
                self.insert_internal(text, None, SelectionInsertion::No, ctx);
                let offset_ranges = self.selection_to_char_offset_ranges(ctx);

                // Complete the closing symbol without moving the cursor.
                self.buffer_handle().update(ctx, |buffer, ctx| {
                    if let Err(error) = buffer.edit(offset_ranges.iter().cloned(), *completion, ctx)
                    {
                        log::error!("error inserting text: {error}");
                    };
                });
                self.consecutive_autocomplete_insertion_edits_counter += 1;
                return;
            }
        }

        self.insert_internal(text, None, SelectionInsertion::No, ctx);
    }

    pub fn insert_internal(
        &mut self,
        text: &str,
        text_style: Option<TextStyle>,
        select_insertion: SelectionInsertion,
        ctx: &mut ModelContext<Self>,
    ) {
        let offset_ranges = self.selection_to_char_offset_ranges(ctx);
        if self.text_insertion_would_exceed_max_buffer_len(text, ctx) {
            return;
        }
        if self.text_is_invalid_input_type(text) {
            return;
        }
        let old_buffer_count = self.buffer_len(ctx);

        self.buffer_handle().update(ctx, |buffer, ctx| {
            if let Err(error) = buffer.edit(
                offset_ranges.iter().cloned(),
                Text::new(text, text_style),
                ctx,
            ) {
                log::error!("error inserting text: {error}");
            };
        });

        let char_count = text.chars().count() as isize;
        let mut delta = 0_isize;
        let mut total_deleted = 0_isize;

        let buffer = self.buffer(ctx);
        let new_selections = offset_ranges.mapped(|range| {
            let range_start = range.start.as_usize() as isize;
            let range_end = range.end.as_usize() as isize;
            let end = buffer
                .anchor_before(CharOffset::from(
                    (range_start + delta + char_count) as usize,
                ))
                .unwrap();
            let start = match select_insertion {
                SelectionInsertion::Yes => buffer
                    .anchor_before(CharOffset::from((range_start + delta) as usize))
                    .unwrap(),
                SelectionInsertion::No => end.clone(),
            };
            let deleted_count = range_end - range_start;
            total_deleted += deleted_count;
            delta += char_count - deleted_count;
            LocalSelection {
                selection: Selection {
                    start,
                    end,
                    reversed: false,
                },
                clamp_direction: Default::default(),
                goal_start_column: None,
                goal_end_column: None,
            }
        });

        if total_deleted as usize == old_buffer_count.as_usize() {
            ctx.emit(EditorModelEvent::BufferReplaced);
        }
        self.change_selections(new_selections, ctx);
    }

    pub fn marked_text_state(&self, ctx: &AppContext) -> MarkedTextState {
        self.buffer(ctx).marked_text_state()
    }

    pub fn update_marked_text(
        &mut self,
        text: &str,
        marked_text_style: Option<TextStyle>,
        selected_range: &Range<usize>,
        ctx: &mut ModelContext<Self>,
    ) {
        let marked_text_state = self.marked_text_state(ctx);
        // If there was no marked text before, then we should replace each selection with blank text.
        if marked_text_state == MarkedTextState::Inactive {
            self.insert_internal("", None, SelectionInsertion::No, ctx);
        }

        // Insert the marked text in all selections as selected text.
        self.insert_internal(text, marked_text_style, SelectionInsertion::Yes, ctx);
        self.set_marked_text_state(
            MarkedTextState::Active {
                selected_range: selected_range.clone(),
            },
            ctx,
        );
    }

    fn set_marked_text_state(
        &mut self,
        marked_text_state: MarkedTextState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.buffer_handle().update(ctx, |buffer, _ctx| {
            buffer.set_marked_text_state(marked_text_state);
        });
    }

    pub fn commit_incomplete_marked_text(&mut self, ctx: &mut ModelContext<Self>) {
        // We assume that all "selections" are actually the composed marked text.
        let selected_strings = self.selected_text_strings(ctx);
        let incomplete_marked_text = selected_strings.first().map(|s| s.as_str()).unwrap_or("");
        self.clear_marked_text_and_commit(incomplete_marked_text, ctx);
    }

    pub fn clear_marked_text_and_commit(&mut self, text: &str, ctx: &mut ModelContext<Self>) {
        self.insert_internal(text, None, SelectionInsertion::No, ctx);
        self.set_marked_text_state(MarkedTextState::Inactive, ctx);
    }

    pub fn clear_marked_text(&mut self, ctx: &mut ModelContext<Self>) {
        if self.marked_text_state(ctx) == MarkedTextState::Inactive {
            return;
        }
        self.edit(
            ctx,
            Edits::new().with_update_buffer_options(
                PlainTextEditorViewAction::UpdateMarkedText,
                EditOrigin::UserTyped,
                UpdateBufferOption::None,
                |editor_model, ctx| {
                    editor_model.insert_internal("", None, SelectionInsertion::No, ctx);
                    editor_model.set_marked_text_state(MarkedTextState::Inactive, ctx);
                },
            ),
        );
    }

    // Used for restoring the buffer from a snapshot.
    fn restore_from_snapshot(&mut self, snapshot: EditorSnapshot, ctx: &mut ModelContext<Self>) {
        let version = self.buffer_handle().as_ref(ctx).versions();
        self.clear_selections(ctx);
        self.clear_buffer(ctx);

        let mut text_added_offset = CharOffset::from(0);
        self.buffer_handle().update(ctx, |buffer, ctx| {
            for styled_text in snapshot.buffer_text_runs() {
                if let Err(error) = buffer.edit(
                    [(text_added_offset..text_added_offset)],
                    Text::new(styled_text.text(), Some(styled_text.text_style())),
                    ctx,
                ) {
                    log::error!("error inserting text: {error}");
                };
                text_added_offset += styled_text.text().chars().count();
            }
        });

        let edits = self.buffer(ctx).edits_since(version).collect::<Vec<_>>();
        self.display_map_handle()
            .update(ctx, move |display_map, ctx| {
                display_map.apply_edits(&edits, ctx).unwrap();
            });

        let buffer = self.buffer(ctx);
        let restored_selection = snapshot
            .selections()
            .iter()
            .filter_map(|range| {
                // Handle the error to convert saved selection offsets to editor anchors gracefully
                // as the restored buffer state might not match exactly to the saved snapshot.
                let Some(end) = buffer.anchor_before(range.end).ok() else {
                    log::error!(
                        "error restoring snapshot with selection end {} on text with max range {}",
                        range.end,
                        text_added_offset
                    );
                    return None;
                };
                let Some(start) = buffer.anchor_before(range.start).ok() else {
                    log::error!(
                        "error restoring snapshot with selection start {} on text with max range {}",
                        range.start,
                        text_added_offset
                    );
                    return None;
                };

                let mut selection = LocalSelection {
                    selection: Selection::single_cursor(end),
                    clamp_direction: Default::default(),
                    goal_start_column: None,
                    goal_end_column: None,
                };

                selection.set_head(buffer, start);
                Some(selection)
            })
            .collect_vec();

        let new_selections = if let Ok(restored_selection) = Vec1::try_from_vec(restored_selection)
        {
            restored_selection
        } else {
            // Make sure there is at least one root selection if the
            // restored selection is empty.
            vec1![LocalSelection {
                selection: Selection::single_cursor(Anchor::Start),
                clamp_direction: Default::default(),
                goal_start_column: None,
                goal_end_column: None,
            }]
        };
        self.change_selections(new_selections, ctx);
    }

    pub fn selection_insertion_index(&self, start: &Anchor, app: &AppContext) -> usize {
        self.buffer(app)
            .local_selections()
            .selection_insertion_index(start, self.buffer(app))
    }

    /// Set all selection ranges back to a single cursor originating at the start anchor.
    pub fn deselect(&mut self, ctx: &mut ModelContext<Self>) {
        let mut new_selections = self.selections(ctx).clone();
        let buffer = self.buffer(ctx);
        for selection in new_selections.iter_mut() {
            let Ok(anchor) = buffer.anchor_at(selection.start(), AnchorBias::Left) else {
                continue;
            };
            selection.set_selection(Selection::single_cursor(anchor));
            selection.goal_start_column = None;
            selection.goal_end_column = None;
        }
        self.change_selections(new_selections, ctx);
    }

    /// Helper method for moving the cursor within the input box / editor.
    /// @param keep_selection - If true, the movement will also select the text spanned by the
    /// movement.
    /// @param func - Closure which gets buffer & selection and returns the location to which the
    /// cursor should be moved.
    /// @param ctx
    pub fn move_cursor<F, C>(&mut self, keep_selection: bool, func: F, ctx: &mut ModelContext<Self>)
    where
        C: ToCharOffset,
        F: Fn(&Buffer, &mut LocalSelection) -> C,
    {
        let mut new_selections = self.selections(ctx).clone();
        let buffer = self.buffer(ctx);
        for selection in new_selections.iter_mut() {
            let point = func(buffer, selection);
            let cursor = buffer.anchor_before(point).unwrap();

            if keep_selection {
                selection.set_head(buffer, cursor);
            } else {
                selection.set_selection(Selection::single_cursor(cursor));
            }
            selection.goal_start_column = None;
            selection.goal_end_column = None;
        }
        self.change_selections(new_selections, ctx);
    }

    /// Move each cursor to the column with the left-most non-whitespace character.
    pub fn cursor_line_start_non_whitespace(
        &mut self,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let start = selection.head().to_point(buffer).unwrap();
                let line = buffer.line(start.row).unwrap();
                let non_whitespace =
                    line.chars().position(|c| !c.is_whitespace()).unwrap_or(0) as u32;
                Point::new(start.row, non_whitespace)
            },
            ctx,
        );
    }

    pub fn cursor_line_start(&mut self, keep_selection: bool, ctx: &mut ModelContext<Self>) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let start = selection.head().to_point(buffer).unwrap();
                Point::new(start.row, 0)
            },
            ctx,
        );
    }

    /// Move selection start point to the start of the current line.
    fn selection_line_start(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            let Ok(mut start_point) = selection.start().to_point(buffer) else {
                continue;
            };
            start_point.column = 0;
            let Ok(start_anchor) = buffer.anchor_before(start_point) else {
                continue;
            };
            selection.set_start(start_anchor);
        }
        self.change_selections(new_selections, ctx);
    }

    #[cfg(test)]
    pub fn selection_line_start_test(&mut self, ctx: &mut ModelContext<Self>) {
        self.selection_line_start(ctx)
    }

    /// Move selection end point to the end of the current line.
    fn selection_line_end(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            let Ok(mut end_point) = selection.end().to_point(buffer) else {
                continue;
            };
            let Ok(line_len) = buffer.line_len(end_point.row) else {
                continue;
            };
            end_point.column = line_len;
            let Ok(end_anchor) = buffer.anchor_before(end_point) else {
                continue;
            };
            selection.set_end(end_anchor);
        }
        self.change_selections(new_selections, ctx);
    }

    pub fn cursor_line_end(&mut self, keep_selection: bool, ctx: &mut ModelContext<Self>) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let end = selection.end().to_point(buffer).unwrap();
                Point::new(end.row, buffer.line_len(end.row).unwrap())
            },
            ctx,
        );
    }

    pub fn move_to_buffer_end(&mut self, keep_selection: bool, ctx: &mut ModelContext<Self>) {
        self.move_cursor(keep_selection, |buffer, _| buffer.max_point(), ctx);
    }

    /// Whichever columns the selection range(s) start and end on their respective rows, extend the
    /// range to include whole lines. Set `include_newline` to `true` to add a trailing (preferred)
    /// or leading newline.
    pub fn extend_selection_linewise(
        &mut self,
        include_newline: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selection_line_start(ctx);
        self.selection_line_end(ctx);
        if include_newline {
            self.include_newline_in_selection(ctx);
        }
    }

    /// Like [`Self::extend_selection_linewise`] except this accounts for the case where visual mode
    /// allows the block cursor to be on top of the newline at the end of a line.
    fn extend_selection_linewise_visual_mode(
        &mut self,
        include_newline: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selection_line_start(ctx);

        // Specialized "line end" logic here
        self.move_cursor(
            /* keep_selection */ true,
            |buffer, selection| {
                let end = selection.end().to_point(buffer).unwrap();
                // If the selection ends on column 0, that's actually because the block cursor was
                // on the newline of the above line. In that case, treat it as already on the line
                // end.
                if end.column == 0 {
                    end
                } else {
                    Point::new(end.row, buffer.line_len(end.row).unwrap())
                }
            },
            ctx,
        );

        if include_newline {
            self.include_newline_in_selection(ctx);
        }
    }

    /// This method does Vim's "%" command. This command checks if there is a bracket under the
    /// cursor, or to the right of the cursor on the same line. If so, jump to the bracket that
    /// matches it.
    pub fn vim_move_cursor_to_matching_bracket(
        &mut self,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let offset = selection
                    .head()
                    .to_char_offset(buffer)
                    .expect("selection must be a valid CharOffset");

                // Create an iterator to determine the starting point for the search. Vim will only
                // consider starting points on the current line.
                let mut iter = buffer
                    .chars_at(offset)
                    .expect("infallible with a valid CharOffset")
                    .take_while(|c| *c != '\n');

                // Start by checking the char under the current cursor position.
                let Some(c) = iter.next() else {
                    return offset;
                };
                let (bracket, start_offset) = match BracketChar::try_from(c) {
                    // If the current char is a bracket, match that.
                    Ok(bracket) => (bracket, offset),
                    // If not, move forward on this line until we find a bracket, and begin the
                    // search there.
                    Err(_) => match iter
                        .enumerate()
                        .find_map(|(i, c)| Some((i, BracketChar::try_from(c).ok()?)))
                    {
                        None => return offset,
                        Some((i, bracket)) => (bracket, offset + i + 1),
                    },
                };
                vim_find_matching_bracket(buffer, &bracket, start_offset).unwrap_or(offset)
            },
            ctx,
        );
    }

    /// This method does Vim's `[` command. It moves to the enclosing bracket around the cursor. It
    /// is similar to the `%` command, but it does not require the cursor to start on a bracket.
    pub fn vim_move_cursor_to_unmatched_bracket(
        &mut self,
        bracket: &BracketChar,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let offset = selection
                    .head()
                    .to_char_offset(buffer)
                    .expect("selection must be a valid CharOffset");

                vim_find_matching_bracket(buffer, bracket, offset).unwrap_or(offset)
            },
            ctx,
        );
    }

    /// Expand the selection to the operand for a visual mode command.
    pub fn vim_visual_selection_range(
        &mut self,
        motion_type: MotionType,
        include_newline: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.buffer(ctx);
        let vim_visual_tails = mem::take(&mut self.vim_visual_tails);
        let new_selections = self
            .selections(ctx)
            .iter()
            .zip(vim_visual_tails.iter())
            .filter_map(|(selection, visual_tail)| {
                let mut end = selection.head().to_char_offset(buffer).ok()?;
                let mut start = visual_tail.to_char_offset(buffer).ok()?;
                if start > end {
                    mem::swap(&mut start, &mut end);
                }
                let max_offset = buffer.max_point().to_char_offset(buffer).ok()?;
                // Visual mode includes the char under the block cursor for operators.
                // For linewise, only include +1 if it won't move onto the next line (i.e., the
                // current char at `end` is not a newline).
                if end < max_offset
                    && (motion_type != MotionType::Linewise
                        || buffer
                            .chars_at(end)
                            .is_ok_and(|mut it| it.next().unwrap_or_default() != '\n'))
                {
                    end += 1;
                }
                Some(start..end)
            })
            .collect_vec();
        let _ = self.select_ranges_by_offset(new_selections, ctx);
        if motion_type == MotionType::Linewise {
            self.extend_selection_linewise_visual_mode(include_newline, ctx);
        }
    }

    /// Implements moving left/right using buffer offsets instead of the DisplayMap.
    ///
    /// This is necessary for two reasons:
    ///
    /// 1. The `DisplayMap` doesn't get updated until the current event handler
    /// has finished running.
    /// This means we can't call any function that relies on the `DisplayMap`
    /// in an event handler if the buffer has been edited because it will be
    /// out of date.
    ///
    /// 2. Functions that move the cursor, such as move_up and move_left, rely
    /// on the `DisplayMap` to translate buffer offsets into `DisplayPoints`.
    /// This means those movement functions don't work in unit tests,
    /// unless the window is forcibly painted first.
    ///
    /// Why stop at a line boundary? In Vim mode, moving sideways using h/l
    /// and the left/right arrow keys only navigates within a line, instead
    /// of wrapping around to the line above or below.
    pub fn move_cursors_by_offset(
        &mut self,
        char_count: u32,
        direction: &Direction,
        keep_selection: bool,
        stop_at_line_boundary: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let head = selection
                    .head()
                    .to_char_offset(buffer)
                    .expect("Selection head must be valid CharOffset");

                let offset_change = if stop_at_line_boundary {
                    let head_point = head
                        .to_point(buffer)
                        .expect("Selection head must be valid Point");
                    match direction {
                        // When moving left, there are <current column> characters
                        // between the current position and the start of the line.
                        // Respect the line boundary by moving at most that number
                        // of characters.
                        Direction::Backward => u32::min(head_point.column, char_count),
                        Direction::Forward => {
                            let line_len = buffer
                                .line_len(head_point.row)
                                .expect("Selection head row should have a length");
                            // When moving right, there are (line_len - <current column>)
                            // characters between the current position and the end of the
                            // line. Respect the line boundary by moving at most that
                            // number of characters.
                            u32::min(line_len.saturating_sub(head_point.column), char_count)
                        }
                    }
                } else {
                    char_count
                };

                match direction {
                    Direction::Backward => head.saturating_sub(&(offset_change as usize).into()),
                    Direction::Forward => {
                        let max_offset = buffer
                            .max_point()
                            .to_char_offset(buffer)
                            .expect("Buffer::max_point must be valid CharOffset");
                        cmp::min(max_offset, head + offset_change as usize)
                    }
                }
            },
            ctx,
        );
    }

    /// Implements moving left/right using buffer offsets, skipping past newlines.
    /// See `move_cursors_by_offset` for an explanation of why we use buffer offsets
    /// instead of the DisplayMap.
    ///
    /// This behavior is used by space/backspace navigation in Vim mode.
    pub fn move_cursor_ignoring_newlines(
        &mut self,
        char_count: u32,
        direction: &Direction,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let head = selection
                    .head()
                    .to_char_offset(buffer)
                    .expect("Selection head must be valid CharOffset");

                match direction {
                    Direction::Backward => {
                        let offset_change = buffer
                            .chars_rev_at(head)
                            .expect("Buffer must have characters at the current head.")
                            .enumerate()
                            .fold_while(0, |chars_so_far, (rev_index, c)| {
                                if chars_so_far < char_count {
                                    if c == '\n' {
                                        Continue(chars_so_far)
                                    } else {
                                        Continue(chars_so_far + 1)
                                    }
                                } else {
                                    Done(rev_index as u32)
                                }
                            })
                            .into_inner();
                        head.saturating_sub(&(offset_change as usize).into())
                    }
                    Direction::Forward => {
                        let offset_change = buffer
                            .chars_at(head)
                            .expect("Buffer must have characters at the current selection head.")
                            .enumerate()
                            .fold_while(0, |chars_so_far, (index, c)| {
                                if chars_so_far < char_count {
                                    if c == '\n' {
                                        Continue(chars_so_far)
                                    } else {
                                        Continue(chars_so_far + 1)
                                    }
                                } else {
                                    Done(index as u32)
                                }
                            })
                            .into_inner();
                        let max_offset = buffer
                            .max_point()
                            .to_char_offset(buffer)
                            .expect("Buffer::max_point must be valid CharOffset");
                        cmp::min(max_offset, head + offset_change as usize)
                    }
                }
            },
            ctx,
        );
    }

    /// Implements moving up using buffer offsets instead of the DisplayMap.
    ///
    /// This is necessary for two reasons:
    ///
    /// 1. The `DisplayMap` doesn't get updated until the current event handler
    /// has finished running.
    /// This means we can't call any function that relies on the `DisplayMap`
    /// in an event handler if the buffer has been edited because it will be
    /// out of date.
    ///
    /// 2. Functions that move the cursor, such as move_up and move_left, rely
    /// on the `DisplayMap` to translate buffer offsets into `DisplayPoints`.
    /// This means those movement functions don't work in unit tests,
    /// unless the window is forcibly painted first.
    pub fn move_up_by_offset(&mut self, count: u32, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            let mut point = selection
                .head()
                .to_point(buffer)
                .expect("Selection head must be a valid Point");
            point.row = point.row.saturating_sub(count);
            let goal_column = match selection.goal_end_column {
                Some(goal_column) => cmp::max(goal_column, point.column),
                None => point.column,
            };
            point.column = u32::min(
                goal_column,
                buffer.line_len(point.row).unwrap_or(point.column),
            );
            let Ok(cursor) = buffer.anchor_at(point, AnchorBias::Left) else {
                continue;
            };
            selection.set_selection(Selection::single_cursor(cursor));
            selection.goal_start_column = Some(goal_column);
            selection.goal_end_column = Some(goal_column);
        }
        self.change_selections(new_selections, ctx);
    }

    /// Implements moving down using buffer offsets instead of the DisplayMap.
    ///
    /// This is necessary for two reasons:
    ///
    /// 1. The `DisplayMap` doesn't get updated until the current event handler
    /// has finished running.
    /// This means we can't call any function that relies on the `DisplayMap`
    /// in an event handler if the buffer has been edited because it will be
    /// out of date.
    ///
    /// 2. Functions that move the cursor, such as move_up and move_left, rely
    /// on the `DisplayMap` to translate buffer offsets into `DisplayPoints`.
    /// This means those movement functions don't work in unit tests,
    /// unless the window is forcibly painted first.
    pub fn move_down_by_offset(&mut self, count: u32, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let max_point = buffer.max_point();
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            let mut point = selection
                .head()
                .to_point(buffer)
                .expect("Selection head must be a valid Point");
            point.row = cmp::min(point.row + count, max_point.row);
            let goal_column = match selection.goal_end_column {
                Some(goal_column) => cmp::max(goal_column, point.column),
                None => point.column,
            };
            point.column = cmp::min(
                goal_column,
                buffer.line_len(point.row).unwrap_or(point.column),
            );
            let Ok(cursor) = buffer.anchor_at(point, AnchorBias::Left) else {
                continue;
            };
            selection.set_selection(Selection::single_cursor(cursor));
            selection.goal_start_column = Some(goal_column);
            selection.goal_end_column = Some(goal_column);
        }
        self.change_selections(new_selections, ctx);
    }

    /// See if the character `c` exists on the line of the cursor(s) in the specified direction. If
    /// it does, move the cursor there.
    pub fn vim_find_char(
        &mut self,
        keep_selection: bool,
        occurrence_count: u32,
        motion: &FindCharMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let point = selection
                    .head()
                    .to_point(buffer)
                    .expect("selection head must be a valid Point");
                let Ok(line) = buffer.line(point.row) else {
                    return point;
                };
                let column = point.column as usize;

                if let Some(new_column) =
                    vim_find_char_on_line(&line, column, motion, occurrence_count, keep_selection)
                {
                    Point::new(point.row, new_column as u32)
                } else {
                    point
                }
            },
            ctx,
        );
    }

    /// Toggle case for the selected region.
    /// This function currently only supports a single selection.
    pub fn toggle_selection_case(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        // Currently, this only supports a single selection.
        let selection = self.first_selection(ctx);
        let selection_len = {
            let offsets = selection.to_offset(buffer);
            offsets
                .start
                .as_usize()
                .saturating_sub(offsets.end.as_usize())
        };
        let new_chars = buffer
            .chars_at(selection.start().to_owned())
            .expect("buffer should have characters at the Selection start")
            .take(selection_len)
            .map(|char_at_cursor| match char_at_cursor {
                c if c.is_lowercase() => c.to_uppercase().next().unwrap_or(c),
                c if c.is_uppercase() => c.to_lowercase().next().unwrap_or(c),
                c => c,
            })
            .collect::<String>();

        self.insert_internal(new_chars.as_str(), None, SelectionInsertion::No, ctx);
    }

    /// Uppercase all characters in the selected region.
    /// This function currently only supports a single selection.
    pub fn selection_to_uppercase(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        // Currently, this only supports a single selection.
        let selection = self.first_selection(ctx);
        let uppercased = buffer
            .text_for_range(selection.range(buffer))
            .expect("buffer should have text in the selection")
            .to_uppercase();

        self.insert_internal(uppercased.as_str(), None, SelectionInsertion::No, ctx);
    }

    /// Lowercase all characters in the selected region.
    /// This function currently only supports a single selection.
    pub fn selection_to_lowercase(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        // Currently, this only supports a single selection.
        let selection = self.first_selection(ctx);
        let lowercased = buffer
            .text_for_range(selection.range(buffer))
            .expect("buffer should have text in the selection")
            .to_lowercase();

        self.insert_internal(lowercased.as_str(), None, SelectionInsertion::No, ctx);
    }

    /// Clear any selected regions, leaving the cursor at the end of the first selection
    pub fn clear_selections(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.selections(ctx).is_empty() {
            let buffer = self.buffer(ctx);
            let mut first_selection = self.first_selection(ctx).clone();
            first_selection.set_head(buffer, first_selection.tail().clone());
            self.change_selections(vec1![first_selection], ctx);
        }
    }

    pub fn clear_buffer(&mut self, ctx: &mut ModelContext<Self>) {
        self.buffer_handle().update(ctx, |buffer, ctx| {
            if let Err(error) = buffer.edit(Some(0.into()..buffer.len()), "", ctx) {
                log::error!("error clearing text: {error}");
            };
        });
        self.clear_selections(ctx);
    }

    /// Replaces the first n characters in the buffer with the given text, while
    /// keeping any existing selections untouched (but consolidating in case
    /// the existing selections are no longer valid after the replacement).
    pub fn replace_first_n_characters(
        &mut self,
        n: CharOffset,
        text: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        self.buffer_handle().update(ctx, |buffer, ctx| {
            if let Err(error) = buffer.edit(Some(0.into()..n), text, ctx) {
                log::error!("error replacing first n chars: {error}");
            };
        });
    }

    /// Replaces the last `n` characters in the buffer with the given text, while
    /// keeping any existing selections untouched (but consolidating in case
    /// the existing selections are no longer valid after the replacement).
    pub fn replace_last_n_characters(
        &mut self,
        n: CharOffset,
        text: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        self.buffer_handle().update(ctx, |buffer, ctx| {
            let len = buffer.len();
            let start = len.saturating_sub(&n);
            if let Err(error) = buffer.edit(Some(start..len), text, ctx) {
                log::error!("error replacing last n chars: {error}");
            };
        });
    }

    pub fn reset_selections_to_point(&mut self, point: &Point, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let cursor = buffer
            .anchor_before(point.to_char_offset(buffer).unwrap())
            .unwrap();

        let mut first_selection = self.first_selection(ctx).clone();
        first_selection.set_selection(Selection::single_cursor(cursor));
        self.change_selections(vec1![first_selection], ctx);
    }

    /// Selects ranges by `CharOffset`s. Note if the selections specified in `ranges` are empty, the
    /// selections are not set since there must be at least one selection.
    pub fn select_ranges_by_offset<T, G>(
        &mut self,
        ranges: T,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()>
    where
        T: IntoIterator<Item = Range<G>>,
        G: ToCharOffset,
    {
        let buffer = self.buffer(ctx);
        let new_selections: Vec<_> = ranges
            .into_iter()
            .flat_map(|range| {
                let start = range.start.to_char_offset(buffer).ok()?;
                let end = range.end.to_char_offset(buffer).ok()?;

                Some(LocalSelection {
                    selection: Selection {
                        start: buffer.anchor_after(start).ok()?,
                        end: buffer.anchor_before(end).ok()?,
                        reversed: false,
                    },
                    clamp_direction: Default::default(),
                    goal_start_column: None,
                    goal_end_column: None,
                })
            })
            .sorted_by(|a, b| {
                a.start()
                    .cmp(b.start(), buffer)
                    .expect("anchors should be comparable")
            })
            .collect();

        // Only set the selections if the new selections are not empty.
        if let Ok(new_selections) = Vec1::try_from_vec(new_selections) {
            self.change_selections(new_selections, ctx);
        }

        Ok(())
    }

    pub fn select_ranges_by_display_point<T>(
        &mut self,
        ranges: T,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()>
    where
        T: IntoIterator<Item = Range<DisplayPoint>>,
    {
        let buffer = self.buffer(ctx);
        let map = self.display_map(ctx);
        let mut new_selections = Vec::new();
        for range in ranges {
            new_selections.push(LocalSelection {
                selection: Selection {
                    start: map.anchor_after(range.start, Bias::Left, ctx)?,
                    end: map.anchor_before(range.end, Bias::Left, ctx)?,
                    reversed: false,
                },
                clamp_direction: Default::default(),
                goal_start_column: None,
                goal_end_column: None,
            });
        }
        new_selections.sort_unstable_by(|a, b| a.start().cmp(b.start(), buffer).unwrap());

        if let Ok(new_selections) = Vec1::try_from_vec(new_selections) {
            self.change_selections(new_selections, ctx);
        }

        Ok(())
    }

    pub fn select_word_right(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            let start_position = selection.start().to_point(buffer).unwrap();
            if let Ok(mut word_ends) = buffer.word_ends_from_offset_exclusive(start_position) {
                let word_end = word_ends.next().unwrap_or(start_position);
                let cursor = buffer.anchor_before(word_end).unwrap();
                selection.set_end(cursor);
            }
        }
        self.change_selections(new_selections, ctx);
    }

    pub fn select_word_left(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            let start_position = selection.start().to_point(buffer).unwrap();
            if let Ok(mut word_starts) =
                buffer.word_starts_backward_from_offset_exclusive(start_position)
            {
                let word_start = word_starts.next().unwrap_or(start_position);
                let cursor = buffer.anchor_before(word_start).unwrap();
                selection.set_start(cursor);
            }
        }
        self.change_selections(new_selections, ctx);
    }

    /// Helper method to get an anchor at the start of the word closest to the given position
    pub fn get_word_start(
        &mut self,
        position: DisplayPoint,
        policy: WordBoundariesPolicy,
        ctx: &ModelContext<Self>,
    ) -> Anchor {
        let app = ctx;
        let buffer = self.buffer(ctx);
        let map = self.display_map(ctx);
        let position = position
            .to_buffer_point(map, Bias::Left, ctx)
            .expect("Should be able to get point");

        // Get the previous word start, including the current position if applicable
        let word_start = buffer
            .word_starts_backward_from_offset_inclusive(position)
            .ok()
            .and_then(|word_starts| word_starts.with_policy(policy).next())
            .unwrap_or(position);

        map.anchor_before(
            word_start
                .to_display_point(map, app)
                .expect("Should be able to get start point of word"),
            Bias::Left,
            app,
        )
        .expect("Should be able to get anchor")
    }

    /// Helper method to get an anchor at the end of the word closest to the given position
    pub fn get_word_end(
        &mut self,
        position: DisplayPoint,
        policy: WordBoundariesPolicy,
        ctx: &ModelContext<Self>,
    ) -> Anchor {
        let app = ctx;
        let buffer = self.buffer(ctx);
        let map = self.display_map(ctx);
        let position = position
            .to_buffer_point(map, Bias::Left, ctx)
            .expect("Should be able to get point");

        // Get the next word ending, including the current position if applicable
        let word_end = match buffer
            .word_ends_from_offset_inclusive(position)
            .ok()
            .and_then(|word_ends| word_ends.with_policy(policy).next())
        {
            Some(end) => end,
            None => position,
        };

        map.anchor_before(
            word_end
                .to_display_point(map, app)
                .expect("Should be able to get start point of word"),
            Bias::Left,
            app,
        )
        .expect("Should be able to get anchor")
    }

    pub fn max_point(&self, app: &AppContext) -> DisplayPoint {
        self.display_map(app).max_point(app)
    }

    pub fn backspace(&mut self, ctx: &mut ModelContext<Self>) {
        // Only select left for empty selections
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            if selection.start().to_point(buffer).unwrap()
                == selection.end().to_point(buffer).unwrap()
            {
                let head = selection
                    .head()
                    .to_char_offset(buffer)
                    .expect("Selection head should exist");
                let cursor = buffer
                    .anchor_before(
                        head.saturating_sub(&1.into())
                            .to_point(buffer)
                            .expect("Delete offset should exist"),
                    )
                    .expect("Offset should be in range");
                selection.set_head(buffer, cursor);
            }
        }

        ctx.emit_a11y_content(AccessibilityContent::new(
            self.selected_text(ctx),
            ", deleted",
            WarpA11yRole::UserAction,
        ));
        self.change_selections(new_selections, ctx);
        self.insert("", None, ctx);
    }

    pub fn delete(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            if selection.start().to_point(buffer).unwrap()
                == selection.end().to_point(buffer).unwrap()
            {
                let head = selection.head().to_char_offset(buffer).unwrap();
                if let Ok(point) = (head + 1).to_point(buffer) {
                    let cursor = buffer.anchor_before(point).unwrap();
                    selection.set_head(buffer, cursor);
                }
            }
        }

        ctx.emit_a11y_content(AccessibilityContent::new(
            self.selected_text(ctx),
            ", deleted",
            WarpA11yRole::UserAction,
        ));
        self.change_selections(new_selections, ctx);
        self.insert("", None, ctx);
    }

    // Deletes the character before and after the cursor.
    pub fn remove_before_and_after_cursor(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            if selection.start().to_point(buffer).unwrap()
                == selection.end().to_point(buffer).unwrap()
            {
                let head = selection.head().to_char_offset(buffer).unwrap();

                if selection.reversed() {
                    selection.set_end(
                        buffer
                            .anchor_before(head.saturating_sub(&1.into()).to_point(buffer).unwrap())
                            .expect("Selection offset should be in range"),
                    );
                } else {
                    selection.set_start(
                        buffer
                            .anchor_before(head.saturating_sub(&1.into()).to_point(buffer).unwrap())
                            .expect("Selection offset should be in range"),
                    );
                }

                if let Ok(point) = (head + 1).to_point(buffer) {
                    let cursor = buffer.anchor_before(point).unwrap();
                    selection.set_head(buffer, cursor);
                }
            }
        }
        self.consecutive_autocomplete_insertion_edits_counter -= 1;
        self.change_selections(new_selections, ctx);
        self.insert("", None, ctx);
    }

    /// Selects one word from each cursor
    pub fn vim_select_words(
        &mut self,
        motion: &WordMotion,
        word_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        let WordMotion {
            bound,
            word_type,
            direction,
        } = motion;

        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        new_selections.iter_mut().for_each(|selection| {
            if let Ok(initial_offset) = selection.end().to_char_offset(buffer) {
                if let Ok(boundaries) = vim_word_iterator_from_offset(
                    initial_offset,
                    buffer,
                    *direction,
                    *bound,
                    *word_type,
                ) {
                    let mut word_boundary = boundaries
                        .take(word_count as usize)
                        .last()
                        .unwrap_or(initial_offset);
                    let (new_start, new_end) = match direction {
                        // Account for vim word motion quirks across newlines.
                        Direction::Forward => {
                            // `de`, unlike other word motions, will include character it lands on
                            // in the operation.
                            if *bound == WordBound::End {
                                if let Ok(point) = (word_boundary + 1).to_char_offset(buffer) {
                                    word_boundary = point;
                                }
                            } else if *bound == WordBound::Start && word_count == 1 {
                                // `dw`, can not traverse a newline unless the count > 1. We have
                                // to check this range for newlines and cut the range short in that
                                // case.
                                if let Ok(mut text) =
                                    buffer.chars_for_range(initial_offset..word_boundary)
                                {
                                    if let Some((i, _)) = text.find_position(|c| *c == '\n') {
                                        word_boundary = initial_offset + i;
                                    }
                                }
                            }
                            (initial_offset, word_boundary)
                        }
                        Direction::Backward => {
                            // `db` will traverse *but not delete* a newline if the count is 1 and
                            // the cursor starts on column zero and the line above is not empty.
                            let mut end = initial_offset;
                            if *bound == WordBound::Start && word_count == 1 {
                                if let Ok(mut char_iter) = buffer.chars_rev_at(initial_offset) {
                                    if char_iter.next().is_some_and(|c| c == '\n')
                                        && char_iter.next().is_some_and(|c| c != '\n')
                                    {
                                        end -= 1;
                                    }
                                }
                            }
                            (word_boundary, end)
                        }
                    };

                    if let Ok(anchor_start) = buffer.anchor_before(new_start) {
                        selection.set_start(anchor_start);
                    }
                    if let Ok(anchor_end) = buffer.anchor_before(new_end) {
                        selection.set_end(anchor_end);
                    }
                }
            }
        });
        self.change_selections(new_selections, ctx);
    }

    pub fn vim_select_for_char_motion(
        &mut self,
        motion: &CharacterMotion,
        motion_type: &MotionType,
        operator: &VimOperator,
        operand_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        let include_newline = *operator != VimOperator::Change;
        match motion {
            CharacterMotion::Down => {
                self.extend_selection_below(operand_count, ctx);
            }
            CharacterMotion::Up => {
                self.extend_selection_above(operand_count, ctx);
            }
            CharacterMotion::Left => {
                self.move_cursors_by_offset(
                    operand_count,
                    &Direction::Backward,
                    /* keep_selection */ true,
                    /* stop_at_line_boundary */ true,
                    ctx,
                );
            }
            CharacterMotion::Right => {
                self.move_cursors_by_offset(
                    operand_count,
                    &Direction::Forward,
                    /* keep_selection */ true,
                    /* stop_at_line_boundary */ true,
                    ctx,
                );
            }
            CharacterMotion::WrappingLeft => {
                self.move_cursors_by_offset(
                    operand_count,
                    &Direction::Backward,
                    /* keep_selection */ true,
                    /* stop_at_line_boundary */ false,
                    ctx,
                );
            }
            CharacterMotion::WrappingRight => {
                self.move_cursors_by_offset(
                    operand_count,
                    &Direction::Forward,
                    /* keep_selection */ true,
                    /* stop_at_line_boundary */ false,
                    ctx,
                );
            }
        }
        if *motion_type == MotionType::Linewise {
            self.extend_selection_linewise(include_newline, ctx);
        }
    }

    pub fn vim_select_for_line_motion(
        &mut self,
        motion: &LineMotion,
        count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        match motion {
            LineMotion::Start => {
                self.cursor_line_start(true /* keep_selection */, ctx)
            }
            LineMotion::FirstNonWhitespace => {
                self.cursor_line_start_non_whitespace(true /* keep_selection */, ctx)
            }
            LineMotion::End => self.cursor_line_end(true /* keep_selection */, ctx),
        };
        self.extend_selection_below(count.saturating_sub(1), ctx);
        // Ensure that the seletion covers the whole line below
        // if it happens to be longer than the current one.
        if motion == &LineMotion::End {
            self.cursor_line_end(true, ctx);
        }
    }

    pub fn vim_select_for_first_nonwhitespace_motion(
        &mut self,
        motion: &FirstNonWhitespaceMotion,
        motion_type: &MotionType,
        operator: &VimOperator,
        operand_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        let include_newline = *operator != VimOperator::Change;
        match motion {
            FirstNonWhitespaceMotion::Down => {
                self.extend_selection_below(operand_count, ctx);
            }
            FirstNonWhitespaceMotion::DownMinusOne => {
                self.extend_selection_below(operand_count - 1, ctx);
            }
            FirstNonWhitespaceMotion::Up => {
                self.extend_selection_above(operand_count, ctx);
            }
        }
        self.cursor_line_start_non_whitespace(true /* keep_selection */, ctx);
        if *motion_type == MotionType::Linewise {
            self.extend_selection_linewise(include_newline, ctx);
        }
    }

    pub fn vim_select_for_matching_bracket(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_move_cursor_to_matching_bracket(/* keep_selection */ true, ctx);

        // Annoyingly, operations which have "%" as an operand will include one
        // additional char on the right side. So, for this case we need to loop
        // through the selections and "manually" increment the selection end
        // after altering the selections with `move_cursor_to_matching_bracket`.
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            // Only increment the selection end if a particular selection is
            // actually selecting a range (is _not_ just a cursor). For each
            // selection, `move_cursor_to_matching_bracket` might have had no
            // effect if a matching bracket wasn't found, and those cases leave
            // the cursor as "only a cursor".
            if !selection.is_cursor_only(buffer) {
                let Ok(end_offset) = selection.end().to_char_offset(buffer) else {
                    continue;
                };
                let Ok(new_anchor) = buffer.anchor_at(end_offset + 1, AnchorBias::Right) else {
                    continue;
                };
                selection.set_end(new_anchor);
            }
        }
        self.change_selections(new_selections, ctx);
    }

    pub fn vim_select_text_object(
        &mut self,
        object_type: &TextObjectType,
        inclusion: TextObjectInclusion,
        operator: &VimOperator,
        ctx: &mut ModelContext<Self>,
    ) {
        let new_selections = {
            let buffer = self.buffer(ctx);
            self.selections(ctx)
                .iter()
                .filter_map(|selection| {
                    let offset = selection.head().to_char_offset(buffer).ok()?;
                    match (object_type, inclusion) {
                        (TextObjectType::Word(word_type), TextObjectInclusion::Around) => {
                            vim_a_word(buffer, offset, *word_type)
                        }
                        (TextObjectType::Word(word_type), TextObjectInclusion::Inner) => {
                            vim_inner_word(buffer, offset, *word_type)
                        }
                        (TextObjectType::Paragraph, TextObjectInclusion::Around) => {
                            vim_a_paragraph(buffer, offset)
                        }
                        (TextObjectType::Paragraph, TextObjectInclusion::Inner) => {
                            vim_inner_paragraph(buffer, offset)
                        }
                        (TextObjectType::Quote(quote_type), TextObjectInclusion::Around) => {
                            vim_a_quote(buffer, offset, *quote_type)
                        }
                        (TextObjectType::Quote(quote_type), TextObjectInclusion::Inner) => {
                            vim_inner_quote(buffer, offset, *quote_type)
                        }
                        (TextObjectType::Block(bracket_type), TextObjectInclusion::Around) => {
                            vim_a_block(buffer, offset, *bracket_type)
                        }
                        (TextObjectType::Block(bracket_type), TextObjectInclusion::Inner) => {
                            let preserve_leading_padding = *operator == VimOperator::Change;
                            vim_inner_block(buffer, offset, *bracket_type, preserve_leading_padding)
                        }
                    }
                })
                .collect_vec()
        };
        let _ = self.select_ranges_by_offset(new_selections, ctx);
        if let TextObjectType::Paragraph = object_type {
            let include_newline = *operator != VimOperator::Change;
            self.extend_selection_linewise(include_newline, ctx);
        }
    }

    pub fn vim_move_by_paragraph(
        &mut self,
        count: u32,
        direction: &Direction,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.buffer(ctx);
        let max_offset = buffer.len();

        self.move_cursor(
            keep_selection,
            |buffer, selection| {
                let Ok(mut offset) = selection.head().to_char_offset(buffer) else {
                    return selection.head().clone();
                };
                match direction {
                    Direction::Forward => {
                        for _ in 0..count {
                            offset = find_next_paragraph_end(buffer, offset).unwrap_or(max_offset);
                        }
                    }
                    Direction::Backward => {
                        for _ in 0..count {
                            offset =
                                find_previous_paragraph_start(buffer, offset).unwrap_or_default();
                        }
                    }
                }
                buffer
                    .anchor_before(offset)
                    .unwrap_or_else(|_| selection.head().clone())
            },
            ctx,
        );
    }

    /// Returns true iff any selection is past the last character in the line.
    /// In Vim mode, this scenario needs to be corrected (see [`Self::vim_enforce_cursor_line_cap`]).
    pub fn vim_needs_line_capping<C: ModelAsRef>(&self, ctx: &C) -> bool {
        let buffer = self.buffer(ctx);
        self.selections(ctx).iter().any(|selection| {
            let start_pt = selection.start_to_point(buffer);
            let end_pt = selection.end_to_point(buffer);
            let line_len = buffer.line_len(end_pt.row).unwrap_or_default();

            line_len > 0 && (start_pt.column >= line_len || end_pt.column >= line_len)
        })
    }

    /// If the cursor is after the last character in the line, move it back
    /// so that it's covering the last character in the line instead.
    pub fn vim_enforce_cursor_line_cap(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        new_selections.iter_mut().for_each(|selection| {
            let start_pt = selection.start_to_point(buffer);
            let end_pt = selection.end_to_point(buffer);
            let line_len = buffer.line_len(end_pt.row).unwrap_or_default();

            if line_len > 0 && start_pt.column >= line_len {
                selection.set_start(
                    buffer
                        .anchor_before(
                            selection
                                .start()
                                .to_char_offset(buffer)
                                .unwrap_or_default()
                                .saturating_sub(&1.into()),
                        )
                        .expect(
                            "Selection start - 1 (checked subtraction) should be a valid Anchor",
                        ),
                );
            }
            if line_len > 0 && end_pt.column >= line_len {
                selection.set_end(
                    buffer
                        .anchor_before(
                            selection
                                .end()
                                .to_char_offset(buffer)
                                .unwrap_or_default()
                                .saturating_sub(&1.into()),
                        )
                        .expect("Selection end - 1 (checked subtraction) should be a valid Anchor"),
                );
            }
        });
        self.change_selections(new_selections, ctx);
    }

    pub fn shell_cut(&mut self, ctx: &mut ModelContext<Self>) {
        let text = self.selected_text(ctx);
        if !text.is_empty() {
            ctx.emit(EditorModelEvent::ShellCut(text));
            self.backspace(ctx);
        }
    }

    /// Returns all local selections that intersect the provided range.
    pub fn local_selections_intersecting_range<'a>(
        &'a self,
        range: Range<DisplayPoint>,
        app: &'a AppContext,
    ) -> impl 'a + Iterator<Item = (&'a LocalSelection, Range<DisplayPoint>)> {
        let map = self.display_map(app);
        self.buffer(app)
            .local_selections()
            .selections_intersecting_range(range.clone(), map, app)
    }

    /// Returns drawable local + remote selections that intersect
    /// the provided range.
    pub fn all_drawable_selections_intersecting_range<'a>(
        &'a self,
        range: Range<DisplayPoint>,
        app: &'a AppContext,
    ) -> impl 'a + Iterator<Item = DrawableSelection> {
        let map = self.display_map(app);
        self.buffer(app)
            .local_selections()
            .drawable_selections_intersecting_range(range.clone(), self.replica_id(app), map, app)
            .chain(self.buffer(app).remote_selections().flat_map(
                move |(replica_id, selections)| {
                    selections.drawable_selections_intersecting_range(
                        range.clone(),
                        replica_id.clone(),
                        map,
                        app,
                    )
                },
            ))
    }

    pub fn pending_selection<'a, C: ModelAsRef>(
        &'a self,
        ctx: &'a C,
    ) -> Option<&'a LocalPendingSelection> {
        self.buffer(ctx).local_selections().pending.as_ref()
    }

    pub fn selections<'a, C: ModelAsRef>(&'a self, ctx: &'a C) -> &'a Vec1<LocalSelection> {
        &self.buffer(ctx).local_selections().selections
    }

    fn is_ephemeral(&self) -> bool {
        self.buffer_and_display_map.ephemeral.is_some()
    }

    fn collaborative_buffer(&self) -> &ModelHandle<Buffer> {
        &self.buffer_and_display_map.regular.0
    }

    fn collaborative_display_map(&self) -> &ModelHandle<DisplayMap> {
        &self.buffer_and_display_map.regular.1
    }

    /// We only expose an immutable reference to the [`Buffer`]
    /// so that callers can query anchor positions and such.
    /// We do _not_ want to expose an arbitrary model handle
    /// because that can otherwise be `update`d.
    pub fn buffer<'a, C: ModelAsRef>(&self, ctx: &'a C) -> &'a Buffer {
        self.buffer_handle().as_ref(ctx)
    }

    /// We only expose an immutable reference to the [`Buffer`]
    /// so that callers can query visible ranges and such.
    /// We do _not_ want to expose an arbitrary model handle
    /// because that can otherwise be `update`d.
    pub fn display_map<'a, C: ModelAsRef>(&self, ctx: &'a C) -> &'a DisplayMap {
        self.display_map_handle().as_ref(ctx)
    }

    fn buffer_handle(&self) -> &ModelHandle<Buffer> {
        self.buffer_and_display_map.buffer_handle()
    }

    fn display_map_handle(&self) -> &ModelHandle<DisplayMap> {
        self.buffer_and_display_map.display_map_handle()
    }

    #[cfg(test)]
    pub fn displayed_text(&self, app: &AppContext) -> String {
        self.display_map(app).text(app)
    }

    pub fn buffer_text<C: ModelAsRef>(&self, ctx: &C) -> String {
        self.buffer(ctx).text()
    }

    /// The length of the buffer as a [`CharOffset`].
    pub fn buffer_len<C: ModelAsRef>(&self, ctx: &C) -> CharOffset {
        self.buffer(ctx).len()
    }

    pub fn last_buffer_text(&self) -> &str {
        self.last_buffer_text.as_str()
    }

    pub fn last_action<C: ModelAsRef>(&self, ctx: &C) -> Option<PlainTextEditorViewAction> {
        self.buffer(ctx).last_action()
    }

    pub fn first_selection<'a, C: ModelAsRef>(&'a self, ctx: &'a C) -> &'a LocalSelection {
        self.buffer(ctx).local_selections().first()
    }

    pub fn last_selection<'a, C: ModelAsRef>(&'a self, ctx: &'a C) -> &'a LocalSelection {
        self.buffer(ctx).local_selections().last()
    }

    pub fn is_single_cursor_only<C: ModelAsRef>(&self, ctx: &C) -> bool {
        self.selections(ctx).len() == 1 && self.selections(ctx)[0].is_cursor_only(self.buffer(ctx))
    }

    #[cfg(test)]
    pub fn is_cursor_only<C: ModelAsRef>(&self, selection: &LocalSelection, ctx: &C) -> bool {
        selection.is_cursor_only(self.buffer(ctx))
    }

    pub fn copy_selection_to_vim_register(
        &self,
        register_name: char,
        motion_type: MotionType,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut text = self.selected_text(ctx);

        // Linewise motions have a special case to handle w.r.t. where the newlines are.
        // All linewise copies should add a trailing, not a leading, newline to the selected
        // content. This is already upheld for most cases when [`Self::extend_selections_to_lines`]
        // had been called, but not when the selection includes the final line in the buffer. In
        // that case, since the final line has no trailing newline to select, a leading newline is
        // selected instead. This method will try to detect that case and correct for it by moving
        // that newline to the end.
        if motion_type == MotionType::Linewise {
            let buffer = self.buffer(ctx);
            let max_point = buffer.max_point();
            // Check if any of the selections extend to the end.
            let selection_at_end = self.selections(ctx).iter().any(|selection| {
                selection
                    .end()
                    .to_point(buffer)
                    .expect("Selection end must be valid Point")
                    == max_point
            });
            if selection_at_end {
                // If they do, first add the trailing newline.
                text.push('\n');
                // We can only be sure we should remove the leading newline if there is only one
                // selection. If there are multiple, it may be that another selection range started
                // on a newline that the user explicitly selected and we don't want to trim in that
                // case.
                #[allow(clippy::assigning_clones)]
                if self.selections(ctx).len() == 1 {
                    text = text.trim_start_matches('\n').to_owned();
                }
            }
        }
        VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
            registers.write_to_register(register_name, text, motion_type, ctx);
        });
    }

    /// Extend the current selection to cover line(s) below its current endpoint.
    /// Ex: buffer contains "abcd\nefg" and self.selections contains "b".
    /// After running this function, self.selections will contain "bcd\ne".
    pub fn extend_selection_below(
        &mut self,
        line_extension_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        if line_extension_count == 0 {
            return;
        }
        let buffer = self.buffer(ctx);
        let max_point = self.max_point(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            // Move selection end to the next lines.
            let Ok(mut end_point) = selection.end().to_point(buffer) else {
                continue;
            };
            if end_point.row + line_extension_count > max_point.row() {
                end_point.row = max_point.row();
                end_point.column = max_point.column();
            } else {
                end_point.row += line_extension_count;
                let line_len = buffer.line_len(end_point.row).unwrap_or(0);
                end_point.column = u32::min(end_point.column, line_len);
            }
            let Ok(anchor) = buffer.anchor_after(end_point) else {
                continue;
            };
            selection.set_end(anchor);
        }
        self.change_selections(new_selections, ctx);
    }

    /// Extend the current selection to cover line(s) above its current start point.
    /// Ex: buffer contains "abcd\nefg" and self.selections contains "f".
    /// After running this function, self.selections will contain "cd\nef".
    pub fn extend_selection_above(
        &mut self,
        line_extension_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        if line_extension_count == 0 {
            return;
        }
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        for selection in new_selections.iter_mut() {
            // Move selection start to the previous lines.
            let Ok(mut start_point) = selection.start().to_point(buffer) else {
                continue;
            };
            start_point.row = start_point.row.saturating_sub(line_extension_count);
            start_point.column = u32::min(
                start_point.column,
                buffer
                    .line_len(start_point.row)
                    .unwrap_or(start_point.column),
            );
            let Ok(start_anchor) = buffer.anchor_before(start_point) else {
                continue;
            };
            selection.set_start(start_anchor);
        }
        self.change_selections(new_selections, ctx);
    }

    pub fn vim_set_visual_tail_to_selection_heads<C: ModelAsRef>(&mut self, ctx: &C) {
        self.vim_visual_tails = self
            .selections(ctx)
            .iter()
            .map(|selection| selection.head().clone())
            .collect();
    }

    pub fn vim_set_visual_tails(&mut self, new_tails: Vec<Anchor>) {
        self.vim_visual_tails = new_tails;
    }

    pub fn is_selecting(&self, app: &AppContext) -> bool {
        self.pending_selection(app).is_some()
    }

    pub fn line_len(&self, display_row: u32, app: &AppContext) -> anyhow::Result<u32> {
        self.display_map(app).line_len(display_row, app)
    }

    pub fn chars_preceding_selections<'a, C: ModelAsRef>(
        &'a self,
        app: &'a C,
    ) -> impl Iterator<Item = Chars<'a>> + 'a {
        let buffer = self.buffer(app);
        self.selections(app)
            .iter()
            .filter_map(|selection| buffer.chars_at(selection.start()).ok())
    }

    pub fn any_selections_span_entire_buffer<C: ModelAsRef>(&self, app: &C) -> bool {
        let buffer = self.buffer(app);
        self.selections(app)
            .iter()
            .any(|selection| selection.spans_entire_buffer(buffer))
    }

    #[cfg(test)]
    pub fn selection_to_byte_offset<C: ModelAsRef>(
        &self,
        selection: &LocalSelection,
        ctx: &C,
    ) -> Range<ByteOffset> {
        selection.to_byte_offset(self.buffer(ctx))
    }
}

/// The private interface.
impl EditorModel {
    fn handle_buffer_event(&mut self, event: &buffer::Event, ctx: &mut ModelContext<Self>) {
        match event {
            buffer::Event::Edited { edit_origin, .. } => ctx.emit(EditorModelEvent::Edited {
                edit_origin: *edit_origin,
            }),
            buffer::Event::SelectionsChanged => ctx.emit(EditorModelEvent::SelectionsChanged),
            buffer::Event::StylesUpdated => ctx.emit(EditorModelEvent::StylesUpdated),
            buffer::Event::UpdatePeers { operations } => ctx.emit(EditorModelEvent::UpdatePeers {
                operations: operations.clone(),
            }),
        }
    }

    fn handle_buffer_event_for_non_collaborative_editor(
        &mut self,
        event: &buffer::Event,
        ctx: &mut ModelContext<Self>,
    ) {
        // For non-collaborative editors, we don't care about fanning out updates to peers.
        if !matches!(event, buffer::Event::UpdatePeers { .. }) {
            self.handle_buffer_event(event, ctx);
        }
    }

    fn handle_display_map_event(
        &mut self,
        event: &display_map::Event,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            display_map::Event::Folded | display_map::Event::Unfolded => {
                ctx.emit(EditorModelEvent::DisplayMapUpdated)
            }
        }
    }

    fn selection_to_char_offset_ranges(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> Vec1<Range<CharOffset>> {
        self.selections(ctx).clone().mapped(|selection| {
            let start = selection.start().to_char_offset(self.buffer(ctx)).unwrap();
            let end = selection.end().to_char_offset(self.buffer(ctx)).unwrap();
            start..end
        })
    }

    // Insert characters around the current selections. For example, if the selection is "abc"
    // and the before_cursor_text is "(" and after_cursor_text is ")", the resulting
    // buffer will be "(abc)".
    fn insert_characters_wrap_around_selection(
        &mut self,
        before_cursor_text: &str,
        after_cursor_text: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let offset_ranges = self.selection_to_char_offset_ranges(ctx);
        let before_cursor_char_len = before_cursor_text.chars().count();
        let after_cursor_char_len = after_cursor_text.chars().count();

        self.buffer_handle().update(ctx, |buffer, ctx| {
            // Inserting the user-typed characters.
            if let Err(error) = buffer.edit(
                offset_ranges
                    .iter()
                    .map(|range| range.start..range.start)
                    .clone(),
                before_cursor_text,
                ctx,
            ) {
                log::error!("error inserting text: {error}");
            };

            // Inserting the autocompleted characters. Because the buffer has
            // changed due to the insertion of user-typed characters, we need to
            // shift the insertion range by its index here.
            if let Err(error) = buffer.edit(
                offset_ranges
                    .iter()
                    .enumerate()
                    .map(|(idx, range)| range.end + idx + 1..range.end + idx + 1)
                    .clone(),
                after_cursor_text,
                ctx,
            ) {
                log::error!("error inserting text: {error}");
            };
        });

        let buffer = self.buffer(ctx);
        let mut delta = before_cursor_char_len;

        let new_selections = offset_ranges.mapped(|range| {
            let range_start = range.start;
            let range_end = range.end;
            let end = buffer.anchor_before(range_end + delta).unwrap();
            let start = buffer.anchor_before(range_start + delta).unwrap();

            delta += before_cursor_char_len + after_cursor_char_len;
            LocalSelection {
                selection: Selection {
                    start,
                    end,
                    reversed: false,
                },
                clamp_direction: Default::default(),
                goal_start_column: None,
                goal_end_column: None,
            }
        });
        self.change_selections(new_selections, ctx);
    }

    fn no_cursor_only_selections(&self, ctx: &mut ModelContext<Self>) -> bool {
        self.selections(ctx)
            .iter()
            .all(|selection| !selection.is_cursor_only(self.buffer(ctx)))
    }

    fn all_cursors_next_character_matches_char(
        &self,
        character: char,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let buffer = self.buffer(ctx);

        self.selections(ctx).iter().all(|selection| {
            let position = selection
                .start()
                .to_point(buffer)
                .expect("Start of selection should exist");

            buffer
                .chars_at(position)
                .unwrap()
                .next()
                .map(|right_char| right_char == character)
                .unwrap_or(false)
        })
    }

    /// Attempt to include a newline in the selection, if there one.
    /// By default, attempt to select the newline following the selection.
    /// If that's not possible, attempt to select the newline before the selection.
    fn include_newline_in_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.buffer(ctx);
        let mut new_selections = self.selections(ctx).clone();
        new_selections.iter_mut().for_each(|selection| {
            let start = selection
                .start()
                .to_char_offset(buffer)
                .expect("current selection start should have valid char offset");

            let end = selection
                .end()
                .to_char_offset(buffer)
                .expect("current selection end should have valid char offset");

            let start_newline = start.as_usize() > 0
                && buffer
                    .chars_at(start.saturating_sub(&1.into()))
                    .is_ok_and(|mut chars| chars.next().unwrap_or_default() == '\n');
            let end_newline = buffer
                .chars_at(end)
                .is_ok_and(|mut chars| chars.next().unwrap_or_default() == '\n');

            // By default, attempt to select the newline following the line text.
            if end_newline {
                if let Ok(point) = (end + 1).to_point(buffer) {
                    selection.set_end(
                        buffer
                            .anchor_before(point)
                            .expect("valid point should be valid anchor"),
                    );
                }

            // If that's not possible, attempt to select the newline before the line text.
            } else if start_newline {
                if let Ok(point) = start.saturating_sub(&1.into()).to_point(buffer) {
                    selection.set_start(
                        buffer
                            .anchor_before(point)
                            .expect("valid point should be valid anchor"),
                    );
                }
            }
        });
        self.change_selections(new_selections, ctx);
    }

    /// Determines if adding the given string `text` to the buffer (which replaces all selections)
    /// would exceed the maximum buffer length.
    fn text_insertion_would_exceed_max_buffer_len(&self, text: &str, app: &AppContext) -> bool {
        let Some(max_buffer_len) = self.max_buffer_len else {
            return false;
        };
        let text_len = text.chars().count();
        let selections = self.selected_text_strings(app);
        // `diff` represents the difference in length that inserting `text` into each
        // selection would result in.
        let diff = selections.iter().fold(0, |running_diff, curr| {
            let diff_from_selection = text_len as isize - curr.chars().count() as isize;
            running_diff + diff_from_selection
        });
        let buffer = self.buffer(app);
        buffer.len().as_usize() as isize + diff > max_buffer_len as isize
    }

    fn text_is_invalid_input_type(&self, text: &str) -> bool {
        match self.valid_input_type {
            ValidInputType::All => false,
            ValidInputType::PositiveInteger => text.parse::<u16>().is_err() && !text.is_empty(),
            ValidInputType::NoSpaces => text.contains(char::is_whitespace),
        }
    }

    fn is_line_foldable(&self, display_row: u32, app: &AppContext) -> bool {
        let max_point = self.max_point(app);
        if display_row >= max_point.row() {
            false
        } else {
            let (start_indent, is_blank) = self.line_indent(display_row, app).unwrap();
            if is_blank {
                false
            } else {
                for display_row in display_row + 1..=max_point.row() {
                    let (indent, is_blank) = self.line_indent(display_row, app).unwrap();
                    if !is_blank {
                        return indent > start_indent;
                    }
                }
                false
            }
        }
    }

    fn line_indent(&self, display_row: u32, app: &AppContext) -> anyhow::Result<(usize, bool)> {
        let mut indent = 0;
        let mut is_blank = true;
        for c in self
            .display_map(app)
            .chars_at(DisplayPoint::new(display_row, 0), app)?
        {
            if c == ' ' {
                indent += 1;
            } else {
                is_blank = c == '\n';
                break;
            }
        }
        Ok((indent, is_blank))
    }

    fn foldable_range_for_line(
        &self,
        start_row: u32,
        app: &AppContext,
    ) -> anyhow::Result<Range<Point>> {
        let map = self.display_map(app);
        let max_point = self.max_point(app);

        let (start_indent, _) = self.line_indent(start_row, app)?;
        let start = DisplayPoint::new(start_row, self.line_len(start_row, app)?);
        let mut end = None;
        for row in start_row + 1..=max_point.row() {
            let (indent, is_blank) = self.line_indent(row, app)?;
            if !is_blank && indent <= start_indent {
                end = Some(DisplayPoint::new(row - 1, self.line_len(row - 1, app)?));
                break;
            }
        }

        let end = end.unwrap_or(max_point);
        Ok(start.to_buffer_point(map, Bias::Left, app)?
            ..end.to_buffer_point(map, Bias::Left, app)?)
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
