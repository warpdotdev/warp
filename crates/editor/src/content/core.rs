use super::{
    buffer::{Buffer, EditOrigin, EditResult},
    cursor::BufferSumTree,
    edit::EditDelta,
    text::{
        BlockType, BufferTextStyle, ColorMarker, LinkCount, LinkMarker, MarkerDir, SyntaxColorId,
        TextStyles, TextStylesWithMetadata,
    },
    undo::{ReversibleEditorAction, UndoArg},
};
use crate::content::{
    anchor::{Anchor, AnchorSide, AnchorUpdate},
    buffer::{StyledBlockBoundaryBehavior, ToBufferByteOffset, ToBufferPoint},
    cursor::BufferCursor,
    edit::PreciseDelta,
    text::{
        BlockHeaderSize, BlockLineBreakBehavior, BufferBlockItem, BufferBlockStyle, BufferText,
        StyleSummary,
    },
};
use enum_iterator::all;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use std::ops::Range;
use string_offset::CharOffset;
use sum_tree::SumTree;
use warpui::elements::ListIndentLevel;

#[derive(Debug, Clone)]
pub struct CoreEditorAction {
    pub range: Range<CharOffset>,
    pub action: CoreEditorActionType,
    /// The anchor side for the start of the edit range. Setting it to AnchorSide::Left is useful
    /// when multiple actions need to be applied on the same range and this action should keep its
    /// start position.
    pub start_anchor_bias: AnchorSide,
    /// The anchor side for the end of the edit range. Setting it to AnchorSide::Right is
    /// useful when multiple actions need to be applied on the same range and one of the action
    /// might modify the size of the range.
    pub end_anchor_bias: AnchorSide,
}

impl CoreEditorAction {
    pub fn new(range: Range<CharOffset>, action: CoreEditorActionType) -> Self {
        Self {
            range,
            action,
            start_anchor_bias: AnchorSide::Right,
            end_anchor_bias: AnchorSide::Left,
        }
    }

    pub fn with_end_anchor_bias(mut self, anchor_side: AnchorSide) -> Self {
        self.end_anchor_bias = anchor_side;
        self
    }

    pub fn with_start_anchor_bias(mut self, anchor_side: AnchorSide) -> Self {
        self.start_anchor_bias = anchor_side;
        self
    }
}

#[derive(Debug, Clone)]
pub enum CoreEditorActionType {
    Insert {
        text: FormattedText,
        // Source of the edit. This impacts the formatting behavior of the insertion.
        // For EditOrigin::UserTyped -- The source is a typed keystroke. Inherit styling for the first line if the edit is inline. Trigger block's linebreak behavior.
        // For EditOrigin::UserInitiated -- The source is a user initiated insertion (e.g. undo). Inherit styling for the first line if the edit is inline.
        // For EditOrigin::SystemEdit -- The source is a system inserted change (e.g. block inserted from block insertion menu). Keep the full inserted styling.
        //
        // Note EditOrigin parameter here has a slightly different meaning from those in the high-level edit action. SystemEdit in the high-level edit action represents
        // actions that are completely initiated by the application and not the user (e.g. replacing the entire editor content by some pre-set message).
        source: EditOrigin,
        // Whether to override style of the remainder of the line after edit range.
        // If this is set to true, the remainder of the line's style will be overridden
        // by the style of the replaced content.
        override_next_style: bool,
        // Whether the insertion is applied on a selection offset set. This changes the behavior on how we are updating anchor states since if the selection is NOT applied
        // on an active selection, we want to clamp instead of invalidating anchors.
        insert_on_selection: bool,
    },
    StyleBlock(BufferBlockStyle),
    StyleText(BufferTextStyle),
    UnstyleText(BufferTextStyle),
    StyleLink(String),
    UnstyleLink,
    /// Ensures that, if the edit range includes the end of the buffer, the buffer ends with plain
    /// text. This is a core editor action because it's difficult to know pre-edit whether or not a
    /// new text marker is needed. For example, if inserting formatted text, it depends on the
    /// specific text being inserted.
    EnsurePlainTextMarker,
}

// Helper struct to keep track of a range's start
// and end anchor.
pub(super) struct RangeAnchors {
    pub(super) start: Anchor,
    pub(super) end: Anchor,
}

#[derive(Debug, Clone)]
pub struct ReplacementRange {
    pub new_range: Range<CharOffset>,
    pub old_range: Range<CharOffset>,
}

impl ReplacementRange {
    fn reversed(&self) -> Self {
        Self {
            new_range: self.old_range.clone(),
            old_range: self.new_range.clone(),
        }
    }
}

