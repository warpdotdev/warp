#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]
// Adding this file level gate as some of the code around editability is not used in WASM yet.

use std::future::Future;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::{cmp, mem};

use ai::diff_validation::DiffDelta;
use itertools::Itertools;
use languages::{language_by_filename, language_by_local_filename, language_by_name, Language};
use line_ending::LineEnding;
use num_traits::SaturatingSub;
use rangemap::{RangeMap, RangeSet};
use string_offset::CharOffset;
use syntax_tree::{ColorMap, DecorationStateEvent, SyntaxTreeState};
use vec1::{vec1, Vec1};
use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion,
    InsertPosition, LineMotion, MotionType, TextObjectInclusion, TextObjectType, VimOperator,
    VimTextObject, WordBound, WordMotion, WordType,
};
use vim::{
    find_next_paragraph_end, find_previous_paragraph_start, vim_a_block, vim_a_paragraph,
    vim_a_quote, vim_a_word, vim_find_char_on_line, vim_find_matching_bracket, vim_inner_block,
    vim_inner_paragraph, vim_inner_quote, vim_inner_word, vim_word_iterator_from_offset,
};
use warp_core::platform::SessionPlatform;
use warp_core::semantic_selection::SemanticSelection;
use warp_core::ui::theme::Fill;
use warp_editor::content::anchor::Anchor;
use warp_editor::content::buffer::{
    AutoScrollBehavior, Buffer, BufferEditAction, BufferEvent, BufferSelectAction, EditOrigin,
    InitialBufferState, SelectionOffsets, ShouldAutoscroll, ToBufferCharOffset, ToBufferPoint,
    VimInsertPoint,
};
use warp_editor::content::edit::EditDelta;
use warp_editor::content::find::{SearchConfig, SearchResults};
use warp_editor::content::hidden_lines_model::HiddenLinesModel;
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::{BufferBlockStyle, IndentBehavior, IndentUnit};
use warp_editor::content::version::BufferVersion;
use warp_editor::decoration::DecorationLayer;
use warp_editor::editor::TextDecoration;
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::multiline::{AnyMultilineString, MultilineString, LF};
use warp_editor::render::model::{
    AutoScrollMode, BlockItem, ColumnUnit, Decoration, LineCount, LineDecoration, RenderEvent,
    RenderLineLocation, RenderState, RichTextStyles, StyleUpdateAction,
    UpdateDecorationAfterLayout, WidthSetting,
};
use warp_editor::selection::{SelectionMode, SelectionModel, TextDirection, TextUnit};
use warp_util::standardized_path::StandardizedPath;
use warpui::elements::{
    AnchorPair, OffsetPositioning, OffsetType, PositionedElementOffsetBounds, PositioningAxis,
    XAxisAnchor, YAxisAnchor,
};
use warpui::text::point::Point;
use warpui::text::TextBuffer;
use warpui::units::{IntoPixels, Pixels};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use super::super::DiffResult;
use super::comments::{EditorCommentsModel, PendingComment, PendingCommentEvent};
use super::diff::{
    add_inline_overlay_color, DiffModel, DiffModelEvent, DiffStatus, RenderableDiffHunk,
};
use super::line::EditorLineLocation;
use crate::appearance::Appearance;
use crate::code::editor::line_iterator::LineIterator;
use crate::code_review::comments::{CommentId, CommentOrigin, LineDiffContent};
use crate::editor::InteractionState;
use crate::notebooks::editor::model::word_unit;
use crate::themes::theme::AnsiColorIdentifier;
use crate::util::link_detection::get_word_range_at_offset;

/// An opaque handle to a stable line in the editor content, suitable for scroll
/// position preservation. Contains an internal anchor that tracks through
/// buffer edits so the line position remains accurate even after insertions
/// or deletions elsewhere in the file.
///
/// Obtain via [`CodeEditorModel::line_at_vertical_offset`]; resolve back to a
/// pixel offset via [`CodeEditorModel::line_top`].
#[derive(Debug, Clone)]
pub struct StableEditorLine(StableEditorLineInner);

#[derive(Debug, Clone)]
enum StableEditorLineInner {
    /// Scroll position is on a line in the current buffer.
    CurrentLine {
        /// Anchored at the start of the line; tracks through buffer edits.
        anchor: Anchor,
    },
    /// Scroll position is on a removed/deleted line shown in the diff.
    RemovedLine {
        /// Anchored at the start of the attachment line; tracks through
        /// buffer edits.
        anchor: Anchor,
        temp_index: usize,
    },
}

/// Enum used for vim movements to the start and end of lines
pub enum LineBound {
    Start,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffNavigationState {
    /// Diff hunks are collapsed and not visible
    Collapsed,
    /// All diff hunks are expanded but none is specifically focused
    Expanded,
    /// All diff hunks are expanded and one specific hunk is focused
    Focused(usize),
}

enum IndentMode {
    NewlineAbove,
    NewlineBelow,
    LinewiseChange,
    Enter,
}

struct IndentResult {
    /// The indentation to insert before the cursor
    insert_before_cursor: String,
    /// Optionally, text to insert after the cursor (e.g. in the case
    /// where we hit enter between brackets)
    insert_after_cursor: Option<String>,
}

/// Case transformation kinds used by vim toggle case.
pub enum CaseTransform {
    Toggle,
    Uppercase,
    Lowercase,
}

impl CaseTransform {
    fn apply_to(&self, input: String) -> String {
        match self {
            CaseTransform::Toggle => input
                .chars()
                .map(|c| {
                    if c.is_lowercase() {
                        c.to_uppercase().next().unwrap_or(c)
                    } else if c.is_uppercase() {
                        c.to_lowercase().next().unwrap_or(c)
                    } else {
                        c
                    }
                })
                .collect(),
            CaseTransform::Uppercase => input.to_uppercase(),
            CaseTransform::Lowercase => input.to_lowercase(),
        }
    }
}

/// A hoverable link with an optional on-click handler.
pub struct HoverableLink {
    range: Range<CharOffset>,
    #[allow(clippy::type_complexity)]
    on_click: Option<Box<dyn Fn(&mut AppContext)>>,
}

impl HoverableLink {
    pub fn new(range: Range<CharOffset>) -> Self {
        Self {
            range,
            on_click: None,
        }
    }

    pub fn with_on_click(mut self, on_click: Box<dyn Fn(&mut AppContext)>) -> Self {
        self.on_click = Some(on_click);
        self
    }

    pub fn range(&self) -> &Range<CharOffset> {
        &self.range
    }

    pub fn trigger_on_click(&self, ctx: &mut AppContext) {
        if let Some(on_click) = &self.on_click {
            on_click(ctx);
        }
    }
}

pub enum CodeEditorModelEvent {
    /// Emitted when diff decorations are updated (line highlights, removed lines, etc.)
    DiffUpdated,
    /// Emitted when syntax highlighting decorations are updated
    SyntaxHighlightingUpdated,
    ContentChanged {
        origin: EditOrigin,
    },
    SelectionChanged,
    UnifiedDiffComputed(Rc<DiffResult>),
    ViewportUpdated(BufferVersion),
    InteractionStateChanged,
    DelayedRenderingFlushed,
    /// Emitted when the render state layout has been updated.
    /// Consumers can use this to invalidate cached heights.
    LayoutInvalidated,
    #[cfg(windows)]
    WindowsCtrlC {
        /// True if the `ctrl-c` action was used to copy an active selection.
        copied_selection: bool,
    },
}

/// Triggers to delay rendering until a certain event.
#[derive(Clone, Copy)]
enum DelayRenderingTrigger {
    SyntaxHighlighting(BufferVersion),
    DiffUpdate(BufferVersion),
}

struct DelayRendering {
    edits: Vec<(EditDelta, BufferVersion)>,
    should_autoscroll: ShouldAutoscroll,
    block_until: DelayRenderingTrigger,
}

impl DelayRendering {
    fn new(trigger: DelayRenderingTrigger) -> Self {
        Self {
            edits: Vec::new(),
            should_autoscroll: ShouldAutoscroll::No,
            block_until: trigger,
        }
    }

    fn should_render_for_syntax_highlight(&self, buffer_version: BufferVersion) -> bool {
        match self.block_until {
            DelayRenderingTrigger::DiffUpdate(_) => false,
            DelayRenderingTrigger::SyntaxHighlighting(version) => buffer_version >= version,
        }
    }

    fn should_render_for_diff_update(&self, buffer_version: BufferVersion) -> bool {
        match self.block_until {
            DelayRenderingTrigger::DiffUpdate(version) => buffer_version >= version,
            DelayRenderingTrigger::SyntaxHighlighting(_) => false,
        }
    }

    fn flush_render(self, model: &CodeEditorModel, ctx: &mut ModelContext<CodeEditorModel>) {
        model.render_state.update(ctx, move |render_state, _| {
            let should_autoscroll = self.should_autoscroll;
            for (delta, content_version) in self.edits {
                render_state.add_pending_edit(delta, content_version);
            }
            match should_autoscroll {
                ShouldAutoscroll::Yes => render_state.request_autoscroll(),
                ShouldAutoscroll::VerticalOnly => render_state.request_vertical_autoscroll(),
                ShouldAutoscroll::No => (),
            }
        });

        // Refresh the diff state now that the pending render state has been flushed.
        if model.diff_nav_is_active() {
            model.refresh_diff_state(ctx);
        }

        ctx.emit(CodeEditorModelEvent::DelayedRenderingFlushed);
    }

    /// Consume the delay rendering state without flushing edits to the render state.
    /// Use when a full layout rebuild will follow that supersedes the pending edits.
    /// Still emits `DelayedRenderingFlushed` so downstream listeners (e.g. CodeReviewView)
    /// are notified.
    fn skip(self, ctx: &mut ModelContext<CodeEditorModel>) {
        ctx.emit(CodeEditorModelEvent::DelayedRenderingFlushed);
    }
}

pub struct CodeEditorModel {
    render_state: ModelHandle<RenderState>,
    content: ModelHandle<Buffer>,
    selection_model: ModelHandle<BufferSelectionModel>,
    diff: ModelHandle<DiffModel>,
    selection: ModelHandle<SelectionModel>,
    syntax_tree: ModelHandle<SyntaxTreeState>,
    comments: ModelHandle<EditorCommentsModel>,
    hidden_lines: ModelHandle<HiddenLinesModel>,
    /// The current state of diff navigation (collapsed, expanded, or focused on a specific hunk)
    diff_navigation_state: DiffNavigationState,
    interaction_state: InteractionState,
    /// Only applies to scenarios where current line highlighting is possible.
    /// For example, current line highlighting will always be disabled during diff navigation.
    show_current_line_highlights: bool,
    /// Delay rendering of content updates until a certain trigger.
    delay_rendering: Option<DelayRendering>,
    /// Stores the selection "tails" when entering Vim visual mode so we can derive
    /// visual selections that may differ from the cursor positions.
    vim_visual_tails: Vec<CharOffset>,
    hovered_symbol_range: Option<HoverableLink>,
    /// Automatically hide lines outside of the active diff with X context lines.
    hide_lines_outside_of_active_diff: Option<usize>,
    /// Whether this editor was configured to use lazy layout.
    lazy_layout_enabled: bool,
    /// Whether the editor has completed at least one layout cycle.
    lazy_layout_initialized: bool,
    /// Whether syntax parsing should be bootstrapped from the latest full buffer content.
    pending_syntax_tree_bootstrap: bool,
}

impl CodeEditorModel {
    pub fn new(
        text_styles: RichTextStyles,
        session_platform: Option<SessionPlatform>,
        lazy_layout: bool,
        buffer: Option<ModelHandle<Buffer>>, // Whether the editor is using an underlying shared buffer.
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let content = buffer.unwrap_or_else(|| {
            ctx.add_model(|_| {
                Buffer::new(Box::new(|block_style, _| match block_style {
                    // Use 4 spaces as the default indentation unit.
                    BufferBlockStyle::PlainText => IndentBehavior::TabIndent(IndentUnit::Space(4)),
                    _ => IndentBehavior::Ignore,
                }))
            })
        });
        content.update(ctx, |buffer, _| {
            buffer.set_session_platform(session_platform);
        });
        ctx.subscribe_to_model(&content, |me, _, event, ctx| {
            me.handle_content_model_event(event, ctx);
        });

        let selection_model = ctx.add_model(|_ctx| BufferSelectionModel::new(content.clone()));

        let color_map = Self::syntax_highlighting_color_map(ctx);
        let buffer_version = content.as_ref(ctx).buffer_version();
        let buffer_handle = content.downgrade();
        let syntax_tree =
            ctx.add_model(|_ctx| SyntaxTreeState::new(buffer_handle, buffer_version, color_map));
        ctx.subscribe_to_model(&syntax_tree, |me, _, event, ctx| {
            me.handle_syntax_tree_model_event(event, ctx);
        });

        let diff = ctx.add_model(|_ctx| DiffModel::new());
        ctx.subscribe_to_model(&diff, |me, _, event, ctx| {
            me.handle_diff_model_event(event, ctx);
        });

        let hidden_lines =
            ctx.add_model(|_| HiddenLinesModel::new(content.clone(), selection_model.clone()));

        let render_state = ctx.add_model(|ctx| {
            RenderState::new(text_styles, lazy_layout, Some(hidden_lines.clone()), ctx)
                .with_width_setting(WidthSetting::InfiniteWidth)
        });
        ctx.subscribe_to_model(&render_state, |me, _, event, ctx| {
            me.handle_render_state_model_event(event, ctx);
        });
        let selection = ctx.add_model(|ctx| {
            SelectionModel::new(
                content.clone(),
                render_state.clone(),
                selection_model.clone(),
                Some(hidden_lines.clone()),
                ctx,
            )
            .with_disable_hidden_navigation()
        });

        let comments = ctx.add_model(|_| EditorCommentsModel {
            pending_comment: PendingComment::Closed,
        });

        Self {
            render_state,
            diff,
            content,
            selection_model,
            selection,
            syntax_tree,
            comments,
            hidden_lines,
            diff_navigation_state: DiffNavigationState::Collapsed,
            interaction_state: InteractionState::Editable,
            show_current_line_highlights: true,
            delay_rendering: None,
            vim_visual_tails: vec![],
            hovered_symbol_range: None,
            hide_lines_outside_of_active_diff: None,
            lazy_layout_enabled: lazy_layout,
            lazy_layout_initialized: false,
            pending_syntax_tree_bootstrap: false,
        }
    }

    fn should_defer_syntax_tree_parsing(&self) -> bool {
        self.lazy_layout_enabled && !self.lazy_layout_initialized
    }

    fn mark_lazy_layout_initialized(&mut self, ctx: &mut ModelContext<Self>) {
        if self.lazy_layout_enabled {
            self.lazy_layout_initialized = true;
        }
        self.maybe_bootstrap_syntax_tree(ctx);
    }

