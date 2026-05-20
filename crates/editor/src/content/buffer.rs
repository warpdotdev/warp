use core::fmt;
use itertools::{Either, Itertools};
use line_ending::LineEnding;
use markdown_parser::{
    CodeBlockText, FormattedIndentTextInline, FormattedTable, FormattedTaskList, FormattedText,
    FormattedTextFragment, FormattedTextHeader, FormattedTextLine, FormattedTextStyles,
    OrderedFormattedIndentTextInline, parse_markdown, parse_markdown_with_gfm_tables,
};
use num_traits::SaturatingSub;
use pathfinder_color::ColorU;
use rand::{Rng, distributions::Alphanumeric};
use serde_yaml::Mapping;
use std::{
    iter::{self, FusedIterator, once},
    mem,
    ops::Range,
    sync::Arc,
};
use vec1::{Vec1, vec1};
use warp_util::content_version::ContentVersion;

use super::{
    anchor::{Anchor, AnchorSide, Anchors},
    cursor::BufferCursor,
    edit::EditDelta,
    markdown::{BufferMarkdownParser, BufferToFormattedText, ExportedBufferBlocks, MarkdownStyle},
    selection::{Selection, TextStyleBias},
    text::{
        BlockCount, BlockLineBreakBehavior, BlockType, BufferBlockItem, BufferBlockStyle,
        BufferSummary, BufferText, BufferTextStyle, Bytes, CodeBlockType, IndentBehavior,
        LineCount, LinkCount, LinkMarker, MarkerDir, StyleSummary, SyntaxColorId, TextStyles,
        TextStylesWithMetadata, TextSummary, inline_to_text,
    },
    undo::{NonAtomicType, UndoActionType, UndoArg, UndoStack},
    validation::validate_content,
};
use warp_core::{platform::SessionPlatform, safe_error};

use crate::{
    content::{
        anchor::AnchorUpdate,
        core::{CoreEditorAction, CoreEditorActionType, RangeAnchors},
        cursor::BufferSumTree,
        edit::PreciseDelta,
        selection_model::{BufferSelectionModel, SelectionSnapshot},
        text::{ColorMarker, IndentUnit},
        undo::{ReversibleEditorActions, ReversibleSelectionState},
        version::BufferVersion,
    },
    multiline::{self, AnyMultilineString, LF, MultilineString},
    render::model::{EmbeddedItem, RenderedSelection, RenderedSelectionBias, RenderedSelectionSet},
};
use enum_iterator::all;
use string_offset::{ByteOffset, CharOffset};
use sum_tree::{SeekBias, SumTree};
use warpui::{AppContext, Entity, ModelContext};
use warpui::{EntityId, ModelHandle, elements::ListIndentLevel};
use warpui::{
    fonts::Weight,
    text::{TextBuffer, char_slice, point::Point},
};

/// Format of the passed in text.
#[derive(Clone, Debug, Copy)]
pub enum ContentFormat {
    Markdown,
    PlainText,
}

/// Configuration struct that holds all the fields needed to reset the entire editor.
/// This consolidates parameters that were previously passed individually to reset functions.
#[derive(Debug, Clone)]
pub struct InitialBufferState<'a> {
    pub text: &'a str,
    pub format: ContentFormat,
    pub version: ContentVersion,
}

impl<'a> InitialBufferState<'a> {
    /// Create a new InitialBufferState with plain text format
    pub fn plain_text(text: &'a str) -> Self {
        Self {
            text,
            format: ContentFormat::PlainText,
            version: ContentVersion::new(),
        }
    }

    /// Create a new InitialBufferState with markdown format
    pub fn markdown(text: &'a str) -> Self {
        Self {
            text,
            format: ContentFormat::Markdown,
            version: ContentVersion::new(),
        }
    }

    /// Set the content version
    pub fn with_version(mut self, version: ContentVersion) -> Self {
        self.version = version;
        self
    }

    /// Set the format
    pub fn with_format(mut self, format: ContentFormat) -> Self {
        self.format = format;
        self
    }
}

#[derive(Default, Clone, Debug)]
pub enum AutoScrollBehavior {
    /// Do not autoscroll.
    None,
    /// Scroll using the provided autoscroll mode.
    Override(crate::render::model::AutoScrollMode),
    /// Autoscroll to the updated selection.
    #[default]
    Selection,
}

#[derive(Clone, Debug, Copy)]
pub enum ShouldAutoscroll {
    No,
    Yes,
    VerticalOnly,
}

#[derive(Clone, Debug)]
pub enum BufferEvent {
    /// The current content-layer selection has changed. When this happens,
    /// the editor layer should:
    /// 1. Update the rendering layer to re-draw its cursors and selection highlights.
    /// 2. Update any editing/formatting controls with the active styles at the
    ///    new selection
    ///
    /// The editor layer must use the styles included in this event, rather than
    /// re-querying the content model. Otherwise, it will not inherit styles from
    /// deleted text.
    SelectionChanged {
        active_text_styles: TextStylesWithMetadata,
        active_block_type: BlockType,
        should_autoscroll: AutoScrollBehavior,
        buffer_version: BufferVersion,
    },
    ContentChanged {
        delta: EditDelta,
        origin: EditOrigin,
        should_autoscroll: ShouldAutoscroll,
        buffer_version: BufferVersion,
        /// ID of the selection model that triggered the content update. This could be used by the upstream code
        /// editor model to decide whether the content change is triggered by an external source.
        selection_model_id: Option<EntityId>,
    },
    ContentReplaced {
        buffer_version: BufferVersion,
    },
    AnchorUpdated {
        update: Vec<AnchorUpdate>,
        /// Anchors in the active selection model are updated eagerly to prevent race conditions with post-edit
        /// selection updates. All other selection models are updated lazily via events.
        excluding_model: Option<EntityId>,
    },
}

#[derive(Debug, Clone)]
pub enum BufferSelectAction {
    MoveLeft,
    MoveRight,
    ExtendLeft,
    ExtendRight,
    SelectAll,
    SetLastHead {
        offset: CharOffset,
    },
    SetLastSelection {
        head: CharOffset,
        tail: CharOffset,
    },
    AddCursorAt {
        offset: CharOffset,
        clear_selections: bool,
    },
    AddSelection {
        head: CharOffset,
        tail: CharOffset,
        clear_selections: bool,
    },
    // Update the selection offsets but don't change the bias.
    // The Vec must have the same number of elements as the current selections.
    UpdateSelectionOffsets {
        selections: Vec1<SelectionOffsets>,
    },
    // Clear all selections and declare the offsets for new selections.
    // Compared to UpdateSelectionOffsets, this Vec does not have to have the same number of elements as the current selections.
    // This will also clear all selection bias.
    SetSelectionOffsets {
        selections: Vec1<SelectionOffsets>,
    },
}

impl BufferSelectAction {
    pub fn selection_offsets(
        &self,
        current_selections: Option<&Vec1<SelectionOffsets>>,
    ) -> Option<Vec1<SelectionOffsets>> {
        match self {
            Self::AddCursorAt { offset, .. } => Some(Vec1::new(SelectionOffsets {
                head: *offset,
                tail: *offset,
            })),
            Self::AddSelection { head, tail, .. } | Self::SetLastSelection { head, tail } => {
                Some(Vec1::new(SelectionOffsets {
                    head: *head,
                    tail: *tail,
                }))
            }
            Self::SetLastHead { offset } => current_selections.map(|selections| {
                let mut selection = *selections.last();
                selection.head = *offset;
                Vec1::new(selection)
            }),
            Self::UpdateSelectionOffsets { selections }
            | Self::SetSelectionOffsets { selections } => Some(selections.clone()),
            _ => None,
        }
    }

    pub fn with_selection_offsets(self, selections: Vec1<SelectionOffsets>) -> Self {
        match self {
            Self::AddCursorAt {
                clear_selections, ..
            } => Self::AddCursorAt {
                offset: selections.first().head,
                clear_selections,
            },
            Self::AddSelection {
                clear_selections, ..
            } => {
                let selection = *selections.first();
                Self::AddSelection {
                    head: selection.head,
                    tail: selection.tail,
                    clear_selections,
                }
            }
            Self::SetLastHead { .. } | Self::SetLastSelection { .. } => {
                let selection = *selections.first();
                Self::SetLastSelection {
                    head: selection.head,
                    tail: selection.tail,
                }
            }
            Self::UpdateSelectionOffsets { .. } => Self::UpdateSelectionOffsets { selections },
            Self::SetSelectionOffsets { .. } => Self::SetSelectionOffsets { selections },
            action => action,
        }
    }
}

#[derive(Debug)]
pub enum BufferEditAction<'a> {
    Insert {
        text: &'a str,
        style: TextStyles,
        /// Optional text style to override the active text style after the insertion.
        override_text_style: Option<TextStyles>,
    },
    /// Apply a list of insertions at their corresponding char offset range. The text will be inserted as plain text.
    InsertAtCharOffsetRanges {
        edits: &'a Vec1<(String, Range<CharOffset>)>,
    },
    InsertForEachSelection {
        /// A vector of inserted text and delta to shift the active selection.
        texts: &'a Vec1<(String, usize)>,
    },
    InsertBlockItem {
        block_item: BufferBlockItem,
    },
    Enter {
        /// Whether or not to force inserting a newline, instead of rich-text-aware Enter behavior.
        force_newline: bool,
        style: TextStyles,
    },
    InsertFormatted(FormattedText),
    TogglePrefixForLines {
        lines: Vec1<usize>,
        prefix: &'a str,
        remove: bool,
    },
    Backspace,
    /// Delete a character range. Unlike `Backspace`, this will not remove block styles if deleting
    /// at the start of a block. Instead, it's used for word- or line-based deletion.
    Delete(Vec1<Range<CharOffset>>),
    Style(TextStyles),
    Unstyle(TextStyles),
    Link {
        tag: String,
        url: String,
    },
    Unlink,
    StyleBlock(BufferBlockStyle),
    RemovePrefixAndStyleBlocks(BlockType),
    /// Replace the existing buffer using InitialBufferState.
    ReplaceWith(InitialBufferState<'a>),
    /// Inserts placeholder text at the given location.
    InsertPlaceholder {
        text: &'a str,
        location: CharOffset,
    },
    InsertBlockAfterBlockWithOffset {
        block_type: BlockType,
        offset: CharOffset,
    },
    Undo,
    Redo,
    Indent {
        /// The number of units to apply indentation. If the current unit is
        /// tabs and num_unit is 2, the action will add 2 tabs to start of line.
        num_unit: u8,
        shift: bool,
    },
    UpdateCodeBlockTypeAtOffset {
        start: CharOffset,
        code_block_type: CodeBlockType,
    },
    ToggleTaskListAtOffset {
        start: CharOffset,
    },
    ColorCodeBlock {
        offset: CharOffset,
        color: &'a [(Range<ByteOffset>, ColorU)],
    },
    RemoveEmbeddingAtOffset {
        offset_before_marker: CharOffset,
    },
    /// Replace the embedded item at given offset.
    /// This is a system action that is should not
    /// be undo-able.
    ReplaceEmbeddingAtOffset {
        offset_before_marker: CharOffset,
        embedding: Arc<dyn EmbeddedItem>,
    },
    /// VimEvent consisting of inserting given text at a point relative
    /// to the current cursor, then repositioning the cursor by an offset.
    VimEvent {
        text: String,
        insert_point: VimInsertPoint,
        cursor_offset_len: usize,
    },
}

#[derive(Debug)]
pub enum VimInsertPoint {
    /// Before the cursor, like `i`
    BeforeCursor,
    /// Just after the cursor, like `a`
    AtCursor,
    /// At the end of the line, like `A`
    LineEnd,
    /// At the non-whitespace start of the line, like `I`
    LineFirstNonWhitespace,
    /// At the start of the line, like `O` (if a newline is added after it)
    LineStart,
    /// On the next line, like `o`
    NextLine,
}

impl BufferEditAction<'_> {
    fn undo_action_type(&self) -> UndoActionType {
        match &self {
            Self::Insert { .. } => UndoActionType::NonAtomic(NonAtomicType::Insert),
            Self::Backspace => UndoActionType::NonAtomic(NonAtomicType::Backspace),
            _ => UndoActionType::Atomic,
        }
    }

    // Whether the buffer action should autoscroll the viewport.
    fn should_autoscroll(&self, origin: EditOrigin) -> ShouldAutoscroll {
        if matches!(origin, EditOrigin::SystemEdit) {
            return ShouldAutoscroll::No;
        }

        match &self {
            BufferEditAction::Style(_)
            | BufferEditAction::Unstyle(_)
            | BufferEditAction::ReplaceWith(_)
            | BufferEditAction::InsertPlaceholder { .. }
            | BufferEditAction::ColorCodeBlock { .. }
            | BufferEditAction::ToggleTaskListAtOffset { .. }
            | BufferEditAction::UpdateCodeBlockTypeAtOffset { .. }
            | BufferEditAction::ReplaceEmbeddingAtOffset { .. }
            | BufferEditAction::InsertAtCharOffsetRanges { .. } => ShouldAutoscroll::No,
            BufferEditAction::Insert { .. }
            | BufferEditAction::InsertForEachSelection { .. }
            | BufferEditAction::InsertBlockItem { .. }
            | BufferEditAction::Enter { .. }
            | BufferEditAction::InsertFormatted(_)
            | BufferEditAction::Backspace
            | BufferEditAction::Delete(_)
            | BufferEditAction::Link { .. }
            | BufferEditAction::Unlink
            | BufferEditAction::StyleBlock(_)
            | BufferEditAction::RemovePrefixAndStyleBlocks(_)
            | BufferEditAction::InsertBlockAfterBlockWithOffset { .. }
            | BufferEditAction::Undo
            | BufferEditAction::Redo
            | BufferEditAction::Indent { .. }
            | BufferEditAction::RemoveEmbeddingAtOffset { .. }
            | BufferEditAction::TogglePrefixForLines { .. }
            | BufferEditAction::VimEvent { .. } => ShouldAutoscroll::Yes,
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
}

impl EditOrigin {
    pub fn from_user(&self) -> bool {
        matches!(self, EditOrigin::UserTyped | EditOrigin::UserInitiated)
    }
}

#[derive(Debug, Default, Clone)]
pub(super) struct EditResult {
    pub(super) undo_item: Option<UndoArg>,
    pub(super) delta: Option<EditDelta>,
    pub(super) anchor_updates: Vec<AnchorUpdate>,
}

/// This represents the movement of selection offsets.  It tracks the initial position of the head and tail with
/// an anchor so that the movement can be applied after any series of edits.
#[derive(Debug)]
pub(super) struct SelectionDelta {
    delta: usize,
    head_anchor: Anchor,
    tail_anchor: Anchor,
}

/// The head and tail character offsets for a selection.
/// We could use a Range but this is more explicit about the head and tail,
/// and either could be before or after the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionOffsets {
    pub head: CharOffset,
    pub tail: CharOffset,
}

/// This represents some CoreEditorActions as well as what should happen to the
/// selection offsets after the action is applied.  We store the CharOffset delta and anchors
/// for where the selection head and tail should be located before the action was applied.
///
/// Building up a list of these edits allows us to apply all of the CoreEditorActions
/// from many ActionWithSelectionDeltas at once, which is easier for undo/redo.
///
/// We can then use the cursor anchors and deltas to compute where the new selection offsets should
/// be located after the edit.
#[derive(Debug)]
pub(super) struct ActionWithSelectionDelta {
    actions: Vec<CoreEditorAction>,
    selection_delta: SelectionDelta,
}

impl ActionWithSelectionDelta {
    /// Create a new ActionWithSelectionDelta with the given actions.
    /// It will create a new anchor at the `head` and `tail` locations and store the delta.
    pub(super) fn new_with_offsets(
        actions: Vec<CoreEditorAction>,
        anchors: &mut Anchors,
        head: CharOffset,
        tail: CharOffset,
        delta: usize,
        side: AnchorSide,
    ) -> Self {
        let head_anchor = anchors.create_anchor(head, side);
        let tail_anchor = anchors.create_anchor(tail, side);
        Self {
            actions,
            selection_delta: SelectionDelta {
                head_anchor,
                tail_anchor,
                delta,
            },
        }
    }

    /// Create a new ActionWithSelectionDelta with the given actions.
    /// It will create a new anchor at the `start` location and store the delta.
    /// Similar to ::new except the head is the same as the tail.
    pub(super) fn new_for_cursor(
        actions: Vec<CoreEditorAction>,
        anchors: &mut Anchors,
        start: CharOffset,
        delta: usize,
    ) -> Self {
        Self::new_with_offsets(actions, anchors, start, start, delta, AnchorSide::Left)
    }
}

type EmbeddedItemConversion = fn(mapping: Mapping) -> Option<Arc<dyn EmbeddedItem>>;
type TabIndentation = Box<dyn Fn(&BufferBlockStyle, bool) -> IndentBehavior>;

/// An immutable, cheaply cloneable snapshot of a buffer's content at a point in time.
#[derive(Clone)]
pub struct BufferSnapshot {
    content: SumTree<BufferText>,
    byte_len: ByteOffset,
}

impl BufferSnapshot {
    /// Creates a `BufferSnapshot` from plain text, including the initial `BlockMarker`.
    /// Useful for tests that need a snapshot without a full `Buffer` context.
    #[cfg(any(test, feature = "test-util"))]
    pub fn from_plain_text(text: &str) -> Self {
        let mut content = SumTree::new();
        content.push(BufferText::BlockMarker {
            marker_type: BufferBlockStyle::PlainText,
        });
        content.append_str(text);
        let byte_len = content.extent::<ByteOffset>();
        Self { content, byte_len }
    }

    /// Returns a `Bytes` iterator positioned at the start of the content,
    /// suitable for reuse across multiple seeks during tree-sitter parsing.
    pub fn bytes(&self) -> Bytes<'_> {
        Bytes::from_sum_tree(&self.content, ByteOffset::from(0), self.byte_len)
    }
}

/// Model for storing the content of an editor.
pub struct Buffer {
    pub(super) content: SumTree<BufferText>,
    /// The set of anchors to use internally by the buffer. This should not be used directly outside
    /// of the buffer module.
    pub(super) internal_anchors: Anchors,
    pub(super) undo_stack: UndoStack,
    pub(super) embedded_item_conversion: Option<EmbeddedItemConversion>,
    pub(super) tab_indentation: TabIndentation,
    /// Note that content_version is different from buffer_version. Content version is used to uniquely
    /// track each user edited buffer content version. Whereas buffer version represent a unique buffer state
    /// whenever its content changes, regardless of system or user edits. It also increases monotonically and
    /// cannot be set by an external method.
    pub(super) content_version: ContentVersion,
    pub(super) version: BufferVersion,
    /// The line ending mode for the buffer content. This is inferred from the content when the
    /// buffer is loaded or reset, and used when retrieving text with original line endings.
    line_ending_mode: LineEnding,
    /// The session platform, used as a fallback when inferring line endings from content that
    /// has no line endings (e.g. single-line text).
    session_platform: Option<SessionPlatform>,
}

impl Default for Buffer {
    fn default() -> Self {
        let mut content = SumTree::new();

        // The buffer content should always have at least one block marker. In the case of an
        // empty buffer, we will by default insert a plain text start marker.
        content.push(BufferText::BlockMarker {
            marker_type: BufferBlockStyle::PlainText,
        });

        let version = ContentVersion::new();
        Self {
            undo_stack: UndoStack::new(30, version),
            internal_anchors: Anchors::new(),
            content,
            embedded_item_conversion: None,
            tab_indentation: Box::new(|_, _| IndentBehavior::Ignore),
            content_version: version,
            version: BufferVersion::new(),
            line_ending_mode: LineEnding::LF,
            session_platform: None,
        }
    }
}

impl Buffer {
    pub fn new(tab_indentation: TabIndentation) -> Self {
        Self {
            tab_indentation,
            ..Default::default()
        }
    }

    /// Given a line, returns whether the line has the given prefix. Note that this check ignores starting tab stops.
    pub fn line_decorated_with_prefix(&self, line: usize, prefix: &str) -> bool {
        let prefix_char_count = prefix.chars().count();
        let line_start = Point::new(line as u32, 0).to_buffer_char_offset(self);
        let block_type = self.block_type_at_point(line_start);

        let block_style = match block_type {
            BlockType::Item(_) => return false,
            BlockType::Text(style) => style,
        };

        let offset = self.non_tab_stop_offset_at_line(line_start, &block_style);
        let Ok(chars) = TextBuffer::chars_at(self, offset) else {
            return false;
        };

        let mut delta = 0;
        for (c1, c2) in chars.zip(prefix.chars()) {
            if c1 == c2 {
                delta += 1;
            } else {
                break;
            }
        }

        delta == prefix_char_count
    }

    pub fn with_embedded_item_conversion(
        mut self,
        embedded_item_conversion: EmbeddedItemConversion,
    ) -> Self {
        self.embedded_item_conversion = Some(embedded_item_conversion);
        self
    }

    pub fn set_embedded_item_conversion(
        &mut self,
        embedded_item_conversion: EmbeddedItemConversion,
    ) {
        self.embedded_item_conversion = Some(embedded_item_conversion);
    }

    pub fn set_tab_indentation(&mut self, tab_indentation: TabIndentation) {
        self.tab_indentation = tab_indentation;
    }

    /// The line ending mode inferred from the buffer content.
    pub fn line_ending_mode(&self) -> LineEnding {
        self.line_ending_mode
    }

    pub fn set_line_ending_mode(&mut self, mode: LineEnding) {
        self.line_ending_mode = mode;
    }

    pub fn set_session_platform(&mut self, platform: Option<SessionPlatform>) {
        self.session_platform = platform;
    }

    pub fn random_edit<R: Rng>(
        &mut self,
        num: usize,
        seed: &mut R,
        replacement_max_length: usize,
        insertion_max_length: usize,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        for _ in 0..num {
            let text_len = self.len().as_usize();
            let position = seed.gen_range(0..text_len);
            let length = seed.gen_range(0..replacement_max_length.max(text_len - position));
            let new_text = "s".repeat(seed.gen_range(0..insertion_max_length));

            self.edit_internal_first_selection(
                CharOffset::from(position)..CharOffset::from(position + length),
                new_text,
                Default::default(),
                selection_model.clone(),
                ctx,
            );
        }
    }

    pub fn random_style<R: Rng>(
        &mut self,
        num: usize,
        seed: &mut R,
        decoration_max_length: usize,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        for _ in 0..num {
            let text_len = self.len().as_usize();
            let position = seed.gen_range(0..text_len);
            let length = seed.gen_range(0..decoration_max_length.max(text_len - position));
            selection_model.update(ctx, |selection_model, _ctx| {
                selection_model.set_selection_offsets(vec1![SelectionOffsets {
                    tail: CharOffset::from(position),
                    head: CharOffset::from(position + length),
                }])
            });
            self.style_internal(TextStyles::default().bold(), selection_model.clone(), ctx);
        }
    }