pub struct CoreEditorActionResult {
    /// The range of inserted characters after the core editor action.
    pub updated_range: Range<CharOffset>,
    /// The corresponding anchor update applied with the action.
    pub anchor_update: Option<AnchorUpdate>,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum CursorType {
    // Cursor is at the start of the buffer. We need to add a new starting
    // block marker.
    BufferStart,
    // Cursor is at the start of a newline.
    NewLineStart,
    // Cursor is inline.
    Inline,
}

impl Buffer {
    pub(super) fn apply_core_edit_actions(
        &mut self,
        actions: impl IntoIterator<Item = CoreEditorAction>,
    ) -> EditResult {
        // If there are no actions, return early.
        let mut actions_iter = actions.into_iter().peekable();
        if actions_iter.peek().is_none() {
            return EditResult::default();
        }

        log::trace!("Applying core editor actions");
        let mut action_and_anchors = Vec::new();
        let mut replaced_ranges: Option<Range<CharOffset>> = None;

        for action in actions_iter {
            let range = action.range.clone();

            let start_side = action.start_anchor_bias;

            // If the start of the range is the same as the end of the range, make sure their side is
            // consistent to avoid the case where start is after end offset after updates.
            // If the start has left bias, then the end will be >= the start, regardless of its
            // bias.
            let end_side = if range.start == range.end && start_side == AnchorSide::Right {
                AnchorSide::Right
            } else {
                action.end_anchor_bias
            };

            let anchors = RangeAnchors {
                start: self.internal_anchors.create_anchor(range.start, start_side),
                end: self.internal_anchors.create_anchor(range.end, end_side),
            };

            action_and_anchors.push((action, anchors));

            // For old replaced ranges we don't need to use anchors as they shouldn't be shifted by
            // the updates we apply.
            let replace_start = self.block_or_line_start(range.start);
            // block_or_line_end, containing_line_end, and containing_block_end all assume that
            // blocks end in a newline, block marker, or block item, so they unconditionally add 1
            // to get the exclusive end of a block (the first character after it).
            //
            // This assumption is true of all blocks except a plain-text block at the end of the
            // buffer, which generally will not end in a newline.
            //
            // We _could_ account for that case in those functions, by checking what the text at
            // the expected end offset is. However, various editing operations also rely on this
            // behavior (e.g. by subtracting 1 to get the offset of the ending marker/item), so
            // those functions need to consistently add 1.
            // Instead, handle it here by checking max_charoffset, so that the ranges for
            // re-rendering are as expected.
            let replace_end = self.block_or_line_end(range.end).min(self.max_charoffset());

            if let Some(replaced_ranges) = &mut replaced_ranges {
                replaced_ranges.start = replaced_ranges.start.min(replace_start);
                replaced_ranges.end = replaced_ranges.end.max(replace_end);
            } else {
                replaced_ranges = Some(replace_start..replace_end);
            }
        }

        let mut reverse_actions = Vec::new();
        let mut new_range_anchors = Vec::new();
        let mut precise_deltas = Vec::new();
        let mut anchor_updates = Vec::new();
        // Anchors tracking each delta's new content range, resolved after all edits
        // to get correct final-buffer coordinates.
        let mut new_content_range_anchors: Vec<RangeAnchors> = Vec::new();

        for (action, anchors) in action_and_anchors {
            // When applying the action, we need to read out its updated offset range from its anchors
            // since a previous action could shift the offset.
            let edit_start = self
                .internal_anchors
                .resolve(&anchors.start)
                .expect("Anchor should exist");
            let edit_end = self
                .internal_anchors
                .resolve(&anchors.end)
                .expect("Anchor should exist");
            log::trace!("Start anchor => {edit_start}, end anchor => {edit_end}");

            // Safety: if a previous edit in this batch caused the anchors for this
            // action to cross (start > end), skip the action rather than panicking.
            // This can happen when overlapping DiffDeltas slip through the diff
            // matching layer (see WARP-CLIENT-DEV-NYY).
            if edit_start > edit_end {
                log::warn!(
                    "Skipping edit action with inverted range {edit_start}..{edit_end} \
                     (anchors crossed after a prior edit in the same batch)"
                );
                continue;
            }

            let edit_range = edit_start..edit_end;
            let replaced_points = self.offset_range_to_point_range(edit_range.clone());

            // Compute pre-edit byte range from the correct intermediate buffer state.
            let old_byte_start = edit_range.start.to_buffer_byte_offset(self);
            let old_byte_end = edit_range.end.to_buffer_byte_offset(self);

            let reverse_action_type =
                self.reverse_core_edit_action(action.clone(), edit_range.clone());

            let result = self.apply_core_edit_action(CoreEditorAction::new(
                edit_range.clone(),
                action.action.clone(),
            ));

            if let Some(anchor_update) = result.anchor_update {
                anchor_updates.push(anchor_update);
            }
            let update_range = result.updated_range;
            log::trace!("=> Reverse action: {reverse_action_type:?}");

            // Compute post-edit byte length and end point from the correct intermediate
            // buffer state (after this edit, before subsequent edits).
            let new_byte_length = update_range
                .end
                .to_buffer_byte_offset(self)
                .as_usize()
                .saturating_sub(old_byte_start.as_usize());
            let new_end_point = update_range.end.to_buffer_point(self);

            // We could skip pushing no-op edits to precise delta.
            if !edit_range.is_empty() || !update_range.is_empty() {
                // Anchor the new content range so it can be resolved against the final
                // buffer state after all edits are applied.
                new_content_range_anchors.push(RangeAnchors {
                    start: self
                        .internal_anchors
                        .create_anchor(update_range.start, AnchorSide::Right),
                    end: self
                        .internal_anchors
                        .create_anchor(update_range.end, AnchorSide::Left),
                });
                precise_deltas.push(PreciseDelta {
                    replaced_range: edit_range,
                    replaced_points,
                    // Placeholder — resolved below after all edits are applied.
                    resolved_range: update_range.clone(),
                    replaced_byte_range: old_byte_start..old_byte_end,
                    new_byte_length,
                    new_end_point,
                });
            }

            let reverse_action = CoreEditorAction::new(update_range.clone(), reverse_action_type);
            reverse_actions.push(ReversibleEditorAction {
                next: reverse_action,
                reverse: action,
            });

            let new_range = self.block_or_line_start(update_range.start)
                ..self
                    .block_or_line_end(update_range.end)
                    // See the comment on replace_end above for why the max_offset check is needed.
                    .min(self.max_charoffset());
            log::trace!("=> New block range: {}..{}", new_range.start, new_range.end);

            let new_anchors = RangeAnchors {
                start: self
                    .internal_anchors
                    .create_anchor(new_range.start, AnchorSide::Right),
                end: self
                    .internal_anchors
                    .create_anchor(new_range.end, AnchorSide::Left),
            };

            new_range_anchors.push(new_anchors);
        }

        // If every action in the batch was skipped (e.g. all had inverted ranges),
        // there is nothing to commit — return early.
        if new_range_anchors.is_empty() {
            return EditResult::default();
        }

        // Resolve each delta's new content range anchors against the final buffer state.
        // If a later action in the batch deletes the content an earlier action inserted,
        // the earlier action's anchors will have been invalidated — drop those deltas.
        let precise_deltas: Vec<PreciseDelta> = precise_deltas
            .into_iter()
            .zip(new_content_range_anchors)
            .filter_map(|(mut delta, anchors)| {
                match (
                    self.internal_anchors.resolve(&anchors.start),
                    self.internal_anchors.resolve(&anchors.end),
                ) {
                    (Some(start), Some(end)) => {
                        delta.resolved_range = start..end;
                        Some(delta)
                    }
                    _ => None,
                }
            })
            .collect();

        let old_range = replaced_ranges.expect("Should have one range");
        reverse_actions.reverse();
        let replacement_range = ReplacementRange {
            old_range,
            new_range: self.anchors_to_range(new_range_anchors),
        };
        let undo_arg = UndoArg {
            actions: reverse_actions,
            replacement_range: replacement_range.reversed(),
        };

        log::debug!(
            "=> Overall previous range: {:?}",
            replacement_range.old_range
        );
        log::debug!("=> Overall new range: {:?}", replacement_range.new_range);

        let new_lines = self.styled_blocks_in_range(
            replacement_range.new_range,
            StyledBlockBoundaryBehavior::Exclusive,
        );

        EditResult {
            undo_item: Some(undo_arg),
            delta: Some(EditDelta {
                precise_deltas,
                old_offset: replacement_range.old_range,
                new_lines,
            }),
            anchor_updates,
        }
    }

    /// If the active block style is plain text, return line start. Else, return block start.
    pub fn block_or_line_start(&self, offset: CharOffset) -> CharOffset {
        if offset == CharOffset::zero() {
            // Normally, CharOffset 0 is inaccessible to editor actions - we consider the first
            // character of a block to be the first character _after_ its start marker, so 0 is
            // not really part of a block. However, we need to be able to edit it in some cases
            // when styling at the start of the buffer.
            offset
        } else if self.block_type_at_point(offset) == BlockType::Text(BufferBlockStyle::PlainText) {
            self.containing_line_start(offset)
        } else {
            self.containing_block_start(offset)
        }
    }