    fn maybe_bootstrap_syntax_tree(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.pending_syntax_tree_bootstrap || !self.lazy_layout_initialized {
            return;
        }
        if !self.syntax_tree.as_ref(ctx).has_supported_highlighting() {
            return;
        }

        let content = self.content.as_ref(ctx);
        let buffer_version = content.buffer_version();
        let buffer_snapshot = content.buffer_snapshot();

        self.syntax_tree.update(ctx, move |syntax_tree, ctx| {
            syntax_tree.update_internal_state_with_delta(&[], buffer_version, buffer_snapshot, ctx);
        });
        self.pending_syntax_tree_bootstrap = false;
    }

    fn handle_render_state_model_event(
        &mut self,
        event: &RenderEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            RenderEvent::PendingEditsFlushed => {
                self.mark_lazy_layout_initialized(ctx);
            }
            RenderEvent::ViewportUpdated(version) => {
                self.mark_lazy_layout_initialized(ctx);
                if let Some(version) = version {
                    ctx.emit(CodeEditorModelEvent::ViewportUpdated(*version));
                }
            }
            RenderEvent::LayoutUpdated => {
                ctx.emit(CodeEditorModelEvent::LayoutInvalidated);
            }
            RenderEvent::NeedsResize => {}
        }
    }

    /// Set hide_lines_outside_of_active_diff. This will automatically set a delay rendering trigger to wait
    /// for the next diff to be computed.
    pub fn hide_lines_outside_of_active_diff(
        &mut self,
        context_lines: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer_version = self.buffer_version(ctx);

        self.hide_lines_outside_of_active_diff = Some(context_lines);
        self.delay_rendering = Some(DelayRendering::new(DelayRenderingTrigger::DiffUpdate(
            buffer_version,
        )));
    }

    /// We need to set the diff model base to the normalized version of the text. This is because the internal text
    /// representation of the content used for syntax tree highlighting and text rendering uses standard LF.
    pub fn set_base(&self, base: &str, recompute_diff: bool, ctx: &mut ModelContext<Self>) {
        let normalized_text = MultilineString::<LF>::apply(base);
        self.diff
            .update(ctx, |diff, _ctx| diff.set_base(normalized_text));

        if recompute_diff {
            let buffer_version = self.buffer_version(ctx);
            let content = self.content().as_ref(ctx).text();
            self.diff.update(ctx, move |diff, ctx| {
                diff.compute_diff(content, true, buffer_version, ctx)
            });
        }
    }