    pub fn random<R: Rng>(rng: &mut R, max_length: usize) -> Self {
        let len = rng.gen_range(0..max_length);
        let mut content = SumTree::new();
        content.push(BufferText::BlockMarker {
            marker_type: BufferBlockStyle::PlainText,
        });
        for _ in 0..len {
            // Start a newline 1/20th of the time
            if rng.gen_ratio(1, 20) {
                content.push(BufferText::Newline);
            } else {
                // This is not the most efficient way of generating random strs. But since the method is only
                // used in tests, this should be fine.
                content.append_str(&char::from(rng.sample(Alphanumeric)).to_string());
            }

            // Change a style 1/20th of the time
            if rng.gen_ratio(1, 10) {
                let current_styles: TextStyles = content.extent::<StyleSummary>().into();
                let style = BufferTextStyle::random(rng);
                let dir = if current_styles.exact_match_style(&style) {
                    MarkerDir::End
                } else {
                    MarkerDir::Start
                };
                content.push(BufferText::Marker {
                    marker_type: style,
                    dir,
                });
            }
        }

        // Close any unfinished styles to ensure well-formed content.
        let current_styles: TextStyles = content.extent::<StyleSummary>().into();
        let mut handled_weight = false;
        for style in all::<BufferTextStyle>() {
            let is_weight = style.has_custom_weight();
            if is_weight && handled_weight {
                continue;
            }
            if current_styles.exact_match_style(&style) {
                handled_weight |= is_weight;
                content.push(BufferText::Marker {
                    marker_type: style,
                    dir: MarkerDir::End,
                });
            }
        }

        let version = ContentVersion::new();
        Self {
            content,
            undo_stack: UndoStack::new(30, version),
            embedded_item_conversion: None,
            tab_indentation: Box::new(|_, _| IndentBehavior::Ignore),
            content_version: version,
            internal_anchors: Anchors::new(),
            version: BufferVersion::new(),
            line_ending_mode: LineEnding::LF,
            session_platform: None,
        }
    }

    pub fn reset_undo_stack(&mut self) {
        self.undo_stack.reset(self.content_version)
    }

    /// Character at the given offset.
    pub fn char_at(&self, offset: CharOffset) -> Option<char> {
        let cursor = self.content.cursor::<CharOffset, ()>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        buffer_cursor.char_at(offset)
    }

    /// Construct a [`Buffer`] from formatted text contents.
    ///
    /// If provided, the [`EmbeddedItemConversion`] is used to parse embedded item YAML
    /// descriptions.
    pub fn from_formatted_text(
        text: FormattedText,
        embedded_item_conversion: Option<EmbeddedItemConversion>,
        tab_indentation: TabIndentation,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut buffer = Buffer::new(tab_indentation);
        buffer.embedded_item_conversion = embedded_item_conversion;
        // If the formatted text is empty, keep the default buffer state - empty buffers are
        // invalid.
        if !text.lines.is_empty() {
            // When initializing a buffer from formatted text, we should remove the default
            // plain text marker as well.
            buffer.replace_with_formatted_text(
                CharOffset::zero()..CharOffset::from(1),
                text,
                EditOrigin::UserInitiated,
                selection_model,
                ctx,
            );
        }
        buffer
    }

    pub fn from_plain_text(
        text: &str,
        embedded_item_conversion: Option<EmbeddedItemConversion>,
        tab_indentation: TabIndentation,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut buffer = Buffer::new(tab_indentation);
        buffer.embedded_item_conversion = embedded_item_conversion;

        // If the parsed markdown is empty, keep the default buffer state - empty buffers are invalid.
        if !text.is_empty() {
            buffer.edit_internal_first_selection(
                CharOffset::zero()..CharOffset::from(1),
                text,
                Default::default(),
                selection_model,
                ctx,
            );
        }
        buffer
    }

    pub(crate) fn from_markdown(
        markdown: &str,
        embedded_item_conversion: Option<EmbeddedItemConversion>,
        tab_indentation: TabIndentation,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let parse_fn = if warp_core::features::FeatureFlag::MarkdownTables.is_enabled() {
            parse_markdown_with_gfm_tables
        } else {
            parse_markdown
        };
        let parsed_formatted_text = match parse_fn(markdown) {
            Ok(parsed) => parsed,
            Err(e) => {
                safe_error! {
                    safe: ("Failed to parse markdown to start notebook"),
                    full: ("Failed to parse markdown to start notebook: {e}")
                }

                // Return a default formatted text instead of panicking.
                FormattedText::new(vec![])
            }
        };
        Self::from_formatted_text(
            parsed_formatted_text,
            embedded_item_conversion,
            tab_indentation,
            selection_model,
            ctx,
        )
    }

    fn replace(
        &mut self,
        state: InitialBufferState,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let old_offset = CharOffset::from(0)..self.max_charoffset();
        let replaced_points = self.offset_range_to_point_range(old_offset.clone());
        let callback = self.embedded_item_conversion.take();
        // Unfortunately we couldn't "take" a Box<dyn Fn>. As a hack, replace it with a default implementation.
        let indentation = mem::replace(
            &mut self.tab_indentation,
            Box::new(|_, _| IndentBehavior::Ignore),
        );

        // For replacement, we need to first eagerly update anchors for the active selection model to make sure they are
        // valid before inserting the new content. This will ensure that the anchor update in the modify_first_selection
        // call below is valid.
        selection_model.update(ctx, |selection, _| {
            selection.update_anchors(vec![AnchorUpdate {
                start: old_offset.start,
                old_character_count: old_offset.end.saturating_sub(&old_offset.start).as_usize(),
                new_character_count: 1,
                clamp: true,
            }]);
        });

        // Preserve session_platform across the replacement since *self is reassigned.
        let session_platform = self.session_platform.clone();

        // Compute pre-edit byte range before *self is reassigned.
        let old_byte_start = ByteOffset::from(1);
        let old_byte_end = self.max_byte_offset();

        *self = match state.format {
            ContentFormat::Markdown => Buffer::from_markdown(
                state.text,
                callback,
                indentation,
                selection_model.clone(),
                ctx,
            ),
            ContentFormat::PlainText => Buffer::from_plain_text(
                state.text,
                callback,
                indentation,
                selection_model.clone(),
                ctx,
            ),
        };

        // Infer line ending from the new content and restore session_platform.
        self.session_platform = session_platform;
        self.line_ending_mode =
            multiline::infer_line_ending(state.text, self.session_platform.as_ref());

        // Compute post-edit byte length and end point from the new buffer.
        let new_byte_length = self.max_byte_offset().as_usize().saturating_sub(1);
        let new_end_point = self.max_point();

        let anchor_updates = vec![AnchorUpdate {
            start: old_offset.start,
            old_character_count: old_offset.end.saturating_sub(&old_offset.start).as_usize(),
            new_character_count: self.max_charoffset().as_usize(),
            clamp: true,
        }];

        EditResult {
            undo_item: None,
            delta: Some(EditDelta {
                precise_deltas: vec![PreciseDelta {
                    replaced_range: CharOffset::from(1)..old_offset.end,
                    replaced_points,
                    resolved_range: CharOffset::from(1)..self.max_charoffset(),
                    replaced_byte_range: old_byte_start..old_byte_end,
                    new_byte_length,
                    new_end_point,
                }],
                old_offset,
                new_lines: self.styled_blocks_in_range(
                    CharOffset::from(1)..self.max_charoffset(),
                    StyledBlockBoundaryBehavior::Exclusive,
                ),
            }),
            anchor_updates,
        }
    }

    /// Exports a range of the buffer as HTML.
    pub fn ranges_as_html(
        &self,
        ranges: Vec1<Range<CharOffset>>,
        app: &AppContext,
    ) -> Option<String> {
        let blocks = ExportedBufferBlocks {
            blocks: self
                .styled_blocks_in_ranges(ranges, StyledBlockBoundaryBehavior::InclusiveBlockItems),
            context: app,
        };
        blocks.serialize_styled_blocks().ok()
    }

    pub fn selected_text_as_html(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        app: &AppContext,
    ) -> Option<String> {
        let selected_ranges = selection_model.as_ref(app).selections_to_offset_ranges();
        let full_ranges: Vec<Range<CharOffset>> = selected_ranges
            .into_iter()
            .filter(|range| !self.range_has_partial_table_selection(range.clone()))
            .collect();
        let full_ranges = Vec1::try_from_vec(full_ranges).ok()?;
        self.ranges_as_html(full_ranges, app)
    }

    pub fn selected_text_as_plain_text(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        app: &AppContext,
    ) -> AnyMultilineString {
        let selected_ranges = selection_model.as_ref(app).selections_to_offset_ranges();
        self.clipboard_text_in_ranges(selected_ranges, self.line_ending_mode)
    }

    fn push_undo_item(
        &mut self,
        prev_selection_range: RenderedSelectionSet,
        curr_selection_range: RenderedSelectionSet,
        arg: UndoArg,
        action: UndoActionType,
    ) {
        self.undo_stack.push_new_edit(
            ReversibleEditorActions {
                actions: arg.actions,
                replacement_range: arg.replacement_range,
                selections: ReversibleSelectionState {
                    next: prev_selection_range,
                    reverse: curr_selection_range,
                },
            },
            action,
            self.content_version,
        );
    }

    /// Debug function that returns the internal buffer representation.
    pub fn debug(&self) -> String {
        self.content.debug()
    }

    /// Debug function that returns the internal representation of the current selection.
    pub fn debug_selection(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        app: &AppContext,
    ) -> String {
        let selection = selection_model
            .as_ref(app)
            .selection_to_first_offset_range();
        let cursor = self.content.cursor::<CharOffset, CharOffset>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        buffer_cursor.seek_to_offset_before_markers(selection.start);
        buffer_cursor
            .slice_to_offset_after_markers(selection.end)
            .debug()
    }

    pub fn indent_unit_at_plain_text(&self) -> Option<IndentUnit> {
        match (self.tab_indentation)(&BufferBlockStyle::PlainText, false /* shift */) {
            IndentBehavior::TabIndent(indent_unit) => Some(indent_unit),
            _ => None,
        }
    }

    /// Return the character offset of the previous block marker of a certain block style. If there is no previous block marker for
    /// such style, return None.
    pub fn previous_block_marker_of_type(
        &self,
        offset: CharOffset,
        style: BufferBlockStyle,
    ) -> Option<CharOffset> {
        let mut cursor = self.content.cursor::<CharOffset, BufferSummary>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        let mut target_marker_count = summary.block.block;

        while target_marker_count > BlockCount::zero() {
            let mut block_cursor = self.content.cursor::<BlockCount, CharOffset>();

            let found = block_cursor.seek(&target_marker_count, SeekBias::Left);

            if !found {
                return None;
            }

            match block_cursor.item() {
                Some(BufferText::BlockMarker { marker_type }) if *marker_type == style => {
                    return Some(*block_cursor.start());
                }
                Some(BufferText::BlockMarker { .. }) | Some(BufferText::BlockItem { .. }) => {
                    target_marker_count -= 1;
                }
                _ => {
                    debug_assert!(false, "Encountering unexpected buffer text type. Breaking");
                    return None;
                }
            }
        }

        None
    }

    /// Return the character offset of the next block marker of a certain block style. If there is no next block marker for
    /// such style, return None.
    pub fn next_block_marker_of_type(
        &self,
        offset: CharOffset,
        style: BufferBlockStyle,
    ) -> Option<CharOffset> {
        let mut cursor = self.content.cursor::<CharOffset, BufferSummary>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        let mut target_marker_count = summary.block.block;

        while target_marker_count > BlockCount::zero() {
            let mut block_cursor = self.content.cursor::<BlockCount, CharOffset>();

            let found = block_cursor.seek(&target_marker_count, SeekBias::Right);

            if !found {
                return None;
            }

            match block_cursor.item() {
                Some(BufferText::BlockMarker { marker_type }) if *marker_type == style => {
                    return Some(*block_cursor.start());
                }
                Some(BufferText::BlockMarker { .. }) | Some(BufferText::BlockItem { .. }) => {
                    target_marker_count += 1;
                }
                None => return None,
                other => {
                    debug_assert!(
                        false,
                        "Encountering unexpected buffer text type {other:?}. Breaking"
                    );
                    return None;
                }
            }
        }

        None
    }

    /// Validates that the buffer is internally consistent, panicking if not.
    pub(crate) fn validate(&self, anchors: &Anchors) {
        validate_content(&self.content);
        anchors.validate(&self.content);
    }

    /// Returns a list of start and end offset for each line in the given range of the buffer.
    fn line_ranges_in_range(&self, range: Range<CharOffset>) -> Vec<Range<CharOffset>> {
        // In the edge case where the range is a cursor, just return the range as it is.
        if range.start == range.end {
            return vec![range];
        }

        let mut cursor = self.content.cursor::<CharOffset, BufferSummary>();

        let mut start = range.start;
        let mut offsets = vec![];

        let summary = cursor.summary::<BufferSummary>(&start, SeekBias::Right);
        let mut target_marker_count = (summary.text.lines.row as usize).into();
        while start < range.end {
            let mut block_cursor = self.content.cursor::<LineCount, CharOffset>();
            block_cursor.seek(&target_marker_count, SeekBias::Right);

            offsets.push(start..*block_cursor.start().min(&range.end));
            start = *block_cursor.start() + CharOffset::from(1);
            target_marker_count += 1;
        }

        offsets
    }

    /// Offset of the first character in the current line.
    pub fn containing_line_start(&self, offset: CharOffset) -> CharOffset {
        let mut cursor = self.content.cursor::<CharOffset, BufferSummary>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        let target_marker_count = (summary.text.lines.row as usize).into();
        self.line_start(target_marker_count)
    }

    /// Offset of the first nonwhitespace character in the current line.
    pub fn containing_line_first_nonwhitespace(&self, offset: CharOffset) -> CharOffset {
        let line_start = self.containing_line_start(offset);
        let line_end = self.containing_line_end(offset);
        let line_text = self
            .text_in_range(line_start..line_end - 1) // containing_line_end returns len + 1
            .into_string();

        // Only adjust the cursor if there is nonwhitespace to jump to

        if line_text.trim().is_empty() {
            line_start
        } else {
            self.indented_line_start(offset).unwrap_or(line_start)
        }
    }

    pub fn line_start(&self, line: LineCount) -> CharOffset {
        let mut cursor = self.content.cursor::<LineCount, CharOffset>();
        cursor.seek(&line, SeekBias::Left);
        *cursor.start() + CharOffset::from(1)
    }

    /// Offset of the last character in the current line plus one (exclusive end).
    ///
    /// If the current line is plain text and the last line of the buffer, this offset will be one
    /// past the end of the buffer due to the assumption that all lines end in an explicit
    /// line-breaking character.
    pub fn containing_line_end(&self, offset: CharOffset) -> CharOffset {
        let mut cursor = self.content.cursor::<CharOffset, BufferSummary>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        let target_marker_count = (summary.text.lines.row as usize).into();
        self.line_end(target_marker_count)
    }

    pub fn line_end(&self, line: LineCount) -> CharOffset {
        let mut cursor = self.content.cursor::<LineCount, CharOffset>();
        cursor.seek(&line, SeekBias::Right);
        *cursor.start() + CharOffset::from(1)
    }

    // Note that link marker is slightly different from block markers as they have both start and end.
    // To get to the end of a link, we just need to increment the current link count by one.
    fn link_count_at_offset(&self, offset: CharOffset) -> Option<LinkCount> {
        let mut cursor = self.content.cursor::<CharOffset, LinkCount>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        if !summary.style_summary().text_styles().is_link() {
            return None;
        }

        Some(*cursor.start())
    }

    fn color_count_at_offset(&self, offset: CharOffset) -> Option<SyntaxColorId> {
        let mut cursor = self.content.cursor::<CharOffset, SyntaxColorId>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        if !summary.style_summary().text_styles().is_colored() {
            return None;
        }

        Some(*cursor.start())
    }

    pub fn color_at_offset(&self, offset: CharOffset) -> Option<ColorU> {
        let color_clount = self.color_count_at_offset(offset)?;
        self.content.color_at_color_count(&color_clount)
    }

    /// If the offset is in a link, return the offset of the first character in the link (inclusive).
    pub fn containing_link_start(&self, offset: CharOffset) -> Option<CharOffset> {
        let current_link_count = self.link_count_at_offset(offset)?;
        let mut link_cursor = self.content.cursor::<LinkCount, CharOffset>();
        link_cursor.seek(&current_link_count, SeekBias::Left);
        Some(*link_cursor.start())
    }

    /// If the offset is in a link, return the offset of the character after the link.
    pub fn containing_link_end(&self, offset: CharOffset) -> Option<CharOffset> {
        let current_link_count = self.link_count_at_offset(offset)?;
        let mut link_cursor = self.content.cursor::<LinkCount, CharOffset>();
        link_cursor.seek(&(current_link_count + 1), SeekBias::Left);
        Some(*link_cursor.start())
    }

    // If the offset is in a link, return the url of the link.
    pub fn link_url_at_offset(&self, offset: CharOffset) -> Option<String> {
        let current_link_count = self.link_count_at_offset(offset)?;
        self.content.url_at_link_count(&current_link_count)
    }

    pub fn link_url_at_selection_head(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        app: &AppContext,
    ) -> Option<String> {
        self.link_url_at_offset(selection_model.as_ref(app).first_selection_head())
    }

    /// Block style at the given character offset.
    pub fn block_type_at_point(&self, offset: CharOffset) -> BlockType {
        let block_marker_position = self
            .containing_block_start(offset)
            .saturating_sub(&CharOffset::from(1));

        let mut cursor = self.content.cursor::<CharOffset, ()>();
        cursor.seek(&block_marker_position, SeekBias::Right);

        match cursor.item() {
            Some(BufferText::BlockMarker { marker_type }) => BlockType::Text(marker_type.clone()),
            Some(BufferText::BlockItem { item_type }) => BlockType::Item(item_type.clone()),
            _ => BlockType::Text(BufferBlockStyle::PlainText),
        }
    }

    /// Returns the character offset at the start of the block that contains the
    /// given offset. Note that for plain text, if there are multiple lines, `containing_block_start`
    /// will return the first character in the first line.
    pub(super) fn containing_block_start(&self, offset: CharOffset) -> CharOffset {
        let mut cursor = self.content.cursor::<CharOffset, BufferSummary>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        let target_marker_count = summary.block.block;
        let mut block_cursor = self.content.cursor::<BlockCount, CharOffset>();

        block_cursor.seek(&target_marker_count, SeekBias::Left);
        *block_cursor.start() + CharOffset::from(1)
    }

    /// Returns the ending character offset of the block that contains the given
    /// offset. This is exclusive - the offset of the first character after the end
    /// of the block. Note that for plain text, if there are multiple lines, `containing_block_end`
    /// will return the end offset of the last line.
    pub(super) fn containing_block_end(&self, offset: CharOffset) -> CharOffset {
        let mut cursor = self.content.cursor::<CharOffset, BufferSummary>();

        let summary = cursor.summary::<BufferSummary>(&offset, SeekBias::Right);
        let target_marker_count = summary.block.block;
        let mut block_cursor = self.content.cursor::<BlockCount, CharOffset>();

        block_cursor.seek(&target_marker_count, SeekBias::Right);
        *block_cursor.start() + CharOffset::from(1)
    }

    /// Given a set of character offsets ranges, return the indices of any ranges that overlap any other range.
    /// Also return total overlapping ranges.
    ///
    /// assert_eq!(Buffer::overlapping_ranges(vec![1..5, 8..10, 3..7]), (vec![0, 2], vec![1..7]))
    pub(super) fn overlapping_ranges<T: Copy + Ord + fmt::Debug>(
        mut ranges: Vec<Range<T>>,
    ) -> (Vec<usize>, Vec<Range<T>>) {
        struct OverlapRange<U> {
            indices: Vec<usize>,
            start: U,
            end: U,
        }

        // Sort the ranges by start.
        ranges.sort_by_key(|f| f.start);

        let mut overlapping_ranges = vec![];
        let mut overlapping_indices = vec![];

        let mut current_range = None;

        for (i, range) in ranges.iter().enumerate() {
            current_range = match current_range {
                None => Some(OverlapRange {
                    indices: vec![i],
                    start: range.start,
                    end: range.end,
                }),
                Some(mut overlap) if range.start <= overlap.end => {
                    overlap.indices.push(i);
                    overlap.end = range.end.max(overlap.end);
                    Some(overlap)
                }
                Some(overlap) => {
                    if overlap.indices.len() > 1 {
                        overlapping_ranges.push(overlap.start..overlap.end);
                        overlapping_indices.extend(overlap.indices);
                    }
                    Some(OverlapRange {
                        indices: vec![i],
                        start: range.start,
                        end: range.end,
                    })
                }
            }
        }

        if let Some(overlap) = current_range
            && overlap.indices.len() > 1
        {
            overlapping_ranges.push(overlap.start..overlap.end);
            overlapping_indices.extend(overlap.indices);
        }

        (overlapping_indices, overlapping_ranges)
    }