    pub fn block_start(&self, offset: CharOffset) -> CharOffset {
        if offset == CharOffset::zero() {
            // Normally, CharOffset 0 is inaccessible to editor actions - we consider the first
            // character of a block to be the first character _after_ its start marker, so 0 is
            // not really part of a block. However, we need to be able to edit it in some cases
            // when styling at the start of the buffer.
            offset
        } else {
            self.containing_block_start(offset)
        }
    }

    /// If the active block style is plain text, return line end. Else, return block end.
    pub fn block_or_line_end(&self, offset: CharOffset) -> CharOffset {
        if offset == CharOffset::zero() {
            // End offsets are exclusive, so the "block" ends at 1. The very start of the buffer
            // generally requires special handling anyways.
            CharOffset::from(1)
        } else if self.block_type_at_point(offset) == BlockType::Text(BufferBlockStyle::PlainText) {
            self.containing_line_end(offset)
        } else {
            self.containing_block_end(offset)
        }
    }

    // Find the minimal range of character offset that covers the list of range
    // represented by anchors.
    fn anchors_to_range(&self, anchors: Vec<RangeAnchors>) -> Range<CharOffset> {
        let range_start = anchors
            .iter()
            .filter_map(|range| self.internal_anchors.resolve(&range.start))
            .min();
        let range_end = anchors
            .iter()
            .filter_map(|range| self.internal_anchors.resolve(&range.end))
            .max();

        range_start.expect("Range start should be non-empty")
            ..range_end.expect("Range end should be non-empty")
    }

    pub(super) fn apply_core_edit_action(
        &mut self,
        action: CoreEditorAction,
    ) -> CoreEditorActionResult {
        log::trace!("Applying {:?}", action.action);
        log::trace!("=> Edit range: {:?}", action.range);
        log::trace!("=> Initial buffer: [{}]", self.debug());
        let result = match action.action {
            CoreEditorActionType::Insert {
                text,
                source,
                override_next_style,
                insert_on_selection: clamp_anchor,
            } => self.edit(
                action.range,
                text,
                clamp_anchor,
                source,
                override_next_style,
            ),
            CoreEditorActionType::StyleBlock(style) => self.style_block(action.range, style),
            CoreEditorActionType::StyleText(style) => self.style_text(action.range, style),
            CoreEditorActionType::UnstyleText(style) => self.unstyle_text(action.range, style),
            CoreEditorActionType::StyleLink(url) => self.style_link(action.range, url),
            CoreEditorActionType::UnstyleLink => self.unstyle_link(action.range),
            CoreEditorActionType::EnsurePlainTextMarker => self.ensure_plain_text(action.range),
        };
        log::trace!("=> Updated buffer: [{}]", self.debug());
        log::trace!("=> Update range: {:?}", result.updated_range);
        result
    }

    pub fn reverse_core_edit_action(
        &self,
        action: CoreEditorAction,
        range: Range<CharOffset>,
    ) -> CoreEditorActionType {
        match action.action {
            CoreEditorActionType::StyleBlock(_) => match self.block_type_at_point(range.start) {
                BlockType::Item(_) => {
                    panic!("Reverse action on style block should not start with a block item")
                }
                BlockType::Text(block_style) => CoreEditorActionType::StyleBlock(block_style),
            },
            CoreEditorActionType::UnstyleText(style) => CoreEditorActionType::StyleText(style),
            CoreEditorActionType::StyleText(style) => CoreEditorActionType::UnstyleText(style),
            CoreEditorActionType::Insert { .. } => {
                let original_text =
                    self.range_to_formatted_text(range, StyledBlockBoundaryBehavior::Inclusive);
                CoreEditorActionType::Insert {
                    text: original_text,
                    source: EditOrigin::UserInitiated,
                    override_next_style: true,
                    insert_on_selection: true,
                }
            }
            CoreEditorActionType::StyleLink(_) => CoreEditorActionType::UnstyleLink,
            CoreEditorActionType::UnstyleLink => CoreEditorActionType::StyleLink(
                self.link_url_at_offset(range.start)
                    .expect("url should exist"),
            ),
            // EnsurePlainTextMarker captures whether or not a marker was added with its updated
            // range, so the reverse is to delete whatever was in that range.
            CoreEditorActionType::EnsurePlainTextMarker => CoreEditorActionType::Insert {
                text: FormattedText::new([]),
                source: EditOrigin::SystemEdit,
                override_next_style: false,
                insert_on_selection: true,
            },
        }
    }

    fn edit(
        &mut self,
        range: Range<CharOffset>,
        text: FormattedText,
        insert_on_selection: bool,
        source: EditOrigin,
        override_next_style: bool,
    ) -> CoreEditorActionResult {
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

        // Determine the edit range on the old content, before we modify it.
        let old_content = self.content.clone();
        let cursor = old_content.cursor::<CharOffset, StyleSummary>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        let mut new_content = SumTree::new();

        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.start));
        let character_count_at_range_start = new_content.extent::<CharOffset>();

        let mut previous_block_type = self.block_type_at_point(range.start);

        // An edit is inline if the selection range doesn't start from a line start to a line end.
        let is_edit_inline = range.start != self.containing_line_start(range.start)
            || range.end != (self.containing_line_end(range.end) - 1);
        let mut edit_cursor = if range.start == CharOffset::zero() {
            CursorType::BufferStart
        } else if is_edit_inline {
            CursorType::Inline
        } else {
            CursorType::NewLineStart
        };

        // This tracks whether we should override the style of the remainder of the line. This will be set to true
        // when the last inserted line includes a newline created by the BlockLineBreakBehavior.
        let mut should_override_next_block_style = false;

        // Only inherit styling if this is a user initiated or typed action. For system actions, we should keep the style
        // as it is.
        let mut inherit_styling = source.from_user();