    pub fn positioning(&self, ctx: &AppContext) -> OffsetPositioning {
        let selection_position = self
            .render_state()
            .as_ref(ctx)
            .saved_positions()
            .text_selection_id();

        OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
            )
            .with_conditional_anchor(),
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
            )
            .with_conditional_anchor(),
        )
    }

    pub fn diff(&self) -> &ModelHandle<DiffModel> {
        &self.diff
    }

    pub fn hovered_symbol_range(&self) -> Option<&HoverableLink> {
        self.hovered_symbol_range.as_ref()
    }

    pub fn set_hovered_symbol_range(&mut self, range: Option<HoverableLink>) -> bool {
        if self.hovered_symbol_range.is_none() && range.is_none() {
            return false;
        }
        self.hovered_symbol_range = range;
        true
    }

    pub fn maybe_click_on_hovered_link(&self, offset: &CharOffset, ctx: &mut ModelContext<Self>) {
        if let Some(link) = self.hovered_symbol_range() {
            if link.range().contains(offset) {
                link.trigger_on_click(ctx);
            }
        }
    }

    pub fn max_character_offset(&self, ctx: &AppContext) -> CharOffset {
        self.content().as_ref(ctx).max_charoffset()
    }

    /// When diff navigation is active we expand all diffs. When diff navigation is inactive all diffs
    /// are collapsed.
    pub fn diff_nav_is_active(&self) -> bool {
        !matches!(self.diff_navigation_state, DiffNavigationState::Collapsed)
    }

    /// Allows programmatic selection updates
    pub fn selection(&self) -> &ModelHandle<SelectionModel> {
        &self.selection
    }

    pub fn comments(&self) -> &ModelHandle<EditorCommentsModel> {
        &self.comments
    }

    // Set the following line ranges to be hidden in the editor.
    pub fn set_hidden_lines(
        &mut self,
        ranges: RangeSet<warp_editor::content::text::LineCount>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.hidden_lines.update(ctx, |model, ctx| {
            model.set_hidden_lines(ranges, ctx);
        });
    }

    pub fn hidden_ranges(&self, ctx: &AppContext) -> RangeSet<CharOffset> {
        self.hidden_lines.as_ref(ctx).hidden_ranges_at_latest(ctx)
    }

    // Set the following hidden line ranges to be visible. This is no-op if the lines are already visible.
    pub fn set_visible_line_range(
        &mut self,
        range: Range<warp_editor::content::text::LineCount>,
        ctx: &mut ModelContext<Self>,
    ) {
        let version = self.content().as_ref(ctx).buffer_version();

        // Re-render since the hidden range has updated.
        if let Some(delta) = self
            .hidden_lines
            .update(ctx, |model, ctx| model.set_visible_line_range(range, ctx))
        {
            self.render_state.update(ctx, |render_state, _ctx| {
                render_state.add_pending_edit(delta, version);
            });
        }
    }

    /// Helper fn to set selections easily for vim features
    pub fn vim_set_selections(
        &mut self,
        selections: Vec1<SelectionOffsets>,
        autoscroll: AutoScrollBehavior,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selection.update(ctx, |selection, ctx| {
            selection.update_selection(
                BufferSelectAction::SetSelectionOffsets { selections },
                autoscroll,
                ctx,
            );
        });
    }

    /// Set selections but save goal columns.
    /// update_selection typically overwrites goal columns; this fn will save and restore them.
    pub fn vim_set_selections_preserving_goal_xs(
        &mut self,
        selections: Vec1<SelectionOffsets>,
        autoscroll: AutoScrollBehavior,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selection.update(ctx, |selection, ctx| {
            let saved_goal_xs = selection.goal_xs.clone();
            selection.update_selection(
                BufferSelectAction::SetSelectionOffsets { selections },
                autoscroll,
                ctx,
            );
            selection.goal_xs = saved_goal_xs;
        });
    }

    pub fn focused_diff_index(&self) -> Option<usize> {
        match self.diff_navigation_state {
            DiffNavigationState::Focused(index) => Some(index),
            _ => None,
        }
    }

    pub fn diff_navigation_state(&self) -> &DiffNavigationState {
        &self.diff_navigation_state
    }

    pub fn nav_diff_up(&mut self, ctx: &mut ModelContext<Self>) {
        let selected_index = match self.diff_navigation_state {
            DiffNavigationState::Focused(index) => index,
            _ => return,
        };
        let total = self.diff().as_ref(ctx).diff_hunk_count();

        // Cycle down to the end if navigating up at index 0.
        let new_index = if selected_index == 0 {
            total.saturating_sub(1)
        } else {
            selected_index - 1
        };

        self.diff_navigation_state = DiffNavigationState::Focused(new_index);
        self.refresh_diff_state(ctx);
    }

    pub fn nav_diff_down(&mut self, ctx: &mut ModelContext<Self>) {
        let selected_index = match self.diff_navigation_state {
            DiffNavigationState::Focused(index) => index,
            _ => return,
        };
        let total = self.diff().as_ref(ctx).diff_hunk_count();

        // Cycle down to the end if navigating up at index 0.
        let new_index = if selected_index == total.saturating_sub(1) {
            0
        } else {
            selected_index + 1
        };

        self.diff_navigation_state = DiffNavigationState::Focused(new_index);
        self.refresh_diff_state(ctx);
    }

    pub fn revert_diff_index(&mut self, ctx: &mut ModelContext<Self>) {
        let total = self.diff().as_ref(ctx).diff_hunk_count();

        let active_index = match self.diff_navigation_state {
            DiffNavigationState::Focused(index) => index,
            _ => return,
        };
        self.reverse_diff_by_index(active_index, ctx);

        if active_index + 1 == total {
            self.diff_navigation_state =
                DiffNavigationState::Focused(active_index.saturating_sub(1));
        }

        self.refresh_diff_state(ctx);
    }

    /// For a diff hunk index, update the buffer with the reverse action that undo-es the diff.
    pub fn reverse_diff_by_index(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        let Some((replace_range, text)) = self
            .diff
            .as_ref(ctx)
            .reverse_action_by_diff_hunk_index(index)
        else {
            return;
        };

        let selection_model = self.selection_model.clone();
        self.update_content(
            |mut content, ctx| {
                // TODO(CLD-558).
                let buffer = content.buffer();
                let start =
                    Point::new(replace_range.start as u32 + 1, 0).to_buffer_char_offset(buffer);
                let end = Point::new(replace_range.end as u32 + 1, 0).to_buffer_char_offset(buffer);
                let edit = Vec1::new((text, start..end));
                content.apply_edit(
                    BufferEditAction::InsertAtCharOffsetRanges { edits: &edit },
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
    }

    /// Returns the string content of the active buffer using its inferred line ending mode.
    pub fn content_string(&self, ctx: &AppContext) -> AnyMultilineString {
        self.content().as_ref(ctx).text_with_line_ending()
    }

    /// Returns the content of a line given the location of the line,
    /// along with the number of added and removed lines.
    /// For Current and Collapsed lines, retrieves from the buffer.
    /// For Removed lines, retrieves from the diff base content.
    /// Prepends '+' for added/modified lines and '-' for removed lines.
    pub fn get_diff_content_for_line(
        &self,
        line: &EditorLineLocation,
        ctx: &AppContext,
    ) -> LineDiffContent {
        match line {
            EditorLineLocation::Collapsed { .. } | EditorLineLocation::Current { .. } => {
                let buffer = self.buffer().as_ref(ctx);

                let (modified, mut content) = if let Some(line_number) = line.line_number() {
                    // TODO(CLD-558) Buffer lines are 1-indexed.
                    let start_offset =
                        Point::new(line_number.as_u32() + 1, 0).to_buffer_char_offset(buffer);
                    let end_offset =
                        Point::new(line_number.as_u32() + 2, 0).to_buffer_char_offset(buffer);

                    let modified = self.diff.as_ref(ctx).is_line_added_or_changed(&line_number);
                    (
                        modified,
                        buffer.text_in_range(start_offset..end_offset).into_string(),
                    )
                } else {
                    (false, String::new())
                };

                // Prepend '+' for modified lines
                if modified {
                    content = format!("+{content}");
                }

                LineDiffContent {
                    content,
                    lines_added: if modified {
                        LineCount::from(1)
                    } else {
                        LineCount::from(0)
                    },
                    lines_removed: LineCount::from(0),
                }
            }
            EditorLineLocation::Removed { .. } => {
                let mut content = self
                    .diff
                    .as_ref(ctx)
                    .deleted_line_content(line)
                    .unwrap_or_default();

                // Prepend '-' for removed lines
                content = format!("-{content}");

                LineDiffContent {
                    content,
                    lines_added: LineCount::from(0),
                    lines_removed: LineCount::from(1),
                }
            }
        }
    }

    /// Find the 0-based index of the temporary block containing the given
    /// content-space vertical offset within its diff hunk.
    ///
    /// Uses the render state's sumtree cursor for an O(log n) seek plus
    /// an O(k) backward walk (k = position within hunk). Called only on
    /// scroll-settle (debounced), not per-frame.
    pub fn removed_line_hunk_index(&self, target_offset: Pixels, ctx: &AppContext) -> usize {
        let render_state = self.render_state.as_ref(ctx);
        render_state
            .content()
            .temp_block_hunk_index_at_height(target_offset.as_f32() as f64)
            .unwrap_or(0)
    }

    /// Given a content-space vertical offset, identifies which line (current or
    /// removed) is at that position. Returns the stable line identifier
    /// (including an internal anchor for edit-tracking) and the intra-line
    /// pixel offset from the top of that line.
    ///
    /// Returns `None` if the offset is beyond the content height or the block
    /// lookup fails.
    pub fn line_at_vertical_offset(
        &self,
        offset: Pixels,
        ctx: &mut ModelContext<Self>,
    ) -> Option<(StableEditorLine, Pixels)> {
        // Phase 1: Gather all information from immutable borrows (render state,
        // buffer, diff model). Extract values and drop borrows before phase 2.
        // `removed_info` is `Some(temp_index)` for removed lines, `None` for
        // current-buffer lines.
        let (anchor_char_offset, removed_info, intra_line_offset) = {
            let render_state = self.render_state.as_ref(ctx);
            if offset >= render_state.height() {
                return None;
            }

            let content = render_state.content();
            let positioned_block = content.block_at_height(offset.as_f32() as f64)?;
            let block_top = positioned_block.start_y_offset;
            let intra_line_offset = (offset - block_top).max(Pixels::zero());

            match positioned_block.item {
                BlockItem::TemporaryBlock { .. } => {
                    let line_number = positioned_block.start_line;
                    let temp_index = self.removed_line_hunk_index(offset, ctx);

                    let buffer = self.content.as_ref(ctx);
                    let char_offset =
                        Point::new(line_number.as_usize() as u32, 0).to_buffer_char_offset(buffer);

                    (char_offset, Some(temp_index), intra_line_offset)
                }
                BlockItem::Paragraph(_)
                | BlockItem::TextBlock { .. }
                | BlockItem::RunnableCodeBlock { .. }
                | BlockItem::MermaidDiagram { .. }
                | BlockItem::TaskList { .. }
                | BlockItem::UnorderedList { .. }
                | BlockItem::OrderedList { .. }
                | BlockItem::Header { .. }
                | BlockItem::Embedded(_)
                | BlockItem::HorizontalRule(_)
                | BlockItem::Image { .. }
                | BlockItem::Table(_)
                | BlockItem::TrailingNewLine(_)
                | BlockItem::Hidden(_) => {
                    (positioned_block.start_char_offset, None, intra_line_offset)
                }
            }
        };

        // Phase 2: Create anchor (requires mutable borrow).
        let anchor = self
            .selection_model
            .update(ctx, |sel, ctx| sel.anchor(anchor_char_offset, ctx));

        let inner = match removed_info {
            Some(temp_index) => StableEditorLineInner::RemovedLine { anchor, temp_index },
            None => StableEditorLineInner::CurrentLine { anchor },
        };

        Some((StableEditorLine(inner), intra_line_offset))
    }

    /// Returns the content-space vertical offset of the top of the given line.
    /// The internal anchor is resolved to obtain the current line position,
    /// so this remains accurate after buffer edits.
    ///
    /// Returns `None` if the line no longer exists in the render model.
    pub fn line_top(&self, line: &StableEditorLine, ctx: &AppContext) -> Option<Pixels> {
        let selection_model = self.selection_model.as_ref(ctx);
        let render_location = match &line.0 {
            StableEditorLineInner::CurrentLine { anchor } => {
                let line_number =
                    LineCount::from(selection_model.line_number_from_anchor(anchor, ctx)?);
                RenderLineLocation::Current(line_number)
            }
            StableEditorLineInner::RemovedLine { anchor, temp_index } => {
                let line_number =
                    LineCount::from(selection_model.line_number_from_anchor(anchor, ctx)?);
                RenderLineLocation::Temporary {
                    at_line: line_number,
                    index_from_at_line: *temp_index,
                }
            }
        };

        let render_state = self.render_state.as_ref(ctx);
        // vertical_offset_at_render_location returns viewport-relative offset
        // (subtracts scroll_top). Add scroll_top back to get content-space.
        let viewport_offset = render_state.vertical_offset_at_render_location(render_location)?;
        Some(viewport_offset + render_state.viewport().scroll_top())
    }
    pub fn buffer(&self) -> &ModelHandle<Buffer> {
        &self.content
    }

    /// Returns the character at the given offset in the buffer, if it exists.
    pub fn char_at(&self, offset: CharOffset, ctx: &AppContext) -> Option<char> {
        self.content.as_ref(ctx).char_at(offset)
    }

    pub fn buffer_selection_model(&self) -> &ModelHandle<BufferSelectionModel> {
        &self.selection_model
    }

    /// Apply a vector of diffs on the current buffer without changing the active selection.
    pub fn apply_diffs(&mut self, diffs: Vec<DiffDelta>, ctx: &mut ModelContext<Self>) {
        let insertion = {
            let buffer = self.content().as_ref(ctx);
            diffs
                .into_iter()
                .map(|diff| {
                    let line_start = diff.replacement_line_range.start as u32;
                    let line_end = diff.replacement_line_range.end as u32;

                    let start = Point::new(line_start, 0).to_buffer_char_offset(buffer);
                    let end = Point::new(line_end, 0).to_buffer_char_offset(buffer);

                    // If the replacement range spans the entire buffer and the buffer is empty, special
                    // case to ensure we are actually replacing the initial block marker
                    //
                    // Internally, an empty buffer has an initial plaintext block marker (a buffer with no markers is considered invalid).
                    // If we try to replace range (0..0), we'll end up replacing everything except
                    // the trailing newline produced by this marker, which will cause us to inadvertently keep an extra newline
                    // in the buffer.
                    let (start, end) = if buffer.is_empty()
                        && diff.replacement_line_range.start == 0
                        && diff.replacement_line_range.end == 0
                    {
                        (CharOffset::from(0), CharOffset::from(1))
                    } else {
                        (start, end)
                    };

                    let mut formatted = diff.insertion.clone();

                    if !formatted.is_empty()
                        && !formatted.ends_with("\n")
                        // Don't add a newline if this is content in a brand new file.
                        && !self.content().as_ref(ctx).is_empty()
                    {
                        // Make sure the formatted text ends with a linebreak.
                        formatted.push('\n');
                    }

                    (formatted, start..end)
                })
                .collect_vec()
        };

        let Ok(edit) = Vec1::try_from_vec(insertion) else {
            return;
        };

        let selection_model = self.selection_model.clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::InsertAtCharOffsetRanges { edits: &edit },
                    EditOrigin::SystemEdit,
                    selection_model,
                    ctx,
                );
                content.buffer().reset_undo_stack();
            },
            ctx,
        )
    }

    /// Refresh the diff nav state. When diff navigation is active we expand all diffs, so we show
    /// their removed and added lines. When diff navigation is inactive we don't show any line
    /// decorations for the diffs in the editor. The [`super::element::EditorWrapper`] may still
    /// display indicators for collapsed diff hunks in its `GutterElement`.
    fn refresh_diff_state(&self, ctx: &mut ModelContext<Self>) {
        let mut all_diffs_removed_lines = Vec::new();
        let mut all_diffs_line_decorations = Vec::new();
        let mut all_diffs_text_decorations = Vec::new();

        match self.diff_navigation_state {
            DiffNavigationState::Collapsed => {
                // No diff decorations when collapsed
            }
            DiffNavigationState::Expanded | DiffNavigationState::Focused(_) => {
                let focused_diff_index = self.focused_diff_index();
                // Show all diffs expanded, with the focused one getting special highlighting
                let Some(base) = self.diff.as_ref(ctx).base() else {
                    return;
                };

                let mut line_iterator = LineIterator::new(base.lines());

                for diff_index in 0..self.diff().as_ref(ctx).diff_hunk_count() {
                    let appearance = Appearance::as_ref(ctx);
                    let should_highlight = match focused_diff_index {
                        Some(focused_index) => diff_index == focused_index,
                        None => true,
                    };
                    let (mut removed_lines, mut line_decoration, mut inline_decorations) =
                        match self.diff.as_ref(ctx).renderable_diff_hunk_by_index(
                            diff_index,
                            &mut line_iterator,
                            appearance,
                        ) {
                            Some(RenderableDiffHunk::Add { line_decoration }) => {
                                (Vec::new(), Some(line_decoration), None::<Vec<Decoration>>)
                            }
                            Some(RenderableDiffHunk::Deletion { removed_lines }) => {
                                (removed_lines, None, None::<Vec<Decoration>>)
                            }
                            Some(RenderableDiffHunk::Replace {
                                line_decoration,
                                inline_highlights,
                                mut removed_lines,
                            }) => {
                                // Highlight the focused diff hunk OR only 1-line replacements if no diff is focused.
                                let should_highlight_changes = match focused_diff_index {
                                    Some(focused_index) => diff_index == focused_index,
                                    None => {
                                        let new_lines = (line_decoration.end
                                            - line_decoration.start)
                                            .as_usize();
                                        new_lines == 1 && removed_lines.len() == 1
                                    }
                                };
                                let highlight_text = if should_highlight_changes {
                                    let buffer = self.content.as_ref(ctx);
                                    let highlight_text = inline_highlights
                                        .into_iter()
                                        .map(|(row_start, inline)| {
                                            let start_char = Point::new((row_start + 1) as u32, 0)
                                                .to_buffer_char_offset(buffer);
                                            Decoration {
                                                start: start_char + inline.start - 1,
                                                end: start_char + inline.end - 1,
                                                background: Some(Fill::Solid(
                                                    add_inline_overlay_color(appearance),
                                                )),
                                                dashed_underline: None,
                                            }
                                        })
                                        .collect_vec();

                                    Some(highlight_text)
                                } else {
                                    for removed_line in &mut removed_lines {
                                        removed_line.inline_text_decorations.clear();
                                    }
                                    None
                                };

                                (removed_lines, Some(line_decoration), highlight_text)
                            }
                            None => (Vec::new(), None, None::<Vec<Decoration>>),
                        };

                    // Fade decorations for non-focused diffs
                    if !should_highlight {
                        let background_color =
                            Fill::Solid(Appearance::as_ref(ctx).theme().background().into_solid());

                        if let Some(line_decoration) = line_decoration.as_mut() {
                            line_decoration.overlay = line_decoration
                                .overlay
                                .fade_into_background(&background_color);
                        };

                        // Do not render inline decorations when diff is not focused.
                        if let Some(inline_decorations) = inline_decorations.as_mut() {
                            inline_decorations.clear();
                        }

                        for removed_line in &mut removed_lines {
                            removed_line.line_decoration =
                                removed_line.line_decoration.map(|line_decoration| {
                                    line_decoration.fade_into_background(&background_color)
                                });

                            removed_line.inline_text_decorations.clear();
                        }
                    }

                    if let Some(line_decoration) = line_decoration {
                        all_diffs_line_decorations.push(line_decoration);
                    }
                    all_diffs_removed_lines.extend(removed_lines);
                    if let Some(inline_decorations) = inline_decorations {
                        all_diffs_text_decorations.extend(inline_decorations);
                    }
                }
            }
        }

        // If there is no diff navigation, we update the `RenderState` to have no temporary blocks
        // or decorations. Other events can add decorations, e.g. the active cursor line highlight.
        self.render_state.update(ctx, |render_state, _| {
            render_state.add_temporary_blocks(all_diffs_removed_lines);
            render_state.set_decorations_after_layout(UpdateDecorationAfterLayout::LineAndText {
                line: all_diffs_line_decorations,
                text: all_diffs_text_decorations,
            });
        });
    }

    /// Returns `true` if the diff navigation was toggled on. Returns `false` if the diff navigation
    /// toggled off.
    pub fn toggle_diff_nav(
        &mut self,
        line_range: Option<Range<LineCount>>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.diff_nav_is_active() {
            self.diff_navigation_state = DiffNavigationState::Collapsed;
            self.refresh_diff_state(ctx);
            return false;
        }

        let index = if let Some(range) = line_range {
            self.diff()
                .as_ref(ctx)
                .diff_hunk_count_before_line(range.start.as_usize())
        } else {
            0
        };
        self.diff_navigation_state = DiffNavigationState::Focused(index);
        self.refresh_diff_state(ctx);
        ctx.emit(CodeEditorModelEvent::DiffUpdated);

        true
    }

    /// Expands all diff hunks without focusing any specific diff hunk.
    /// All diff hunks will be shown expanded with normal highlighting.
    pub fn expand_diffs(&mut self, ctx: &mut ModelContext<Self>) {
        self.diff_navigation_state = DiffNavigationState::Expanded;
        self.refresh_diff_state(ctx);
        ctx.emit(CodeEditorModelEvent::DiffUpdated);
    }

    pub fn diff_status(&self, app: &AppContext) -> DiffStatus {
        self.diff.as_ref(app).diff_status().clone()
    }

    /// Set the language of the syntax map based on the file path.
    pub fn set_language_with_path(
        &mut self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let language = language_by_filename(path);

        if let Some(language) = language {
            self.set_language(language, ctx);
        }
    }

    /// Set the language of the syntax map based on the local filesystem path.
    pub fn set_language_with_local_path(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        let language = language_by_local_filename(path);

        if let Some(language) = language {
            self.set_language(language, ctx);
        }
    }

    pub fn set_language_with_name(&mut self, name: &str, ctx: &mut ModelContext<Self>) {
        let language = language_by_name(name);
        if let Some(language) = language {
            self.set_language(language, ctx);
        }
    }

    fn set_language(&mut self, language: Arc<Language>, ctx: &mut ModelContext<Self>) {
        let unit = language.indent_unit;
        self.content.update(ctx, |buffer, _ctx| {
            buffer.set_tab_indentation(Box::new(move |block_style, _| match block_style {
                BufferBlockStyle::PlainText => IndentBehavior::TabIndent(unit),
                _ => IndentBehavior::Ignore,
            }))
        });

        self.syntax_tree.update(ctx, |syntax_tree, _ctx| {
            syntax_tree.set_language(language);
        });
        self.maybe_bootstrap_syntax_tree(ctx);
    }

    /// Rebuilds the layout and bootstraps the syntax tree for an editor that was created
    /// with an existing buffer. This is more efficient than calling `reset()` because it
    /// directly triggers the syntax tree parsing without recreating the entire buffer state.
    pub fn rebuild_layout_with_syntax_highlighting(&mut self, ctx: &mut ModelContext<Self>) {
        let content = self.content.as_ref(ctx);
        let delta = content.invalidate_layout();
        let buffer_version = content.buffer_version();
        if self.should_defer_syntax_tree_parsing() {
            self.pending_syntax_tree_bootstrap = true;
        } else {
            let buffer_snapshot = content.buffer_snapshot();
            let precise_deltas = delta.precise_deltas.clone();

            // Update syntax tree with the delta to trigger syntax tree parsing
            self.syntax_tree.update(ctx, move |syntax_tree, ctx| {
                syntax_tree.update_internal_state_with_delta(
                    &precise_deltas,
                    buffer_version,
                    buffer_snapshot,
                    ctx,
                )
            });
        }

        if let Some(delay_rendering) = &mut self.delay_rendering {
            delay_rendering.edits.push((delta.clone(), buffer_version));
        } else {
            // Update render state to rebuild the layout
            self.render_state.update(ctx, move |render_state, _| {
                render_state.add_pending_edit(delta, buffer_version);
            });

            if self.diff_nav_is_active() {
                self.refresh_diff_state(ctx);
            }
        }
    }

    /// Rebuild layout and make sure the temporary blocks for diff state has the right styling + anchored
    /// at the right lines after the rebuild. This should be used over the default implementation of rebuild_layout.
    fn rebuild_layout_and_refresh_diff(&self, ctx: &mut ModelContext<Self>) {
        self.rebuild_layout(ctx);
        if self.diff_nav_is_active() {
            self.refresh_diff_state(ctx);
        }
    }

    fn syntax_highlighting_color_map(ctx: &mut ModelContext<Self>) -> ColorMap {
        let appearance = Appearance::as_ref(ctx);
        let terminal_color = appearance.theme().terminal_colors().normal;

        // TODO: This mapping is not finalized. We still need to double check with design.
        ColorMap {
            keyword_color: AnsiColorIdentifier::Magenta
                .to_ansi_color(&terminal_color)
                .into(),
            function_color: AnsiColorIdentifier::Blue
                .to_ansi_color(&terminal_color)
                .into(),
            string_color: AnsiColorIdentifier::Green
                .to_ansi_color(&terminal_color)
                .into(),
            type_color: AnsiColorIdentifier::Red
                .to_ansi_color(&terminal_color)
                .into(),
            number_color: AnsiColorIdentifier::Green
                .to_ansi_color(&terminal_color)
                .into(),
            comment_color: AnsiColorIdentifier::Yellow
                .to_ansi_color(&terminal_color)
                .into(),
            property_color: AnsiColorIdentifier::Cyan
                .to_ansi_color(&terminal_color)
                .into(),
            tag_color: AnsiColorIdentifier::Red
                .to_ansi_color(&terminal_color)
                .into(),
        }
    }

    pub fn text_decoration_for_ranges<'a>(
        &'a self,
        ranges: RangeSet<CharOffset>,
        render_buffer_version: Option<BufferVersion>,
        ctx: &'a AppContext,
    ) -> TextDecoration<'a> {
        let theme = Appearance::as_ref(ctx).theme();
        let underline_color = theme.accent().into_solid();

        let base_color_map =
            self.syntax_tree
                .as_ref(ctx)
                .highlights_in_ranges(ranges, render_buffer_version, ctx);

        let underline_range = self
            .hovered_symbol_range
            .as_ref()
            .map(|link| RangeMap::from_iter([(link.range.clone(), underline_color)]));

        TextDecoration {
            base_color_map,
            override_color_map: underline_range.clone(),
            underline_range,
        }
    }

    /// Re-calculate the hidden range given the active diff state.
    fn calculate_hidden_lines(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(context_line) = self.hide_lines_outside_of_active_diff {
            let line_count = self.line_count(ctx);

            // Calculate the visible line ranges (with context)
            let mut visible_ranges: RangeSet<warp_editor::content::text::LineCount> =
                RangeSet::new();

            // Add ranges for diffs
            for range in self.diff().as_ref(ctx).modified_lines() {
                // Convert 1-indexed line ranges to 0-indexed
                let start_line = range.start.saturating_sub(1);
                let end_line = range.end.saturating_sub(1);

                let context_start = start_line.saturating_sub(context_line);
                let context_end = end_line + context_line;

                if context_start < context_end {
                    visible_ranges.insert(context_start.into()..context_end.into());
                }
            }

            // Calculate hidden ranges as the complement of visible ranges
            let all_lines: Range<warp_editor::content::text::LineCount> =
                warp_editor::content::text::LineCount::from(0)
                    ..warp_editor::content::text::LineCount::from(line_count);

            // Find gaps in the visible ranges
            let hidden_ranges = visible_ranges
                .gaps(&all_lines)
                .collect::<RangeSet<warp_editor::content::text::LineCount>>();

            self.set_hidden_lines(hidden_ranges, ctx);
        }
    }

    fn handle_diff_model_event(&mut self, event: &DiffModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            DiffModelEvent::DiffUpdated {
                version,
                should_recalculate_hidden_lines,
            } => {
                // If we are hiding lines based on active diffs, there are 3 steps here once the diff is computed:
                // 1) If we should, recalculate hidden lines based on the updated diff state.
                // 2) Flush any delayed rendering based on diff update trigger.
                // 3) If hidden lines are recalculated, rebuild the current layout.
                if *should_recalculate_hidden_lines {
                    self.calculate_hidden_lines(ctx);
                }

                // Do not refresh diff state if there is an active delayed rendering. We should wait until the delayed rendering
                // is flushed so we could insert temporary blocks based on the accurate line ranges.
                if self.delay_rendering.is_none() && self.diff_nav_is_active() {
                    self.refresh_diff_state(ctx);
                }

                let will_rebuild_layout = *should_recalculate_hidden_lines
                    && self.hide_lines_outside_of_active_diff.is_some();

                if self
                    .delay_rendering
                    .as_ref()
                    .map(|delay_rendering| delay_rendering.should_render_for_diff_update(*version))
                    .unwrap_or(false)
                {
                    let delay_rendering = self.delay_rendering.take().expect("Checked above");
                    if will_rebuild_layout {
                        // Full rebuild will supersede pending edits — skip the expensive render state update.
                        delay_rendering.skip(ctx);
                    } else {
                        delay_rendering.flush_render(self, ctx);
                    }
                }

                // This could be optimized to not rebuild the entire layout and only the part of the hidden ranges that are changed.
                // It is challenging tho because we will need to somehow expand and calculate style blocks based on past buffer versions.
                //
                // Realistically, the impact of rebuilding layout should be minimal given 1) it is only triggered on the first edit within
                // the hidden range 2) we are not re-rendering hidden sections.
                if will_rebuild_layout {
                    self.rebuild_layout_and_refresh_diff(ctx);
                }

                ctx.emit(CodeEditorModelEvent::DiffUpdated);
            }
            DiffModelEvent::UnifiedDiffComputed(unified_diff) => {
                ctx.emit(CodeEditorModelEvent::UnifiedDiffComputed(
                    unified_diff.clone(),
                ));
            }
        }
    }

    fn handle_syntax_tree_model_event(
        &mut self,
        event: &DecorationStateEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            DecorationStateEvent::DecorationUpdated { version } => {
                // Once highlighting completes for a content version matching or after the delay rendering block,
                // we flush the delay rendering state to diff and rendering model.
                if self
                    .delay_rendering
                    .as_ref()
                    .map(|delay_rendering| {
                        delay_rendering.should_render_for_syntax_highlight(*version)
                    })
                    .unwrap_or(false)
                {
                    let delay_rendering = self.delay_rendering.take().expect("Checked above");
                    delay_rendering.flush_render(self, ctx);
                }
                ctx.emit(CodeEditorModelEvent::SyntaxHighlightingUpdated);
            }
        }
    }

    fn handle_content_model_event(&mut self, event: &BufferEvent, ctx: &mut ModelContext<Self>) {
        match event {
            BufferEvent::ContentChanged {
                delta,
                origin,
                should_autoscroll,
                buffer_version,
                selection_model_id,
            } => {
                let buffer = self.content().as_ref(ctx);
                let content = buffer.text();
                if self.should_defer_syntax_tree_parsing() {
                    self.pending_syntax_tree_bootstrap = true;
                } else {
                    let buffer_snapshot = buffer.buffer_snapshot();

                    self.syntax_tree.update(ctx, move |syntax_tree, ctx| {
                        syntax_tree.update_internal_state_with_delta(
                            &delta.precise_deltas,
                            *buffer_version,
                            buffer_snapshot,
                            ctx,
                        )
                    });
                }

                let is_from_external_source =
                    *selection_model_id != Some(self.selection_model.id());
                let mut should_recalculate_hidden_lines = false;

                // If the edit is from an external source, we need to 1) materialize hidden range offsets for rendering the edit
                // 2) if the edit overlaps with a hidden range, re-calculate what the new hidden range should be.
                if is_from_external_source {
                    let edit_range = &delta.old_offset;
                    should_recalculate_hidden_lines =
                        self.hidden_lines.update(ctx, |hidden_lines, ctx| {
                            if !hidden_lines.has_offsets_for_version(*buffer_version) {
                                hidden_lines.materialize_hidden_range_offsets(*buffer_version, ctx);
                            }

                            hidden_lines.range_intersects_with_hidden_range_at_version(
                                edit_range,
                                *buffer_version,
                            )
                        });
                }

                if should_recalculate_hidden_lines {
                    if let Some(delay_rendering) = &mut self.delay_rendering {
                        delay_rendering.block_until =
                            DelayRenderingTrigger::DiffUpdate(*buffer_version);
                    } else {
                        self.delay_rendering = Some(DelayRendering::new(
                            DelayRenderingTrigger::DiffUpdate(*buffer_version),
                        ));
                    }
                }

                self.diff.update(ctx, move |diff, ctx| {
                    diff.compute_diff(
                        content,
                        should_recalculate_hidden_lines,
                        *buffer_version,
                        ctx,
                    )
                });

                // If we are delaying rendering, push these updates to the delay rendering state. Otherwise, flush them to diff and rendering model.
                if let Some(delay_rendering) = &mut self.delay_rendering {
                    delay_rendering.edits.push((delta.clone(), *buffer_version));
                    delay_rendering.should_autoscroll = *should_autoscroll;
                } else {
                    self.render_state.update(ctx, move |render_state, _| {
                        render_state.add_pending_edit(delta.clone(), *buffer_version);
                        match should_autoscroll {
                            ShouldAutoscroll::Yes => render_state.request_autoscroll(),
                            ShouldAutoscroll::VerticalOnly => {
                                render_state.request_vertical_autoscroll()
                            }
                            ShouldAutoscroll::No => (),
                        }
                    });
                }

                ctx.emit(CodeEditorModelEvent::ContentChanged { origin: *origin });
            }
            BufferEvent::ContentReplaced { buffer_version } => {
                // Skip delay rendering for test environments. They expect rendering to happen synchronously
                if cfg!(test) {
                    return;
                }
                // On content replacement with active hidden ranges, we should always recalculate hidden lines and delay rendering
                // since all anchors will all be invalidated.
                if self.hide_lines_outside_of_active_diff.is_some() {
                    let content = self.content().as_ref(ctx).text();
                    self.diff.update(ctx, move |diff, ctx| {
                        diff.compute_diff(content, true, *buffer_version, ctx)
                    });

                    if self.delay_rendering.is_none() {
                        self.delay_rendering = Some(DelayRendering::new(
                            DelayRenderingTrigger::DiffUpdate(*buffer_version),
                        ));
                    }
                } else if self.should_defer_syntax_tree_parsing() {
                    self.pending_syntax_tree_bootstrap = true;
                } else if self.delay_rendering.is_none()
                    && self.syntax_tree.as_ref(ctx).has_supported_highlighting()
                {
                    self.delay_rendering = Some(DelayRendering::new(
                        DelayRenderingTrigger::SyntaxHighlighting(*buffer_version),
                    ));
                }
            }
            BufferEvent::SelectionChanged {
                should_autoscroll,
                buffer_version,
                ..
            } => {
                let content = self.content.as_ref(ctx);
                let mut selections =
                    content.to_rendered_selection_set(self.selection_model.clone(), ctx);

                let mut all_selections_entirely_visible = true;
                for selection in selections.iter_mut() {
                    let range = if selection.head <= selection.tail {
                        selection.head..selection.tail
                    } else {
                        selection.tail..selection.head
                    };

                    if self
                        .hidden_lines
                        .as_ref(ctx)
                        .contains_hidden_section(&range, ctx)
                    {
                        all_selections_entirely_visible = false;
                    }
                    selection.head -= CharOffset::from(1);
                    selection.tail -= CharOffset::from(1);
                }

                if !all_selections_entirely_visible
                    && self.interaction_state == InteractionState::Editable
                {
                    self.set_interaction_state(InteractionState::EditableWithInvalidSelection, ctx);
                } else if all_selections_entirely_visible
                    && self.interaction_state == InteractionState::EditableWithInvalidSelection
                {
                    self.set_interaction_state(InteractionState::Editable, ctx);
                }

                self.render_state.update(ctx, move |render_state, _| {
                    render_state.update_selection(selections, *buffer_version);
                    match should_autoscroll {
                        AutoScrollBehavior::Selection => render_state.request_autoscroll(),
                        AutoScrollBehavior::Override(mode) => {
                            render_state.request_autoscroll_to(mode.clone())
                        }
                        AutoScrollBehavior::None => (),
                    }
                });

                self.update_cursor_line_highlights(ctx);
                ctx.emit(CodeEditorModelEvent::SelectionChanged);
            }
            // Handled by selection model.
            BufferEvent::AnchorUpdated { .. } => (),
        }
    }

    /// Update the line highlights for the current cursor positions.
    fn update_cursor_line_highlights(&self, ctx: &mut ModelContext<CodeEditorModel>) {
        let selection_model = self.selection_model.as_ref(ctx);

        let overlay = Appearance::as_ref(ctx).theme().surface_2();

        let highlight_line = if self.diff_nav_is_active() {
            // We don't show current line highlights during diff navigation so we don't need
            // to update the `RenderState`. This lets us keep the line decorations we set
            // for the expanded diff hunks as well.
            None
        } else if selection_model.all_single_cursors() && self.show_current_line_highlights {
            // When diff is not expanded, the only source of line decoration is highlights
            // from the active cursor, e.g. the current line highlight.
            Some(
                selection_model
                    .selected_lines(ctx)
                    .into_iter()
                    .map(|line| {
                        LineDecoration::new(
                            // TODO(CLD-558)
                            LineCount::from(line - 1),
                            LineCount::from(line),
                            overlay,
                        )
                    })
                    .collect(),
            )
        } else {
            // Don't show current line highlights either when text is selected or when
            // current line highlighting is disabled.
            Some(vec![])
        };

        if let Some(highlight_line) = highlight_line {
            self.render_state.update(ctx, move |render_state, _| {
                render_state.set_decorations_after_layout(UpdateDecorationAfterLayout::Line(
                    highlight_line,
                ));
            });
        }
    }

    /// Handle a theme or font change, applying the new rich text styles to the editor.
    /// This will also set the syntax color map and cursor line highlight based on the new theme.
    pub fn handle_appearance_or_font_change(
        &self,
        new_styles: RichTextStyles,
        ctx: &mut ModelContext<Self>,
    ) {
        let style_update = self.render_state.update(ctx, |render_state, _| {
            render_state.update_styles(new_styles)
        });

        match style_update {
            StyleUpdateAction::Relayout => {
                self.rebuild_layout_and_refresh_diff(ctx);
            }
            StyleUpdateAction::None => return,
            StyleUpdateAction::Repaint => (),
        };

        // Rebuild the color map for syntax highlighting as the theme may have changed.
        self.set_color_map(ctx);
        self.update_cursor_line_highlights(ctx);
        ctx.notify();
    }

    /// Begin selecting at `offset`.
    pub fn select_at(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.begin_selection(offset, SelectionMode::Character, !multiselect, ctx);
    }

    // TODO(CLD-1593): This would need to be changed in the future when we have a syntax tree representation.
    pub fn select_word_at(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let policy = SemanticSelection::as_ref(ctx).word_boundary_policy();
        self.begin_selection(offset, SelectionMode::Word(policy), !multiselect, ctx);
    }

    pub fn select_line_at(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.begin_selection(offset, SelectionMode::Line, !multiselect, ctx);
    }

    // TODO(CLD-1593)
    pub fn forward_word(&mut self, select: bool, ctx: &mut ModelContext<Self>) {
        self.forward_word_with_unit(select, word_unit(ctx), ctx)
    }

    // TODO(CLD-1593)
    pub fn backward_word(&mut self, select: bool, ctx: &mut ModelContext<Self>) {
        self.backward_word_with_unit(select, word_unit(ctx), ctx)
    }

    /// Returns the word under or immediately after the primary cursor on the current line,
    /// if one exists. The search does not cross line boundaries.
    /// This is used to populate the find bar for vim's `search_word_at_cursor` (`*` and `#`)
    pub fn word_under_cursor_for_search(&self, app: &AppContext) -> Option<String> {
        let buffer = self.content().as_ref(app);
        let selections = self.selections(app);
        let selection = *selections.first();
        let cursor_offset = selection.head;

        get_word_range_at_offset(buffer, cursor_offset, None)
            .map(|range| buffer.text_in_range(range).into_string())
    }

    pub fn word_range_at_offset(
        &self,
        offset: CharOffset,
        app: &AppContext,
    ) -> Option<Range<CharOffset>> {
        get_word_range_at_offset(self.content().as_ref(app), offset, None)
    }

    pub fn run_search(
        &self,
        config: &SearchConfig,
        ctx: &AppContext,
    ) -> anyhow::Result<impl Future<Output = SearchResults>> {
        let buffer = self.content().as_ref(ctx);
        let search_future = buffer.search(buffer.prepare_search(config)?);

        Ok(async move {
            match search_future.await.1 {
                Ok(mut search_results) => {
                    for match_result in &mut search_results.matches {
                        match_result.start =
                            match_result.start.saturating_sub(&CharOffset::from(1));
                        match_result.end = match_result.end.saturating_sub(&CharOffset::from(1));
                    }
                    search_results
                }
                Err(err) => {
                    log::warn!("Search failed: {err:?}");
                    SearchResults {
                        matches: Vec::new(),
                    }
                }
            }
        })
    }

    /// Copy the current selection.
    /// Note that this is _not_ the only possible code path that could copy selected text to the
    /// clipboard. For example, [`CodeEditorView`] exposes a method that allows the owner of the
    /// view to access selected text and copy it to the clipboard.
    pub fn copy(&self, ctx: &mut ModelContext<Self>) {
        let clipboard = self.read_selected_text_as_clipboard_content(ctx);
        ctx.clipboard().write(clipboard);
    }

    #[cfg(windows)]
    /// If there is selected text, copy it. Otherwise, emit an event to allow
    /// an ancestor to handle the `WindowsCtrlC` event.
    pub fn handle_windows_ctrl_c(&self, ctx: &mut ModelContext<Self>) {
        let buffer = self.content().as_ref(ctx);
        let selected_text =
            buffer.selected_text_as_plain_text(self.buffer_selection_model().clone(), ctx);
        if !selected_text.as_str().is_empty() {
            self.copy(ctx);
            // If the code editor is in a blocklist, this won't clear the
            // selection model there, we have to do that separately. That is
            // handled by emitted the `WindowsCtrlC` event.
            self.clear_selections(ctx);
            ctx.emit(CodeEditorModelEvent::WindowsCtrlC {
                copied_selection: true,
            });
        } else {
            ctx.emit(CodeEditorModelEvent::WindowsCtrlC {
                copied_selection: false,
            });
        }
    }

    pub fn cut(&mut self, ctx: &mut ModelContext<Self>) {
        self.copy(ctx);
        self.backspace(ctx);
    }

    pub fn reset_content(&mut self, state: InitialBufferState, ctx: &mut ModelContext<Self>) {
        self.set_base(state.text, false, ctx);
        // Line ending is now inferred automatically by Buffer::reset.
        self.reset(state, ctx);
    }

    pub fn jump_to_line_column(
        &self,
        line: usize,
        column: Option<usize>,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);

        // If column number is not provided, we should place the cursor before the first non-tab stop character in the line.
        let col_num = match column {
            Some(idx) => idx,
            None => {
                let offset = Point::new(line as u32, 0).to_buffer_char_offset(buffer);
                buffer
                    .indented_line_delta(offset)
                    .unwrap_or(CharOffset::zero())
                    .as_usize()
            }
        };

        let offset = Point::new(line as u32, col_num as u32).to_buffer_char_offset(buffer);

        self.selection_model().update(ctx, |selection, ctx| {
            selection.update_selection(
                BufferSelectAction::AddCursorAt {
                    offset,
                    clear_selections: true,
                },
                AutoScrollBehavior::Override(AutoScrollMode::PositionOffsetInViewportCenter(
                    offset,
                )),
                ctx,
            )
        });
        self.validate(ctx)
    }

    pub fn delete_all_left(&mut self, ctx: &mut ModelContext<Self>) {
        let selection_model = self.selection_model.as_ref(ctx);

        if selection_model.cursors_at_line_start(ctx) {
            self.backspace(ctx);
        } else {
            self.delete(
                TextDirection::Backwards,
                TextUnit::ParagraphBoundary,
                false,
                ctx,
            );
        }
    }

    // The character offset at the start of the input row.
    pub fn start_of_line_offset(&self, row: usize, ctx: &AppContext) -> CharOffset {
        let buffer = self.content().as_ref(ctx);
        // TODO(CLD-558)
        Point::new(row as u32, 0)
            .to_buffer_char_offset(buffer)
            .saturating_sub(&CharOffset::from(1))
    }

    /// Approximate the number of lines in the current viewport with viewport_height / base_line_height.
    pub fn lines_in_viewport(&self, ctx: &AppContext) -> usize {
        let render_state = self.render_state().as_ref(ctx);
        let viewport_height = render_state.viewport().height();
        let line_height = render_state.styles().base_line_height();

        (viewport_height.as_f32() / line_height.as_f32()).ceil() as usize
    }

    pub fn line_count(&self, ctx: &AppContext) -> usize {
        self.buffer().as_ref(ctx).max_point().row.saturating_sub(1) as usize
    }

    pub fn line_height(&self, ctx: &AppContext) -> f32 {
        let render_state = self.render_state().as_ref(ctx);
        render_state.styles().base_line_height().as_f32()
    }

    pub fn paste(&mut self, ctx: &mut ModelContext<Self>) {
        // We only want to read the plain text contents for code editor.
        let content = ctx.clipboard().read();
        self.insert(content.plain_text.as_str(), EditOrigin::UserInitiated, ctx);
    }

    /// Append text to the end of the buffer regardless of cursor position.
    /// This is used for streaming content where we always want to append at the end,
    /// not at the current cursor position.
    pub fn append_at_end(&mut self, text: &str, ctx: &mut ModelContext<Self>) {
        let buffer = self.content().as_ref(ctx);
        let max_offset = buffer.max_charoffset();

        let edits = vec1![(text.to_string(), max_offset..max_offset)];

        let selection_model = self.selection_model.clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                    EditOrigin::SystemEdit,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
        self.validate(ctx);
    }

    /// Set Vim visual tails to the current selection heads (cursor positions).
    pub fn vim_set_visual_tail_to_selection_heads(&mut self, ctx: &mut ModelContext<Self>) {
        let selection_model = self.selection_model.as_ref(ctx);
        let selections = selection_model.selection_offsets();
        self.vim_visual_tails = selections.iter().map(|s| s.head).collect();
    }

    pub fn vim_visual_tails(&self) -> &Vec<CharOffset> {
        &self.vim_visual_tails
    }

    /// Expand the current selection(s) for a visual-mode operation using stored visual tails.
    /// Charwise visual mode includes the character under the block cursor; linewise visual mode
    /// expands to the full line bounds.
    ///
    /// This is used by operators to get the actual selection range for visual operations.
    pub fn vim_visual_selection_range(
        &mut self,
        motion_type: MotionType,
        include_newline: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let selection_model = self.selection_model.as_ref(ctx);
        let buffer = self.content().as_ref(ctx);
        let vim_visual_tails = mem::take(&mut self.vim_visual_tails);

        let new_selections = selection_model
            .selection_offsets()
            .iter()
            .zip(vim_visual_tails.iter())
            .map(|(selection, visual_tail)| {
                let mut start = *visual_tail;
                let mut end = selection.head;
                if start > end {
                    mem::swap(&mut start, &mut end);
                }

                let max_offset = buffer.max_charoffset();
                // Include the char under the block cursor for charwise/visual.
                // For linewise, only include +1 if it won't move onto the next line (i.e., the
                // current char at `end` is not a newline).
                if end < max_offset
                    && (motion_type != MotionType::Linewise
                        || buffer.char_at(end).map(|c| c != '\n').unwrap_or(false))
                {
                    end += 1;
                }

                if motion_type == MotionType::Linewise {
                    let point = start.to_buffer_point(buffer);

                    let start_point = Point::new(point.row, 0);
                    let end_point = end.to_buffer_point(buffer);
                    let new_end = if end_point.column == 0 {
                        end_point
                    } else {
                        Point::new(end_point.row, buffer.line_len(end_point.row))
                    };

                    start = start_point.to_buffer_char_offset(buffer);
                    end = new_end.to_buffer_char_offset(buffer);

                    if include_newline {
                        let start_newline = start.as_usize() > 0
                            && buffer
                                .char_at(start.saturating_sub(&1.into()))
                                .map(|c| c == '\n')
                                .unwrap_or(false);
                        let end_newline = buffer.char_at(end).map(|c| c == '\n').unwrap_or(false);

                        if end_newline && end < max_offset {
                            end += 1;
                        } else if start_newline {
                            start = start.saturating_sub(&1.into());
                        }
                    }
                }

                SelectionOffsets {
                    head: start,
                    tail: end,
                }
            })
            .collect_vec();

        if let Ok(new_selections) = Vec1::try_from_vec(new_selections) {
            self.vim_set_selections_preserving_goal_xs(
                new_selections,
                AutoScrollBehavior::Selection,
                ctx,
            );
        }
    }

    pub fn toggle_comments(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(prefix) = self.syntax_tree.as_ref(ctx).comment_prefix() else {
            return;
        };

        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let lines = selection_model.selected_lines(ctx);
        let all_selected_lines_commented = lines
            .iter()
            .all(|line| buffer.line_decorated_with_prefix(*line, prefix));

        // When we are adding comments, typically editors add an additional whitespace between prefix
        // and the original content of the line.
        let prefix = if !all_selected_lines_commented {
            format!("{prefix} ")
        } else {
            prefix.to_string()
        };

        let selection_model = self.selection_model.clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::TogglePrefixForLines {
                        lines,
                        prefix: &prefix,
                        remove: all_selected_lines_commented,
                    },
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
        self.validate(ctx)
    }

    /// Whether the given range is wrapped in the supported bracket pairs of the active language.
    pub fn range_wrapped_in_bracket(&self, range: Range<CharOffset>, ctx: &AppContext) -> bool {
        let Some(bracket_pairs) = self.syntax_tree.as_ref(ctx).bracket_pairs() else {
            return false;
        };

        let buffer = self.content.as_ref(ctx);

        // Early return if the range is at the start of buffer.
        if range.start == CharOffset::zero() {
            return false;
        }

        let Some(opening_char) = buffer.char_at(range.start - 1) else {
            return false;
        };

        let Some(ending_char) = buffer.char_at(range.end) else {
            return false;
        };

        bracket_pairs
            .iter()
            .any(|(start, end)| *start == opening_char && *end == ending_char)
    }

    pub fn retrieve_unified_diff(&self, file_name: String, ctx: &mut ModelContext<Self>) {
        // Use the buffer's text with normalized line endings, for consistency with how we calculate diffs.
        let content = self
            .content()
            .as_ref(ctx)
            .text_with_line_ending_mode(LineEnding::LF);

        self.diff.update(ctx, move |diff, ctx| {
            diff.retrieve_unified_diff(content, file_name, ctx)
        });
    }

    pub fn interaction_state(&self) -> InteractionState {
        self.interaction_state
    }

    pub fn set_interaction_state(&mut self, state: InteractionState, ctx: &mut ModelContext<Self>) {
        self.interaction_state = state;
        ctx.emit(CodeEditorModelEvent::InteractionStateChanged);
    }

    pub fn set_show_current_line_highlights(&mut self, show_current_line_highlights: bool) {
        self.show_current_line_highlights = show_current_line_highlights;
    }

    pub fn autocomplete_symbol(&mut self, open: char, close: char, ctx: &mut ModelContext<Self>) {
        let buffer = self.content().as_ref(ctx);
        let selections = self.selection_model.as_ref(ctx).selection_offsets();

        let all_cursors = selections.iter().all(|s| s.head == s.tail);

        if all_cursors {
            let pair: String = [open, close].iter().collect();
            let texts: Vec<(String, usize)> =
                selections.iter().map(|_| (pair.clone(), 1)).collect();
            let selection_model = self.selection_model.clone();
            if let Ok(texts) = vec1::Vec1::try_from_vec(texts) {
                self.update_content(
                    |mut content, ctx| {
                        content.apply_edit(
                        warp_editor::content::buffer::BufferEditAction::InsertForEachSelection {
                            texts: &texts,
                        },
                        warp_editor::content::buffer::EditOrigin::UserTyped,
                        selection_model,
                        ctx,
                    );
                    },
                    ctx,
                );
                self.validate(ctx);
            }
            return;
        }

        let edits: Vec<(String, std::ops::Range<CharOffset>)> = selections
            .iter()
            .map(|s| {
                let start = s.head.min(s.tail);
                let end = s.head.max(s.tail);
                let original = buffer.text_in_range(start..end).into_string();
                let mut wrapped = String::new();
                wrapped.push(open);
                wrapped.push_str(&original);
                wrapped.push(close);
                (wrapped, start..end)
            })
            .collect();

        if let Ok(edits) = vec1::Vec1::try_from_vec(edits) {
            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        warp_editor::content::buffer::BufferEditAction::InsertAtCharOffsetRanges {
                            edits: &edits,
                        },
                        warp_editor::content::buffer::EditOrigin::UserTyped,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
            self.validate(ctx);
        }
    }

    pub fn all_cursors_next_character_matches_char(
        &self,
        character: char,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let selections = selection_model.selection_offsets();
        selections
            .iter()
            .all(|s| buffer.char_at(s.head).is_some_and(|c| c == character))
    }

    /// Replace char_count characters starting at the cursor, used by vim
    pub fn replace_char(&mut self, c: char, char_count: u32, ctx: &mut ModelContext<Self>) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let selections = selection_model.selection_offsets();
        let mut edits = Vec::new();
        let mut new_selections_vec = Vec::with_capacity(selections.len());

        for selection in selections.iter() {
            let start = selection.head;
            let line_end = buffer.containing_line_end(start);

            // Don't make edits if we don't have space for the entire replacement on the line.
            // We subtract 1 from the line end to exclude the newline.
            let remaining = (line_end - 1) - start;
            if remaining.as_usize() < char_count as usize {
                // No replacement; keep cursor as-is
                new_selections_vec.push(SelectionOffsets {
                    head: selection.head,
                    tail: selection.tail,
                });
                continue;
            }

            let replacement = c.to_string().repeat(char_count as usize);
            let end = start + CharOffset::from(char_count as usize);
            edits.push((replacement, start..end));
            // After replacement, place cursor on the last replaced character (end - 1)
            let new_pos = end.saturating_sub(&CharOffset::from(1));
            new_selections_vec.push(SelectionOffsets {
                head: new_pos,
                tail: new_pos,
            });
        }

        if let Ok(edits) = Vec1::try_from_vec(edits) {
            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
            // Update selections to the computed positions
            if let Ok(new_selections) = vec1::Vec1::try_from_vec(new_selections_vec) {
                self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
            }
        }
    }

    fn set_color_map(&self, ctx: &mut ModelContext<Self>) {
        let color_map = Self::syntax_highlighting_color_map(ctx);

        self.syntax_tree.update(ctx, |syntax_tree, _ctx| {
            syntax_tree.set_color_map(color_map);
        });
    }

    /// Clear any selections, leaving the cursor at the end of the first selection.
    pub fn clear_selections(&self, ctx: &mut ModelContext<Self>) {
        self.selection.update(ctx, |selection, ctx| {
            let first_selection = *selection.selections(ctx).first();
            selection.update_selection(
                BufferSelectAction::SetSelectionOffsets {
                    selections: vec1![SelectionOffsets {
                        head: first_selection.tail,
                        tail: first_selection.tail,
                    }],
                },
                AutoScrollBehavior::None,
                ctx,
            );
        })
    }

    /// Clear any selections, leaving the cursor at the start of the selection.
    /// This is used for vim-style deselection where the cursor should end up at
    /// the beginning of what was selected (e.g., after yank operations).
    pub fn vim_clear_selections(&mut self, ctx: &mut ModelContext<Self>) {
        self.selection.update(ctx, |selection, ctx| {
            let first_selection = *selection.selections(ctx).first();
            selection.update_selection(
                BufferSelectAction::SetSelectionOffsets {
                    selections: vec1![SelectionOffsets {
                        head: first_selection.head,
                        tail: first_selection.head,
                    }],
                },
                AutoScrollBehavior::None,
                ctx,
            );
        })
    }

    /// Returns true iff any selection is past the last character in the line.
    /// In Vim mode, this scenario needs to be corrected (see [`Self::vim_enforce_cursor_line_cap`]).
    pub fn vim_needs_line_capping(&self, ctx: &AppContext) -> bool {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);

        // Only consider capping when we're using block cursors (i.e., all selections are cursors).
        // Otherwise, if a user clicks-and-drags to select a range of text with vim mode enabled, they
        // might not be able to select the last char in a line.
        if !selection_model.all_single_cursors() {
            return false;
        }

        let selections = selection_model.selection_offsets();

        selections.iter().any(|selection| {
            let head_point = selection.head.to_buffer_point(buffer);
            let line_len = buffer.line_len(head_point.row);
            line_len > 0 && head_point.column >= line_len
        })
    }

    /// If the cursor is after the last character in the line, move it back
    /// so that it's covering the last character in the line instead.
    pub fn vim_enforce_cursor_line_cap(&mut self, ctx: &mut ModelContext<Self>) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let head_point = selection.head.to_buffer_point(buffer);
            let tail_point = selection.tail.to_buffer_point(buffer);

            // Compute line lengths for the head and tail rows independently so we don't
            // incorrectly clamp one end of a multi-line selection using the other's row length.
            let head_line_len = buffer.line_len(head_point.row);
            let tail_line_len = buffer.line_len(tail_point.row);

            let mut new_head = selection.head;
            let mut new_tail = selection.tail;

            if head_line_len > 0 && head_point.column >= head_line_len {
                new_head = selection.head.saturating_sub(&CharOffset::from(1));
            }
            if tail_line_len > 0 && tail_point.column >= tail_line_len {
                new_tail = selection.tail.saturating_sub(&CharOffset::from(1));
            }

            SelectionOffsets {
                head: new_head,
                tail: new_tail,
            }
        });

        self.vim_set_selections_preserving_goal_xs(new_selections, AutoScrollBehavior::None, ctx);
    }

    /// Horizontal cursor movement for vim in the code editor.
    /// Separate from the model's `move_left` and `move_right` functions to allow for stopping at
    /// line boundaries and vim-specific selection logic.
    pub fn vim_move_horizontal_by_offset(
        &mut self,
        char_count: u32,
        direction: &Direction,
        keep_selection: bool,
        stop_at_line_boundary: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let mut head = selection.head;

            if stop_at_line_boundary {
                // For repeat motions that should not cross line boundaries, bound the movement within the line.
                let head_point = head.to_buffer_point(buffer);
                let offset_change = match direction {
                    Direction::Backward => u32::min(head_point.column, char_count),
                    Direction::Forward => {
                        let line_len = buffer.line_len(head_point.row);
                        u32::min(line_len.saturating_sub(head_point.column), char_count)
                    }
                };

                head = match direction {
                    Direction::Backward => {
                        head.saturating_sub(&CharOffset::from(offset_change as usize))
                    }
                    Direction::Forward => {
                        let max_offset = buffer.max_charoffset();
                        cmp::min(max_offset, head + offset_change as usize)
                    }
                };
            } else {
                // Wrapping behavior: step using CharOffsets only and skip over newline characters
                let max_offset = buffer.max_charoffset();
                for _ in 0..char_count {
                    match direction {
                        Direction::Forward => {
                            if head >= max_offset {
                                break;
                            }
                            let next = cmp::min(max_offset, head + 1);

                            if let Some('\n') = buffer.char_at(next) {
                                if keep_selection {
                                    // When selecting (operator-pending), treat the newline as a counted step.
                                    head = next;
                                } else {
                                    let after_next = cmp::min(max_offset, next + 1);
                                    if let Some('\n') = buffer.char_at(after_next) {
                                        // If two chars away is a newline, we have an empty line below
                                        // us that we want to land on
                                        head = next;
                                    } else {
                                        // If two chars away is not a newline, skip the end-of-line newline
                                        head = after_next;
                                    }
                                }
                            } else {
                                head = next;
                            }
                        }
                        Direction::Backward => {
                            if head <= CharOffset::from(1) {
                                break;
                            }
                            let prev = head.saturating_sub(&CharOffset::from(1));

                            if let Some('\n') = buffer.char_at(prev) {
                                if keep_selection {
                                    // When selecting (operator-pending), treat the newline as a counted step.
                                    head = prev;
                                } else {
                                    let prev2 = prev.saturating_sub(&CharOffset::from(1));
                                    if let Some('\n') = buffer.char_at(prev2) {
                                        // If two chars before is a newline, we have an empty line above
                                        // us that we want to land on
                                        head = prev;
                                    } else {
                                        // If two chars before is not a newline, skip the end-of-line newline
                                        head = prev2;
                                    }
                                }
                            } else {
                                // Normal move
                                head = prev;
                            }
                        }
                    }
                }
            }

            SelectionOffsets {
                head,
                tail: if keep_selection { selection.tail } else { head },
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    /// Vertical cursor movement for vim in the code editor.
    /// Treats movement as logical lines (ignores soft-wrap rows).
    pub fn vim_move_vertical_by_offset(
        &mut self,
        count: u32,
        direction: TextDirection,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        // Determine desired column for each selection. If goal_xs exist, interpret them as columns;
        // otherwise initialize from current buffer columns.
        let goal_cols: Vec<u32> =
            if let Some(existing) = self.selection().as_ref(ctx).goal_xs.as_ref() {
                existing
                    .iter()
                    .map(|col| col.as_pixels().as_f32().round() as u32)
                    .collect()
            } else {
                current_selections
                    .iter()
                    .map(|sel| sel.head.to_buffer_point(buffer).column)
                    .collect()
            };

        let max_row = buffer.max_point().row;

        let new_selections_vec: Vec<_> = current_selections
            .iter()
            .enumerate()
            .map(|(i, current_selection)| {
                let cursor = current_selection.head;
                let point = cursor.to_buffer_point(buffer);

                let target_row = match direction {
                    TextDirection::Backwards => point.row.saturating_sub(count),
                    TextDirection::Forwards => cmp::min(max_row, point.row.saturating_add(count)),
                };

                let line_len = buffer.line_len(target_row);
                let new_col = cmp::min(goal_cols[i], line_len);
                let new_offset = Point::new(target_row, new_col).to_buffer_char_offset(buffer);

                SelectionOffsets {
                    head: new_offset,
                    tail: if keep_selection {
                        current_selection.tail
                    } else {
                        new_offset
                    },
                }
            })
            .collect();

        if let Ok(new_selections) = Vec1::try_from_vec(new_selections_vec) {
            self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);

            // Update goal_xs to the desired columns (stored as ColumnUnit::Pixels for
            // consistency with the GUI SelectionModel pixel path)
            let goal_pixels: Vec<_> = goal_cols
                .into_iter()
                .map(|c| ColumnUnit::Pixels((c as usize).into_pixels()))
                .collect();
            self.selection().update(ctx, |selection, _| {
                selection.goal_xs = Vec1::try_from_vec(goal_pixels).ok();
            });
        }
    }

    /// Move cursor to the absolute beginning or end of the line.
    /// Unlike move_to_line_start/end, this always goes to column 0 or the line end regardless of whitespace.
    pub fn vim_move_to_line_bound(
        &mut self,
        bound: LineBound,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_selections = self.selections(ctx);
        let content = self.content().as_ref(ctx);

        let new_selections = current_selections.mapped(|selection_offset| {
            let cursor_offset = selection_offset.head;
            let point = cursor_offset.to_buffer_point(content);

            let new_column = match bound {
                LineBound::Start => 0,
                LineBound::End => content.line_len(point.row),
            };

            let new_offset = Point::new(point.row, new_column).to_buffer_char_offset(content);

            SelectionOffsets {
                head: new_offset,
                tail: if keep_selection {
                    selection_offset.tail
                } else {
                    new_offset
                },
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    /// Move by paragraph boundaries for vim motions.
    pub fn vim_move_by_paragraph(
        &mut self,
        count: u32,
        direction: &Direction,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();
        let max = buffer.max_charoffset();
        let new_selections = current_selections.mapped(|selection| {
            let mut offset = selection.head;
            match direction {
                Direction::Forward => {
                    for _ in 0..count {
                        offset = find_next_paragraph_end(buffer, offset).unwrap_or(max);
                    }
                }
                Direction::Backward => {
                    for _ in 0..count {
                        offset = find_previous_paragraph_start(buffer, offset)
                            .unwrap_or(CharOffset::from(1));
                    }
                }
            }
            SelectionOffsets {
                head: offset,
                tail: if keep_selection {
                    selection.tail
                } else {
                    offset
                },
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    /// Navigate by words using vim-specific word boundaries.
    pub fn vim_navigate_word(
        &mut self,
        direction: Direction,
        bound: WordBound,
        word_type: WordType,
        word_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let start_offset = selection.head;

            let end_offset = if let Ok(boundaries) =
                vim_word_iterator_from_offset(start_offset, buffer, direction, bound, word_type)
            {
                boundaries
                    .take(word_count as usize)
                    .last()
                    .unwrap_or(start_offset)
            } else {
                start_offset
            };

            SelectionOffsets {
                head: end_offset,
                tail: end_offset,
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    pub fn vim_select_for_char_motion(
        &mut self,
        char_motion: &CharacterMotion,
        motion_type: &MotionType,
        operator: &VimOperator,
        operand_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        match char_motion {
            CharacterMotion::Right => {
                self.vim_move_horizontal_by_offset(
                    operand_count,
                    &Direction::Forward,
                    true, // keep_selection
                    true, // stop_at_line_boundary
                    ctx,
                );
            }
            CharacterMotion::Left => {
                self.vim_move_horizontal_by_offset(
                    operand_count,
                    &Direction::Backward,
                    true, // keep_selection
                    true, // stop_at_line_boundary
                    ctx,
                );
            }
            CharacterMotion::Down => {
                self.vim_move_vertical_by_offset(operand_count, TextDirection::Forwards, true, ctx);
            }
            CharacterMotion::Up => {
                self.vim_move_vertical_by_offset(
                    operand_count,
                    TextDirection::Backwards,
                    true,
                    ctx,
                );
            }
            CharacterMotion::WrappingLeft => {
                self.vim_move_horizontal_by_offset(
                    operand_count,
                    &Direction::Backward,
                    true,  // keep_selection
                    false, // don't stop_at_line_boundary (allows wrapping)
                    ctx,
                );
            }
            CharacterMotion::WrappingRight => {
                self.vim_move_horizontal_by_offset(
                    operand_count,
                    &Direction::Forward,
                    true,  // keep_selection
                    false, // don't stop_at_line_boundary (allows wrapping)
                    ctx,
                );
            }
        }

        let include_newline = *operator != VimOperator::Change;
        if *motion_type == MotionType::Linewise {
            self.vim_extend_selection_linewise(include_newline, ctx);
        }
    }

    pub fn vim_select_for_word_motion(
        &mut self,
        word_motion: &WordMotion,
        word_count: u32,
        motion_type: &MotionType,
        operator: &VimOperator,
        ctx: &mut ModelContext<Self>,
    ) {
        let WordMotion {
            direction,
            bound,
            word_type,
        } = word_motion;

        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();
        let new_selections = current_selections.mapped(|selection| {
            let start_offset = selection.head;
            let (cursor_position, selection_start) = if let Ok(boundaries) =
                vim_word_iterator_from_offset(start_offset, buffer, *direction, *bound, *word_type)
            {
                let mut target_pos = boundaries
                    .take(word_count as usize)
                    .last()
                    .unwrap_or(start_offset);

                // Apply vim word boundary quirks and calculate selection boundaries
                match direction {
                    Direction::Forward => {
                        // `de`, unlike other word motions, will include character it lands on
                        // in the operation.
                        if *bound == WordBound::End {
                            target_pos += 1;
                        } else if *bound == WordBound::Start && word_count == 1 {
                            // `dw`, cannot traverse a newline unless the count > 1. We have
                            // to check this range for newlines and cut the range short in that
                            // case.
                            let text = buffer.text_in_range(start_offset..target_pos);
                            if let Some(newline_pos) = text.as_str().find('\n') {
                                target_pos = start_offset + newline_pos;
                            }
                        }
                        (target_pos, start_offset)
                    }
                    Direction::Backward => {
                        // `db` will traverse *but not delete* a newline if the count is 1 and
                        // the cursor starts on column zero and the line above is not empty.
                        let mut actual_start = start_offset;
                        if *bound == WordBound::Start && word_count == 1 {
                            if let Ok(mut char_iter) = buffer.chars_rev_at(start_offset) {
                                if char_iter.next().is_some_and(|c| c == '\n')
                                    && char_iter.next().is_some_and(|c| c != '\n')
                                {
                                    actual_start -= 1;
                                }
                            }
                        }
                        (target_pos, actual_start)
                    }
                }
            } else {
                (start_offset, start_offset)
            };

            SelectionOffsets {
                head: cursor_position,
                tail: selection_start,
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);

        let include_newline = *operator != VimOperator::Change;
        if *motion_type == MotionType::Linewise {
            self.vim_extend_selection_linewise(include_newline, ctx);
        }
    }

    pub fn vim_select_for_line_motion(
        &mut self,
        line_motion: &LineMotion,
        operand_count: u32,
        motion_type: &MotionType,
        operator: &VimOperator,
        ctx: &mut ModelContext<Self>,
    ) {
        match line_motion {
            LineMotion::Start => {
                self.vim_move_to_line_bound(LineBound::Start, true, ctx);
            }
            LineMotion::End => {
                // For $ with count, move down lines first
                if operand_count > 1 {
                    self.vim_move_vertical_by_offset(
                        operand_count - 1,
                        TextDirection::Forwards,
                        true,
                        ctx,
                    );
                }
                self.vim_move_to_line_bound(LineBound::End, true, ctx);
            }
            LineMotion::FirstNonWhitespace => {
                self.vim_move_to_first_nonwhitespace(true, ctx);
            }
        }

        let include_newline = *operator != VimOperator::Change;
        if *motion_type == MotionType::Linewise {
            self.vim_extend_selection_linewise(include_newline, ctx);
        }
    }

    pub fn vim_select_for_first_nonwhitespace_motion(
        &mut self,
        nonws_motion: &FirstNonWhitespaceMotion,
        motion_type: &MotionType,
        operator: &VimOperator,
        operand_count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        match nonws_motion {
            FirstNonWhitespaceMotion::Up => {
                self.vim_move_vertical_by_offset(
                    operand_count,
                    TextDirection::Backwards,
                    true,
                    ctx,
                );
            }
            FirstNonWhitespaceMotion::Down => {
                self.vim_move_vertical_by_offset(operand_count, TextDirection::Forwards, true, ctx);
            }
            FirstNonWhitespaceMotion::DownMinusOne => {
                if operand_count > 0 {
                    self.vim_move_vertical_by_offset(
                        operand_count - 1,
                        TextDirection::Forwards,
                        true,
                        ctx,
                    );
                }
            }
        };

        self.vim_move_to_first_nonwhitespace(true, ctx);

        let include_newline = *operator != VimOperator::Change;
        if *motion_type == MotionType::Linewise {
            self.vim_extend_selection_linewise(include_newline, ctx);
        }
    }

    pub fn vim_select_to_buffer_bound(
        &mut self,
        direction: TextDirection,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let cursor_pos = selection.head;

            let target_pos = match direction {
                TextDirection::Backwards => Point::new(0, 0).to_buffer_char_offset(buffer),
                TextDirection::Forwards => buffer.max_charoffset(),
            };

            SelectionOffsets {
                head: target_pos,
                tail: cursor_pos,
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    /// Select from cursor to buffer start (for dgg command).
    pub fn vim_select_to_buffer_start(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_select_to_buffer_bound(TextDirection::Backwards, ctx);
    }

    /// Select from cursor to buffer end (for dG command).
    pub fn vim_select_to_buffer_end(&mut self, ctx: &mut ModelContext<Self>) {
        self.vim_select_to_buffer_bound(TextDirection::Forwards, ctx);
    }

    pub fn vim_extend_selection_linewise(
        &mut self,
        include_newline: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let start_pos = selection.tail.min(selection.head);
            let end_pos = selection.tail.max(selection.head);

            let start_point = start_pos.to_buffer_point(buffer);
            let end_point = end_pos.to_buffer_point(buffer);

            let line_start = Point::new(start_point.row, 0).to_buffer_char_offset(buffer);
            let start_of_end_line = Point::new(end_point.row, 0).to_buffer_char_offset(buffer);
            let line_end = if include_newline {
                buffer.containing_line_end(start_of_end_line)
            } else {
                // Don't include newline (for change operations)
                buffer.containing_line_end(start_of_end_line) - 1
            };

            SelectionOffsets {
                head: line_end,
                tail: line_start,
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    fn calculate_text_object_range(
        &self,
        text_object: &VimTextObject,
        cursor_pos: CharOffset,
        operator: Option<&VimOperator>,
        ctx: &ModelContext<Self>,
    ) -> Option<Range<CharOffset>> {
        let buffer = self.content().as_ref(ctx);

        match text_object.object_type {
            TextObjectType::Word(word_type) => match text_object.inclusion {
                TextObjectInclusion::Inner => vim_inner_word(buffer, cursor_pos, word_type),
                TextObjectInclusion::Around => vim_a_word(buffer, cursor_pos, word_type),
            },
            TextObjectType::Paragraph => {
                let paragraph_range = match text_object.inclusion {
                    TextObjectInclusion::Inner => vim_inner_paragraph(buffer, cursor_pos),
                    TextObjectInclusion::Around => vim_a_paragraph(buffer, cursor_pos),
                };
                if operator.is_none() {
                    // `vim_a_paragraph` and `vim_inner_paragraph` do _not_ include the trailing
                    // newline. For other operators, we rely on [`Self::vim_extend_selection_linewise`]
                    // to grow the range to include the newline, but we don't call that for visual
                    // mode expansion of text objects.
                    paragraph_range.map(|range| range.start..range.end + 1)
                } else {
                    paragraph_range
                }
            }
            TextObjectType::Quote(quote_type) => match text_object.inclusion {
                TextObjectInclusion::Inner => vim_inner_quote(buffer, cursor_pos, quote_type),
                TextObjectInclusion::Around => vim_a_quote(buffer, cursor_pos, quote_type),
            },
            TextObjectType::Block(bracket_type) => match text_object.inclusion {
                TextObjectInclusion::Inner => {
                    // For inner blocks, preserve leading padding for change operations
                    // Change operations (c) preserve leading padding, delete operations (d) don't
                    // Visual mode (operator is None) doesn't preserve padding
                    let preserve_leading_padding = operator
                        .map(|op| *op == VimOperator::Change)
                        .unwrap_or(false);
                    vim_inner_block(buffer, cursor_pos, bracket_type, preserve_leading_padding)
                }
                TextObjectInclusion::Around => vim_a_block(buffer, cursor_pos, bracket_type),
            },
        }
    }

    /// Select a vim text object, preserving leading padding for change ops.
    /// If operator is None, does selection for visual mode.
    pub fn vim_select_text_object(
        &mut self,
        text_object: &VimTextObject,
        operator: Option<&VimOperator>,
        ctx: &mut ModelContext<Self>,
    ) {
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        // Visual mode (operator is None): compute and store visual tails
        match operator {
            None => {
                let mut visual_tails: Vec<CharOffset> = Vec::new();

                let new_selections = current_selections.mapped(|selection| {
                    let cursor_pos = selection.head;

                    if let Some(mut range) =
                        self.calculate_text_object_range(text_object, cursor_pos, operator, ctx)
                    {
                        visual_tails.push(range.start);
                        if range.end > range.start {
                            range.end -= 1;
                        }

                        let buffer = self.content().as_ref(ctx);
                        // For visual text objects, start the selection at the beginning of the line
                        // (column = 0) of the computed range end.
                        let end_point = range.end.to_buffer_point(buffer);
                        let new_head = Point::new(end_point.row, 0).to_buffer_char_offset(buffer);
                        SelectionOffsets {
                            head: new_head,
                            tail: new_head,
                        }
                    } else {
                        visual_tails.push(cursor_pos);
                        SelectionOffsets {
                            head: cursor_pos,
                            tail: cursor_pos,
                        }
                    }
                });

                self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
                self.vim_visual_tails = visual_tails;
            }
            Some(op) => {
                let new_selections = current_selections.mapped(|selection| {
                    let cursor_pos = selection.head;

                    if let Some(range) =
                        self.calculate_text_object_range(text_object, cursor_pos, operator, ctx)
                    {
                        SelectionOffsets {
                            head: range.end,
                            tail: range.start,
                        }
                    } else {
                        SelectionOffsets {
                            head: cursor_pos,
                            tail: cursor_pos,
                        }
                    }
                });

                self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
                if let TextObjectType::Paragraph = text_object.object_type {
                    let include_newline = *op != VimOperator::Change;
                    self.vim_extend_selection_linewise(include_newline, ctx);
                }
            }
        }
    }

    /// This method does Vim's "%" command. This command checks if there is a bracket under the
    /// cursor, or to the right of the cursor on the same line. If so, jump to the bracket that
    /// matches it.
    pub fn vim_jump_to_matching_bracket(
        &mut self,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let cursor = selection.head;

            // Create char iterator from current position to the end of the line
            let line_end = buffer.containing_line_end(selection.head);
            let line_text = buffer.text_in_range(cursor..line_end).into_string();
            let mut iter = line_text.chars();

            let Some(c) = iter.next() else {
                return selection;
            };

            let (bracket, start_offset) = match BracketChar::try_from(c) {
                Ok(bracket) => (bracket, cursor),

                Err(_) => match iter
                    .enumerate()
                    .find_map(|(i, c)| Some((i, BracketChar::try_from(c).ok()?)))
                {
                    None => return selection,
                    Some((i, bracket)) => (bracket, cursor + i + 1),
                },
            };

            if let Some(bracket_position) =
                vim_find_matching_bracket(buffer, &bracket, start_offset)
            {
                SelectionOffsets {
                    head: bracket_position,
                    tail: if keep_selection {
                        cursor
                    } else {
                        bracket_position
                    },
                }
            } else {
                selection
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    /// This method does Vim's `[` command. It moves to the enclosing bracket around the cursor. It
    /// is similar to the `%` command, but it does not require the cursor to start on a bracket.
    pub fn vim_jump_to_unmatched_bracket(
        &mut self,
        bracket: &BracketChar,
        keep_selection: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let cursor = selection.head;

            if let Some(bracket_position) = vim_find_matching_bracket(buffer, bracket, cursor) {
                SelectionOffsets {
                    head: bracket_position,
                    tail: if keep_selection {
                        cursor
                    } else {
                        bracket_position
                    },
                }
            } else {
                selection
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    /// Vim find character functionality (f, F, t, T commands)
    /// Searches for a character on the current line and moves the cursor to it or before it
    pub fn vim_find_char(
        &mut self,
        keep_selection: bool,
        occurrence_count: u32,
        motion: &FindCharMotion,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let current_selections = selection_model.selection_offsets();

        let new_selections = current_selections.mapped(|selection| {
            let head_point = selection.head.to_buffer_point(buffer);
            let current_column = head_point.column as usize;

            let line_start = buffer.containing_line_start(selection.head);
            let line_end = buffer.containing_line_end(selection.head);
            let line_text = buffer.text_in_range(line_start..line_end).into_string();

            if let Some(new_column) = vim_find_char_on_line(
                &line_text,
                current_column,
                motion,
                occurrence_count,
                keep_selection,
            ) {
                let target_point = Point::new(head_point.row, new_column as u32);
                let new_head = target_point.to_buffer_char_offset(buffer);

                SelectionOffsets {
                    head: new_head,
                    tail: if keep_selection {
                        selection.tail
                    } else {
                        new_head
                    },
                }
            } else {
                // Character not found
                selection
            }
        });

        self.vim_set_selections(new_selections, AutoScrollBehavior::Selection, ctx);
    }

    fn compute_smart_indent_results(
        &self,
        mode: IndentMode,
        ctx: &mut ModelContext<Self>,
    ) -> vec1::Vec1<IndentResult> {
        let buffer = self.content().as_ref(ctx);
        self.selections(ctx).mapped(|selection| {
            let indent_unit = self.syntax_tree.as_ref(ctx).indent_unit();
            let (start, end) = if selection.head > selection.tail {
                (selection.tail, selection.head)
            } else {
                (selection.head, selection.tail)
            };

            let mut position = end.to_buffer_point(buffer);

            // Adjust the position so we calculate the correct smart indentation
            match mode {
                IndentMode::NewlineAbove => {
                    // For indentation, we care about the syntax level of the end of the line above us
                    position.row = position.row.saturating_sub(1);
                    position.column = buffer.line_len(position.row);
                }
                IndentMode::NewlineBelow => {
                    // Want to use end of row position for newline below indent behavior
                    position.column = buffer.line_len(position.row);
                }
                IndentMode::LinewiseChange => {
                    // Want to use the beginning of line indent level for `cc` and newline below
                    position.column = 0;
                }
                IndentMode::Enter => {}
            };

            // The current indent level of the end selection position (used only by
            // IndentMode::Enter).
            let current_indent_num = buffer
                .indented_line_tab_stops(Point::new(position.row, 0).to_buffer_char_offset(buffer))
                .unwrap_or(0);

            // TODO(CLD-558): point and buffer indices off by one
            position.row = position.row.saturating_sub(1);

            let mut levels_to_indent = self
                .syntax_tree
                .as_ref(ctx)
                .indentation_at_point(position, ctx)
                .map(|res| res.delta)
                .unwrap_or(0);

            // Check if the range is wrapped in a bracket for enter behavior.
            //
            // This is relevant in cases like fn {|}, if the cursor is the | and
            // we want to create a higher level of indentation and extra newline
            // when we press enter.
            let range_wrapped_in_bracket = self.range_wrapped_in_bracket(start..end, ctx);
            if range_wrapped_in_bracket {
                // Make sure we don't increase more than one from the current indent level of the line.
                levels_to_indent = (levels_to_indent + 1).min(current_indent_num as u8 + 1);
            }

            let mut insert_before_cursor = String::new();
            let mut insert_after_cursor = None;
            if let Some(text) =
                indent_unit.map(|indent_unit| indent_unit.text_with_num_tab_stops(1))
            {
                insert_before_cursor.push_str(&text.as_str().repeat(levels_to_indent.into()));

                if range_wrapped_in_bracket {
                    // If we are applying bracket expansion, we need to insert additional newline and indentation
                    // for the trailing bracket after the selection.
                    let mut s = String::from("\n");
                    let trail_unit = levels_to_indent.saturating_sub(1);
                    if trail_unit > 0 {
                        s.push_str(&text.as_str().repeat(trail_unit.into()))
                    }
                    insert_after_cursor = Some(s);
                }
            }

            IndentResult {
                insert_before_cursor,
                insert_after_cursor,
            }
        })
    }

    /// Insert a newline above or below the current line with smart indentation
    pub fn vim_newline(&mut self, is_above: bool, ctx: &mut ModelContext<Self>) {
        let indent_mode = if is_above {
            IndentMode::NewlineAbove
        } else {
            IndentMode::NewlineBelow
        };

        let res = self.compute_smart_indent_results(indent_mode, ctx);
        let indents: Vec<String> = res.iter().map(|r| r.insert_before_cursor.clone()).collect();

        if let Ok(indents_vec1) = vec1::Vec1::try_from_vec(indents) {
            let to_insert = format!("{}\n", indents_vec1.first());

            let insert_point = match is_above {
                true => VimInsertPoint::LineStart,
                false => VimInsertPoint::NextLine,
            };

            let offset = to_insert.len();

            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::VimEvent {
                            text: to_insert,
                            insert_point,
                            cursor_offset_len: offset - 1,
                        },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            )
        }
    }

    /// Insert text for Vim insert-repeat ("3I", "4o", etc.) respecting smart indents for `o` and
    /// `O` commands
    pub fn vim_insert_text(
        &mut self,
        text: &str,
        position: &InsertPosition,
        count: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        use InsertPosition as I;

        let text_to_repeat = match position {
            I::LineAbove | I::LineBelow => {
                let is_above = matches!(position, I::LineAbove);

                let indent_mode = if is_above {
                    IndentMode::NewlineAbove
                } else {
                    IndentMode::NewlineBelow
                };
                let res = self.compute_smart_indent_results(indent_mode, ctx);

                let indents: Vec<String> =
                    res.iter().map(|r| r.insert_before_cursor.clone()).collect();
                let indent = vec1::Vec1::try_from_vec(indents)
                    .map(|v| v.first().clone())
                    .unwrap_or_default();

                let mut indented_text = String::with_capacity(indent.len() + text.len() + 1);
                if is_above {
                    // Insert above: indent + text + newline
                    indented_text.push_str(&indent);
                    indented_text.push_str(text);
                    indented_text.push('\n');
                } else {
                    // Insert below: newline + indent + text
                    indented_text.push('\n');
                    indented_text.push_str(&indent);
                    indented_text.push_str(text);
                }
                indented_text
            }
            _ => text.to_owned(),
        };

        let repeated = text_to_repeat.repeat(count as usize);

        let insert_point = match position {
            I::AtCursor => VimInsertPoint::BeforeCursor,
            I::AfterCursor => VimInsertPoint::AtCursor,
            I::LineFirstNonWhitespace => VimInsertPoint::LineFirstNonWhitespace,
            I::LineEnd => VimInsertPoint::LineEnd,
            I::LineAbove => VimInsertPoint::LineStart,
            I::LineBelow => VimInsertPoint::LineEnd,
        };

        let cursor_offset_len = match position {
            I::LineAbove => {
                // Insert text only handles (count - 1) lines of the repetitions.
                // For the `O` command, we need to move the cursor back down to
                // the end of the new count-line insertion.
                (text_to_repeat.len() * (count + 1) as usize).saturating_sub(1)
            }
            _ => repeated.len().saturating_sub(1),
        };

        let selection_model = self.selection_model.clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::VimEvent {
                        text: repeated,
                        insert_point,
                        cursor_offset_len,
                    },
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
    }

    /// Change a line (`cc`) with smart indentation
    pub fn vim_change_line_with_smart_indent(&mut self, ctx: &mut ModelContext<Self>) {
        let res = self.compute_smart_indent_results(IndentMode::LinewiseChange, ctx);
        let texts: Vec<(String, usize)> = res
            .iter()
            .map(|r| {
                let indent = r.insert_before_cursor.clone();
                (indent, r.insert_before_cursor.chars().count())
            })
            .collect();
        if let Ok(texts_vec1) = vec1::Vec1::try_from_vec(texts) {
            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::InsertForEachSelection { texts: &texts_vec1 },
                        EditOrigin::UserTyped,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
        }
    }

    /// Apply a case transformation after adjusting selections with the provided closure.
    pub fn apply_case_transformation_with_selection_change(
        &mut self,
        selection_change: impl FnOnce(&mut CodeEditorModel, &mut ModelContext<CodeEditorModel>),
        transform: CaseTransform,
        ctx: &mut ModelContext<Self>,
    ) {
        selection_change(self, ctx);

        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let ranges = selection_model.selections_to_offset_ranges();

        let edits_vec: Vec<(String, Range<CharOffset>)> = ranges
            .iter()
            .map(|range| {
                let original = buffer.text_in_range(range.clone()).into_string();
                let transformed = transform.apply_to(original);
                (transformed, range.clone())
            })
            .collect();

        if let Ok(edits) = vec1::Vec1::try_from_vec(edits_vec) {
            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
        }

        // Clear selection after transformation
        self.vim_clear_selections(ctx);
    }

    /// Transform the current selections without changing them first (useful for Visual mode).
    pub fn transform_current_selections_case(
        &mut self,
        transform: CaseTransform,
        ctx: &mut ModelContext<Self>,
    ) {
        let buffer = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);
        let ranges = selection_model.selections_to_offset_ranges();

        let edits_vec: Vec<(String, Range<CharOffset>)> = ranges
            .iter()
            .map(|range| {
                let original = buffer.text_in_range(range.clone()).into_string();
                let transformed = transform.apply_to(original);
                (transformed, range.clone())
            })
            .collect();

        if let Ok(edits) = vec1::Vec1::try_from_vec(edits_vec) {
            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
        }

        self.vim_clear_selections(ctx);
    }

    /// Toggle the case of the next `count` characters on the current line and move the cursor forward.
    pub fn vim_toggle_case_chars(&mut self, count: u32, ctx: &mut ModelContext<Self>) {
        // Determine how many characters to toggle on this line
        let buffer = self.content().as_ref(ctx);
        let current_offset = self.selections(ctx).first().head;
        let point = current_offset.to_buffer_point(buffer);
        let line_len = buffer.line_len(point.row);
        let max_count = line_len.saturating_sub(point.column);
        let chars_to_toggle = u32::min(count, max_count);

        // Select from current cursor position forward by chars_to_toggle
        let new_selections = self.selections(ctx).mapped(|selection| SelectionOffsets {
            head: selection.head,
            tail: selection.head + chars_to_toggle as usize,
        });
        self.vim_set_selections(new_selections, AutoScrollBehavior::None, ctx);

        // Apply toggle-case to the current selections
        self.transform_current_selections_case(CaseTransform::Toggle, ctx);

        // Move the cursor to the right by the number of toggled characters
        if chars_to_toggle > 0 {
            self.vim_move_horizontal_by_offset(
                chars_to_toggle,
                &Direction::Forward,
                false,
                true,
                ctx,
            );
        }
    }

    /// Given a line number, return the text for that line.
    fn text_for_line(&self, line_number: LineCount, ctx: &AppContext) -> String {
        let buffer = self.content().as_ref(ctx);
        let offset_start =
            Point::new(line_number.as_usize() as u32, 0).to_buffer_char_offset(buffer);
        let offset_end =
            Point::new(line_number.as_usize() as u32 + 1, 0).to_buffer_char_offset(buffer);

        buffer.text_in_range(offset_start..offset_end).into_string()
    }

    /// Given an original text and its original index, find the closest line in the new version that matches the text.
    fn match_line_to_text(
        &self,
        original_text: &str,
        current_idx: usize,
        max_line: usize,
        predicate: impl Fn(&Self, &str, usize, &AppContext) -> bool,
        ctx: &AppContext,
    ) -> Option<usize> {
        let max_distance = current_idx.max(max_line.saturating_sub(current_idx));

        // For better performance, we search by gradually increasing the search window and early return once we found a match.
        for distance in 0..=max_distance {
            let mut possible_matches = vec![];

            if current_idx >= distance {
                possible_matches.push(current_idx - distance);
            }

            let next_line = current_idx + distance;
            if next_line <= max_line && Some(&next_line) != possible_matches.last() {
                possible_matches.push(next_line);
            }

            for line in possible_matches {
                if predicate(self, original_text, line, ctx) {
                    return Some(line);
                }
            }
        }

        None
    }

    /// After a modification to the code, update the locations of review comments to match their new positions.
    pub fn get_new_line_location(
        &self,
        location: &EditorLineLocation,
        line_text: String,
        ctx: &ModelContext<Self>,
    ) -> (EditorLineLocation, LineDiffContent, bool) {
        let mut used_fallback = false;
        let buffer = self.content.as_ref(ctx);
        let diff_model = self.diff.as_ref(ctx);

        let max_line = buffer.max_point().row as usize;

        let updated_loc = match location {
            EditorLineLocation::Current { line_number, .. } => {
                // For lines in the current version, find the matching comment line
                // closest to the original line number.
                let current_idx = line_number.as_usize();
                let matched_line = self.match_line_to_text(
                    line_text.as_str(),
                    current_idx,
                    max_line,
                    |me, original_text, line, ctx| {
                        let line_text = me.text_for_line(LineCount::from(line + 1), ctx);
                        line_text.trim_end_matches('\n') == original_text
                    },
                    ctx,
                );

                let new_line_number = if let Some(idx) = matched_line {
                    LineCount::from(idx)
                } else {
                    used_fallback = true;
                    *line_number
                };

                let line_range = diff_model
                    .diff_status()
                    .added_diff_range(new_line_number)
                    .unwrap_or_else(|| {
                        new_line_number..LineCount::from(new_line_number.as_usize() + 1)
                    });

                EditorLineLocation::Current {
                    line_number: new_line_number,
                    line_range,
                }
            }
            EditorLineLocation::Removed {
                line_number, index, ..
            } => {
                // For removed lines, we perform the following to find the best match
                // 1. Approximate the line number of the removed line in the base version (This is best effort since we
                // don't track the exact line number in editor locations)
                // 2. Find the matching line closest to (1) in the base version
                // 3. Convert that matching line into a diff editor location
                let current_idx = line_number.as_usize() + *index;

                let max_line = diff_model.base_line_count();
                let matched_line = self.match_line_to_text(
                    line_text.as_str(),
                    current_idx,
                    max_line,
                    |me, original_text, line, ctx| {
                        let line_text = me.diff().as_ref(ctx).base_line(line);
                        line_text.as_ref().map(|s| s.trim_end_matches('\n')) == Some(original_text)
                    },
                    ctx,
                );

                let location =
                    matched_line.and_then(|line| diff_model.base_line_index_to_line_location(line));

                // If we can't convert the location, attach to the closest existing line.
                // TODO: We should have a better flow to handle lines we can't match.
                location.unwrap_or_else(|| {
                    used_fallback = true;
                    let new_line_number = *line_number;
                    let line_range = diff_model
                        .diff_status()
                        .removed_diff_range(new_line_number)
                        .unwrap_or_else(|| {
                            new_line_number..LineCount::from(new_line_number.as_usize() + 1)
                        });

                    EditorLineLocation::Current {
                        line_number: new_line_number,
                        line_range,
                    }
                })
            }
            EditorLineLocation::Collapsed { .. } => location.clone(),
        };

        // Get the new content for the updated location
        let content = if used_fallback {
            LineDiffContent::from_content(line_text.as_str())
        } else {
            self.get_diff_content_for_line(&updated_loc, ctx)
        };
        (updated_loc, content, used_fallback)
    }
}

impl CoreEditorModel for CodeEditorModel {
    type T = CodeEditorModel;

    fn on_buffer_version_updated(
        &self,
        buffer_version: BufferVersion,
        ctx: &mut ModelContext<Self::T>,
    ) {
        // Synchronously convert hidden range anchors into offsets for the given version. This allows the render model
        // to accurately hide line ranges based on the corresponding incoming buffer state.
        self.hidden_lines.update(ctx, |hidden_lines_model, ctx| {
            hidden_lines_model.materialize_hidden_range_offsets(buffer_version, ctx);
        });
    }

    fn content(&self) -> &ModelHandle<Buffer> {
        &self.content
    }

    fn buffer_selection_model(&self) -> &ModelHandle<BufferSelectionModel> {
        &self.selection_model
    }

    fn selection_model(&self) -> &ModelHandle<SelectionModel> {
        &self.selection
    }

    fn render_state(&self) -> &ModelHandle<RenderState> {
        &self.render_state
    }

    fn indent(&mut self, shift: bool, ctx: &mut ModelContext<Self>) {
        let content = self.content().as_ref(ctx);
        let selection_model = self.selection_model.as_ref(ctx);

        // Only apply auto-indent if 1) not shift indenting 2) there is only one single cursor.
        let mut unit = 1;
        if !shift
            && selection_model.is_single_selection()
            && selection_model.first_selection_is_single_cursor()
        {
            let selection_head = selection_model.first_selection_head();
            let mut position = selection_head.to_buffer_point(content);
            // TODO(CLD-558)
            position.row = position.row.saturating_sub(1);

            // Do not apply more than 1 indentation unit if the cursor is not in the leading indentation
            // of the line.
            if content
                .indented_line_start(selection_head)
                .map(|indented_start| indented_start >= selection_head)
                .unwrap_or(false)
            {
                // Check the current indentation level at the cursor position.
                let current_indent_level = content
                    .indented_units_at_offset(selection_head)
                    .unwrap_or(0);
                // The extra indentation delta should be expected_indent_level - current_indent_level.
                // Minimum the delta should be 1.
                unit = self
                    .syntax_tree
                    .as_ref(ctx)
                    .indentation_at_point(position, ctx)
                    .map(|res| res.delta.saturating_sub(current_indent_level).max(1))
                    .unwrap_or(1);
            }
        }

        let selection_model = self.selection_model.clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::Indent {
                        num_unit: unit,
                        shift,
                    },
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
        self.validate(ctx)
    }

    // TODO(kevin): Add validation to the content model.
    fn validate(&self, _ctx: &impl warpui::ModelAsRef) {}

    // Since this is a plain text editor, there is no text styles.
    fn active_text_style(&self) -> warp_editor::content::text::TextStyles {
        Default::default()
    }

    fn backspace(&mut self, ctx: &mut ModelContext<Self::T>) {
        // Check if all selections are a single cursor and one of them is directly after hidden section
        let selection_model = self.buffer_selection_model().as_ref(ctx);

        if selection_model.all_single_cursors()
            && self.hidden_lines.as_ref(ctx).after_hidden_section(ctx)
        {
            return;
        }

        let selection_model = self.selection_model.clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::Backspace,
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                )
            },
            ctx,
        );

        self.validate(ctx);
    }

    fn delete_internal<B>(
        &mut self,
        direction: TextDirection,
        unit: TextUnit,
        cut: bool,
        write_to_clipboard: B,
        ctx: &mut ModelContext<Self::T>,
    ) where
        B: FnOnce(
            &ModelHandle<Buffer>,
            &ModelHandle<BufferSelectionModel>,
            Option<vec1::Vec1<std::ops::Range<string_offset::CharOffset>>>,
            &mut ModelContext<Self::T>,
        ),
    {
        // Use the shared method to compute ranges for deletion
        if let Some(ranges) = self.replacement_range_for_deletion(direction, unit, ctx) {
            // Check if any range would affect hidden sections
            if ranges.iter().any(|range| {
                self.hidden_lines
                    .as_ref(ctx)
                    .contains_hidden_section(range, ctx)
            }) {
                return;
            }

            if cut {
                let content = self.content();
                write_to_clipboard(
                    content,
                    self.buffer_selection_model(),
                    Some(ranges.clone()),
                    ctx,
                );
            }

            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::Delete(ranges),
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            )
        } else {
            // If there's already a selection range, treat this as backspace instead
            if cut {
                let content = self.content();
                write_to_clipboard(content, self.buffer_selection_model(), None, ctx);
            }
            self.backspace(ctx);
        }

        self.validate(ctx);
    }
}

impl CodeEditorModel {
    pub fn open_comment_line(&mut self, line: &EditorLineLocation, ctx: &mut ModelContext<Self>) {
        self.comments.update(ctx, |comments, ctx| {
            comments.pending_comment = PendingComment::Open { line: line.clone() };
            ctx.emit(PendingCommentEvent::NewPendingComment(line.clone()));
        });
    }

    pub fn reopen_comment_line(
        &mut self,
        id: &CommentId,
        line: &EditorLineLocation,
        comment_text: &str,
        origin: &CommentOrigin,
        ctx: &mut ModelContext<Self>,
    ) {
        self.comments.update(ctx, |comments, ctx| {
            comments.pending_comment = PendingComment::Open { line: line.clone() };
            ctx.emit(PendingCommentEvent::ReopenPendingComment {
                id: *id,
                line: line.to_owned(),
                comment_text: comment_text.to_owned(),
                origin: origin.to_owned(),
            });
        });
    }
}

impl PlainTextEditorModel for CodeEditorModel {
    fn enter(&mut self, ctx: &mut ModelContext<Self>) {
        let res = self.compute_smart_indent_results(IndentMode::Enter, ctx);
        let texts: Vec<(String, usize)> = res
            .iter()
            .map(|r| {
                let mut content = String::from('\n');
                content.push_str(&r.insert_before_cursor);
                if let Some(after) = &r.insert_after_cursor {
                    content.push_str(after);
                }
                (content, r.insert_before_cursor.chars().count() + 1) // add one for the newline
            })
            .collect();
        if let Ok(texts) = vec1::Vec1::try_from_vec(texts) {
            let selection_model = self.selection_model.clone();
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::InsertForEachSelection { texts: &texts },
                        EditOrigin::UserTyped,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
        }
    }
}

impl Entity for CodeEditorModel {
    type Event = CodeEditorModelEvent;
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