    fn extend_selection_left(
        &mut self,
        character_count: usize,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut selections = selection_model.as_ref(ctx).selections().clone();
        for selection in selections.iter_mut() {
            let head_offset = selection_model
                .as_ref(ctx)
                .resolve_anchor(selection.head())
                .expect("anchor should exist");

            let offset = self.clamp(head_offset.saturating_sub(&CharOffset::from(character_count)));
            selection_model.update(ctx, |selection_model, _| {
                selection_model.set_clamped_selection_head(selection, offset);
            });
        }

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selections(selections);
        });
    }

    fn extend_selection_right(
        &mut self,
        character_count: usize,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut selections = selection_model.as_ref(ctx).selections().clone();
        for selection in selections.iter_mut() {
            let head_offset = selection_model
                .as_ref(ctx)
                .resolve_anchor(selection.head())
                .expect("anchor should exist");

            let offset = self.clamp(head_offset + CharOffset::from(character_count));
            selection_model.update(ctx, |selection_model, _| {
                selection_model.set_clamped_selection_head(selection, offset);
            });
        }

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selections(selections);
        });
    }

    fn move_selection_left(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut selections = selection_model.as_ref(ctx).selections().clone();
        for selection in selections.iter_mut() {
            let head_offset = selection_model
                .as_ref(ctx)
                .resolve_anchor(selection.head())
                .expect("anchor should exist");
            let tail_offset = selection_model
                .as_ref(ctx)
                .resolve_anchor(selection.tail())
                .expect("anchor should exist");

            // If selection is a single cursor, move both head and tail to the left.
            if selection_model
                .as_ref(ctx)
                .selection_is_single_cursor(selection)
            {
                let inline_code_boundary = Buffer::inline_style_boundary_at(
                    &self.content,
                    BufferTextStyle::InlineCode,
                    head_offset,
                );

                match inline_code_boundary {
                    Some(BoundaryEdge::Start) if selection.bias() == TextStyleBias::InStyle => {
                        selection.set_bias(TextStyleBias::OutOfStyle)
                    }
                    Some(BoundaryEdge::End) if selection.bias() == TextStyleBias::OutOfStyle => {
                        selection.set_bias(TextStyleBias::InStyle)
                    }
                    _ => {
                        let new_offset =
                            self.clamp(head_offset.saturating_sub(&CharOffset::from(1)));
                        selection_model.update(ctx, |selection_model, _| {
                            selection_model.set_clamped_selection_head(selection, new_offset);
                            selection_model.set_clamped_selection_tail(selection, new_offset);
                        });

                        let after_code_boundary = Buffer::inline_style_boundary_at(
                            &self.content,
                            BufferTextStyle::InlineCode,
                            new_offset,
                        );
                        match after_code_boundary {
                            Some(BoundaryEdge::Start) => selection.set_bias(TextStyleBias::InStyle),
                            Some(BoundaryEdge::End) => {
                                selection.set_bias(TextStyleBias::OutOfStyle)
                            }
                            _ => selection.set_bias(TextStyleBias::OutOfStyle),
                        }
                    }
                }
            // If selection is not a single cursor and tail is to the right of head,
            // move selection tail to the same position as head.
            } else if head_offset < tail_offset {
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.set_clamped_selection_tail(selection, head_offset);
                });
            // If selection is not a single cursor and tail is to the left of head,
            // move selection head to the same position as tail.
            } else {
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.set_clamped_selection_head(selection, tail_offset);
                });
            }
        }

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selections(selections);
        });
    }

    fn move_selection_right(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut selections = selection_model.as_ref(ctx).selections().clone();
        for selection in selections.iter_mut() {
            let head_offset = selection_model
                .as_ref(ctx)
                .resolve_anchor(selection.head())
                .expect("anchor should exist");
            let tail_offset = selection_model
                .as_ref(ctx)
                .resolve_anchor(selection.tail())
                .expect("anchor should exist");

            // If selection is a single cursor, move both head and tail to the right.
            if selection_model
                .as_ref(ctx)
                .selection_is_single_cursor(selection)
            {
                let inline_code_boundary = Buffer::inline_style_boundary_at(
                    &self.content,
                    BufferTextStyle::InlineCode,
                    head_offset,
                );

                match inline_code_boundary {
                    Some(BoundaryEdge::Start) if selection.bias() == TextStyleBias::OutOfStyle => {
                        selection.set_bias(TextStyleBias::InStyle)
                    }
                    Some(BoundaryEdge::End) if selection.bias() == TextStyleBias::InStyle => {
                        selection.set_bias(TextStyleBias::OutOfStyle)
                    }
                    _ => {
                        let new_offset = self.clamp(head_offset + CharOffset::from(1));
                        selection_model.update(ctx, |selection_model, _| {
                            selection_model.set_clamped_selection_head(selection, new_offset);
                            selection_model.set_clamped_selection_tail(selection, new_offset);
                        });

                        let after_code_boundary = Buffer::inline_style_boundary_at(
                            &self.content,
                            BufferTextStyle::InlineCode,
                            new_offset,
                        );
                        match after_code_boundary {
                            Some(BoundaryEdge::Start) => {
                                selection.set_bias(TextStyleBias::OutOfStyle)
                            }
                            Some(BoundaryEdge::End) => selection.set_bias(TextStyleBias::InStyle),
                            _ => selection.set_bias(TextStyleBias::OutOfStyle),
                        }
                    }
                }
            // If selection is not a single cursor and tail is to the right of head,
            // move selection head to the same position as tail.
            } else if head_offset < tail_offset {
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.set_clamped_selection_head(selection, tail_offset);
                });
            // If selection is not a single cursor and tail is to the left of head,
            // move selection tail to the same position as head.
            } else {
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.set_clamped_selection_tail(selection, head_offset);
                });
            }
        }

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selections(selections);
        });
    }

    /// Update the selection offsets to the provided values.
    /// This does not affect the bias of the selections.
    /// The number of offsets provided must match the number of active selections.
    fn update_selection_offsets(
        &mut self,
        selections: Vec1<SelectionOffsets>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Clamp the provided selections and set them
        let clamped_selections = selections.mapped(|offsets| SelectionOffsets {
            head: self.clamp(offsets.head),
            tail: self.clamp(offsets.tail),
        });
        selection_model.update(ctx, |selection_model, _| {
            selection_model.update_selection_offsets(clamped_selections);
        });
    }

    fn add_cursor(
        &mut self,
        offset: CharOffset,
        clear_selections: bool,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.add_selection(offset, offset, clear_selections, selection_model, ctx);
    }

    fn add_selection(
        &mut self,
        head: CharOffset,
        tail: CharOffset,
        clear_selections: bool,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        let head = self.clamp(head);
        let tail = self.clamp(tail);

        if clear_selections {
            selection_model.update(ctx, |selection_model, _| {
                selection_model.set_selection_offsets(vec1![SelectionOffsets { head, tail }]);
            });
        } else {
            let mut selections = selection_model.as_ref(ctx).selection_offsets();
            selections.push(SelectionOffsets { head, tail });
            selection_model.update(ctx, |selection_model, _| {
                selection_model.set_selection_offsets(selections);
            });
        }
    }

    fn set_last_head(
        &mut self,
        offset: CharOffset,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut selections = selection_model.as_ref(ctx).selection_offsets();
        selections.last_mut().head = self.clamp(offset);
        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selection_offsets(selections);
        });
    }

    pub fn update_selection(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        action: BufferSelectAction,
        should_autoscroll: AutoScrollBehavior,
        ctx: &mut ModelContext<Self>,
    ) {
        let snapshot = self.snapshot_selection(selection_model.clone(), ctx);

        // Before updating any selections, squash any overlapping selections.
        selection_model.update(ctx, |selection_model, _| {
            selection_model.merge_overlapping_selections();
        });

        log::debug!("Before selection action {action:?}",);
        match action {
            // TODO: This need to be updated to account for emojis and multi-character grapheme clusters.
            BufferSelectAction::ExtendLeft => {
                self.extend_selection_left(1, selection_model.clone(), ctx)
            }
            BufferSelectAction::ExtendRight => {
                self.extend_selection_right(1, selection_model.clone(), ctx)
            }
            BufferSelectAction::MoveLeft => self.move_selection_left(selection_model.clone(), ctx),
            BufferSelectAction::MoveRight => {
                self.move_selection_right(selection_model.clone(), ctx)
            }
            BufferSelectAction::SelectAll => {
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.set_selection_offsets(vec1![SelectionOffsets {
                        tail: CharOffset::from(1),
                        head: self.max_charoffset(),
                    }]);
                });
            }
            BufferSelectAction::SetLastHead { offset } => {
                self.set_last_head(offset, selection_model.clone(), ctx)
            }
            BufferSelectAction::SetLastSelection { head, tail } => {
                let mut selections = selection_model.as_ref(ctx).selection_offsets();
                let last_selection = selections.last_mut();
                last_selection.head = self.clamp(head);
                last_selection.tail = self.clamp(tail);
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.set_selection_offsets(selections);
                });
            }
            BufferSelectAction::AddCursorAt {
                offset,
                clear_selections,
            } => {
                self.add_cursor(offset, clear_selections, selection_model.clone(), ctx);
            }
            BufferSelectAction::UpdateSelectionOffsets { selections } => {
                self.update_selection_offsets(selections, selection_model.clone(), ctx);
            }
            BufferSelectAction::SetSelectionOffsets { selections } => {
                let clamped = selections.mapped(|offsets| SelectionOffsets {
                    head: self.clamp(offsets.head),
                    tail: self.clamp(offsets.tail),
                });
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.set_selection_offsets(clamped);
                });
            }
            BufferSelectAction::AddSelection {
                head,
                tail,
                clear_selections,
            } => {
                self.add_selection(head, tail, clear_selections, selection_model.clone(), ctx);
            }
        };

        let updated = self.snapshot_selection(selection_model.clone(), ctx);

        log::debug!(
            "After selection action selection is at {:?} (content length is {})",
            updated.selections,
            self.max_charoffset()
        );

        // After selection changed, we should always create a new undo item.
        self.undo_stack.clear_previous_non_atomic_type();

        if snapshot == updated {
            log::debug!("No-op selection change; will not emit an update.");
        } else {
            log::debug!("Selection changed; emitting an update.");
            ctx.emit(BufferEvent::SelectionChanged {
                active_text_styles: updated.active_text_styles,
                active_block_type: updated.active_block_type,
                should_autoscroll,
                buffer_version: self.version,
            });
        }
    }

    pub fn replace_embedding_at_offset(
        &mut self,
        offset: CharOffset,
        embedding: Arc<dyn EmbeddedItem>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_content(
            BufferEditAction::ReplaceEmbeddingAtOffset {
                offset_before_marker: offset,
                embedding,
            },
            EditOrigin::SystemEdit,
            selection_model,
            ctx,
        );
    }

    /// Update content, with default autoscrolling behavior.
    pub(crate) fn update_content(
        &mut self,
        action: BufferEditAction,
        origin: EditOrigin,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        let should_autoscroll = action.should_autoscroll(origin);
        self.update_content_with_autoscroll(
            action,
            origin,
            should_autoscroll,
            selection_model,
            ctx,
        );
    }

    /// Update content, with manually set autoscrolling behavior.
    pub(crate) fn update_content_with_autoscroll(
        &mut self,
        action: BufferEditAction,
        origin: EditOrigin,
        should_autoscroll: ShouldAutoscroll,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Merge any overlapping selections before we begin editing.
        let range = selection_model.update(ctx, |selection_model, _ctx| {
            selection_model.merge_overlapping_selections();
            selection_model.selection_to_first_offset_range()
        });

        let mut text_style_override = None;
        let prev_selection_range = self.to_rendered_selection_set(selection_model.clone(), ctx);

        log::debug!("Applying {action:?}");
        let undo_action_type = if range.start == range.end {
            action.undo_action_type()
        } else {
            // Always create a new undo item if the active selection is not a cursor.
            UndoActionType::Atomic
        };
        let is_undo_redo = matches!(action, BufferEditAction::Redo | BufferEditAction::Undo);
        let mut is_replace = false;
        let edit_result = match action {
            // We need to override active text style with the style of the deleted fragment for deletions.
            BufferEditAction::Backspace => {
                self.backspace(&mut text_style_override, selection_model.clone(), ctx)
            }
            BufferEditAction::Delete(delete_ranges) => self.delete(
                &mut text_style_override,
                delete_ranges,
                selection_model.clone(),
                ctx,
            ),
            BufferEditAction::InsertAtCharOffsetRanges { edits } => {
                self.insert_at_offsets(edits, selection_model.clone(), ctx)
            }
            BufferEditAction::Enter {
                force_newline,
                style,
            } => self.enter(force_newline, style, selection_model.clone(), ctx),
            BufferEditAction::Insert {
                text,
                style,
                override_text_style,
            } => {
                text_style_override = override_text_style
                    .map(|style| TextStylesWithMetadata::from_text_styles(style, None, None));
                self.edit_internal(text, style, selection_model.clone(), ctx)
            }
            BufferEditAction::InsertForEachSelection { texts } => {
                self.edit_for_each_selection(texts, selection_model.clone(), ctx)
            }
            BufferEditAction::TogglePrefixForLines {
                lines,
                prefix,
                remove,
            } => {
                if remove {
                    self.remove_prefix_from_lines(lines, prefix, selection_model.clone(), ctx)
                } else {
                    self.decorate_lines_with_prefix(lines, prefix, selection_model.clone(), ctx)
                }
            }
            BufferEditAction::InsertFormatted(text) => self.insert_formatted_text_at_selections(
                text,
                EditOrigin::UserInitiated,
                selection_model.clone(),
                ctx,
            ),
            BufferEditAction::Style(style) => {
                self.style_internal(style, selection_model.clone(), ctx)
            }
            BufferEditAction::Unstyle(style) => {
                self.unstyle_internal(style, selection_model.clone(), ctx)
            }
            BufferEditAction::Link { tag, url } => {
                self.style_link_internal(tag, url, selection_model.clone(), ctx)
            }
            BufferEditAction::Unlink => self.unstyle_link_internal(selection_model.clone(), ctx),
            BufferEditAction::StyleBlock(style) => {
                self.block_style_internal(style, selection_model.clone(), ctx)
            }
            BufferEditAction::RemovePrefixAndStyleBlocks(block_type) => {
                self.remove_prefix_and_style_blocks(block_type, selection_model.clone(), ctx)
            }
            BufferEditAction::ReplaceWith(state) => {
                is_replace = true;
                self.replace(state, selection_model.clone(), ctx)
            }
            BufferEditAction::InsertPlaceholder { text, location } => {
                self.insert_placeholder(location, text, selection_model.clone(), ctx)
            }
            BufferEditAction::ColorCodeBlock { offset, color } => {
                self.color_code_block_ranges_internal(offset, color)
            }
            BufferEditAction::InsertBlockItem { block_item } => {
                self.insert_block_item(block_item, range)
            }
            BufferEditAction::InsertBlockAfterBlockWithOffset { block_type, offset } => self
                .insert_block_after_block_with_offset(
                    offset,
                    block_type,
                    selection_model.clone(),
                    ctx,
                ),
            BufferEditAction::Indent {
                num_unit: unit,
                shift,
            } => {
                if shift {
                    // TODO: We should support multi-unit unindent as well.
                    self.unindent(selection_model.clone(), ctx)
                } else {
                    self.indent(unit, selection_model.clone(), ctx)
                }
            }
            BufferEditAction::Undo => self.undo(selection_model.clone(), ctx),
            BufferEditAction::Redo => self.redo(selection_model.clone(), ctx),
            BufferEditAction::ToggleTaskListAtOffset { start } => {
                let end = self.containing_block_end(start + 1);
                let block_type = self.block_type_at_point(start + 1);

                if let BlockType::Text(BufferBlockStyle::TaskList {
                    indent_level,
                    complete,
                }) = block_type
                {
                    self.block_style_range(
                        (start + 1)..(end - 1),
                        BufferBlockStyle::TaskList {
                            indent_level,
                            complete: !complete,
                        },
                        selection_model.clone(),
                        ctx,
                    )
                } else {
                    EditResult::default()
                }
            }
            BufferEditAction::UpdateCodeBlockTypeAtOffset {
                start,
                code_block_type,
            } => {
                // We start the range at the start offset + 1 since the first index is
                // a marker indicating this is a code block. We need a -1 at the end since
                // `containing_block_end` is exclusive
                let end = self.containing_block_end(start + 1);
                self.block_style_range(
                    (start + 1)..(end - 1),
                    BufferBlockStyle::CodeBlock { code_block_type },
                    selection_model.clone(),
                    ctx,
                )
            }
            BufferEditAction::ReplaceEmbeddingAtOffset {
                offset_before_marker,
                embedding,
            } => self.replace_embedding_at_offset_internal(offset_before_marker, embedding),
            BufferEditAction::RemoveEmbeddingAtOffset {
                offset_before_marker,
            } => {
                self.remove_embedding_at_offset(offset_before_marker, selection_model.clone(), ctx)
            }
            BufferEditAction::VimEvent {
                text,
                insert_point,
                cursor_offset_len,
            } => self.vim_event(
                text,
                insert_point,
                cursor_offset_len,
                selection_model.clone(),
                ctx,
            ),
        };

        if !is_undo_redo && edit_result.undo_item.is_none() && edit_result.delta.is_some() {
            debug_assert!(
                matches!(origin, EditOrigin::SystemEdit),
                "Only system edits should have empty undo item with a non-empty render state update"
            );
        }

        if !matches!(origin, EditOrigin::SystemEdit)
            && (edit_result.undo_item.is_some() || edit_result.delta.is_some())
        {
            // Update the content version if there was an edit or undo item.
            self.content_version = ContentVersion::new();
        }

        let curr_selection_range = self.to_rendered_selection_set(selection_model.clone(), ctx);

        if let Some(undo_item) = edit_result.undo_item {
            self.push_undo_item(
                prev_selection_range,
                curr_selection_range,
                undo_item,
                undo_action_type,
            );
        }

        let Some(content_update) = edit_result.delta else {
            log::debug!("Editor action was no-op");
            return;
        };

        self.version = BufferVersion::new();

        if is_replace {
            ctx.emit(BufferEvent::ContentReplaced {
                buffer_version: self.version,
            });
        }

        if !edit_result.anchor_updates.is_empty() {
            // Always clamp when emitting to non-primary selection models. The primary model's
            // anchors are eagerly updated and explicitly repositioned by modify_each_selection,
            // but external observers only receive this event and nobody repositions their
            // selections afterward. Clamping ensures anchors in a deleted range move to the
            // edit boundary rather than being removed, which would leave selections in an
            // unrecoverable invalid state.
            let clamped_updates = edit_result
                .anchor_updates
                .into_iter()
                .map(|u| AnchorUpdate { clamp: true, ..u })
                .collect();
            ctx.emit(BufferEvent::AnchorUpdated {
                update: clamped_updates,
                excluding_model: Some(selection_model.id()),
            });
        }

        log::debug!(
            "Action successful. Will re-render old range {:?} as {} blocks",
            content_update.old_offset,
            content_update.new_lines.len()
        );

        ctx.emit(BufferEvent::SelectionChanged {
            active_text_styles: text_style_override.unwrap_or(
                self.active_style_with_metadata_at_selection(selection_model.as_ref(ctx)),
            ),
            active_block_type: self
                .active_block_type_at_first_selection(selection_model.as_ref(ctx)),
            should_autoscroll: AutoScrollBehavior::None,
            buffer_version: self.version,
        });
        ctx.emit(BufferEvent::ContentChanged {
            delta: content_update,
            origin,
            should_autoscroll,
            buffer_version: self.version,
            selection_model_id: Some(selection_model.id()),
        });
    }

    /// Whether or not the current selection is a single cursor that's within a block type that
    /// allows formatting.
    pub fn can_format_at_cursor(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &AppContext,
    ) -> bool {
        let allows_formatting =
            match self.active_block_type_at_first_selection(selection_model.as_ref(ctx)) {
                BlockType::Item(_) => false,
                BlockType::Text(style) => style.allows_formatting(),
            };
        selection_model
            .as_ref(ctx)
            .first_selection_is_single_cursor()
            && allows_formatting
    }

    // Returns the content from the start of block to start of the selection.
    pub fn content_from_block_start_to_selection_start(&self, start: CharOffset) -> String {
        self.text_in_range(self.block_or_line_start(start)..start)
            .into_string()
    }

    pub fn content_from_line_start_to_selection_start(&self, head: CharOffset) -> String {
        self.text_in_range(self.containing_line_start(head)..head)
            .into_string()
    }

    /// Returns the active text styles at the start of the given range.
    /// This does not take into account the entire selection range, only the start position.
    /// If the range is a single cursor on the boundary of an inline code block, then
    /// the style of the inline code block is dependent on whether the cursor bias is in style or not.
    pub fn active_style_at(
        &self,
        range: Range<CharOffset>,
        bias: TextStyleBias,
    ) -> TextStylesWithMetadata {
        if range.start == range.end {
            let mut metadata =
                self.text_styles_with_metadata_at(range.start.saturating_sub(&CharOffset::from(1)));

            if Buffer::inline_style_boundary_at(
                &self.content,
                BufferTextStyle::InlineCode,
                range.start,
            )
            .is_some()
            {
                let inline_code = metadata.inline_code_mut();
                *inline_code = bias == TextStyleBias::InStyle;
            }

            metadata
        } else {
            self.text_styles_with_metadata_at(range.start)
        }
    }

    pub fn to_rendered_selection_set(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &AppContext,
    ) -> RenderedSelectionSet {
        let selection_model = selection_model.as_ref(ctx);

        selection_model
            .selections()
            .selection_map(|s| self.to_rendered_selection(s, selection_model))
            .into()
    }

    fn to_rendered_selection(
        &self,
        selection: &Selection,
        selection_model: &BufferSelectionModel,
    ) -> RenderedSelection {
        let head_offset = selection_model.selection_head(selection);
        let tail_offset = selection_model.selection_tail(selection);

        let bias = if head_offset == tail_offset {
            match Buffer::inline_style_boundary_at(
                &self.content,
                BufferTextStyle::InlineCode,
                head_offset,
            ) {
                Some(BoundaryEdge::Start)
                    if selection_model.selection().bias() == TextStyleBias::InStyle =>
                {
                    Some(RenderedSelectionBias::Right)
                }
                Some(BoundaryEdge::Start)
                    if selection_model.selection().bias() == TextStyleBias::OutOfStyle =>
                {
                    Some(RenderedSelectionBias::Left)
                }
                Some(BoundaryEdge::End)
                    if selection_model.selection().bias() == TextStyleBias::InStyle =>
                {
                    Some(RenderedSelectionBias::Left)
                }
                Some(BoundaryEdge::End)
                    if selection_model.selection().bias() == TextStyleBias::OutOfStyle =>
                {
                    Some(RenderedSelectionBias::Right)
                }
                _ => None,
            }
        } else {
            None
        };
        RenderedSelection {
            head: head_offset,
            tail: tail_offset,
            cursor_bias: bias,
        }
    }

    // If the old text bias is in style, check if after the edit we are still at a text boundary.
    // If the above is true, we continue to set the text bias to be instyle. Otherwise, reset the bias to be out of style.
    pub(super) fn reset_selection_bias_after_edit(
        &mut self,
        selection: &mut Selection,
        new_selection_head: CharOffset,
    ) {
        let bias = selection.bias();
        if bias == TextStyleBias::InStyle
            && Buffer::inline_style_boundary_at(
                &self.content,
                BufferTextStyle::InlineCode,
                new_selection_head,
            )
            .is_some()
        {
            selection.set_bias(TextStyleBias::InStyle);
        } else {
            selection.set_bias(TextStyleBias::OutOfStyle);
        }
    }

    /// Check whether the offset is at the boundary of an inline style.
    pub(super) fn inline_style_boundary_at(
        content: &SumTree<BufferText>,
        style: BufferTextStyle,
        offset: CharOffset,
    ) -> Option<BoundaryEdge> {
        let cursor = content.cursor::<CharOffset, StyleSummary>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        buffer_cursor.seek_to_offset_before_markers(offset);
        let before_style = buffer_cursor
            .start()
            .text_styles()
            .exact_match_style(&style);

        buffer_cursor.seek_to_offset_after_markers(offset);
        let after_style = buffer_cursor
            .start()
            .text_styles()
            .exact_match_style(&style);

        if before_style && !after_style {
            Some(BoundaryEdge::End)
        } else if !before_style && after_style {
            Some(BoundaryEdge::Start)
        } else {
            None
        }
    }

    /// Clamp an offset to the editable range of the buffer.
    pub(super) fn clamp(&self, offset: CharOffset) -> CharOffset {
        Buffer::clamp_with_max_offset(offset, self.max_charoffset())
    }

    fn clamp_with_max_offset(offset: CharOffset, max_offset: CharOffset) -> CharOffset {
        offset.clamp(CharOffset::from(1), max_offset)
    }

    pub fn markdown(&self) -> String {
        self.to_markdown(MarkdownStyle::Internal)
    }

    /// Serialize the buffer as Markdown without escaping any special characters.
    pub fn markdown_unescaped(&self) -> String {
        self.to_markdown(MarkdownStyle::Export {
            app_context: None,
            should_not_escape_markdown_punctuation: true,
        })
    }

    /// Serialize the buffer as Markdown.
    pub fn to_markdown(&self, style: MarkdownStyle) -> String {
        BufferMarkdownParser::new(
            style,
            self.styled_blocks_at(
                // TODO(ben): does this need the +1?
                CharOffset::from(1)..self.max_charoffset() + 1,
                StyledBlockBoundaryBehavior::Exclusive,
            ),
        )
        .to_markdown()
    }

    pub fn export_to_markdown(
        markdown: FormattedText,
        embedded_item_conversion: Option<EmbeddedItemConversion>,
        style: MarkdownStyle,
    ) -> String {
        let mut buffer = Buffer {
            embedded_item_conversion,
            ..Default::default()
        };
        let edit = buffer.edits_for_formatted_text(
            CharOffset::zero()..CharOffset::from(1),
            markdown,
            EditOrigin::UserInitiated,
        );

        buffer.apply_core_edit_actions(edit.actions);
        buffer.to_markdown(style)
    }

    pub fn range_to_formatted_text(
        &self,
        range: Range<CharOffset>,
        boundary_behavior: StyledBlockBoundaryBehavior,
    ) -> FormattedText {
        BufferToFormattedText::new(self.styled_blocks_at(range, boundary_behavior))
            .to_formatted_text()
    }

    pub fn text_summary(&self) -> TextSummary {
        self.content.extent::<TextSummary>()
    }

    /// Buffer's plain text content (omitting all styles).
    pub fn text(&self) -> MultilineString<LF> {
        self.text_in_range(CharOffset::from(1)..self.max_charoffset())
    }

    /// The text in a single range of characters, as plain text.
    /// This always returns LF-normalized text since the internal buffer representation uses LF.
    /// Use [`text_with_line_ending`] or [`text_with_line_ending_mode`] to get text with a specific line ending.
    pub fn text_in_range(&self, range: Range<CharOffset>) -> MultilineString<LF> {
        let (head, tail) = (range.start.min(range.end), range.end.max(range.start));
        self.text_in_ranges(vec1![head..tail], LineEnding::LF)
            .try_into()
            .expect("Line ending is set to LF")
    }

    /// Returns the buffer's text content using the inferred line ending mode.
    pub fn text_with_line_ending(&self) -> AnyMultilineString {
        self.text_with_line_ending_mode(self.line_ending_mode)
    }

    pub fn text_with_line_ending_mode(&self, line_ending_mode: LineEnding) -> AnyMultilineString {
        self.text_in_ranges(
            vec1![CharOffset::from(1)..self.max_charoffset()],
            line_ending_mode,
        )
    }

    /// The text in the given ranges of characters, as plain text.
    pub fn text_in_ranges(
        &self,
        ranges: Vec1<Range<CharOffset>>,
        line_ending_mode: LineEnding,
    ) -> AnyMultilineString {
        let text = ranges.into_iter().map(|range| {
            let blocks = self
                .styled_blocks_at(range, StyledBlockBoundaryBehavior::InclusiveBlockItems)
                .with_line_end_mode(line_ending_mode);
            blocks
                .into_iter()
                .map(|block| block.content())
                .collect::<String>()
        });
        AnyMultilineString::from_lines(text, line_ending_mode)
    }

    fn clipboard_text_in_ranges(
        &self,
        ranges: Vec1<Range<CharOffset>>,
        line_ending_mode: LineEnding,
    ) -> AnyMultilineString {
        let text = ranges
            .into_iter()
            .map(|range| self.clipboard_text_in_range(range, line_ending_mode));
        AnyMultilineString::from_lines(text, line_ending_mode)
    }

    fn clipboard_text_in_range(
        &self,
        range: Range<CharOffset>,
        line_ending_mode: LineEnding,
    ) -> String {
        let mut text = String::new();
        let mut offset = range.start;

        while offset < range.end {
            let block_start = self.containing_block_start(offset);
            let block_end = self.containing_block_end(offset);
            let segment_end = range.end.min(block_end);
            let segment = offset..segment_end;
            let block_type = self.block_type_at_point(offset);

            match block_type {
                BlockType::Text(BufferBlockStyle::Table { .. }) => {
                    text.push_str(&self.clipboard_table_text_in_range(
                        block_start,
                        segment,
                        line_ending_mode,
                    ));
                }
                _ => {
                    text.push_str(
                        &self
                            .styled_blocks_at(segment, StyledBlockBoundaryBehavior::Exclusive)
                            .with_line_end_mode(line_ending_mode)
                            .map(|block| block.content())
                            .collect::<String>(),
                    );
                }
            }

            offset = segment_end;
        }

        text
    }

    fn clipboard_table_text_in_range(
        &self,
        block_start: CharOffset,
        range: Range<CharOffset>,
        line_ending_mode: LineEnding,
    ) -> String {
        let fallback_text = || {
            self.styled_blocks_at(range.clone(), StyledBlockBoundaryBehavior::Exclusive)
                .with_line_end_mode(line_ending_mode)
                .map(|block| block.content())
                .collect()
        };
        let block_end = self.containing_block_end(block_start);
        let block = self
            .styled_blocks_in_range(
                block_start..block_end,
                StyledBlockBoundaryBehavior::Exclusive,
            )
            .into_iter()
            .next();

        let Some(StyledBufferBlock::Text(table_block)) = block else {
            return fallback_text();
        };

        let BufferBlockStyle::Table { alignments, cache } = &table_block.style else {
            return fallback_text();
        };

        let table_text = table_block
            .block
            .iter()
            .map(|run| run.run.as_str())
            .collect::<String>();
        let cached = cache.get_or_populate(&table_text, alignments);
        let table = &cached.table;
        let cell_offset_maps = &cached.cell_offset_maps;
        let offset_map = &cached.offset_map;

        let cell_text = std::iter::once(&table.headers)
            .chain(table.rows.iter())
            .map(|row| row.iter().map(|cell| inline_to_text(cell)).collect_vec())
            .collect_vec();
        let relative_start = range.start - block_start;
        let relative_end = range.end - block_start;
        let mut text = String::new();

        for (row_idx, row) in cell_text.iter().enumerate() {
            for (col_idx, cell) in row.iter().enumerate() {
                let Some(cell_range) = offset_map.cell_range(row_idx, col_idx) else {
                    continue;
                };
                let Some(cell_offset_map) = cell_offset_maps
                    .get(row_idx)
                    .and_then(|row_maps| row_maps.get(col_idx))
                else {
                    continue;
                };

                if cell_range.end > relative_start && cell_range.start < relative_end {
                    let slice_start = cell_offset_map.source_to_rendered(
                        relative_start.max(cell_range.start) - cell_range.start,
                    );
                    let slice_end = cell_offset_map
                        .source_to_rendered(relative_end.min(cell_range.end) - cell_range.start);
                    if slice_end > slice_start {
                        text.push_str(
                            char_slice(cell, slice_start.as_usize(), slice_end.as_usize())
                                .unwrap_or(cell),
                        );
                    }
                }

                let separator_offset = cell_range.end;
                if relative_start <= separator_offset && separator_offset < relative_end {
                    if col_idx + 1 < row.len() {
                        text.push('\t');
                    } else {
                        text.push_str(line_ending_mode.as_str());
                    }
                }
            }
        }
        text
    }

    fn range_has_partial_table_selection(&self, range: Range<CharOffset>) -> bool {
        let mut offset = range.start;

        while offset < range.end {
            let block_start = self.containing_block_start(offset);
            let block_end = self.containing_block_end(offset);
            let segment_end = range.end.min(block_end);

            if matches!(
                self.block_type_at_point(offset),
                BlockType::Text(BufferBlockStyle::Table { .. })
            ) && (offset > block_start || segment_end < block_end)
            {
                return true;
            }

            offset = segment_end;
        }

        false
    }

    pub fn text_in_ranges_with_expanded_embedded_items(
        &self,
        ranges: Vec1<Range<CharOffset>>,
        app: &AppContext,
    ) -> String {
        let block_strings = ranges
            .iter()
            .map(|range| {
                self.styled_blocks_at(
                    range.start..range.end,
                    StyledBlockBoundaryBehavior::InclusiveBlockItems,
                )
            })
            .map(|blocks| {
                Either::Left(blocks.map(|block| block.content_with_expanded_embedded_items(app)))
            });

        itertools::Itertools::intersperse_with(block_strings, || {
            Either::Right(iter::once("\n".to_string()))
        })
        .flatten()
        .collect()
    }

    /// Total number of character offset of the buffer.
    pub fn len(&self) -> CharOffset {
        self.content.extent::<CharOffset>()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 1.into()
    }

    /// Point coordinate at the end of buffer.
    pub fn max_point(&self) -> Point {
        self.content.extent()
    }

    /// CharOffset at the end of buffer.
    pub fn max_charoffset(&self) -> CharOffset {
        self.content.extent()
    }

    /// ByteOffset at the end of buffer.
    pub fn max_byte_offset(&self) -> ByteOffset {
        self.content.extent()
    }

    /// Total number of character offset of the given row.
    pub fn line_len(&self, row: u32) -> u32 {
        let row_start_offset = Point::new(row, 0).to_buffer_char_offset(self);
        let row_end_offset = if row >= self.max_point().row {
            self.len()
        } else {
            Point::new(row + 1, 0).to_buffer_char_offset(self) - 1
        };

        (row_end_offset.as_usize() - row_start_offset.as_usize()) as u32
    }

    /// Builds an [`EditDelta`] to lay out the entire buffer. This is used when the layout state is invalidated, such as
    /// by a font-size change or resizing the editor.
    pub fn invalidate_layout(&self) -> EditDelta {
        let full_range = CharOffset::from(1)..self.max_charoffset();
        self.invalidate_layout_internal(full_range)
    }

    fn invalidate_layout_internal(&self, range: Range<CharOffset>) -> EditDelta {
        let full_points = self.offset_range_to_point_range(range.clone());
        // No content change—compute a no-op byte edit.
        let old_byte_start = range.start.to_buffer_byte_offset(self);
        let old_byte_end = range.end.to_buffer_byte_offset(self);
        EditDelta {
            precise_deltas: vec![PreciseDelta {
                replaced_range: range.clone(),
                replaced_points: full_points.clone(),
                resolved_range: range.clone(),
                replaced_byte_range: old_byte_start..old_byte_end,
                new_byte_length: old_byte_end
                    .as_usize()
                    .saturating_sub(old_byte_start.as_usize()),
                new_end_point: full_points.end,
            }],
            old_offset: range.clone(),
            new_lines: self.styled_blocks_in_range(range, StyledBlockBoundaryBehavior::Exclusive),
        }
    }

    pub fn invalidate_layout_for_range(&self, range: Range<CharOffset>) -> EditDelta {
        let block_or_line_start = self.block_or_line_start(range.start);
        let block_or_line_end = self.block_or_line_end(range.end);
        self.invalidate_layout_internal(block_or_line_start..block_or_line_end)
    }

    pub(super) fn offset_range_to_point_range(&self, range: Range<CharOffset>) -> Range<Point> {
        range.start.to_buffer_point(self)..range.end.to_buffer_point(self)
    }

    /// Whether the provided ranges of character offset is fully decorated with the style.
    pub fn ranges_fully_styled(
        &self,
        ranges: Vec1<Range<CharOffset>>,
        text_style: TextStyles,
    ) -> bool {
        for range in ranges {
            let items = self.content.items_in_range::<StyleSummary>(range);
            let prefix_styling = items.start().text_styles();

            // If the first character is not fully decorated with the style, return false.
            for style in all::<BufferTextStyle>() {
                if text_style.exact_match_style(&style) && !prefix_styling.exact_match_style(&style)
                {
                    return false;
                }
            }

            for item in items {
                if let BufferText::Marker { marker_type, .. } = item
                    && text_style.exact_match_style(marker_type)
                {
                    return false;
                }
            }
        }

        true
    }

    /// Query for the set of text styles that are all fully active over a range.
    /// A style is fully active if it applies to every character in the range.
    /// This is a counterpart to [`Self::range_fully_styled`].
    pub(super) fn range_text_styles(&self, range: Range<CharOffset>) -> TextStylesWithMetadata {
        let items = self.content.items_in_range::<StyleSummary>(range.clone());
        let mut range_styles = items.start().text_styles();
        for item in items {
            if let BufferText::Marker { marker_type, .. } = item {
                if let Some(mut_style) = range_styles.style_mut(marker_type) {
                    *mut_style = false;
                } else {
                    range_styles.set_weight(Weight::Normal);
                }
            }

            if let BufferText::Link(_) = item {
                *range_styles.link_mut() = false;
            }
        }

        if range_styles.is_link() {
            TextStylesWithMetadata::from_text_styles(
                range_styles,
                self.link_url_at_offset(range.start),
                None, // Syntax color should not be read for user actions.
            )
        } else {
            TextStylesWithMetadata::from_text_styles(range_styles, None, None)
        }
    }

    /// Return the text style at the given offset.
    pub fn text_styles_with_metadata_at(&self, offset: CharOffset) -> TextStylesWithMetadata {
        let mut cursor = self.content.cursor::<CharOffset, LinkCount>();
        let summary =
            cursor.summary::<BufferSummary>(&offset.max(CharOffset::from(1)), SeekBias::Right);

        let text_styles = summary.style_summary().text_styles();
        // Fill the link metadata if it is active.
        let url = if text_styles.is_link() {
            let current_link_count = *cursor.start();
            let mut link_cursor = self.content.cursor::<LinkCount, CharOffset>();
            link_cursor.seek(&current_link_count, SeekBias::Left);

            match link_cursor.item() {
                Some(BufferText::Link(LinkMarker::Start(url))) => Some(url.clone()),
                _ => None,
            }
        } else {
            None
        };

        // Syntax color should not be read for user actions.
        TextStylesWithMetadata::from_text_styles(text_styles, url, None)
    }

    // Returns style runs within the given range of rows. The range parameter is considered to be block boundaries and not in the middle of a block
    pub(super) fn styled_blocks_in_range(
        &self,
        range: Range<CharOffset>,
        boundary_behavior: StyledBlockBoundaryBehavior,
    ) -> Vec<StyledBufferBlock> {
        self.styled_blocks_in_ranges(vec1![range], boundary_behavior)
    }

    // Returns style runs within the given range of rows for the given ranges. The range parameter is considered to be block boundaries and not in the middle of a block
    pub(super) fn styled_blocks_in_ranges(
        &self,
        ranges: Vec1<Range<CharOffset>>,
        boundary_behavior: StyledBlockBoundaryBehavior,
    ) -> Vec<StyledBufferBlock> {
        ranges
            .into_iter()
            .flat_map(|range| self.styled_blocks_at(range, boundary_behavior))
            .collect_vec()
    }

    fn styled_blocks_at(
        &self,
        range: Range<CharOffset>,
        boundary_behavior: StyledBlockBoundaryBehavior,
    ) -> StyledBufferBlocks<'_> {
        StyledBufferBlocks::new(self, range, boundary_behavior)
    }

    fn insert_block_item(
        &mut self,
        block_item: BufferBlockItem,
        range: Range<CharOffset>,
    ) -> EditResult {
        let formatted_text = FormattedText::new(vec![block_item.to_formatted_text_line()]);

        let edit_range_start = if range.start == CharOffset::from(1) {
            range.start - 1
        } else {
            range.start
        };

        self.apply_core_edit_actions(vec![
            CoreEditorAction::new(
                edit_range_start..range.end,
                CoreEditorActionType::Insert {
                    text: formatted_text,
                    source: EditOrigin::SystemEdit,
                    override_next_style: false,
                    insert_on_selection: true,
                },
            ),
            self.update_buffer_end(range.end),
        ])
    }

    fn insert_block_after_block_with_offset(
        &mut self,
        block_offset: CharOffset,
        block_type: BlockType,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let insertion_offset = self.block_or_line_end(block_offset) - 1;
        let previous_block_type = self.block_type_at_point(block_offset);

        let editor_action_set = match block_type {
            BlockType::Item(item_type) => {
                let after_block_type = self.block_type_at_point(insertion_offset + 1);
                let formatted_text = FormattedText::new(vec![item_type.to_formatted_text_line()]);

                // When inserting after a plain text block, we want to replace the ending linebreak
                // with the block item so we don't create an extra newline.
                let edit_range_end = if previous_block_type
                    == BlockType::Text(BufferBlockStyle::PlainText)
                    && after_block_type == BlockType::Text(BufferBlockStyle::PlainText)
                {
                    insertion_offset + 1
                } else {
                    insertion_offset
                };

                vec![
                    CoreEditorAction::new(
                        insertion_offset..edit_range_end,
                        CoreEditorActionType::Insert {
                            text: formatted_text,
                            source: EditOrigin::SystemEdit,
                            override_next_style: false,
                            insert_on_selection: true,
                        },
                    ),
                    self.update_buffer_end(edit_range_end),
                ]
            }
            BlockType::Text(style) => {
                // When inserting after a block that is not plain text, the default insertion behavior will
                // create a new text marker. To avoid adding duplicate lines, we don't need to add any content
                // in the formatted text line.
                let content = if previous_block_type == BlockType::Text(BufferBlockStyle::PlainText)
                {
                    FormattedText::new(vec![FormattedTextLine::LineBreak])
                } else {
                    FormattedText::new(vec![FormattedTextLine::Line(vec![])])
                };

                let mut editor_action_set = vec![];
                editor_action_set.push(CoreEditorAction::new(
                    insertion_offset..insertion_offset,
                    CoreEditorActionType::Insert {
                        text: content,
                        source: EditOrigin::SystemEdit,
                        override_next_style: false,
                        insert_on_selection: true,
                    },
                ));

                // Style the line inserted to the desired block style.
                if style != BufferBlockStyle::PlainText {
                    editor_action_set.push(CoreEditorAction::new(
                        insertion_offset..insertion_offset,
                        CoreEditorActionType::StyleBlock(style),
                    ));
                }

                editor_action_set.push(self.update_buffer_end(insertion_offset));
                editor_action_set
            }
        };

        let result = self.apply_core_edit_actions(editor_action_set);

        // Make sure active selection is at the right cursor position.
        selection_model.update(ctx, |selection_model, _ctx| {
            selection_model.set_single_cursor(insertion_offset + 1);
        });
        result
    }

    /// Remove prefix from beginning of the block to range end and style the line after range end.
    fn remove_prefix_and_style_blocks(
        &mut self,
        style: BlockType,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let cursor_position = selection_model.selection_head(selection);
                let start = buffer.block_or_line_start(cursor_position);
                let end = buffer.block_or_line_end(cursor_position);

                // Placing anchor at the start of line. This should resolve to the start of block
                // once resolved after buffer edits.
                let editor_action_set = match &style {
                    BlockType::Text(block_style) => {
                        let mut action_set = vec![];

                        action_set.extend(
                            buffer.block_style_editor_actions(start..end - 1, block_style.clone()),
                        );
                        action_set.push(CoreEditorAction::new(
                            start..cursor_position,
                            CoreEditorActionType::Insert {
                                text: convert_text_with_style_to_formatted_text(
                                    "",
                                    TextStyles::default(),
                                    block_style.clone(),
                                ),
                                source: EditOrigin::SystemEdit,
                                override_next_style: false,
                                insert_on_selection: true,
                            },
                        ));
                        action_set
                    }
                    BlockType::Item(block_type) => {
                        let mut actions = vec![];
                        let mut restyle_actions = vec![];

                        // Note the edit range is from start - 1 to cursor_position + 1 to prevent introducing additional
                        // linebreaks when we insert the block item.
                        let style_start = start - 1;
                        let style_end = cursor_position + 1;

                        if start > CharOffset::from(1) {
                            match buffer.block_type_at_point(style_start) {
                                BlockType::Text(style)
                                    if !matches!(style, BufferBlockStyle::PlainText) =>
                                {
                                    // Note that we need to unstyle and restyle blocks here because single core edit spanning across multiple block
                                    // types would break undo/redo (see how we handle edits in `edit_internal`).
                                    let block_range =
                                        buffer.containing_block_start(style_start)..style_start;
                                    actions.extend(buffer.block_style_editor_actions(
                                        block_range.clone(),
                                        BufferBlockStyle::PlainText,
                                    ));
                                    restyle_actions.extend(
                                        buffer.block_style_editor_actions(block_range, style),
                                    );
                                }
                                _ => (),
                            }
                        }

                        if style_end < buffer.max_charoffset() {
                            match buffer.block_type_at_point(style_end) {
                                BlockType::Text(style)
                                    if !matches!(style, BufferBlockStyle::PlainText) =>
                                {
                                    // Note that we need to unstyle and restyle blocks here because single core edit spanning across multiple block
                                    // types would break undo/redo (see how we handle edits in `edit_internal`).
                                    let block_range =
                                        style_end..buffer.containing_block_end(style_end) - 1;
                                    actions.extend(buffer.block_style_editor_actions(
                                        block_range.clone(),
                                        BufferBlockStyle::PlainText,
                                    ));
                                    restyle_actions.extend(
                                        buffer.block_style_editor_actions(block_range, style),
                                    );
                                }
                                _ => (),
                            }
                        }

                        actions.push(CoreEditorAction::new(
                            style_start..style_end,
                            CoreEditorActionType::Insert {
                                text: FormattedText::new(vec![block_type.to_formatted_text_line()]),
                                source: EditOrigin::SystemEdit,
                                override_next_style: false,
                                insert_on_selection: true,
                            },
                        ));
                        actions.extend(restyle_actions);
                        actions.push(buffer.update_buffer_end(style_end));
                        actions
                    }
                };
                ActionWithSelectionDelta::new_with_offsets(
                    editor_action_set,
                    &mut buffer.internal_anchors,
                    start,
                    start,
                    0,
                    AnchorSide::Left,
                )
            },
            ctx,
        )
    }

    /// For each line, add the prefix text after the tab stops. This action should not change the active selection range.
    /// The prefixes will be added at the minimum column index across all lines.
    fn decorate_lines_with_prefix(
        &mut self,
        lines: Vec1<usize>,
        prefix: &str,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let mut editor_actions = vec![];

        let mut min_col: Option<u32> = None;
        for line in lines {
            let line_start = Point::new(line as u32, 0).to_buffer_char_offset(self);

            // Skip empty lines.
            if self.char_at(line_start) == Some('\n') {
                continue;
            }

            let block_type = self.block_type_at_point(line_start);

            let block_style = match block_type {
                BlockType::Item(_) => continue,
                BlockType::Text(style) => style,
            };

            let offset = self.non_tab_stop_offset_at_line(line_start, &block_style);
            let col_idx = offset.to_buffer_point(self).column;
            min_col = match min_col {
                Some(prev) => Some(prev.min(col_idx)),
                None => Some(col_idx),
            };

            editor_actions.push((
                line,
                CoreEditorActionType::Insert {
                    text: convert_text_with_style_to_formatted_text(
                        prefix,
                        TextStyles::default(),
                        block_style,
                    ),
                    source: EditOrigin::UserInitiated,
                    override_next_style: false,
                    insert_on_selection: true,
                },
            ));
        }

        // Resolve column and row number to an actual offset.
        let update = match min_col {
            Some(col) => {
                let actions = editor_actions
                    .into_iter()
                    .map(|(row, action)| {
                        let offset = Point::new(row as u32, col).to_buffer_char_offset(self);
                        CoreEditorAction::new(offset..offset, action)
                    })
                    .collect_vec();

                self.apply_core_edit_actions(actions)
            }
            None => EditResult::default(),
        };

        // Eagerly update the active selection model's anchors to avoid any race conditions.
        selection_model.update(ctx, |selection_model, _ctx| {
            selection_model.update_anchors(update.anchor_updates.clone());
        });

        update
    }

    /// For each line, remove the prefix if it exists. It also strips up to one whitespace after the prefix.
    fn remove_prefix_from_lines(
        &mut self,
        lines: Vec1<usize>,
        prefix: &str,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let mut editor_action_set = vec![];
        let prefix_char_count = prefix.chars().count();

        for line in lines {
            let line_start = Point::new(line as u32, 0).to_buffer_char_offset(self);
            let block_type = self.block_type_at_point(line_start);

            let block_style = match block_type {
                BlockType::Item(_) => continue,
                BlockType::Text(style) => style,
            };

            let offset = self.non_tab_stop_offset_at_line(line_start, &block_style);

            let Ok(chars) = TextBuffer::chars_at(self, offset) else {
                continue;
            };

            let mut delta = 0;
            // We want to remove at most one following whitespace after the prefix as well. We could achieve this by
            // chaining the prefix chars with an iterator that yields whitespace once.
            for (c1, c2) in chars.zip(prefix.chars().chain(once(' '))) {
                if c1 == c2 {
                    delta += 1;
                } else {
                    break;
                }
            }

            if delta >= prefix_char_count {
                // Since this is a deletion not operating on the selection range. We could end up deallocating some
                // selection anchors. To prevent this, we could iterate over all selections and shift their anchor offset
                // out of the deletion range.
                selection_model.update(ctx, |selection_model, _| {
                    selection_model.shift_selections_after_offset(offset, delta);
                });

                editor_action_set.push(CoreEditorAction::new(
                    offset..offset + delta,
                    CoreEditorActionType::Insert {
                        text: convert_text_with_style_to_formatted_text(
                            "",
                            TextStyles::default(),
                            block_style,
                        ),
                        source: EditOrigin::UserInitiated,
                        override_next_style: false,
                        insert_on_selection: true,
                    },
                ));
            }
        }

        let update = self.apply_core_edit_actions(editor_action_set);
        selection_model.update(ctx, |selection_model, _| {
            selection_model.update_anchors(update.anchor_updates.clone());
        });
        update
    }

    /// Calculate the first offset in the line that is not a tab stop.
    fn non_tab_stop_offset_at_line(
        &self,
        line_start: CharOffset,
        block_style: &BufferBlockStyle,
    ) -> CharOffset {
        let tab_width = match (self.tab_indentation)(block_style, false) {
            IndentBehavior::TabIndent(indent_unit) => indent_unit.width(),
            _ => 1,
        };

        LineIndentation::from_line_start(self, line_start)
            .leading_indentation(tab_width)
            .as_offset()
    }

    fn block_style_editor_actions(
        &self,
        range: Range<CharOffset>,
        style: BufferBlockStyle,
    ) -> Vec<CoreEditorAction> {
        let cursor = self.content.cursor::<CharOffset, BufferSummary>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        buffer_cursor.seek_to_offset_after_markers(range.start);

        let mut editor_action_set = Vec::new();

        // Set up the start of the styling range to make sure the subsequent styling actions are valid.
        // We are doing two things here:
        // 1) Insert newline if range start is at beginning of the buffer or in the middle of a paragraph.
        // 2) Insert newline if range end is at the end of the buffer or in the middle of a paragraph.
        let mut style_start = self.block_or_line_start(range.start);
        let line_start = self.containing_line_start(range.start);
        let start_block_type = self.block_type_at_point(range.start);

        if !style.allows_formatting() {
            // If the block style range start is at the start of a line, we need to unstyle the previous linebreak
            // as well to make sure the previous styles do not get leaked into the current block.
            let unstyle_range = if range.start == style_start {
                (style_start - 1)..range.end
            } else {
                range.clone()
            };
            editor_action_set
                .extend(self.unstyle_internal_editor_actions(unstyle_range, TextStyles::all()));
        }

        let new_line_before_block_styling = if start_block_type != BlockType::Text(style.clone())
            && (range.start == CharOffset::zero() || range.start > line_start)
        {
            editor_action_set.push(CoreEditorAction::new(
                range.start..range.start,
                CoreEditorActionType::Insert {
                    text: FormattedText::new([FormattedTextLine::LineBreak]),
                    source: EditOrigin::UserTyped,
                    override_next_style: false,
                    insert_on_selection: true,
                },
            ));
            style_start = range.start;
            range.start
        } else {
            range.start - 1
        };

        let mut style_end = self.block_or_line_end(range.end);
        let line_end = self.containing_line_end(range.end);
        let end_block_type = self.block_type_at_point(range.end);

        if end_block_type != BlockType::Text(style.clone())
            && (range.end >= self.max_charoffset() || range.end < line_end - 1)
        {
            editor_action_set.push(CoreEditorAction::new(
                range.end..range.end,
                CoreEditorActionType::Insert {
                    text: FormattedText::new([FormattedTextLine::LineBreak]),
                    source: EditOrigin::UserTyped,
                    override_next_style: false,
                    insert_on_selection: true,
                },
            ));
            style_end = range.end + 1;
        }

        fn push_existing_range_to_editor_action_set(
            buffer: &Buffer,
            editor_action_set: &mut Vec<CoreEditorAction>,
            style_start: CharOffset,
            style_end: CharOffset,
            style: &BufferBlockStyle,
        ) {
            // Make sure the decoration range is valid.
            if style_end < style_start {
                return;
            }
            if style.line_break_behavior() == BlockLineBreakBehavior::NewLine {
                editor_action_set.push(CoreEditorAction::new(
                    style_start..style_end,
                    CoreEditorActionType::StyleBlock(style.clone()),
                ));
            } else {
                // If the block style only supports single line, we need to style each line individually.
                editor_action_set.extend(
                    buffer
                        .line_ranges_in_range(style_start..style_end)
                        .into_iter()
                        .map(|range| {
                            CoreEditorAction::new(
                                range,
                                CoreEditorActionType::StyleBlock(style.clone()),
                            )
                        }),
                );
            }
        }

        let mut style_reapply_actions = Vec::new();

        // Unstyle any code block that overlaps with the styling range. If a block range has
        // section that doesn't need to be re-styled, we need to record their range and style so
        // we could restore their style at the end.
        while buffer_cursor.item().is_some() {
            let summary = buffer_cursor.start();
            let offset = summary.text.chars;
            let active_block_type = self.block_type_at_point(offset);

            let block_end = self.containing_block_end(offset);

            if let BlockType::Text(active_block_style) = active_block_type {
                if active_block_style != BufferBlockStyle::PlainText {
                    // If the active block is single line only, we don't need to restyle the entire
                    // block if range.start is after block start since we would have inserted a newline
                    // to create a new block already.
                    let block_start = if active_block_style.line_break_behavior()
                        == BlockLineBreakBehavior::NewLine
                    {
                        self.containing_block_start(offset)
                    } else {
                        self.containing_block_start(offset).max(range.start)
                    };
                    let re_style_end = block_end.min(style_end);

                    editor_action_set.push(CoreEditorAction::new(
                        block_start..re_style_end - 1,
                        CoreEditorActionType::StyleBlock(BufferBlockStyle::PlainText),
                    ));

                    if block_start < new_line_before_block_styling && active_block_style != style {
                        style_reapply_actions.push(CoreEditorAction::new(
                            block_start..new_line_before_block_styling,
                            CoreEditorActionType::StyleBlock(active_block_style.clone()),
                        ));
                    }

                    if re_style_end > range.end + 2 && active_block_style != style {
                        style_reapply_actions.push(CoreEditorAction::new(
                            range.end + 1..re_style_end - 1,
                            CoreEditorActionType::StyleBlock(active_block_style),
                        ));
                    }
                }
            } else if style != BufferBlockStyle::PlainText {
                // We should not decorate the rich block item. Push the existing ranges
                // to the editor_action_set.
                push_existing_range_to_editor_action_set(
                    self,
                    &mut editor_action_set,
                    style_start,
                    offset - 1,
                    &style,
                );

                style_start = block_end;
            }

            if block_end > range.end {
                break;
            }

            buffer_cursor.seek_to_offset_after_markers(block_end);
        }
        drop(buffer_cursor);

        if style != BufferBlockStyle::PlainText {
            push_existing_range_to_editor_action_set(
                self,
                &mut editor_action_set,
                style_start,
                style_end - 1,
                &style,
            );
            editor_action_set.push(self.update_buffer_end(style_end - 1));
        }

        editor_action_set.extend(style_reapply_actions);
        editor_action_set
    }

    /// Decorate all selections in the buffer with the provided block style.
    pub(super) fn block_style_internal(
        &mut self,
        style: BufferBlockStyle,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let editor_actions =
                    buffer.block_style_editor_actions(range.clone(), style.clone());

                ActionWithSelectionDelta::new_with_offsets(
                    editor_actions,
                    &mut buffer.internal_anchors,
                    selection_model.selection_head(selection),
                    selection_model.selection_tail(selection),
                    0,
                    AnchorSide::Right,
                )
            },
            ctx,
        )
    }

    /// Decorate the given range in the buffer with the provided block style.
    pub(super) fn block_style_range(
        &mut self,
        range: Range<CharOffset>,
        style: BufferBlockStyle,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let editor_actions = self.block_style_editor_actions(range, style);
        let update = self.apply_core_edit_actions(editor_actions);
        selection_model.update(ctx, |selection_model, _ctx| {
            selection_model.update_anchors(update.anchor_updates.clone());
        });
        update
    }

    /// Add link style with the given url on the range.
    fn style_link_internal(
        &mut self,
        tag: String,
        url: String,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let cursor = buffer.content.cursor::<CharOffset, BufferSummary>();
                let mut buffer_cursor = BufferCursor::new(cursor);
                buffer_cursor.seek_to_offset_after_markers(range.start);
                let mut editor_action_set = Vec::new();
                let mut style_reapply_actions = Vec::new();

                let mut style_start = range.start;
                let mut style_end = range.end;

                while buffer_cursor.item().is_some() {
                    let summary = buffer_cursor.start();
                    let active_style = summary.style_summary().text_styles();
                    let offset = buffer_cursor.offset();
                    if offset >= range.end {
                        break;
                    }

                    if active_style.is_link() {
                        let start = buffer
                            .containing_link_start(offset)
                            .expect("Link should exist");
                        let end = buffer
                            .containing_link_end(offset)
                            .expect("Link should exist");
                        let url_content = buffer
                            .link_url_at_offset(offset)
                            .expect("Link should exist");

                        // Unstyle any existing link range we overlap with first.
                        editor_action_set.push(CoreEditorAction::new(
                            start..end,
                            CoreEditorActionType::UnstyleLink,
                        ));

                        if start < range.start {
                            // If the url is the same, we should just extend the style range.
                            if url_content == url {
                                style_start = start;
                            // If the url is not the same, we need to restyle the part that is not included
                            // in the style range.
                            } else {
                                style_reapply_actions.push(CoreEditorAction::new(
                                    start..range.start,
                                    CoreEditorActionType::StyleLink(url_content.clone()),
                                ));
                            }
                        }

                        if end > range.end {
                            // If the url is the same, we should just extend the style range.
                            if url_content == url {
                                style_end = end;
                            // If the url is not the same, we need to restyle the part that is not included
                            // in the style range.
                            } else {
                                style_reapply_actions.push(CoreEditorAction::new(
                                    range.end..end,
                                    CoreEditorActionType::StyleLink(url_content.clone()),
                                ));
                            }
                        }

                        buffer_cursor.seek_to_offset_after_markers(end);
                    } else {
                        buffer_cursor.seek_to_offset_after_markers(offset + 1);
                    }
                }
                drop(buffer_cursor);

                // If the replaced tag is the same as the original text, do not change its style.
                let original_text = buffer.text_in_range(range.start..range.end).into_string();
                if original_text != tag {
                    editor_action_set.push(CoreEditorAction::new(
                        range.start..range.end,
                        CoreEditorActionType::Insert {
                            text: convert_text_with_style_to_formatted_text(
                                tag.as_str(),
                                TextStyles::default(),
                                BufferBlockStyle::PlainText,
                            ),
                            source: EditOrigin::UserTyped,
                            override_next_style: false,
                            insert_on_selection: true,
                        },
                    ));
                }

                // Note we need to override the anchor bias here because the following action could decorate the same
                // range as the previous edit action. Using AnchorSide::Left for the start bias and
                // AnchorSide::Right for the end bias ensures that the whole new text is styled, whether
                // it's longer or shorter than the replaced text.
                editor_action_set.push(
                    CoreEditorAction::new(
                        style_start..style_end,
                        CoreEditorActionType::StyleLink(url.clone()),
                    )
                    .with_start_anchor_bias(AnchorSide::Left)
                    .with_end_anchor_bias(AnchorSide::Right),
                );

                editor_action_set.extend(style_reapply_actions);
                ActionWithSelectionDelta::new_with_offsets(
                    editor_action_set,
                    &mut buffer.internal_anchors,
                    selection_model.selection_head(selection),
                    selection_model.selection_tail(selection),
                    0,
                    AnchorSide::Right,
                )
            },
            ctx,
        )
    }

    /// Remove link style on the given range. It is a no-op if the range
    /// is not decorated with the link style.
    fn unstyle_link_internal(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let cursor = buffer.content.cursor::<CharOffset, BufferSummary>();
                let mut buffer_cursor = BufferCursor::new(cursor);
                buffer_cursor.seek_to_offset_after_markers(range.start);

                let mut editor_action_set = Vec::new();
                let mut style_reapply_actions = Vec::new();

                while buffer_cursor.item().is_some() {
                    let summary = buffer_cursor.start();
                    let active_style = summary.style_summary().text_styles();
                    let offset = summary.text.chars;
                    if offset >= range.end {
                        break;
                    }

                    if active_style.is_link() {
                        let start = buffer
                            .containing_link_start(offset)
                            .expect("Link should exist");
                        let end = buffer
                            .containing_link_end(offset)
                            .expect("Link should exist");
                        let url_content = buffer
                            .link_url_at_offset(offset)
                            .expect("Link should exist");

                        editor_action_set.push(CoreEditorAction::new(
                            start..end,
                            CoreEditorActionType::UnstyleLink,
                        ));

                        if start < range.start {
                            // Re-apply parts that are not included in the unstyle range.
                            style_reapply_actions.push(CoreEditorAction::new(
                                start..range.start,
                                CoreEditorActionType::StyleLink(url_content.clone()),
                            ));
                        }

                        if end > range.end {
                            // Re-apply parts that are not included in the unstyle range.
                            style_reapply_actions.push(CoreEditorAction::new(
                                range.end..end,
                                CoreEditorActionType::StyleLink(url_content.clone()),
                            ));
                        }

                        buffer_cursor.seek_to_offset_after_markers(end);
                    } else {
                        buffer_cursor.seek_to_offset_after_markers(offset + 1);
                    }
                }

                drop(buffer_cursor);
                editor_action_set.extend(style_reapply_actions);

                ActionWithSelectionDelta::new_with_offsets(
                    editor_action_set,
                    &mut buffer.internal_anchors,
                    selection_model.selection_head(selection),
                    selection_model.selection_tail(selection),
                    0,
                    AnchorSide::Right,
                )
            },
            ctx,
        )
    }

    /// Decorate the given range in the buffer with the provided `text_style`.
    /// This is a no-op if the given range is already decorated with `text_style`.
    fn style_internal(
        &mut self,
        text_style: TextStyles,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let mut editor_action_set = Vec::new();

                let mut handled_weight = false;
                for style in all::<BufferTextStyle>() {
                    let is_weight = style.has_custom_weight();
                    if is_weight && handled_weight {
                        continue;
                    }
                    if text_style.exact_match_style(&style) {
                        handled_weight |= is_weight;
                        // We only want to style the sub-ranges that has not yet been styled with the target text style.
                        let cursor = buffer.content.cursor::<CharOffset, BufferSummary>();
                        let mut buffer_cursor = BufferCursor::new(cursor);
                        buffer_cursor.seek_to_offset_after_markers(range.start);
                        let mut style_range_start = None;

                        while buffer_cursor.item().is_some() {
                            let summary = buffer_cursor.start();
                            let active_style = summary.style_summary().text_styles();
                            let offset = buffer_cursor.offset();

                            let active_block_type = buffer.block_type_at_point(offset);
                            let allows_formatting = match active_block_type {
                                BlockType::Item(_) => false,
                                BlockType::Text(block_style) => block_style.allows_formatting(),
                            };

                            // If the active block does not permit text formatting, push the active styling range and
                            // jump to the end of the block.
                            if !allows_formatting {
                                if let Some(start) = style_range_start.take() {
                                    let offset_before_marker =
                                        offset.saturating_sub(&CharOffset::from(1));
                                    if start < offset_before_marker {
                                        editor_action_set.push(CoreEditorAction::new(
                                            start..offset_before_marker,
                                            CoreEditorActionType::StyleText(style),
                                        ))
                                    }
                                }

                                let block_end = buffer.containing_block_end(offset);
                                buffer_cursor.seek_to_offset_after_markers(block_end);
                                continue;
                            }

                            if offset >= range.end {
                                break;
                            }

                            if active_style.exact_match_style(&style) {
                                if let Some(start) = style_range_start.take() {
                                    editor_action_set.push(CoreEditorAction::new(
                                        start..offset,
                                        CoreEditorActionType::StyleText(style),
                                    ))
                                }
                            } else if style_range_start.is_none() {
                                style_range_start = Some(offset);
                            }

                            buffer_cursor.seek_to_offset_after_markers(offset + 1);
                        }

                        if let Some(start) = style_range_start.take() {
                            editor_action_set.push(CoreEditorAction::new(
                                start..range.end,
                                CoreEditorActionType::StyleText(style),
                            ))
                        }
                    }
                }
                ActionWithSelectionDelta::new_with_offsets(
                    editor_action_set,
                    &mut buffer.internal_anchors,
                    selection_model.selection_head(selection),
                    selection_model.selection_tail(selection),
                    0,
                    AnchorSide::Right,
                )
            },
            ctx,
        )
    }

    /// Remove the provided `text_style` from the given range in the buffer.
    /// This is a no-op if the given range is not decorated with `text_style`.
    fn unstyle_internal_editor_actions(
        &self,
        range: Range<CharOffset>,
        text_style: TextStyles,
    ) -> Vec<CoreEditorAction> {
        let mut editor_action_set = Vec::new();

        let mut handled_weight = false;
        for style in all::<BufferTextStyle>() {
            let is_weight = style.has_custom_weight();
            if is_weight && handled_weight {
                continue;
            }
            if text_style.exact_match_style(&style) {
                handled_weight |= is_weight;
                // We only want to unstyle the sub-ranges that has been styled with the target text style.
                let cursor = self.content.cursor::<CharOffset, BufferSummary>();
                let mut buffer_cursor = BufferCursor::new(cursor);
                buffer_cursor.seek_to_offset_after_markers(range.start);
                let mut unstyle_range_start = None;

                while buffer_cursor.item().is_some() {
                    let summary = buffer_cursor.start();
                    let active_style = summary.style_summary().text_styles();
                    let offset = buffer_cursor.offset();

                    if offset >= range.end {
                        break;
                    }

                    if !active_style.exact_match_style(&style) {
                        if let Some(start) = unstyle_range_start.take() {
                            editor_action_set.push(CoreEditorAction::new(
                                start..offset,
                                CoreEditorActionType::UnstyleText(style),
                            ))
                        }
                    } else if unstyle_range_start.is_none() {
                        unstyle_range_start = Some(offset);
                    }

                    buffer_cursor.seek_to_offset_after_markers(offset + 1);
                }

                if let Some(start) = unstyle_range_start.take() {
                    editor_action_set.push(CoreEditorAction::new(
                        start..range.end,
                        CoreEditorActionType::UnstyleText(style),
                    ))
                }
            }
        }

        editor_action_set
    }

    fn unstyle_internal(
        &mut self,
        text_style: TextStyles,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let editor_action_set = buffer.unstyle_internal_editor_actions(range, text_style);

                ActionWithSelectionDelta::new_with_offsets(
                    editor_action_set,
                    &mut buffer.internal_anchors,
                    selection_model.selection_head(selection),
                    selection_model.selection_tail(selection),
                    0,
                    AnchorSide::Right,
                )
            },
            ctx,
        )
    }

    /// Indents all lines in the given range. This will follow each block's [`IndentBehavior`].
    /// We also support indent multiple units in one action if the block's indent behavior is tab indent.
    fn indent(
        &mut self,
        unit: u8,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let mut editor_action_set = Vec::new();

                let mut line_start = buffer.containing_line_start(range.start);
                let mut line_end = buffer.containing_line_end(range.start);

                while line_start <= range.end {
                    match buffer.block_type_at_point(line_start) {
                        BlockType::Item(_) => (),
                        BlockType::Text(block_style) => {
                            match (buffer.tab_indentation)(&block_style, false) {
                                IndentBehavior::Restyle(new_style) => {
                                    editor_action_set.push(CoreEditorAction::new(
                                        line_start..line_end - 1,
                                        CoreEditorActionType::StyleBlock(new_style),
                                    ))
                                }
                                IndentBehavior::TabIndent(indent_unit) => {
                                    // For a single cursor, tab at the cursor location (which must be in this
                                    // line). Otherwise, indent the entire line, starting at the first
                                    // non-whitespace character.
                                    let current_tab_stop = if range.start == range.end {
                                        TabStop::from_offset(
                                            range.start,
                                            line_start,
                                            indent_unit.width(),
                                        )
                                    } else {
                                        LineIndentation::from_line_start(buffer, line_start)
                                            .leading_indentation(indent_unit.width())
                                    };
                                    let tab_start = current_tab_stop.as_offset();
                                    let tab_width = current_tab_stop.to_next();

                                    let base_text = indent_unit.text_with_num_tab_stops(1);
                                    let mut text = base_text[..tab_width].to_string();
                                    if unit > 1 {
                                        text.push_str(&base_text.repeat((unit - 1) as usize));
                                    }

                                    editor_action_set.push(CoreEditorAction::new(
                                        tab_start..tab_start,
                                        CoreEditorActionType::Insert {
                                            text: convert_text_with_style_to_formatted_text(
                                                &text,
                                                Default::default(),
                                                block_style,
                                            ),
                                            source: EditOrigin::UserTyped,
                                            override_next_style: false,
                                            insert_on_selection: true,
                                        },
                                    ))
                                }
                                IndentBehavior::Ignore => (),
                            }
                        }
                    }

                    line_start = line_end;
                    line_end = buffer.containing_line_end(line_end);
                }
                ActionWithSelectionDelta::new_with_offsets(
                    editor_action_set,
                    &mut buffer.internal_anchors,
                    selection_model.selection_head(selection),
                    selection_model.selection_tail(selection),
                    0,
                    AnchorSide::Right,
                )
            },
            ctx,
        )
    }

    /// Unindent all lines in the given range. This will follow each block's [`IndentBehavior`].
    fn unindent(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let mut editor_action_set = Vec::new();

                let mut line_start = buffer.containing_line_start(range.start);
                let mut line_end = buffer.containing_line_end(range.start);

                while line_start <= range.end {
                    match buffer.block_type_at_point(line_start) {
                        BlockType::Item(_) => (),
                        BlockType::Text(block_style) => {
                            match (buffer.tab_indentation)(&block_style, true) {
                                IndentBehavior::Restyle(new_style) => {
                                    editor_action_set.push(CoreEditorAction::new(
                                        line_start..line_end - 1,
                                        CoreEditorActionType::StyleBlock(new_style),
                                    ))
                                }
                                IndentBehavior::TabIndent(indent_unit) => {
                                    let indentation =
                                        LineIndentation::from_line_start(buffer, line_start);

                                    if let Some(to_remove) =
                                        indentation.unindent(indent_unit.width())
                                    {
                                        editor_action_set.push(CoreEditorAction::new(
                                            to_remove,
                                            CoreEditorActionType::Insert {
                                                text: FormattedText::new([]),
                                                source: EditOrigin::UserTyped,
                                                override_next_style: false,
                                                // We need to set insert_on_selection to false since the edit could happen outside of the
                                                // active selection anchor (e.g. unindenting in the middle of an indentation range). This
                                                // allows the buffer to clamp the anchor instead of invalidating it.
                                                insert_on_selection: false,
                                            },
                                        ));
                                    }
                                }
                                IndentBehavior::Ignore => (),
                            }
                        }
                    }

                    line_start = line_end;
                    line_end = buffer.containing_line_end(line_end);
                }

                ActionWithSelectionDelta::new_with_offsets(
                    editor_action_set,
                    &mut buffer.internal_anchors,
                    selection_model.selection_head(selection),
                    selection_model.selection_tail(selection),
                    0,
                    AnchorSide::Right,
                )
            },
            ctx,
        )
    }

    /// The indented start of the hard-wrapped line containing `offset`, if it is within an
    /// indentable block.
    ///
    /// For example, in this text, the indented start is at `a`:
    ///
    /// ```text
    ///     abc
    /// ```
    pub fn indented_line_start(&self, offset: CharOffset) -> Option<CharOffset> {
        let line_start = self.containing_line_start(offset);
        self.indented_line_delta(line_start)
            .map(|delta| delta + line_start)
    }

    pub fn indented_line_delta(&self, line_start: CharOffset) -> Option<CharOffset> {
        if matches!(self.block_type_at_point(line_start), BlockType::Text(text_type) if matches!((self.tab_indentation)(&text_type, false), IndentBehavior::TabIndent(_)))
        {
            Some(LineIndentation::from_line_start(self, line_start).indent_length())
        } else {
            None
        }
    }

    /// Return the number of tab stops a line in the buffer has.
    pub fn indented_line_tab_stops(&self, line_start: CharOffset) -> Option<usize> {
        if let BlockType::Text(text_type) = self.block_type_at_point(line_start)
            && let IndentBehavior::TabIndent(indentation) =
                (self.tab_indentation)(&text_type, false)
        {
            return Some(
                LineIndentation::from_line_start(self, line_start)
                    .leading_indentation(indentation.width())
                    .tabs,
            );
        }
        None
    }

    pub fn indented_units_at_offset(&self, offset: CharOffset) -> Option<u8> {
        if let BlockType::Text(text_type) = self.block_type_at_point(offset)
            && let IndentBehavior::TabIndent(unit) = (self.tab_indentation)(&text_type, false)
        {
            let point = offset.to_buffer_point(self);
            let line_start = self.containing_line_start(offset);
            let tab_stop = TabStop::from_column(
                CharOffset::from(point.column as usize),
                line_start,
                unit.width(),
            );

            return Some(tab_stop.tabs as u8);
        }
        None
    }

    fn backspace(
        &mut self,
        text_style_override: &mut Option<TextStylesWithMetadata>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let prev_style = self.active_style_with_metadata_at_selection(selection_model.as_ref(ctx));
        let result = self.modify_each_selection(
            selection_model.clone(),
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let backspace_range = if selection_model.selection_is_single_cursor(selection) {
                    match buffer.block_type_at_point(range.start) {
                        // When the cursor is on the block marker right after block item, replace the block marker
                        // and the block item with newline.
                        BlockType::Text(BufferBlockStyle::PlainText)
                            if matches!(
                                buffer.block_type_at_point(
                                    range.start.saturating_sub(&CharOffset::from(1))
                                ),
                                BlockType::Item(_)
                            ) =>
                        {
                            Some(range.start - 2..range.start - 1)
                        }
                        // When cursor is at the start of a block type, backspacing should remove any active block style.
                        BlockType::Text(style)
                            if style != BufferBlockStyle::PlainText
                                && buffer.containing_block_start(range.start) == range.start =>
                        {
                            let block_end = buffer.block_or_line_end(range.start);
                            let actions = buffer.block_style_editor_actions(
                                range.start..block_end - 1,
                                BufferBlockStyle::PlainText,
                            );
                            return ActionWithSelectionDelta::new_for_cursor(
                                actions,
                                &mut buffer.internal_anchors,
                                range.start,
                                0,
                            );
                        }
                        BlockType::Text(style) => {
                            match (buffer.tab_indentation)(&style, true) {
                                IndentBehavior::TabIndent(indent_unit) => {
                                    // If we're within or just after the leading whitespace of an indentable block,
                                    // remove indentation to the previous tab stop. To do that, figure out what
                                    // we'd remove assuming the cursor is within the leading indentation, and
                                    // confirm that it's in bounds.
                                    let indentation = LineIndentation::from_line_start(
                                        buffer,
                                        buffer.containing_line_start(range.start),
                                    );

                                    if range.start <= indentation.first_character() {
                                        TabStop::from_offset(
                                            range.start,
                                            indentation.line_start,
                                            indent_unit.width(),
                                        )
                                        .unindent_range()
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            }
                        }
                        // When the cursor is right after a starting block item. User should be able to hit backspace
                        // and delete it. Use an override range here since `extend_selection_left` caps selection start
                        // to CharOffset::from(1).
                        BlockType::Item(_) if range.start == CharOffset::from(1) => {
                            Some(range.start - 1..range.start)
                        }
                        _ => None,
                    }
                } else {
                    // If there's a selection, we delete it as-is.
                    Some(selection_model.selection_to_offset_range(selection))
                };

                // The default, if we we aren't in any of the special cases above, is to delete the
                // previous character.
                let backspace_range = match backspace_range {
                    Some(range) => range,
                    None => {
                        let head_offset = selection_model
                            .anchors
                            .resolve(selection.head())
                            .expect("anchor should exist");

                        let offset = buffer.clamp(head_offset.saturating_sub(&CharOffset::from(1)));
                        offset..head_offset
                    }
                };

                buffer.replacement_actions(backspace_range, "", Default::default())
            },
            ctx,
        );

        // Only take the first cursor into account for now.
        *text_style_override = Some(prev_style.text_style_override_after_deletion(
            self.active_style_with_metadata_at_selection(selection_model.as_ref(ctx)),
        ));

        result
    }

    /// Delete a range of text. This is similar to `backspace`, but used for deleting ranges of words or lines.
    fn delete(
        &mut self,
        text_style_override: &mut Option<TextStylesWithMetadata>,
        ranges: Vec1<Range<CharOffset>>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let editable_ranges = ranges.mapped(|range| self.clamp(range.start)..self.clamp(range.end));
        *text_style_override = Some(self.text_styles_with_metadata_at(editable_ranges[0].start));

        self.modify_each_selection(
            selection_model,
            |buffer, _selection, _, index| {
                let range = editable_ranges[index].clone();
                buffer.replacement_actions(range, "", Default::default())
            },
            ctx,
        )
    }

    fn offset_at_block_item(&self, offset: CharOffset) -> bool {
        let style = self.block_type_at_point(offset);
        matches!(style, BlockType::Item(_))
    }

    /// Whether or not the current selection is a single cursor at the start of an empty
    /// list item.
    /// Checks if the cursor is at an empty list supporting styling on backspace.
    /// If it is, return the new block style.
    pub fn offset_at_empty_list_styling(&self, offset: CharOffset) -> Option<BufferBlockStyle> {
        let block_type = self.block_type_at_point(offset);

        if let BlockType::Text(style) = block_type
            && offset == self.containing_line_start(offset)
            && offset == self.containing_line_end(offset) - 1
        {
            return match style {
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                }
                | BufferBlockStyle::OrderedList {
                    indent_level: ListIndentLevel::One,
                    ..
                }
                | BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::One,
                    ..
                } => Some(BufferBlockStyle::PlainText),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::Two,
                } => Some(BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                }),
                BufferBlockStyle::OrderedList {
                    indent_level: ListIndentLevel::Two,
                    number,
                } => Some(BufferBlockStyle::OrderedList {
                    indent_level: ListIndentLevel::One,
                    number,
                }),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::Three,
                } => Some(BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::Two,
                }),
                BufferBlockStyle::OrderedList {
                    indent_level: ListIndentLevel::Three,
                    number,
                } => Some(BufferBlockStyle::OrderedList {
                    indent_level: ListIndentLevel::Two,
                    number,
                }),
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::Two,
                    complete,
                } => Some(BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::One,
                    complete,
                }),
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::Three,
                    complete,
                } => Some(BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::Two,
                    complete,
                }),
                _ => None,
            };
        }

        None
    }

    /// Helper for Enter at the start of a block. If the user presses Enter at the start of a
    /// heading or code block, we insert a plain-text block _above_ the cursor location.
    ///
    /// For a heading, the default behavior would insert a `<text>` marker between the heading
    /// marker and the heading text, effectively pushing the text out of the heading (which is
    /// undesirable).
    ///
    /// For a code block, the default behavior inserts a new line within the code block. If the
    /// code block is at the start of the buffer, this makes it difficult to insert new blocks
    /// above the code block.
    pub fn offset_at_block_start_styling(&self, offset: CharOffset) -> Option<BufferBlockStyle> {
        if self.block_start(offset) == offset {
            match self.block_type_at_point(offset) {
                BlockType::Text(BufferBlockStyle::Header { .. }) => {
                    Some(BufferBlockStyle::PlainText)
                }
                BlockType::Text(BufferBlockStyle::CodeBlock { .. }) => {
                    Some(BufferBlockStyle::PlainText)
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Enter action. This generally inserts a new line, but has special interactions with
    /// certain block types:
    /// * Enter at the start of an empty list item un-indents it
    /// * Enter at the start of a code block or header shifts the whole block down
    ///
    /// If `force_newline` is `true`, the special interactions are disabled.
    fn enter(
        &mut self,
        force_newline: bool,
        text_style: TextStyles,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let is_cursor = range.start == range.end;
                if is_cursor && buffer.offset_at_block_item(range.start) {
                    // Enter at the end of a BlockItem should be treated as inserting a linebreak.
                    let actions = vec![CoreEditorAction::new(
                        range.start..range.start,
                        CoreEditorActionType::Insert {
                            text: FormattedText::new([FormattedTextLine::LineBreak]),
                            source: EditOrigin::UserInitiated,
                            override_next_style: false,
                            insert_on_selection: true,
                        },
                    )];
                    ActionWithSelectionDelta::new_for_cursor(
                        actions,
                        &mut buffer.internal_anchors,
                        range.start,
                        1,
                    )
                } else if force_newline {
                    buffer.replacement_actions(range, "\n", text_style)
                } else if is_cursor {
                    if let Some(style) = buffer.offset_at_empty_list_styling(range.start) {
                        let cursor_start = range.start;
                        let actions = buffer.block_style_editor_actions(range, style);
                        ActionWithSelectionDelta::new_for_cursor(
                            actions,
                            &mut buffer.internal_anchors,
                            cursor_start,
                            0,
                        )
                    } else if let Some(next_style) =
                        buffer.offset_at_block_start_styling(range.start)
                    {
                        // If the cursor is at the start of a styled block, we want to keep the block's style applied
                        // to its content, and instead add a newline above it. For certain block types, this
                        // needs special handling, as the default linebreak behavior would shift the block's
                        // content surprisingly.

                        let marker_offset = range.start.saturating_sub(&1.into());
                        let actions = vec![
                            CoreEditorAction::new(
                                marker_offset..marker_offset,
                                CoreEditorActionType::Insert {
                                    text: FormattedText::new([FormattedTextLine::LineBreak]),
                                    source: EditOrigin::UserInitiated,
                                    override_next_style: false,
                                    insert_on_selection: true,
                                },
                            ),
                            CoreEditorAction::new(
                                marker_offset..marker_offset,
                                CoreEditorActionType::StyleBlock(next_style),
                            )
                            .with_end_anchor_bias(AnchorSide::Left),
                        ];
                        ActionWithSelectionDelta::new_for_cursor(
                            actions,
                            &mut buffer.internal_anchors,
                            range.start,
                            0,
                        )
                    } else {
                        // The default behavior is to insert a new line. edit_internal will add a newline
                        // character or block marker as needed.
                        buffer.replacement_actions(range, "\n", text_style)
                    }
                } else {
                    // The default behavior is to insert a new line. edit_internal will add a newline
                    // character or block marker as needed.
                    buffer.replacement_actions(range, "\n", text_style)
                }
            },
            ctx,
        )
    }

    fn edit_for_each_selection(
        &mut self,
        texts: &Vec1<(String, usize)>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        debug_assert!(texts.len() == selection_model.as_ref(ctx).selections().len());
        let mut ind = 0;

        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let Some((text, delta)) = texts.get(ind) else {
                    return ActionWithSelectionDelta::new_with_offsets(
                        vec![],
                        &mut buffer.internal_anchors,
                        selection_model.selection_head(selection),
                        selection_model.selection_tail(selection),
                        0,
                        AnchorSide::Right,
                    );
                };
                ind += 1;

                let mut selection_delta =
                    buffer.replacement_actions(range, text, TextStyles::default());
                selection_delta.selection_delta.delta = *delta;
                selection_delta
            },
            ctx,
        )
    }

    fn insert_at_offsets(
        &mut self,
        edits: &Vec1<(String, Range<CharOffset>)>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let edit_actions = edits.iter().map(|(content, range)| {
            CoreEditorAction::new(
                range.clone(),
                CoreEditorActionType::Insert {
                    text: convert_text_with_style_to_formatted_text(
                        content,
                        TextStyles::default(),
                        BufferBlockStyle::PlainText,
                    ),
                    source: EditOrigin::UserTyped,
                    override_next_style: false,
                    insert_on_selection: false,
                },
            )
        });
        let update = self.apply_core_edit_actions(edit_actions);
        selection_model.update(ctx, |selection, _| {
            selection.update_anchors(update.anchor_updates.clone());
        });
        update
    }

    /// Replace each current active selection with the given text and style.
    fn edit_internal(
        &mut self,
        text: &str,
        style: TextStyles,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                buffer.replacement_actions(range, text, style)
            },
            ctx,
        )
    }

    /// Modify each selection in the buffer.
    ///
    /// This function takes a closure that takes a buffer and a selection, and returns a
    /// `ActionWithSelectionDelta` that contains the editor actions to apply and the new selection offsets.
    ///
    /// This closure is run on each active selection in the buffer, and the return value defines
    /// how the selection should be modified, and where the selection offsets should end up after the modification.
    pub(super) fn modify_each_selection<T>(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        mut compute_selection_edits: T,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult
    where
        T: FnMut(
            &mut Buffer,
            &mut Selection,
            &BufferSelectionModel,
            usize,
        ) -> ActionWithSelectionDelta,
    {
        let selection_model_ref = selection_model.as_ref(ctx);
        let mut new_selections = selection_model_ref.selections().clone();

        // Collect the editor actions for each selection.
        let (edits, cursor_deltas): (Vec<_>, Vec<_>) = new_selections
            .iter_mut()
            .enumerate()
            .map(|(index, selection)| {
                compute_selection_edits(self, selection, selection_model_ref, index)
            })
            .map(|actions| (actions.actions, actions.selection_delta))
            .unzip();

        // Apply the editor actions.
        let edit_result = self.apply_core_edit_actions(edits.into_iter().flatten());

        // Eagerly update the active selection model's anchors to avoid any race conditions.
        selection_model.update(ctx, |selection_model, _ctx| {
            selection_model.update_anchors(edit_result.anchor_updates.clone());
        });

        // Update the cursor position for each selection.
        for (selection, selection_delta) in new_selections.iter_mut().zip(cursor_deltas) {
            // Optimization: If the delta is zero, and the existing head and tail are the same anchors,
            // then don't create a new anchor object.
            if selection_delta.delta == 0
                && selection.head() == &selection_delta.head_anchor
                && selection.tail() == &selection_delta.tail_anchor
            {
                continue;
            }

            let new_head_offset = self.clamp(
                self.internal_anchors
                    .resolve(&selection_delta.head_anchor)
                    .expect("Anchor should be valid")
                    + selection_delta.delta,
            );
            let new_tail_offset = self.clamp(
                self.internal_anchors
                    .resolve(&selection_delta.tail_anchor)
                    .expect("Anchor should be valid")
                    + selection_delta.delta,
            );

            selection_model.update(ctx, |selection_model, _| {
                selection_model.set_clamped_selection_head(selection, new_head_offset);
                selection_model.set_clamped_selection_tail(selection, new_tail_offset);
            });
            self.reset_selection_bias_after_edit(selection, new_head_offset);
        }
        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selections(new_selections)
        });
        edit_result
    }

    /// Snapshot the current selection state, in order to identify no-op selection changes.
    pub(super) fn snapshot_selection(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &AppContext,
    ) -> SelectionSnapshot {
        let selection_model = selection_model.as_ref(ctx);
        // TODO(ben): We do a lot of unnecessary selection cloning in general. It might be worth
        // using SmallVec for the common case of single selections, and/or having SelectionSnapshot
        // hash all the relevant state rather than holding it directly.
        SelectionSnapshot {
            selections: selection_model.selection_offsets(),
            active_text_styles: self.active_style_with_metadata_at_selection(selection_model),
            active_block_type: self.active_block_type_at_first_selection(selection_model),
        }
    }

    // Todo: kc (CLD1018) - Temporary until multiselect is fully implemented.
    pub fn active_block_type_at_first_selection(
        &self,
        selection_model: &BufferSelectionModel,
    ) -> BlockType {
        self.active_block_type_at_selection(selection_model.selection(), selection_model)
    }

    pub(super) fn active_style_with_metadata_at_selection(
        &self,
        selection_model: &BufferSelectionModel,
    ) -> TextStylesWithMetadata {
        // Return all styles that are active for all selections.
        selection_model
            .selections()
            .iter()
            .map(|selection| {
                let range = selection_model.selection_to_offset_range(selection);
                self.active_style_at(range, selection.bias())
            })
            .reduce(|style1, style2| style1.mutual_styles(style2))
            .expect("At least one style range should exist.")
    }

    pub fn active_block_type_at_selection(
        &self,
        selection: &Selection,
        selection_model: &BufferSelectionModel,
    ) -> BlockType {
        let range = selection_model.selection_to_offset_range(selection);
        self.block_type_at_point(range.start)
    }

    /// Check whether the block type for every selection allow formatting.
    pub fn all_selections_allow_formatting(
        &self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &AppContext,
    ) -> bool {
        let selection_model = selection_model.as_ref(ctx);
        selection_model.selections().iter().all(|selection| {
            match self.active_block_type_at_selection(selection, selection_model) {
                BlockType::Text(block_style) => block_style.allows_formatting(),
                // If pasting content directly after a rich block item, we should keep the content's original
                // styling.
                BlockType::Item(_) => true,
            }
        })
    }

    pub(super) fn modify_first_selection(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        action: ActionWithSelectionDelta,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let mut selections = selection_model.as_ref(ctx).selections().clone();

        let edit_result = self.apply_core_edit_actions(action.actions);

        // Eagerly update the active selection model's anchors to avoid any race conditions.
        selection_model.update(ctx, |selection_model, _ctx| {
            selection_model.update_anchors(edit_result.anchor_updates.clone());
        });

        let new_offset = self.clamp(
            self.internal_anchors
                .resolve(&action.selection_delta.head_anchor)
                .expect("Anchor should be valid")
                + action.selection_delta.delta,
        );

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_clamped_selection_head(selections.first_mut(), new_offset);
            selection_model.set_clamped_selection_tail(selections.first_mut(), new_offset);
        });

        self.reset_selection_bias_after_edit(selections.first_mut(), new_offset);

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selections(selections)
        });
        edit_result
    }

    /// Todo: kc (CLD1018) Temporary until multiselect is finished.
    /// Replace the given range in the buffer with `text` and the provided `text_style`.
    /// This will only affect the first selection.
    pub(super) fn edit_internal_first_selection(
        &mut self,
        range: Range<CharOffset>,
        text: impl AsRef<str>,
        text_style: TextStyles,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let actions = self.replacement_actions(range, text, text_style);
        self.modify_first_selection(selection_model, actions, ctx)
    }

    /// Apply incremental edits to the buffer, minimizing anchor disruption.
    ///
    /// This method is similar to using `InsertAtCharOffsetRanges` via `update_content`, but:
    /// - Does not require a selection model (updates all models via events)
    /// - Only affects the specific ranges that changed
    ///
    /// The edits should be provided in the order they appear in the original text.
    /// `apply_core_edit_actions` handles offset shifting internally via anchors.
    ///
    /// NOTE: This is intended for auto-reload scenarios where we want to preserve user state.
    pub fn insert_at_char_offset_ranges(
        &mut self,
        edits: Vec<(Range<CharOffset>, String)>,
        new_version: ContentVersion,
        ctx: &mut ModelContext<Self>,
    ) {
        if edits.is_empty() {
            // TODO: This is temporary. We will add support to properly maintain the undo stack after incremental updates.
            self.reset_undo_stack();
            self.set_version(new_version);
            return;
        }

        // Create edit actions similar to InsertAtCharOffsetRanges/insert_at_offsets.
        // apply_core_edit_actions handles offset shifting internally via anchors.
        let edit_actions = edits.iter().map(|(range, content)| {
            CoreEditorAction::new(
                range.clone(),
                CoreEditorActionType::Insert {
                    text: convert_text_with_style_to_formatted_text(
                        content,
                        TextStyles::default(),
                        BufferBlockStyle::PlainText,
                    ),
                    source: EditOrigin::UserTyped,
                    override_next_style: false,
                    insert_on_selection: false,
                },
            )
        });

        let edit_result = self.apply_core_edit_actions(edit_actions);

        let Some(content_update) = edit_result.delta else {
            log::debug!("Editor action was no-op");
            return;
        };

        self.version = BufferVersion::new();
        // TODO: This is temporary. We will add support to properly maintain the undo stack after incremental updates.
        self.reset_undo_stack();

        self.set_version(new_version);

        // Emit anchor updates to all selection models
        if !edit_result.anchor_updates.is_empty() {
            ctx.emit(BufferEvent::AnchorUpdated {
                update: edit_result.anchor_updates,
                excluding_model: None,
            });
        }

        // Emit SelectionChanged so that the render state updates the cursor position.
        // We don't have access to a specific selection model here, so we emit with
        // default styles - the actual selection positions come from the anchors which
        // were already updated by the AnchorUpdated event above.
        ctx.emit(BufferEvent::SelectionChanged {
            active_text_styles: TextStylesWithMetadata::default(),
            active_block_type: BlockType::Text(BufferBlockStyle::PlainText),
            should_autoscroll: AutoScrollBehavior::None,
            buffer_version: self.version,
        });

        ctx.emit(BufferEvent::ContentChanged {
            delta: content_update,
            origin: EditOrigin::SystemEdit,
            should_autoscroll: ShouldAutoscroll::No,
            buffer_version: self.version,
            selection_model_id: None,
        });
    }

    /// Replace all of the content in the buffer. This does not take in a selection model as its independent of
    /// the active selection. As a result, we will also not emit an selection update event (selection will get updated
    /// automatically with anchor update events).
    ///
    /// NOTE: This will not replace the first block style anchor as it's currently implemented (this is used exclusively for
    /// code editor right now). But if you plan to use this for rich text editor, you will need to update the logic here.
    pub fn replace_all(&mut self, text: impl AsRef<str>, ctx: &mut ModelContext<Self>) {
        // Infer line ending from the new content.
        self.line_ending_mode =
            multiline::infer_line_ending(text.as_ref(), self.session_platform.as_ref());

        let range = CharOffset::from(1)..self.max_charoffset();
        let editor_action_set = vec![
            CoreEditorAction::new(
                range.clone(),
                CoreEditorActionType::Insert {
                    text: convert_text_with_style_to_formatted_text(
                        text.as_ref(),
                        TextStyles::default(),
                        BufferBlockStyle::PlainText,
                    ),
                    source: EditOrigin::UserTyped,
                    override_next_style: false,
                    insert_on_selection: false, // We need insert_on_selection to be false here since we are replacing the entire buffer.
                },
            ),
            self.update_buffer_end(range.end),
        ];

        let edit_result = self.apply_core_edit_actions(editor_action_set);

        let Some(content_update) = edit_result.delta else {
            log::debug!("Editor action was no-op");
            return;
        };

        self.version = BufferVersion::new();
        self.reset_undo_stack();

        ctx.emit(BufferEvent::ContentReplaced {
            buffer_version: self.version,
        });

        if !edit_result.anchor_updates.is_empty() {
            ctx.emit(BufferEvent::AnchorUpdated {
                update: edit_result.anchor_updates,
                excluding_model: None,
            });
        }

        ctx.emit(BufferEvent::ContentChanged {
            delta: content_update,
            origin: EditOrigin::SystemEdit,
            should_autoscroll: ShouldAutoscroll::No,
            buffer_version: self.version,
            selection_model_id: None,
        });
    }

    /// Create the CoreEditorActions for replacing the given range in the buffer
    /// with the given text and style.  It will return the actions, along with
    /// anchors and a CharOffset delta for computing the new cursor positions.
    fn replacement_actions(
        &mut self,
        range: Range<CharOffset>,
        text: impl AsRef<str>,
        text_style: TextStyles,
    ) -> ActionWithSelectionDelta {
        let mut editor_action_set = Vec::new();
        debug_assert!(
            range.start <= range.end,
            "Invalid edit range {}..{}",
            range.start,
            range.end
        );
        debug_assert!(
            range.start <= self.max_charoffset(),
            "Edit starts at {}, but max char offset is {}",
            range.start,
            self.max_charoffset()
        );

        let mut new_cursor_delta = text.as_ref().chars().count();
        // When inserting right after a block item, we are adding an extra block marker. Take that
        // into consideration when calculating new cursor position.
        if !text.as_ref().is_empty()
            && (matches!(self.block_type_at_point(range.start), BlockType::Item(_))
                || range.start == CharOffset::zero())
        {
            new_cursor_delta += 1;
        }

        let block_type_at_range_start = self.block_type_at_point(range.start);
        let block_type_at_range_end = self.block_type_at_point(range.end);

        // To make sure all core editor action is atomic and uniquely reversible. Edit could be broken down into four high-level steps:
        // 1. If there is an active block style at the start of the edit range, unstyle it first.
        // 2. If there is an active block style at the end of the edit range, unstyle it first.
        // 3. Insert the new content into the replaced range.
        // 4. If we have unstyled the start range, we need to reapply the block style.
        let block_start = self.block_or_line_start(range.start);
        let block_end = self.block_or_line_end(range.end);

        let mut need_restyle = None;

        let block_end_from_start = self.block_or_line_end(range.start);

        if let BlockType::Text(block_style) = &block_type_at_range_start {
            // If the start of the selection is at the beginning of a block and the edit range includes
            // the entire block, we should remove its styling.
            if block_style != &BufferBlockStyle::PlainText && range.end >= block_end_from_start {
                editor_action_set.push(CoreEditorAction::new(
                    block_start..block_end_from_start - 1,
                    CoreEditorActionType::StyleBlock(BufferBlockStyle::PlainText),
                ));
                need_restyle = Some(block_style.clone());
            }
        }

        let block_start_from_end = self.containing_block_start(range.end);
        if let BlockType::Text(block_style) = block_type_at_range_end {
            // If we are not removing the starting block's styling and the block style after the end
            // of the selection is different, we need to style the end of selection to have consistent styling.
            if block_style != BufferBlockStyle::PlainText && range.start < block_start_from_end {
                let unstyle_range = block_start_from_end..block_end - 1;
                editor_action_set.push(CoreEditorAction::new(
                    unstyle_range,
                    CoreEditorActionType::StyleBlock(BufferBlockStyle::PlainText),
                ));
            }
        }

        let inheritance_style = match block_type_at_range_start {
            BlockType::Text(block_style) if need_restyle.is_none() => block_style,
            _ => BufferBlockStyle::PlainText,
        };

        editor_action_set.push(CoreEditorAction::new(
            range.clone(),
            CoreEditorActionType::Insert {
                text: convert_text_with_style_to_formatted_text(
                    text.as_ref(),
                    text_style,
                    inheritance_style,
                ),
                source: EditOrigin::UserTyped,
                override_next_style: false,
                insert_on_selection: true,
            },
        ));

        if let Some(restyle) = need_restyle {
            let style_end = if restyle.line_break_behavior() == BlockLineBreakBehavior::NewLine {
                block_end
            } else {
                // For blocks that only support single lines, only convert the first line to
                // match the style.
                self.containing_line_end(range.end)
            };
            editor_action_set.push(CoreEditorAction::new(
                block_start..style_end - 1,
                CoreEditorActionType::StyleBlock(restyle),
            ));
        }

        editor_action_set.push(self.update_buffer_end(range.end));

        ActionWithSelectionDelta::new_for_cursor(
            editor_action_set,
            &mut self.internal_anchors,
            range.start,
            new_cursor_delta,
        )
    }

    fn remove_embedding_at_offset(
        &mut self,
        offset_before_marker: CharOffset,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        debug_assert!(
            matches!(
                self.block_type_at_point(offset_before_marker + 1),
                BlockType::Item(BufferBlockItem::Embedded { .. })
            ),
            "Trying to remove embedding at {offset_before_marker}, but offset is not an embedding.",
        );
        self.edit_internal_first_selection(
            offset_before_marker..offset_before_marker + 1,
            "",
            TextStyles::default(),
            selection_model,
            ctx,
        )
    }

    // Replace the embedded item at offset.
    fn replace_embedding_at_offset_internal(
        &mut self,
        offset_before_marker: CharOffset,
        embedding: Arc<dyn EmbeddedItem>,
    ) -> EditResult {
        debug_assert!(
            matches!(
                self.block_type_at_point(offset_before_marker + 1),
                BlockType::Item(BufferBlockItem::Embedded { .. })
            ),
            "Trying to replace embedding at {offset_before_marker}, but offset is not an embedding.",
        );

        let old_range = offset_before_marker + 1..offset_before_marker + 2;
        let replaced_points = self.offset_range_to_point_range(old_range.clone());

        // In-place embedding swap—no text content change.
        let old_byte_start = old_range.start.to_buffer_byte_offset(self);
        let old_byte_end = old_range.end.to_buffer_byte_offset(self);

        self.content = {
            let cursor = self.content.cursor::<CharOffset, ()>();
            let mut buffer_cursor = BufferCursor::new(cursor);

            let mut new_content = buffer_cursor.slice_to_offset_after_markers(offset_before_marker);
            new_content.push(BufferText::BlockItem {
                item_type: BufferBlockItem::Embedded { item: embedding },
            });
            buffer_cursor.next();
            new_content.push_tree(buffer_cursor.suffix());
            new_content
        };

        let new_byte_length = old_range
            .end
            .to_buffer_byte_offset(self)
            .as_usize()
            .saturating_sub(old_byte_start.as_usize());
        let new_end_point = old_range.end.to_buffer_point(self);

        EditResult {
            undo_item: None,
            delta: Some(EditDelta {
                precise_deltas: vec![PreciseDelta {
                    replaced_range: old_range.clone(),
                    replaced_points,
                    resolved_range: old_range.clone(),
                    replaced_byte_range: old_byte_start..old_byte_end,
                    new_byte_length,
                    new_end_point,
                }],
                // Note that we need to shift the range to the right by one here since
                // the offset we take as the parameter is right before the block item
                // marker.
                old_offset: old_range.clone(),
                new_lines: self
                    .styled_blocks_in_range(old_range, StyledBlockBoundaryBehavior::Exclusive),
            }),
            ..Default::default()
        }
    }

    pub fn color_code_block_ranges(
        &mut self,
        offset: CharOffset,
        color: &[(Range<ByteOffset>, ColorU)],
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_content(
            BufferEditAction::ColorCodeBlock { offset, color },
            EditOrigin::SystemEdit,
            selection_model,
            ctx,
        );
    }

    pub(super) fn color_code_block_ranges_internal(
        &mut self,
        style_start: CharOffset,
        colors: &[(Range<ByteOffset>, ColorU)],
    ) -> EditResult {
        debug_assert!(
            matches!(
                self.block_type_at_point(style_start),
                BlockType::Text(BufferBlockStyle::CodeBlock { .. })
            ),
            "Trying to color code block at {style_start}, but range is not code block styled"
        );
        debug_assert!(
            self.containing_block_start(style_start) == style_start || colors.is_empty(),
            "Trying to apply color to code block at {style_start}, but range is not from start of the code block"
        );

        let style_end = self.containing_block_end(style_start);
        let old_range = style_start..style_end;
        let replaced_points = self.offset_range_to_point_range(old_range.clone());

        // Style-only change—compute byte info before content modification.
        let old_byte_start = old_range.start.to_buffer_byte_offset(self);
        let old_byte_end = old_range.end.to_buffer_byte_offset(self);

        let mut active_color_index = 0;

        self.content = {
            let cursor = self.content.cursor::<CharOffset, CharOffset>();
            let mut buffer_cursor = BufferCursor::new(cursor);
            let mut new_content = buffer_cursor.slice_to_offset_before_markers(style_start);
            let started_colored = new_content
                .summary()
                .style_summary()
                .text_styles()
                .is_colored();

            let mut byte_index = ByteOffset::from(0);
            let mut active_color = None;
            let mut is_first_item = true;
            while let Some(item) = buffer_cursor.item() {
                // If current buffer cursor is at the end of the styling and the current item is not a color marker (taking
                // into account trailing color end markers). Break out of the loop.
                if buffer_cursor.offset() >= style_end - 1 && !matches!(item, BufferText::Color(_))
                {
                    break;
                }

                // First, end any text color that has finished. Move to the next active color index.
                match colors.get(active_color_index) {
                    Some((color_range, _))
                        if color_range.end <= byte_index && active_color.is_some() =>
                    {
                        new_content.push(BufferText::Color(ColorMarker::End));
                        active_color = None;
                        active_color_index += 1;
                    }
                    _ if started_colored && is_first_item => {
                        new_content.push(BufferText::Color(ColorMarker::End));
                    }
                    _ => (),
                };

                is_first_item = false;

                // Check if we are starting any new text color.
                match colors.get(active_color_index) {
                    Some((color_range, color))
                        if color_range.start == byte_index && active_color != Some(*color) =>
                    {
                        new_content.push(BufferText::Color(ColorMarker::Start(*color)));
                        active_color = Some(*color);
                    }
                    _ => (),
                };

                if let Some(c) = buffer_cursor.char() {
                    byte_index += c.len_utf8();
                    new_content.append_str(&c.to_string());
                    buffer_cursor.next_char_position()
                } else {
                    buffer_cursor.next()
                }
            }

            // Make sure we don't leave any unclosed color ranges.
            if let Some((color_range, _)) = colors.get(active_color_index)
                && color_range.end >= byte_index
                && active_color.is_some()
            {
                new_content.push(BufferText::Color(ColorMarker::End));
            }

            new_content.push_tree(buffer_cursor.suffix());
            new_content
        };

        let new_byte_length = old_range
            .end
            .to_buffer_byte_offset(self)
            .as_usize()
            .saturating_sub(old_byte_start.as_usize());
        let new_end_point = old_range.end.to_buffer_point(self);

        EditResult {
            undo_item: None,
            delta: Some(EditDelta {
                precise_deltas: vec![PreciseDelta {
                    replaced_range: old_range.clone(),
                    replaced_points,
                    resolved_range: old_range.clone(),
                    replaced_byte_range: old_byte_start..old_byte_end,
                    new_byte_length,
                    new_end_point,
                }],
                old_offset: old_range.clone(),
                new_lines: self
                    .styled_blocks_in_range(old_range, StyledBlockBoundaryBehavior::Exclusive),
            }),
            anchor_updates: vec![],
        }
    }

    pub fn bytes_in_range(&self, start: ByteOffset, end: ByteOffset) -> Bytes<'_> {
        Bytes::new(self, start, end)
    }

    pub fn buffer_snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            content: self.content.clone(),
            byte_len: self.content.extent::<ByteOffset>(),
        }
    }

    fn insert_placeholder(
        &mut self,
        at: CharOffset,
        text: impl Into<String>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let buffer_end = self.max_charoffset();
        debug_assert!(
            at <= buffer_end,
            "Tried to insert a placeholder at {at}, but max char offset is {buffer_end}"
        );

        // At the content level, the placeholder occupies 1 character.
        // See Buffer::apply_core_edit_actions on the need for `.min(buffer_end)`
        let old_range = self.block_or_line_start(at)..self.block_or_line_end(at).min(buffer_end);
        let replaced_points = self.offset_range_to_point_range(at..at);

        // Placeholder is non-text; compute byte info before content modification.
        let byte_at = at.to_buffer_byte_offset(self);

        self.content = {
            let cursor = self.content.cursor::<CharOffset, ()>();
            let mut buffer_cursor = BufferCursor::new(cursor);
            let mut new_content = buffer_cursor.slice_to_offset_before_markers(at);
            new_content.push(BufferText::Placeholder {
                content: text.into(),
            });
            new_content.push_tree(buffer_cursor.suffix());
            new_content
        };

        // Placeholders do not split blocks (just like non-newline characters). Therefore, inserting
        // a placeholder doesn't change which blocks are affected, it only makes the affected block
        // 1 character longer. Instead of re-seeking in the content SumTree, we can adjust the
        // original range.
        let new_range = old_range.start..old_range.end + 1;
        let anchor_update = AnchorUpdate {
            start: at,
            old_character_count: 0,
            new_character_count: 1,
            clamp: false,
        };
        self.internal_anchors.update(anchor_update);
        selection_model.update(ctx, |selection, _| {
            selection.update_anchors(vec![anchor_update]);
        });

        // Placeholder occupies 1 CharOffset and 1 ByteOffset.
        let new_byte_length = 1;
        let new_end_point = (at + 1).to_buffer_point(self);

        EditResult {
            undo_item: None,
            delta: Some(EditDelta {
                precise_deltas: vec![PreciseDelta {
                    replaced_range: at..at,
                    replaced_points,
                    resolved_range: at..at + 1,
                    replaced_byte_range: byte_at..byte_at,
                    new_byte_length,
                    new_end_point,
                }],
                old_offset: old_range,
                new_lines: self
                    .styled_blocks_in_range(new_range, StyledBlockBoundaryBehavior::Exclusive),
            }),
            anchor_updates: vec![anchor_update],
        }
    }

    /// Helper to build a core edit action that ensures the buffer ends with plain text.
    /// This must be called from every high-level action that affects block styles.
    ///
    /// This is used to ensure the buffer always ends in plain text, for two reasons:
    /// - It ensures the user can always navigate down to the end of the buffer and add new
    ///   content, even if the last block is multi-line (e.g. pressing Enter in a code block
    ///   continues the code block).
    /// - It lets us render the trailing newline block.
    ///
    /// The `edit_end` offset should be the very end of the high-level action's edit range.
    pub(super) fn update_buffer_end(&self, edit_end: CharOffset) -> CoreEditorAction {
        CoreEditorAction::new(
            edit_end..edit_end,
            CoreEditorActionType::EnsurePlainTextMarker,
        )
    }

    fn undo(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let result = self.undo_stack.undo();

        let Some(undo_item) = result else {
            return EditResult::default();
        };

        // Note that we have some slight difference when applying an undo vs redo item.
        // When applying undo, the replacement range is offset based since we read them directly
        // from the edit delta. So we could directly apply them to the buffer model with `apply_core_edit_action`.
        //
        // When applying redo, the replacement range is anchor based since we need to re-apply
        // a previous high-level editor action. In this case we need to apply them with `apply_core_edit_actions`.
        log::trace!(
            "Applying {} undo actions",
            undo_item.actions.iter().map(|a| a.len()).sum::<usize>()
        );
        let mut precise_deltas = Vec::new();
        let mut anchor_updates = Vec::new();
        let mut new_content_range_anchors: Vec<RangeAnchors> = Vec::new();
        for action in undo_item.actions.into_iter().flatten() {
            let edit_range = action.range.clone();
            let replaced_points = self.offset_range_to_point_range(edit_range.clone());

            // Compute pre-edit byte range from the correct intermediate buffer state.
            let old_byte_start = edit_range.start.to_buffer_byte_offset(self);
            let old_byte_end = edit_range.end.to_buffer_byte_offset(self);

            let result = self.apply_core_edit_action(action);

            if let Some(anchor_update) = result.anchor_update {
                anchor_updates.push(anchor_update);
            }
            let updated_range = result.updated_range;

            // Compute post-edit byte length and end point from the correct intermediate state.
            let new_byte_length = updated_range
                .end
                .to_buffer_byte_offset(self)
                .as_usize()
                .saturating_sub(old_byte_start.as_usize());
            let new_end_point = updated_range.end.to_buffer_point(self);

            new_content_range_anchors.push(RangeAnchors {
                start: self
                    .internal_anchors
                    .create_anchor(updated_range.start, AnchorSide::Right),
                end: self
                    .internal_anchors
                    .create_anchor(updated_range.end, AnchorSide::Left),
            });
            precise_deltas.push(PreciseDelta {
                replaced_range: edit_range,
                replaced_points,
                resolved_range: updated_range.clone(),
                replaced_byte_range: old_byte_start..old_byte_end,
                new_byte_length,
                new_end_point,
            });
        }

        // Resolve each delta's new content range anchors against the final buffer state.
        // If a later action deletes content an earlier action inserted, the anchors may
        // have been invalidated. In that case, keep the placeholder value.
        for (delta, anchors) in precise_deltas.iter_mut().zip(new_content_range_anchors) {
            if let (Some(start), Some(end)) = (
                self.internal_anchors.resolve(&anchors.start),
                self.internal_anchors.resolve(&anchors.end),
            ) {
                delta.resolved_range = start..end;
            }
        }

        selection_model.update(ctx, |selection_model, _| {
            selection_model.update_anchors(anchor_updates.clone());
        });

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selection_offsets(undo_item.selection.selection_map(|selection| {
                SelectionOffsets {
                    head: selection.head,
                    tail: selection.tail,
                }
            }));
        });

        EditResult {
            undo_item: None,
            delta: Some(EditDelta {
                precise_deltas,
                old_offset: undo_item.replacement_range.old_range,
                new_lines: self.styled_blocks_in_range(
                    undo_item.replacement_range.new_range,
                    StyledBlockBoundaryBehavior::Exclusive,
                ),
            }),
            anchor_updates,
        }
    }

    fn redo(
        &mut self,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        let result = self.undo_stack.redo();

        let Some(undo_item) = result else {
            return EditResult::default();
        };

        let mut precise_deltas = Vec::new();
        let mut anchor_updates = Vec::new();
        for actions in undo_item.actions {
            let result = self.apply_core_edit_actions(actions);
            anchor_updates.extend(result.anchor_updates);

            if let Some(delta) = result.delta {
                precise_deltas.extend(delta.precise_deltas);
            }
        }

        selection_model.update(ctx, |selection_model, _| {
            selection_model.update_anchors(anchor_updates.clone());
        });

        selection_model.update(ctx, |selection_model, _| {
            selection_model.set_selection_offsets(undo_item.selection.selection_map(|selection| {
                SelectionOffsets {
                    head: selection.head,
                    tail: selection.tail,
                }
            }));
        });

        EditResult {
            undo_item: None,
            delta: Some(EditDelta {
                precise_deltas,
                old_offset: undo_item.replacement_range.old_range,
                new_lines: self.styled_blocks_in_range(
                    undo_item.replacement_range.new_range,
                    StyledBlockBoundaryBehavior::Exclusive,
                ),
            }),
            anchor_updates,
        }
    }

    /// Check whether the version passed in matches the current content in the buffer.
    /// This must be used to see if a version matches the buffer instead of exact equality to Buffer.version
    /// because the Buffer version is incremented on every edit, including undo/redo.
    /// After an undo or redo, the buffer version will be incremented, but the content is the same as a previous
    /// version.  So we must check the undo stack version as well as the buffer version.
    pub fn version_match(&self, version: &ContentVersion) -> bool {
        self.content_version == *version || self.undo_stack.version_match(version)
    }

    /// Set the version of the buffer. This is used to track the version of the content in the buffer.
    pub fn set_version(&mut self, version: ContentVersion) {
        self.content_version = version;
        // If the undo stack is empty, set the initial version.
        // If the stack is not empty, we cannot set the version in the stack.
        if self.undo_stack.is_empty() {
            self.undo_stack.set_initial_version(version)
        } else {
            log::warn!("Setting version on a buffer with undo stack. This is unexpected.");
        }
    }

    pub fn version(&self) -> ContentVersion {
        self.content_version
    }

    pub fn buffer_version(&self) -> BufferVersion {
        self.version
    }

    /// Handle a VimEvent, which usually consists of inserting text at a given point relative to
    /// the current cursor, then repositioning the cursor from the start of that inserted text by
    /// an offset amount.
    fn vim_event(
        &mut self,
        text: String,
        insert_point: VimInsertPoint,
        cursor_offset_len: usize,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _| {
                let head = selection_model.selection_head(selection);
                let mut insert_point = match insert_point {
                    VimInsertPoint::BeforeCursor => head,
                    VimInsertPoint::AtCursor => head + 1,
                    VimInsertPoint::LineStart => buffer.containing_line_start(head),
                    VimInsertPoint::LineEnd => buffer.containing_line_end(head) - 1,
                    VimInsertPoint::NextLine => buffer.containing_line_end(head),
                    VimInsertPoint::LineFirstNonWhitespace => {
                        buffer.containing_line_first_nonwhitespace(head)
                    }
                };

                // Clamp to the end of the buffer.
                insert_point = insert_point.min(buffer.max_charoffset());

                let block_type = buffer.block_type_at_point(head);
                let inheritance_block_style = match block_type {
                    BlockType::Text(style) => style,
                    BlockType::Item(_) => BufferBlockStyle::PlainText,
                };
                let formatted = convert_text_with_style_to_formatted_text(
                    &text,
                    TextStyles::default(),
                    inheritance_block_style,
                );

                let actions = vec![CoreEditorAction::new(
                    insert_point..insert_point,
                    CoreEditorActionType::Insert {
                        text: formatted,
                        source: EditOrigin::UserInitiated,
                        override_next_style: false,
                        insert_on_selection: true,
                    },
                )];

                ActionWithSelectionDelta::new_with_offsets(
                    actions,
                    &mut buffer.internal_anchors,
                    insert_point,
                    insert_point,
                    cursor_offset_len,
                    AnchorSide::Left,
                )
            },
            ctx,
        )
    }
}