        for line in text.lines {
            should_override_next_block_style = false;
            match line {
                FormattedTextLine::LineBreak => {
                    end_all_active_text_styles(&mut new_content);

                    match previous_block_type.clone() {
                        BlockType::Item(_) => {
                            update_content_after_block_item(
                                &mut new_content,
                                &mut previous_block_type,
                                BlockType::Text(BufferBlockStyle::PlainText),
                                None,
                            );
                        }
                        BlockType::Text(previous_block_style) => {
                            match source {
                                EditOrigin::UserTyped => {
                                    match previous_block_style.line_break_behavior() {
                                        // When at buffer start, we need to push a plain text marker to make
                                        // sure the buffer content is valid.
                                        BlockLineBreakBehavior::NewLine
                                            if edit_cursor == CursorType::BufferStart =>
                                        {
                                            new_content.push(BufferText::BlockMarker {
                                                marker_type: BufferBlockStyle::PlainText,
                                            });
                                            previous_block_type =
                                                BlockType::Text(BufferBlockStyle::PlainText);
                                        }
                                        BlockLineBreakBehavior::NewLine => {
                                            new_content.push(BufferText::Newline)
                                        }
                                        BlockLineBreakBehavior::BlockMarker(marker_type) => {
                                            new_content.push(BufferText::BlockMarker {
                                                marker_type: marker_type.clone(),
                                            });
                                            should_override_next_block_style = true;
                                            previous_block_type = BlockType::Text(marker_type);
                                        }
                                    }
                                }
                                EditOrigin::SystemEdit | EditOrigin::UserInitiated => {
                                    match previous_block_style.line_break_behavior() {
                                        BlockLineBreakBehavior::NewLine
                                            if edit_cursor != CursorType::BufferStart =>
                                        {
                                            new_content.push(BufferText::Newline)
                                        }
                                        _ => {
                                            new_content.push(BufferText::BlockMarker {
                                                marker_type: BufferBlockStyle::PlainText,
                                            });
                                            previous_block_type =
                                                BlockType::Text(BufferBlockStyle::PlainText);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // We only insert a newline when we have a formatted text line after another one.
                // This is to avoid additional newline inserted when we create other types of blocks (e.g. code block).
                FormattedTextLine::Line(line) => {
                    let block_style = BufferBlockStyle::PlainText;

                    match previous_block_type.clone() {
                        BlockType::Item(_) => {
                            update_content_after_block_item(
                                &mut new_content,
                                &mut previous_block_type,
                                BlockType::Text(block_style),
                                Some(line),
                            );
                        }
                        BlockType::Text(previous_block_style) => {
                            // For plain text lines, if we are inheriting the styling and the cursor is not
                            // at the start of the buffer, we should always push the content without altering
                            // the existing block styling.
                            if inherit_styling
                                && block_style
                                    .should_inherit_style(edit_cursor, previous_block_style.clone())
                            {
                                push_text_fragments_to_block(
                                    &mut new_content,
                                    line,
                                    previous_block_style.clone(),
                                );
                            } else {
                                end_all_active_text_styles(&mut new_content);
                                if previous_block_style != block_style
                                    || edit_cursor == CursorType::BufferStart
                                {
                                    new_content.push(BufferText::BlockMarker {
                                        marker_type: block_style.clone(),
                                    });
                                } else if edit_cursor == CursorType::NewLineStart {
                                    new_content.push(BufferText::Newline);
                                }

                                previous_block_type = BlockType::Text(block_style);
                                update_content_with_text_fragments(&mut new_content, line);
                            }
                        }
                    }
                }
                FormattedTextLine::CodeBlock(block) => {
                    let block_style = BufferBlockStyle::CodeBlock {
                        code_block_type: (&block).into(),
                    };
                    insert_multiline_block(
                        &block.code,
                        block_style,
                        source,
                        inherit_styling,
                        edit_cursor,
                        &mut new_content,
                        &mut previous_block_type,
                    );
                }
                FormattedTextLine::Table(table) => {
                    let internal_text = table.to_internal_format();
                    insert_multiline_block(
                        &internal_text,
                        BufferBlockStyle::table(table.alignments.clone()),
                        source,
                        inherit_styling,
                        edit_cursor,
                        &mut new_content,
                        &mut previous_block_type,
                    );
                }
                FormattedTextLine::Heading(text) => {
                    let header_size: BlockHeaderSize = text
                        .heading_size
                        .try_into()
                        .unwrap_or(BlockHeaderSize::Header1);

                    let block_style = BufferBlockStyle::Header { header_size };

                    match previous_block_type.clone() {
                        BlockType::Item(_) => {
                            update_content_after_block_item(
                                &mut new_content,
                                &mut previous_block_type,
                                BlockType::Text(block_style),
                                Some(text.text),
                            );
                        }
                        BlockType::Text(previous_block_style) => {
                            if inherit_styling
                                && block_style
                                    .should_inherit_style(edit_cursor, previous_block_style.clone())
                            {
                                push_text_fragments_to_block(
                                    &mut new_content,
                                    text.text,
                                    previous_block_style,
                                );
                            } else {
                                maybe_push_new_block_marker(
                                    inherit_styling,
                                    edit_cursor,
                                    previous_block_style,
                                    block_style.clone(),
                                    &mut new_content,
                                );

                                previous_block_type = BlockType::Text(block_style);
                                update_content_with_text_fragments(&mut new_content, text.text);
                            }
                        }
                    }
                }
                FormattedTextLine::UnorderedList(list) => {
                    let block_style = BufferBlockStyle::UnorderedList {
                        indent_level: ListIndentLevel::from_usize(list.indent_level),
                    };

                    match previous_block_type.clone() {
                        BlockType::Item(_) => {
                            update_content_after_block_item(
                                &mut new_content,
                                &mut previous_block_type,
                                BlockType::Text(block_style),
                                Some(list.text),
                            );
                        }
                        BlockType::Text(previous_block_style) => {
                            if inherit_styling
                                && block_style
                                    .should_inherit_style(edit_cursor, previous_block_style.clone())
                            {
                                push_text_fragments_to_block(
                                    &mut new_content,
                                    list.text,
                                    previous_block_style,
                                );
                            } else {
                                maybe_push_new_block_marker(
                                    inherit_styling,
                                    edit_cursor,
                                    previous_block_style,
                                    block_style.clone(),
                                    &mut new_content,
                                );

                                previous_block_type = BlockType::Text(block_style);
                                update_content_with_text_fragments(&mut new_content, list.text);
                            }
                        }
                    }
                }
                FormattedTextLine::OrderedList(list) => {
                    let block_style = BufferBlockStyle::OrderedList {
                        indent_level: ListIndentLevel::from_usize(list.indented_text.indent_level),
                        number: list.number,
                    };

                    match previous_block_type.clone() {
                        BlockType::Item(_) => {
                            update_content_after_block_item(
                                &mut new_content,
                                &mut previous_block_type,
                                BlockType::Text(block_style),
                                Some(list.indented_text.text),
                            );
                        }
                        BlockType::Text(previous_block_style) => {
                            if inherit_styling
                                && block_style
                                    .should_inherit_style(edit_cursor, previous_block_style.clone())
                            {
                                push_text_fragments_to_block(
                                    &mut new_content,
                                    list.indented_text.text,
                                    previous_block_style.clone(),
                                );
                            } else {
                                maybe_push_new_block_marker(
                                    inherit_styling,
                                    edit_cursor,
                                    previous_block_style.clone(),
                                    block_style.clone(),
                                    &mut new_content,
                                );

                                previous_block_type = BlockType::Text(block_style);
                                // When parsing formatted text, we ignore the given numbers. Instead, numbering
                                // is automatically calculated at render-time based on the content structure.
                                update_content_with_text_fragments(
                                    &mut new_content,
                                    list.indented_text.text,
                                );
                            }
                        }
                    }
                }
                FormattedTextLine::TaskList(list) => {
                    let block_style = BufferBlockStyle::TaskList {
                        indent_level: ListIndentLevel::from_usize(list.indent_level),
                        complete: list.complete,
                    };

                    match previous_block_type.clone() {
                        BlockType::Item(_) => {
                            update_content_after_block_item(
                                &mut new_content,
                                &mut previous_block_type,
                                BlockType::Text(block_style),
                                Some(list.text),
                            );
                        }
                        BlockType::Text(previous_block_style) => {
                            if inherit_styling
                                && block_style
                                    .should_inherit_style(edit_cursor, previous_block_style.clone())
                            {
                                push_text_fragments_to_block(
                                    &mut new_content,
                                    list.text,
                                    previous_block_style.clone(),
                                );
                            } else {
                                maybe_push_new_block_marker(
                                    inherit_styling,
                                    edit_cursor,
                                    previous_block_style.clone(),
                                    block_style.clone(),
                                    &mut new_content,
                                );

                                previous_block_type = BlockType::Text(block_style);
                                // When parsing formatted text, we ignore the given numbers. Instead, numbering
                                // is automatically calculated at render-time based on the content structure.
                                update_content_with_text_fragments(&mut new_content, list.text);
                            }
                        }
                    }
                }
                FormattedTextLine::HorizontalRule => {
                    end_all_active_text_styles(&mut new_content);
                    update_content_after_block_item(
                        &mut new_content,
                        &mut previous_block_type,
                        BlockType::Item(BufferBlockItem::HorizontalRule),
                        None,
                    );
                }
                FormattedTextLine::Image(image) => {
                    end_all_active_text_styles(&mut new_content);
                    update_content_after_block_item(
                        &mut new_content,
                        &mut previous_block_type,
                        BlockType::Item(BufferBlockItem::Image {
                            alt_text: image.alt_text.clone(),
                            source: image.source.clone(),
                            title: image.title.clone(),
                        }),
                        None,
                    );
                }
                FormattedTextLine::Embedded(metadata) => {
                    // TODO(kevin): Render broken embedded item state instead of skipping.
                    if let Some(item) = self
                        .embedded_item_conversion
                        .as_ref()
                        .and_then(|conversion| (conversion)(metadata))
                    {
                        end_all_active_text_styles(&mut new_content);
                        update_content_after_block_item(
                            &mut new_content,
                            &mut previous_block_type,
                            BlockType::Item(BufferBlockItem::Embedded { item }),
                            None,
                        );
                    }
                }
            }
            edit_cursor = CursorType::NewLineStart;
            inherit_styling = false;
        }

        let mut total_length = new_content.extent::<CharOffset>() - character_count_at_range_start;

        let active_style_summary = new_content.extent::<StyleSummary>();
        let active_text_style =
            active_text_styles_with_metadata_from_subtree(&new_content, active_style_summary);
        let range_end_block_type = self.block_type_at_point(range.end);

        buffer_cursor.slice_to_offset_after_markers(range.end);

        match previous_block_type {
            BlockType::Item(_) => {
                if let BlockType::Text(block_style) = range_end_block_type {
                    match buffer_cursor.item() {
                        Some(BufferText::BlockItem { .. })
                        | Some(BufferText::BlockMarker { .. }) => (),
                        Some(BufferText::Newline) => {
                            buffer_cursor.next();
                            new_content.push(BufferText::BlockMarker {
                                marker_type: block_style,
                            });
                        }
                        Some(_) => {
                            new_content.push(BufferText::BlockMarker {
                                marker_type: block_style,
                            });
                            total_length += 1;
                        }
                        None => (),
                    }
                }

                // Add starting marker for active text styles.
                let suffix_styling = *buffer_cursor.start();
                let suffix_text_styling =
                    active_text_styles_with_metadata_from_subtree(&old_content, suffix_styling);
                let mut handled_weight = false;
                for style in all::<BufferTextStyle>() {
                    let is_weight = style.has_custom_weight();
                    if is_weight && handled_weight {
                        continue;
                    }
                    if suffix_text_styling.exact_match_style(&style)
                        && !active_text_style.exact_match_style(&style)
                    {
                        handled_weight |= is_weight;
                        new_content.push(BufferText::Marker {
                            marker_type: style,
                            dir: MarkerDir::Start,
                        });
                    }
                }

                transition_styles_with_metadata(
                    &active_text_style,
                    &suffix_text_styling,
                    &mut new_content,
                );
            }
            BlockType::Text(mut previous_block_styling) => {
                // Make sure all style start/end markers are still matched after the insertion.
                let suffix_styling = *buffer_cursor.start();
                let suffix_text_styling =
                    active_text_styles_with_metadata_from_subtree(&old_content, suffix_styling);
                let mut handled_weight = false;
                for style in all::<BufferTextStyle>() {
                    let is_weight = style.has_custom_weight();
                    if is_weight && handled_weight {
                        continue;
                    }
                    if suffix_text_styling.exact_match_style(&style)
                        && !active_text_style.exact_match_style(&style)
                    {
                        handled_weight |= is_weight;
                        new_content.push(BufferText::Marker {
                            marker_type: style,
                            dir: MarkerDir::Start,
                        });
                    } else if !suffix_text_styling.exact_match_style(&style)
                        && active_text_style.exact_match_style(&style)
                    {
                        handled_weight |= is_weight;
                        new_content.push(BufferText::Marker {
                            marker_type: style,
                            dir: MarkerDir::End,
                        });
                    }
                }

                transition_styles_with_metadata(
                    &active_text_style,
                    &suffix_text_styling,
                    &mut new_content,
                );

                let end_of_line = self.containing_line_end(range.end);
                // Set the state of the content fragment from the end of edit range to the line end.
                // If the old content range's style is different from the style of the replacement range and NEITHER:
                // 1) The setting of the edit forces an override of its style
                // 2) The last inserted line creates a newline because of its BlockLineBreakBehavior
                if let BlockType::Text(range_end_block_style) = range_end_block_type
                    && end_of_line - 1 > range.end
                {
                    if range_end_block_style != previous_block_styling
                        && !should_override_next_block_style
                        && !override_next_style
                    {
                        new_content.push(BufferText::BlockMarker {
                            marker_type: range_end_block_style.clone(),
                        });
                        total_length += 1;
                        previous_block_styling = range_end_block_style;
                    }
                    new_content
                        .push_tree(buffer_cursor.slice_to_offset_after_markers(end_of_line - 1));
                }

                // Make sure the block style is closed while keeping the buffer valid. If the next character
                // after the edit line is not another block marker, we need to make sure to insert/replace a block
                // marker.
                let line_end_block_type = self.block_type_at_point(end_of_line);

                let new_content_empty = new_content.last().is_none();
                if let BlockType::Text(line_end_block_styling) = line_end_block_type {
                    let should_push_new_block_marker = match buffer_cursor.item() {
                        Some(BufferText::BlockMarker { marker_type })
                            if *marker_type == BufferBlockStyle::PlainText
                                && previous_block_styling == BufferBlockStyle::PlainText
                                && !new_content_empty =>
                        {
                            buffer_cursor.next();
                            new_content.push(BufferText::Newline);
                            false
                        }
                        Some(BufferText::BlockMarker { .. }) => false,
                        Some(BufferText::Newline)
                            if line_end_block_styling != previous_block_styling
                                || matches!(
                                    previous_block_styling.line_break_behavior(),
                                    BlockLineBreakBehavior::BlockMarker(_)
                                )
                                // If the edit would result in the buffer starting with a newline,
                                // convert it to a block marker.
                                || new_content_empty =>
                        {
                            buffer_cursor.next();
                            true
                        }
                        _ => false,
                    };

                    if should_push_new_block_marker {
                        new_content.push(BufferText::BlockMarker {
                            marker_type: line_end_block_styling,
                        });
                    }
                }
            }
        }

        new_content.push_tree(buffer_cursor.suffix());
        drop(buffer_cursor);
        self.content = new_content;

        // If insertion is not on selection, we want to clamp instead of invalidating the anchors.
        let anchor_udpate = AnchorUpdate {
            start: range.start,
            old_character_count: (range.end - range.start).as_usize(),
            new_character_count: total_length.as_usize(),
            clamp: !insert_on_selection,
        };

        self.internal_anchors.update(anchor_udpate);

        CoreEditorActionResult {
            updated_range: range.start..range.start + total_length,
            anchor_update: Some(anchor_udpate),
        }
    }

    fn ensure_plain_text(&mut self, range: Range<CharOffset>) -> CoreEditorActionResult {
        let updated_range = if range.end >= self.max_charoffset()
            && self.block_type_at_point(range.end) != BlockType::Text(BufferBlockStyle::PlainText)
        {
            log::trace!("Inserting <text> marker at end of buffer");
            self.content.push(BufferText::BlockMarker {
                marker_type: BufferBlockStyle::PlainText,
            });

            range.end..range.end + 1
        } else {
            range.end..range.end
        };

        CoreEditorActionResult {
            updated_range,
            anchor_update: None,
        }
    }

    fn style_link(&mut self, range: Range<CharOffset>, url: String) -> CoreEditorActionResult {
        let old_content = self.content.clone();
        let cursor = old_content.cursor::<CharOffset, StyleSummary>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        let mut new_content = SumTree::new();

        new_content.push_tree(buffer_cursor.slice_to_offset_after_markers(range.start));
        new_content.push(BufferText::Link(LinkMarker::Start(url)));
        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.end));
        new_content.push(BufferText::Link(LinkMarker::End));
        new_content.push_tree(buffer_cursor.suffix());

        drop(buffer_cursor);
        self.content = new_content;
        CoreEditorActionResult {
            updated_range: range,
            anchor_update: None,
        }
    }

    fn unstyle_link(&mut self, range: Range<CharOffset>) -> CoreEditorActionResult {
        let old_content = self.content.clone();
        let cursor = old_content.cursor::<CharOffset, StyleSummary>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        let mut new_content = SumTree::new();

        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.start));

        let markers_before_range_start = buffer_cursor.slice_to_offset_after_markers(range.start);
        let mut marker_cursor = markers_before_range_start.cursor::<(), ()>();
        marker_cursor.descend_to_first_item(&markers_before_range_start, |_| true);
        for item in marker_cursor {
            if !matches!(item, BufferText::Link(LinkMarker::Start(_))) {
                new_content.push(item.clone());
            }
        }

        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.end));
        let markers_after_range_end = buffer_cursor.slice_to_offset_after_markers(range.end);
        let mut marker_cursor = markers_after_range_end.cursor::<(), ()>();
        marker_cursor.descend_to_first_item(&markers_after_range_end, |_| true);
        for item in marker_cursor {
            if !matches!(item, BufferText::Link(LinkMarker::End)) {
                new_content.push(item.clone());
            }
        }

        new_content.push_tree(buffer_cursor.suffix());
        drop(buffer_cursor);
        self.content = new_content;
        CoreEditorActionResult {
            updated_range: range,
            anchor_update: None,
        }
    }

    fn style_text(
        &mut self,
        range: Range<CharOffset>,
        text_style: BufferTextStyle,
    ) -> CoreEditorActionResult {
        let old_content = self.content.clone();
        let cursor = old_content.cursor::<CharOffset, StyleSummary>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        let mut new_content = SumTree::new();

        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.start));

        // In the style markers right before range start, if there is no end marker, we will need to push a new start marker.
        // If there is an end marker, that means the range will be styled by simply removing that end marker. No need
        // to push a new start marker.
        let markers_before_range_start = buffer_cursor.slice_to_offset_after_markers(range.start);
        let mut found_ending_marker = false;

        let mut marker_cursor = markers_before_range_start.cursor::<(), ()>();
        marker_cursor.descend_to_first_item(&markers_before_range_start, |_| true);
        for item in marker_cursor {
            match item {
                BufferText::Marker {
                    marker_type,
                    dir: MarkerDir::End,
                } if *marker_type == text_style => {
                    found_ending_marker = true;
                }
                _ => new_content.push(item.clone()),
            }
        }

        if !found_ending_marker {
            new_content.push(BufferText::Marker {
                marker_type: text_style,
                dir: MarkerDir::Start,
            });
        }

        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.end));

        // In the style markers right before range end (exclusive), if there is no start marker, we will need to push a new end marker.
        // If there is an start marker, that means the range after was already styled. No need to push a new end marker.
        let markers_after_range_end = buffer_cursor.slice_to_offset_after_markers(range.end);
        let mut found_starting_marker = false;

        let mut marker_cursor = markers_after_range_end.cursor::<(), ()>();
        marker_cursor.descend_to_first_item(&markers_after_range_end, |_| true);
        for item in marker_cursor {
            match item {
                BufferText::Marker {
                    marker_type,
                    dir: MarkerDir::Start,
                } if *marker_type == text_style => {
                    found_starting_marker = true;
                }
                _ => new_content.push(item.clone()),
            }
        }

        if !found_starting_marker {
            new_content.push(BufferText::Marker {
                marker_type: text_style,
                dir: MarkerDir::End,
            });
        }

        new_content.push_tree(buffer_cursor.suffix());
        drop(buffer_cursor);
        self.content = new_content;

        CoreEditorActionResult {
            updated_range: range,
            anchor_update: None,
        }
    }

    fn unstyle_text(
        &mut self,
        range: Range<CharOffset>,
        text_style: BufferTextStyle,
    ) -> CoreEditorActionResult {
        let old_content = self.content.clone();
        let cursor = old_content.cursor::<CharOffset, StyleSummary>();
        let mut buffer_cursor = BufferCursor::new(cursor);
        let mut new_content = SumTree::new();

        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.start));

        // In the style markers right before range start, if there is no start marker, we will need to push a new end marker.
        // If there is a start marker, we just need to remove that start marker.
        let markers_before_range_start = buffer_cursor.slice_to_offset_after_markers(range.start);
        let mut found_starting_marker = false;

        let mut marker_cursor = markers_before_range_start.cursor::<(), ()>();
        marker_cursor.descend_to_first_item(&markers_before_range_start, |_| true);
        for item in marker_cursor {
            match item {
                BufferText::Marker {
                    marker_type,
                    dir: MarkerDir::Start,
                } if *marker_type == text_style => {
                    found_starting_marker = true;
                }
                _ => new_content.push(item.clone()),
            }
        }

        if !found_starting_marker {
            new_content.push(BufferText::Marker {
                marker_type: text_style,
                dir: MarkerDir::End,
            });
        }

        new_content.push_tree(buffer_cursor.slice_to_offset_before_markers(range.end));

        // In the style markers right before range end (exclusive), if there is no end marker, we will need to push a new start marker.
        // If there is a end marker, we just need to remove that end marker.
        let markers_after_range_end = buffer_cursor.slice_to_offset_after_markers(range.end);
        let mut found_ending_marker = false;

        let mut marker_cursor = markers_after_range_end.cursor::<(), ()>();
        marker_cursor.descend_to_first_item(&markers_after_range_end, |_| true);
        for item in marker_cursor {
            match item {
                BufferText::Marker {
                    marker_type,
                    dir: MarkerDir::End,
                } if *marker_type == text_style => {
                    found_ending_marker = true;
                }
                _ => new_content.push(item.clone()),
            }
        }

        if !found_ending_marker {
            new_content.push(BufferText::Marker {
                marker_type: text_style,
                dir: MarkerDir::Start,
            });
        }

        new_content.push_tree(buffer_cursor.suffix());
        drop(buffer_cursor);
        self.content = new_content;

        CoreEditorActionResult {
            updated_range: range,
            anchor_update: None,
        }
    }

    fn style_block(
        &mut self,
        range: Range<CharOffset>,
        style: BufferBlockStyle,
    ) -> CoreEditorActionResult {
        debug_assert!(
            range.start > CharOffset::zero(),
            "Invalid style range. Range start should not be zero.",
        );

        let current_style = self.block_type_at_point(range.start);
        let same_style_with_previous_block = if range.start > CharOffset::from(1) {
            self.block_type_at_point(range.start - 1) == BlockType::Text(style.clone())
        } else {
            false
        };

        if matches!(
            current_style,
            BlockType::Text(BufferBlockStyle::CodeBlock { .. })
        ) {
            self.color_code_block_ranges_internal(range.start, &[]);
        }

        let replace_with = if same_style_with_previous_block && style == BufferBlockStyle::PlainText
        {
            BufferText::Newline
        } else {
            BufferText::BlockMarker {
                marker_type: style.clone(),
            }
        };

        let replace_style = if range.end < self.max_charoffset() {
            let next_block = self.block_type_at_point(range.end + 1);

            match next_block {
                BlockType::Item(_) => None,
                BlockType::Text(next_style) => {
                    if BlockType::Text(next_style.clone()) == current_style {
                        Some(BufferText::BlockMarker {
                            marker_type: next_style,
                        })
                    // Plain text is a special case. If we are styling one plain text after another,
                    // we should separate them with a newline instead of a block marker.
                    } else if next_style == style && next_style == BufferBlockStyle::PlainText {
                        Some(BufferText::Newline)
                    } else {
                        None
                    }
                }
            }
        } else {
            None
        };

        self.content
            .replace_item_at_offset(range.start - 1, replace_with);

        if let Some(to_replace) = replace_style {
            self.content.replace_item_at_offset(range.end, to_replace);
        }

        CoreEditorActionResult {
            updated_range: range,
            anchor_update: None,
        }
    }
}