impl Entity for Buffer {
    type Event = BufferEvent;
}

/// The tab/indentation level of a character offset.
#[derive(Debug, Clone, Copy)]
struct TabStop {
    /// The full number of tab stops.
    tabs: usize,
    /// The number of remaining spaces after the tab stop.
    remainder: u8,
    /// The starting offset of the line.
    line_start: CharOffset,
    /// Width of the current indent unit.
    tab_width: usize,
}

impl TabStop {
    /// Calculate the tab stop within a hard-wrapped line. The caller is responsible for ensuring
    /// that `offset` is within the line starting at `line_start`.
    fn from_offset(offset: CharOffset, line_start: CharOffset, tab_width: usize) -> Self {
        Self::from_column(offset - line_start, line_start, tab_width)
    }

    /// Calculate the tab-stop within a hard-wrapped line.
    fn from_column(column: CharOffset, line_start: CharOffset, tab_width: usize) -> Self {
        let tabs = column.as_usize() / tab_width;
        // TAB_WIDTH is less than 256, so this can't overflow.
        let remainder = (column.as_usize() % tab_width) as u8;
        Self {
            tabs,
            remainder,
            line_start,
            tab_width,
        }
    }

    /// Convert the tab stop information back to an offset.
    fn as_offset(self) -> CharOffset {
        self.line_start + (self.tabs * self.tab_width) + (self.remainder as usize)
    }