// Push text fragments to a block. If the block is a runnable code block,
// strip all the active text styling.
fn push_text_fragments_to_block(
    content_tree: &mut SumTree<BufferText>,
    fragments: Vec<FormattedTextFragment>,
    previous_block_style: BufferBlockStyle,
) {
    match previous_block_style {
        BufferBlockStyle::CodeBlock { .. } => {
            push_text_fragments_ignoring_styling(content_tree, fragments);
        }
        _ => update_content_with_text_fragments(content_tree, fragments),
    }
}

fn push_text_fragments_ignoring_styling(
    content_tree: &mut SumTree<BufferText>,
    fragments: Vec<FormattedTextFragment>,
) {
    for fragment in fragments {
        content_tree.append_str(&fragment.text)
    }
}

/// Returns the active text style with metadata at the given subtree and a set StyleSummary.
fn active_text_styles_with_metadata_from_subtree(
    content_tree: &SumTree<BufferText>,
    style: StyleSummary,
) -> TextStylesWithMetadata {
    let text_style: TextStyles = style.into();

    TextStylesWithMetadata::from_text_styles(
        text_style,
        if text_style.is_link() {
            content_tree.url_at_link_count(&LinkCount(style.total_link_counter() as usize))
        } else {
            None
        },
        if text_style.is_colored() {
            content_tree.color_at_color_count(&SyntaxColorId(style.syntax_link_counter() as usize))
        } else {
            None
        },
    )
}

fn insert_multiline_block(
    raw_text: &str,
    block_style: BufferBlockStyle,
    source: EditOrigin,
    inherit_styling: bool,
    edit_cursor: CursorType,
    new_content: &mut SumTree<BufferText>,
    previous_block_type: &mut BlockType,
) {
    // In our formatted text code, we have a trailing newline for every code block.
    let text = if let Some(text) = raw_text.strip_suffix('\n') {
        // If this is user typed, preserve the full text.
        if !matches!(source, EditOrigin::UserTyped) {
            text
        } else {
            raw_text
        }
    } else {
        raw_text
    };

    match previous_block_type.clone() {
        BlockType::Item(_) => {
            update_content_after_block_item(
                new_content,
                previous_block_type,
                BlockType::Text(block_style),
                None,
            );
            new_content.append_str(text);
        }
        BlockType::Text(previous_block_style) => {
            // For code blocks, there are a couple of situations
            // 1) If we are not inherit styling or the cursor is at buffer start, always start a new block.
            // 2) If the current block type is a code block, append the existing content to that code block.
            // 3) Else, append the content as plain text lines to the active block.
            if inherit_styling
                && block_style.should_inherit_style(edit_cursor, previous_block_style.clone())
            {
                if previous_block_style == block_style {
                    new_content.append_str(text);
                } else {
                    let mut first = true;
                    for line in text.split('\n') {
                        // Starting from the second block, we need apply the line break behavior of the active block style.
                        if !first {
                            match previous_block_style.line_break_behavior() {
                                BlockLineBreakBehavior::BlockMarker(marker_type) => {
                                    *previous_block_type = BlockType::Text(marker_type.clone());
                                    new_content.push(BufferText::BlockMarker { marker_type });
                                }
                                BlockLineBreakBehavior::NewLine => {
                                    new_content.push(BufferText::Newline);
                                }
                            }
                        }
                        let fragments = vec![FormattedTextFragment::plain_text(line.to_string())];
                        push_text_fragments_to_block(
                            new_content,
                            fragments,
                            previous_block_style.clone(),
                        );
                        first = false;
                    }
                }
            } else {
                // End all active text styles.
                end_all_active_text_styles(new_content);
                new_content.push(BufferText::BlockMarker {
                    marker_type: block_style.clone(),
                });
                *previous_block_type = BlockType::Text(block_style);
                new_content.append_str(text);
            }
        }
    }
}