    /// The number of spaces needed to reach the next tab stop.
    fn to_next(self) -> usize {
        self.tab_width - self.remainder as usize
    }

    /// The range to delete in order to unindent to the previous tab stop.
    /// The caller is responsible for ensuring that all characters up to this column are
    /// whitespace.
    fn unindent_range(self) -> Option<Range<CharOffset>> {
        let to_remove = if self.remainder > 0 {
            self.remainder as usize
        } else if self.tabs > 0 {
            self.tab_width
        } else {
            return None;
        };
        let offset = self.as_offset();
        Some(offset - to_remove..offset)
    }
}

/// Measures how far a hard-wrapped line is indented by whitespace.
#[derive(Debug, Clone, Copy)]
pub struct LineIndentation {
    /// The start of the line.
    pub line_start: CharOffset,

    /// The number of leading whitespace characters on the line.
    pub indent_length: CharOffset,
}

impl LineIndentation {
    fn from_line_start(buffer: &Buffer, line_start: CharOffset) -> Self {
        let indent_length = match buffer.chars_at(line_start) {
            Ok(chars) => chars.take_while(|c| c.is_whitespace()).count(),
            Err(_) => 0,
        };
        Self {
            line_start,
            indent_length: indent_length.into(),
        }
    }

    /// The offset of the first non-whitespace character in the line.
    fn first_character(self) -> CharOffset {
        self.line_start + self.indent_length
    }

    fn indent_length(self) -> CharOffset {
        self.indent_length
    }

    /// The tab stop of the first non-whitespace character in the line.
    fn leading_indentation(self, tab_width: usize) -> TabStop {
        TabStop::from_column(self.indent_length, self.line_start, tab_width)
    }

    /// The range of characters to delete in order to un-indent the line one tab stop.
    fn unindent(self, tab_width: usize) -> Option<Range<CharOffset>> {
        self.leading_indentation(tab_width).unindent_range()
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StyledBufferRun {
    pub run: String,
    pub text_styles: TextStylesWithMetadata,
    pub block_style: BufferBlockStyle,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum StyledBufferBlock {
    Item(BufferBlockItem),
    Text(StyledTextBlock),
}

impl StyledBufferBlock {
    // Plain text of the internal markdown formatting of the text. This should be used for use cases that doesn't
    // need embedded items to be expanded to their rich format.
    fn content(&self) -> String {
        match &self {
            Self::Item(item_type) => item_type.as_markdown(MarkdownStyle::Internal).to_string(),
            Self::Text(text_block) => text_block.text(),
        }
    }

    // Plain text of the user-facing formatting of the text. This should be used for writing to the clipboard.
    fn content_with_expanded_embedded_items(&self, app: &AppContext) -> String {
        match &self {
            Self::Item(item_type) => item_type.as_rich_format_text(app).to_string(),
            Self::Text(text_block) => text_block.text(),
        }
    }

    pub fn content_length(&self) -> CharOffset {
        match &self {
            Self::Item(_) => CharOffset::from(1),
            Self::Text(text_block) => text_block.content_length,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StyledTextBlock {
    pub block: Vec<StyledBufferRun>,
    pub style: BufferBlockStyle,
    pub content_length: CharOffset,
}

impl StyledTextBlock {
    fn text(&self) -> String {
        self.block.iter().map(|run| run.run.clone()).collect()
    }
}

/// Returns an iterator over the text paragraphs with their style.
/// A paragraph is a plain text line or a block of text with an active paragraph style (e.g. code blocks).
pub struct StyledBufferBlocks<'a> {
    /// Cursor into the underlying buffer content.
    cursor: BufferCursor<'a, CharOffset>,
    /// The exclusive end offset.
    max_character_offset: CharOffset,
    boundary_behavior: StyledBlockBoundaryBehavior,
    /// The block that is currently being built.
    active_block: ActiveStyledBlock,
    line_end_mode: LineEnding,
}

/// Behavior for the [`StyledBufferBlocks`] iterator at block boundaries. In the buffer, block
/// boundaries are marked by block markers, block items, or newlines.
///
/// ### Examples
///
/// Assume the following buffer:
///
/// ```text
/// <text>Hello<code:Shell>echo hello<hr><ul0>First<ul0>Second<text>
/// ````
///
/// Given the range 3..7:
/// * `Exclusive` and `InclusiveBlockItems` result in a text block containing "llo\n"
/// * `Inclusive` results in a text block containing "llo\n" and an empty shell block
///
/// Given the range 7..18:
/// * `Exclusive` results in a shell block containing "echo hello\n"
/// * `Inclusive` and `InclusiveBlockItems` result in a shell block containing "echo hello\n" and a
///   horizontal rule item
///
/// Given the range 19..24, all result in an unordered list block containing "First" (no newline).
#[derive(Debug, Clone, Copy, Default)]
pub enum StyledBlockBoundaryBehavior {
    /// The iteration range is exclusive. If the range ends at a block boundary, the previous block
    /// will end with a newline, but there will be no next block.
    ///
    /// This matches the buffer model most closely, and is a reasonable default.
    #[default]
    Exclusive,
    /// If and only if the iteration range ends at a block boundary, include the _next_ block
    /// marker or item as an empty block.
    ///
    /// This is useful when converting to [`FormattedText`].
    Inclusive,
    /// As [`StyledBlockBoundaryBehavior::Inclusive`], but only if the range ends at a block item.
    InclusiveBlockItems,
}

#[derive(Debug)]
enum ActiveStyledBlock {
    Item(BufferBlockItem),
    Text {
        block: ActiveTextBlock,
        start_offset: CharOffset,
    },
    Finished,
}

#[derive(Debug)]
struct ActiveTextBlock {
    block_style: BufferBlockStyle,
    runs: Vec<StyledBufferRun>,
    /// Whether or not the last run is open for editing, or if it's been completed by a style marker.
    last_run_open: bool,
    current_text_styles: TextStylesWithMetadata,
}

impl<'a> StyledBufferBlocks<'a> {
    /// Build a new iterator over the given range of text, as styled blocks.
    fn new(
        buffer: &'a Buffer,
        range: Range<CharOffset>,
        boundary_behavior: StyledBlockBoundaryBehavior,
    ) -> Self {
        let cursor = buffer.content.cursor();
        let mut buffer_cursor = BufferCursor::new(cursor);
        buffer_cursor.seek_to_offset_before_markers(range.start);

        // This flag is for an edge case where the query range is from 0..1. We are certain
        // that the initial block is going to hit a block item / block marker. In this case,
        // if the boundary behavior is inclusive, we should return the StyledBufferBlock with
        // the block item / empty styled block instead of terminating right away.
        let should_preserve_initial_block = buffer_cursor.prev_item().is_none()
            && matches!(
                boundary_behavior,
                StyledBlockBoundaryBehavior::Inclusive
                    | StyledBlockBoundaryBehavior::InclusiveBlockItems
            );

        let initial_block = match buffer_cursor.prev_item() {
            None => {
                // We're at the start of the buffer, so the current item is the first block
                // marker or item.
                match buffer_cursor.item() {
                    Some(BufferText::BlockItem { item_type }) => {
                        buffer_cursor.next();
                        ActiveStyledBlock::Item(item_type.clone())
                    }
                    Some(BufferText::BlockMarker { marker_type }) => {
                        buffer_cursor.next();
                        ActiveStyledBlock::Text {
                            block: ActiveTextBlock::new(marker_type.clone(), Default::default()),
                            start_offset: buffer_cursor.offset(),
                        }
                    }
                    other => {
                        if cfg!(debug_assertions) {
                            panic!(
                                "Expected the buffer to start with a block marker or item, got {other:?}"
                            );
                        }
                        ActiveStyledBlock::Text {
                            block: ActiveTextBlock::new(
                                BufferBlockStyle::PlainText,
                                Default::default(),
                            ),
                            start_offset: buffer_cursor.offset(),
                        }
                    }
                }
            }
            Some(BufferText::BlockItem { item_type }) => {
                // If the range starts with a block item, queue it up as the active, finished block
                // to return on the first call to `next()` by advancing the cursor to the
                // following block item or block marker. This is needed because block items are
                // self-contained, so we can't initialize an empty one and parse to the next
                // marker, like we would for text.
                ActiveStyledBlock::Item(item_type.clone())
            }
            _ => {
                let block_style = match buffer.block_type_at_point(range.start) {
                    BlockType::Text(style) => style,
                    // This could happen if range.start is just before the block marker after a
                    // block item, in which case we don't know the next block style yet. Default to
                    // plain text.
                    BlockType::Item(_) => BufferBlockStyle::PlainText,
                };

                ActiveStyledBlock::Text {
                    block: ActiveTextBlock::new(
                        block_style,
                        buffer.text_styles_with_metadata_at(range.start),
                    ),
                    start_offset: buffer_cursor.offset(),
                }
            }
        };

        // If the range is empty, the first call to `next()` should return None, rather than
        // whatever block item or empty styled block was at the start of the range. To prevent
        // that, initialize the iterator as finished. Check for this after figuring out the initial
        // state, to cover cases where the range starts at the very beginning of the buffer.
        // If the range starts at the beginning of the buffer AND is empty, the cursor may be past
        // the end of the edit range at this point.
        let active_block = if *buffer_cursor.start() > range.end
            || (!should_preserve_initial_block && *buffer_cursor.start() == range.end)
        {
            ActiveStyledBlock::Finished
        } else {
            initial_block
        };

        Self {
            cursor: buffer_cursor,
            max_character_offset: range.end,
            active_block,
            boundary_behavior,
            line_end_mode: LineEnding::LF,
        }
    }

    fn with_line_end_mode(mut self, line_end_mode: LineEnding) -> Self {
        self.line_end_mode = line_end_mode;
        self
    }

    /// Finish the active block and advance to the next one. The caller is responsible for updating
    /// the cursor.
    ///
    /// If the block ends at exactly the `max_character_offset` of the iterator, the next block
    /// state depends on the configured boundary behavior.
    fn advance_block(&mut self, next_block: ActiveStyledBlock) -> Option<StyledBufferBlock> {
        // If the range exactly ends on a block boundary, then we need to check the boundary
        // behavior to decide whether or not to start the next block. In all cases, we finish the
        // current block.
        let replacement = if self.cursor.offset() == self.max_character_offset {
            match self.boundary_behavior {
                StyledBlockBoundaryBehavior::Exclusive => ActiveStyledBlock::Finished,
                StyledBlockBoundaryBehavior::Inclusive => next_block,
                StyledBlockBoundaryBehavior::InclusiveBlockItems => match next_block {
                    ActiveStyledBlock::Item(_) => next_block,
                    _ => ActiveStyledBlock::Finished,
                },
            }
        } else {
            next_block
        };
        let prev = mem::replace(&mut self.active_block, replacement);
        prev.finish(self.cursor.offset())
    }
}

/// Helper for the [`StyledBufferBlocks`] implementation to get the pieces of the text block that's
/// currently being built. If the active block is a block item, this will panic in debug builds and
/// early-return the block item on stable.
///
/// This _must_ be used before calling [`Cursor::next`], otherwise text will be skipped.
macro_rules! active_text {
    ($self:ident) => {{
        match &mut $self.active_block {
            ActiveStyledBlock::Text { block, .. } => block,
            ActiveStyledBlock::Item(_) => {
                if cfg!(debug_assertions) {
                    panic!("Unexpected block item");
                }
                // In release builds, handle the invalid state by yielding the block item and
                // reverting to plain text.
                return $self.advance_block(ActiveStyledBlock::Text {
                    block: ActiveTextBlock::new(BufferBlockStyle::PlainText, Default::default()),
                    start_offset: $self.cursor.offset(),
                });
            }
            ActiveStyledBlock::Finished => {
                // ActiveStyledBlock::Finished is only used after the iterator completes.
                return None;
            }
        }
    }};
}

impl Iterator for StyledBufferBlocks<'_> {
    type Item = StyledBufferBlock;

    fn next(&mut self) -> Option<Self::Item> {
        // On each call to `next()`, we loop through text fragments until the end of the current
        // block or the maximum character offset, whichever comes first. We then return the
        // parsed block and reset the iterator state for the next one.

        while let Some(fragment) = self.cursor.item() {
            let start = self.cursor.offset();
            if self.max_character_offset <= start {
                // If we reach max_character_offset partway through a block, stop parsing it and
                // return what we've got so far.
                break;
            }

            match fragment {
                BufferText::BlockItem { item_type } => {
                    self.cursor.next();
                    // Block items aren't block markers, but they do end blocks. Before ending the
                    // previous block, ensure it's non-empty (in case it were blank). If the block
                    // ended in a newline or block marker, this would be handled by pushing a `\n`.
                    if let ActiveStyledBlock::Text { block, .. } = &mut self.active_block {
                        block.push_str(self.line_end_mode.as_str());
                    }
                    return self.advance_block(ActiveStyledBlock::Item(item_type.clone()));
                }
                BufferText::BlockMarker { marker_type } => {
                    // Inline text styles carry over between text blocks. If the previous block is
                    // an item though, styles reset to their default.
                    let current_text_styles = match &mut self.active_block {
                        ActiveStyledBlock::Text { block, .. } => {
                            // Even though the newline will get stripped off the laid-out text,
                            // it ensures we count characters accurately for determining the
                            // start and end offsets of each block.
                            block.push_str(self.line_end_mode.as_str());
                            block.current_text_styles.clone()
                        }
                        ActiveStyledBlock::Item(_) => Default::default(),
                        ActiveStyledBlock::Finished => return None,
                    };
                    self.cursor.next();
                    return self.advance_block(ActiveStyledBlock::Text {
                        block: ActiveTextBlock::new(marker_type.clone(), current_text_styles),
                        start_offset: self.cursor.offset(),
                    });
                }

                BufferText::Newline => {
                    let text = active_text!(self);
                    self.cursor.next();
                    debug_assert_eq!(
                        text.block_style.line_break_behavior(),
                        BlockLineBreakBehavior::NewLine,
                        "{:?} does not allow newlines",
                        text.block_style
                    );
                    text.push_str(self.line_end_mode.as_str());

                    if text.block_style == BufferBlockStyle::PlainText {
                        // For plain text specifically, we treat each line as its own block and
                        // advance to the next one.
                        let next_block = ActiveStyledBlock::Text {
                            block: ActiveTextBlock::new(
                                BufferBlockStyle::PlainText,
                                text.current_text_styles.clone(),
                            ),
                            start_offset: self.cursor.offset(),
                        };
                        return self.advance_block(next_block);
                    } else {
                        // Otherwise, end the current run so that the layout logic doesn't need to
                        // check for linebreaks within runs.
                        text.last_run_open = false;
                    }
                }

                BufferText::Marker { marker_type, dir } => {
                    let text = active_text!(self);
                    self.cursor.next();
                    text.update_styles(|styles| {
                        let is_start = match dir {
                            MarkerDir::Start => true,
                            MarkerDir::End => false,
                        };

                        if let Some(bool) = styles.style_mut(marker_type) {
                            *bool = is_start;
                        } else if let Some(custom_weight) = marker_type.custom_weight() {
                            let custom_weight = if is_start { Some(custom_weight) } else { None };
                            styles.set_weight(custom_weight);
                        }
                    });
                }
                BufferText::Color(marker_type) => {
                    let text = active_text!(self);
                    self.cursor.next();
                    text.update_styles(|styles| {
                        *styles.color_mut() = match marker_type {
                            ColorMarker::Start(color) => Some(*color),
                            ColorMarker::End => None,
                        }
                    });
                }
                BufferText::Link(marker) => {
                    let text = active_text!(self);
                    self.cursor.next();
                    text.update_styles(|styles| {
                        *styles.link_mut() = match marker {
                            LinkMarker::Start(url) => Some(url.clone()),
                            LinkMarker::End => None,
                        }
                    });
                }
                BufferText::Text {
                    fragment,
                    char_count,
                } => {
                    let text = active_text!(self);
                    let cursor_start = *self.cursor.start();
                    self.cursor.next();

                    // Slicing could start / end in the middle of a text fragment. Make sure
                    // we only push the text fragment that is within the query range.
                    let mut slice_start = 0;
                    if start > cursor_start {
                        slice_start = (start - cursor_start).as_usize();
                    }

                    if cursor_start + CharOffset::from(*char_count as usize)
                        > self.max_character_offset
                    {
                        let slice_len = (self.max_character_offset - cursor_start).as_usize();
                        text.push_str(
                            char_slice(fragment, slice_start, slice_len).unwrap_or(fragment),
                        );
                    } else if slice_start > 0 {
                        text.push_str(
                            char_slice(fragment, slice_start, fragment.chars().count())
                                .unwrap_or(fragment),
                        );
                    } else {
                        text.push_str(fragment);
                    }
                }
                BufferText::Placeholder { content } => {
                    let text = active_text!(self);
                    self.cursor.next();
                    text.runs.push(StyledBufferRun {
                        run: content.clone(),
                        text_styles: text.current_text_styles.clone().for_placeholder(),
                        block_style: text.block_style.clone(),
                    });
                    text.last_run_open = false;
                }
            }
        }

        self.advance_block(ActiveStyledBlock::Finished)
    }
}

impl FusedIterator for StyledBufferBlocks<'_> {}

impl ActiveStyledBlock {
    fn finish(self, offset: CharOffset) -> Option<StyledBufferBlock> {
        match self {
            ActiveStyledBlock::Item(item) => Some(StyledBufferBlock::Item(item)),
            // Note that, if the range ends in a block marker, we'll include it as both a newline
            // at the end of the previous block and as a completely empty new block (no runs). This
            // is important for undo/redo, so that we don't lose styling information.
            ActiveStyledBlock::Text {
                block,
                start_offset,
            } => Some(StyledBufferBlock::Text(StyledTextBlock {
                block: block.runs,
                style: block.block_style,
                content_length: offset.saturating_sub(&start_offset),
            })),
            ActiveStyledBlock::Finished => None,
        }
    }
}

impl ActiveTextBlock {
    fn new(block_style: BufferBlockStyle, text_styles: TextStylesWithMetadata) -> Self {
        Self {
            block_style,
            current_text_styles: text_styles,
            runs: Vec::new(),
            last_run_open: false,
        }
    }

    /// Update the active text styles using `f`, starting a new run if needed.
    fn update_styles<F: FnMut(&mut TextStylesWithMetadata)>(&mut self, mut f: F) {
        f(&mut self.current_text_styles);
        if let Some(run) = self.runs.last_mut() {
            if run.run.is_empty() {
                // If the existing run is empty (e.g. because we're parsing consecutive style
                // markers), update it in-place instead of creating multiple empty runs.
                f(&mut run.text_styles);
            } else {
                // Otherwise, close the run so that the next character starts a new run with the new styling.
                self.last_run_open = false;
            }
        }
    }

    /// Push a character onto the end of this text block.
    fn push_str(&mut self, s: &str) {
        match self.runs.last_mut() {
            Some(run) if self.last_run_open => {
                run.run.push_str(s);
            }
            _ => {
                self.runs.push(StyledBufferRun {
                    run: s.to_string(),
                    text_styles: self.current_text_styles.clone(),
                    block_style: self.block_style.clone(),
                });
                self.last_run_open = true
            }
        }
    }
}

fn convert_text_with_style_to_formatted_text(
    text: &str,
    style: TextStyles,
    block_style: BufferBlockStyle,
) -> FormattedText {
    if text.is_empty() {
        return FormattedText::new(Vec::new());
    }

    FormattedText::new(match block_style {
        BufferBlockStyle::PlainText => vec![FormattedTextLine::Line(text_to_formatted_fragment(
            text, style,
        ))],
        BufferBlockStyle::TaskList {
            indent_level,
            complete,
        } => {
            let mut formatted_lines = vec![];
            for line in text.lines() {
                if line.is_empty() {
                    formatted_lines.push(FormattedTextLine::LineBreak);
                } else {
                    formatted_lines.push(FormattedTextLine::TaskList(FormattedTaskList {
                        complete,
                        indent_level: indent_level.as_usize(),
                        text: text_to_formatted_fragment(line, style),
                    }));
                }
            }
            formatted_lines
        }
        BufferBlockStyle::UnorderedList { indent_level } => {
            let mut formatted_lines = vec![];
            for line in text.lines() {
                if line.is_empty() {
                    formatted_lines.push(FormattedTextLine::LineBreak);
                } else {
                    formatted_lines.push(FormattedTextLine::UnorderedList(
                        FormattedIndentTextInline {
                            indent_level: indent_level.as_usize(),
                            text: text_to_formatted_fragment(line, style),
                        },
                    ));
                }
            }
            formatted_lines
        }
        BufferBlockStyle::OrderedList {
            indent_level,
            number,
        } => {
            let mut formatted_lines = vec![];
            for line in text.lines() {
                if line.is_empty() {
                    formatted_lines.push(FormattedTextLine::LineBreak);
                } else {
                    formatted_lines.push(FormattedTextLine::OrderedList(
                        OrderedFormattedIndentTextInline {
                            number,
                            indented_text: FormattedIndentTextInline {
                                indent_level: indent_level.as_usize(),
                                text: text_to_formatted_fragment(line, style),
                            },
                        },
                    ))
                }
            }

            formatted_lines
        }
        BufferBlockStyle::Header { header_size } => {
            // Headers only support single line. When we have active header styling, we should only
            // insert the first line as Heading block style. The rest of the delta should be in plain text
            // styling.
            let mut is_first_fragment = false;
            let mut formatted_lines = vec![];
            for line in text.lines() {
                if line.is_empty() {
                    formatted_lines.push(FormattedTextLine::LineBreak);
                } else if is_first_fragment {
                    formatted_lines.push(FormattedTextLine::Heading(FormattedTextHeader {
                        heading_size: header_size.into(),
                        text: text_to_formatted_fragment(line, style),
                    }));
                } else {
                    formatted_lines.push(FormattedTextLine::Line(text_to_formatted_fragment(
                        text, style,
                    )));
                }
                is_first_fragment = false;
            }
            formatted_lines
        }
        // This is a bit hacky. We could not treat a simple linebreak in code block as FormattedTextLine::CodeBlock("\n")
        // because by default all code block ends with a newline. So during insertion the core editor action would strip
        // trailing newline and causing the edit action to be a no-op. This is a special case when inserting newlines in
        // an active code block so it should be fine to explicitly handle it here.
        BufferBlockStyle::CodeBlock { code_block_type } => {
            vec![FormattedTextLine::CodeBlock(CodeBlockText {
                lang: code_block_type.to_string(),
                code: text.to_string(),
            })]
        }
        BufferBlockStyle::Table { .. } => vec![FormattedTextLine::Table(
            FormattedTable::from_internal_format(text),
        )],
    })
}

fn text_to_formatted_fragment(text: &str, style: TextStyles) -> Vec<FormattedTextFragment> {
    vec![FormattedTextFragment {
        text: text.into(),
        styles: FormattedTextStyles {
            weight: style.get_custom_weight(),
            italic: style.is_italic(),
            underline: style.is_underlined(),
            inline_code: style.is_inline_code(),
            strikethrough: style.is_strikethrough(),
            ..Default::default()
        },
    }]
}

pub trait ToBufferCharOffset {
    fn to_buffer_char_offset(&self, buffer: &Buffer) -> CharOffset;
}

impl ToBufferCharOffset for Point {
    fn to_buffer_char_offset(&self, buffer: &Buffer) -> CharOffset {
        let mut fragments_cursor = buffer.content.cursor::<Point, TextSummary>();
        let text_summary = fragments_cursor.summary::<TextSummary>(self, SeekBias::Right);

        let delta = self.column.saturating_sub(text_summary.lines.column);
        text_summary.chars + CharOffset::from(delta as usize)
    }
}

impl ToBufferCharOffset for ByteOffset {
    fn to_buffer_char_offset(&self, buffer: &Buffer) -> CharOffset {
        let mut fragments_cursor = buffer.content.cursor::<ByteOffset, TextSummary>();
        let text_summary = fragments_cursor.summary::<TextSummary>(self, SeekBias::Right);
        let mut total_chars = text_summary.chars;

        let delta = self.saturating_sub(&text_summary.bytes);
        match fragments_cursor.item() {
            Some(BufferText::Text { fragment, .. }) if delta > ByteOffset::zero() => {
                if let Some(sub_fragment) = fragment.get(..delta.as_usize()) {
                    total_chars += sub_fragment.chars().count();
                }
            }
            _ => (),
        };
        total_chars
    }
}

pub trait ToBufferByteOffset {
    fn to_buffer_byte_offset(&self, buffer: &Buffer) -> ByteOffset;
}

impl ToBufferByteOffset for CharOffset {
    fn to_buffer_byte_offset(&self, buffer: &Buffer) -> ByteOffset {
        let mut fragments_cursor = buffer.content.cursor::<CharOffset, TextSummary>();
        let text_summary = fragments_cursor.summary::<TextSummary>(self, SeekBias::Right);
        let mut total_bytes = text_summary.bytes;
        let delta = self.saturating_sub(&text_summary.chars);

        match fragments_cursor.item() {
            Some(BufferText::Text { fragment, .. }) if delta > CharOffset::zero() => {
                if let Some((idx, _)) = fragment.char_indices().nth(delta.as_usize()) {
                    total_bytes += idx;
                }
            }
            _ => (),
        };

        total_bytes
    }
}

pub trait ToBufferPoint {
    fn to_buffer_point(&self, buffer: &Buffer) -> Point;
}

impl ToBufferPoint for CharOffset {
    fn to_buffer_point(&self, buffer: &Buffer) -> Point {
        let mut fragments_cursor = buffer.content.cursor::<CharOffset, TextSummary>();
        let text_summary = fragments_cursor.summary::<TextSummary>(self, SeekBias::Right);
        let delta = self.saturating_sub(&text_summary.chars);
        let mut point = text_summary.lines;

        point.column += delta.as_usize() as u32;
        point
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(super) enum BoundaryEdge {
    Start,
    End,
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
pub mod tests;