fn end_all_active_text_styles(new_content: &mut SumTree<BufferText>) {
    let active_style_summary = new_content.extent::<StyleSummary>();
    let active_text_style =
        active_text_styles_with_metadata_from_subtree(new_content, active_style_summary);
    let mut handled_weight = false;
    for style in all::<BufferTextStyle>() {
        let is_weight = style.has_custom_weight();
        if is_weight && handled_weight {
            continue;
        }
        if active_text_style.exact_match_style(&style) {
            handled_weight |= is_weight;
            new_content.push(BufferText::Marker {
                marker_type: style,
                dir: MarkerDir::End,
            });
        }
    }
    if active_text_style.link_content().is_some() {
        new_content.push(BufferText::Link(LinkMarker::End));
    }
    if active_text_style.color().is_some() {
        new_content.push(BufferText::Color(ColorMarker::End));
    }
}

fn transition_styles_with_metadata(
    prev_text_styles: &TextStylesWithMetadata,
    next_text_styles: &TextStylesWithMetadata,
    content_tree: &mut SumTree<BufferText>,
) {
    match (
        prev_text_styles.link_content(),
        next_text_styles.link_content(),
    ) {
        (Some(active_url), Some(new_url)) if active_url == new_url => (),
        (Some(_), Some(new_url)) => {
            content_tree.push(BufferText::Link(LinkMarker::End));
            content_tree.push(BufferText::Link(LinkMarker::Start(new_url)))
        }
        (Some(_), None) => content_tree.push(BufferText::Link(LinkMarker::End)),
        (None, Some(new_url)) => content_tree.push(BufferText::Link(LinkMarker::Start(new_url))),
        (None, None) => (),
    };

    match (prev_text_styles.color(), next_text_styles.color()) {
        (Some(active_color), Some(new_color)) if active_color == new_color => (),
        (Some(_), Some(new_color)) => {
            content_tree.push(BufferText::Color(ColorMarker::End));
            content_tree.push(BufferText::Color(ColorMarker::Start(new_color)))
        }
        (Some(_), None) => content_tree.push(BufferText::Color(ColorMarker::End)),
        (None, Some(new_color)) => {
            content_tree.push(BufferText::Color(ColorMarker::Start(new_color)))
        }
        (None, None) => (),
    };
}

fn update_content_after_block_item(
    content_tree: &mut SumTree<BufferText>,
    previous_block_type: &mut BlockType,
    active_block_type: BlockType,
    fragments: Option<Vec<FormattedTextFragment>>,
) {
    content_tree.push(match active_block_type.clone() {
        BlockType::Item(item_type) => BufferText::BlockItem { item_type },
        BlockType::Text(block_style) => BufferText::BlockMarker {
            marker_type: block_style,
        },
    });
    *previous_block_type = active_block_type;

    if let Some(fragments) = fragments {
        update_content_with_text_fragments(content_tree, fragments);
    }
}

/// Updates the content subtree with formatted text fragments. The resulting content tree should be formatted with the
/// valid inline styles.
fn update_content_with_text_fragments(
    content_tree: &mut SumTree<BufferText>,
    fragments: Vec<FormattedTextFragment>,
) {
    for fragment in fragments {
        let style = content_tree.extent::<StyleSummary>();
        let active_text_style = active_text_styles_with_metadata_from_subtree(content_tree, style);

        let content = fragment.text;
        let text_style: TextStylesWithMetadata = fragment.styles.into();

        let mut handled_weight = false;
        for style in all::<BufferTextStyle>() {
            let is_weight = style.has_custom_weight();
            if is_weight && handled_weight {
                continue;
            }
            if active_text_style.exact_match_style(&style) && !text_style.exact_match_style(&style)
            {
                handled_weight |= is_weight;
                content_tree.push(BufferText::Marker {
                    marker_type: style,
                    dir: MarkerDir::End,
                });
            } else if !active_text_style.exact_match_style(&style)
                && text_style.exact_match_style(&style)
            {
                handled_weight |= is_weight;
                content_tree.push(BufferText::Marker {
                    marker_type: style,
                    dir: MarkerDir::Start,
                });
            }
        }

        transition_styles_with_metadata(&active_text_style, &text_style, content_tree);
        content_tree.append_str(&content);
    }
}

// Push a new block marker before pushing the content of the block.
// Note that this function is no-op if the cursor is inline and we are
// pushing the same block type.
fn maybe_push_new_block_marker(
    inherit_styling: bool,
    edit_cursor: CursorType,
    previous_block_type: BufferBlockStyle,
    block_style: BufferBlockStyle,
    new_content: &mut SumTree<BufferText>,
) {
    // Add a new marker only if
    // 1) Cursor is not inline nor at a newline start and inherits the active block's styling.
    // 2) The block style is different from the active style.
    let should_add_marker = !(edit_cursor == CursorType::Inline
        || (edit_cursor == CursorType::NewLineStart && inherit_styling))
        || previous_block_type != block_style;

    if should_add_marker {
        // End all active text styles.
        end_all_active_text_styles(new_content);

        new_content.push(BufferText::BlockMarker {
            marker_type: block_style,
        });
    }
}
