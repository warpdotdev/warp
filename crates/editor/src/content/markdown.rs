use std::collections::VecDeque;
use std::fmt::Write;
use std::ops::Range;
use std::{io, iter};

use anyhow::{Context, Result};
use html5ever::serialize::{Serialize, Serializer, TraversalScope};
use html5ever::{QualName, serialize};
use itertools::Itertools;
use markdown_parser::{
    CodeBlockText, FormattedIndentTextInline, FormattedTableAlignment, FormattedTaskList,
    FormattedText, FormattedTextFragment, FormattedTextHeader, FormattedTextInline,
    FormattedTextLine, OrderedFormattedIndentTextInline,
};
use markup5ever::ns;
use string_offset::CharOffset;
use warpui::elements::{ListIndentLevel, ListNumbering};
use warpui::text::point::Point;
use warpui::{AppContext, ModelContext, ModelHandle};

use super::buffer::{
    ActionWithSelectionDelta, Buffer, EditOrigin, EditResult, StyledBufferBlock,
    StyledBufferBlocks, StyledBufferRun,
};
use super::core::{CoreEditorAction, CoreEditorActionType};
use super::text::{
    BlockHeaderSize, BlockType, BufferBlockItem, BufferBlockStyle, FormattedTable,
    TABLE_BLOCK_MARKDOWN_LANG, TextStylesWithMetadata,
};
use crate::content::anchor::AnchorSide;
use crate::content::selection_model::BufferSelectionModel;
use crate::content::version::BufferVersion;

/// A Markdown format to serialize a [`Buffer`] into.
#[derive(Clone, Copy)]
pub enum MarkdownStyle<'a> {
    /// The internal Markdown format used in Warp Drive. References are normalized, so the Markdown
    /// only refers to other objects by their IDs, with no other data.
    Internal,
    /// A Markdown format suitable for external use. If an [`AppContext`] is set, it may be used to
    /// enrich the exported Markdown (e.g. by expanding out embedded objects).
    Export {
        app_context: Option<&'a AppContext>,
        should_not_escape_markdown_punctuation: bool,
    },
}

/// Parser that takes a slice of content model with StyledBufferBlocks and returns
/// the formatted markdown in string.
pub struct BufferMarkdownParser<'a> {
    blocks: StyledBufferBlocks<'a>,
    style: MarkdownStyle<'a>,
}

impl<'a> BufferMarkdownParser<'a> {
    pub fn new(style: MarkdownStyle<'a>, blocks: StyledBufferBlocks<'a>) -> Self {
        Self { blocks, style }
    }

    // Consumes the iterator and returns the corresponding markdown string.
    pub fn to_markdown(&mut self) -> String {
        let mut res = String::new();
        let mut ordered_list_numbering = ListNumbering::new();

        for block in self.blocks.by_ref() {
            match block {
                StyledBufferBlock::Item(item) => {
                    res.push_str(&item.as_markdown(self.style));
                    res.push('\n');
                }
                StyledBufferBlock::Text(text_block) => {
                    if let BufferBlockStyle::Table { alignments, .. } = &text_block.style
                        && matches!(self.style, MarkdownStyle::Export { .. })
                    {
                        let markdown_text = Self::styled_runs_to_markdown_text(&text_block.block);
                        let table = super::text::table_from_internal_format_with_inline_markdown(
                            &markdown_text,
                            alignments.clone(),
                        );
                        Self::serialize_table_to_gfm_markdown(&table, &mut res);
                        continue;
                    }

                    // Push the prefix for the active block.
                    match text_block.style.clone() {
                        BufferBlockStyle::Header { header_size } => {
                            res.push_str(&"#".repeat(header_size.into()));
                            res.push(' ');
                        }
                        BufferBlockStyle::UnorderedList { indent_level } => {
                            res.push_str("    ".repeat(indent_level.as_usize()).as_str());
                            res.push_str("* ")
                        }
                        BufferBlockStyle::OrderedList {
                            indent_level,
                            number,
                        } => {
                            let indent = indent_level.as_usize();
                            res.push_str("    ".repeat(indent).as_str());

                            let _ = write!(
                                &mut res,
                                "{}. ",
                                ordered_list_numbering.advance(indent, number).label_index
                            );
                        }
                        BufferBlockStyle::CodeBlock { code_block_type } => res.push_str(&format!(
                            "```{}\n",
                            &code_block_type.to_markdown_representation(self.style)
                        )),
                        BufferBlockStyle::TaskList {
                            indent_level,
                            complete,
                        } => {
                            res.push_str("    ".repeat(indent_level.as_usize()).as_str());
                            if complete {
                                res.push_str("- [x] ");
                            } else {
                                res.push_str("- [ ] ");
                            }
                        }
                        BufferBlockStyle::Table { .. } => {
                            res.push_str("```");
                            res.push_str(TABLE_BLOCK_MARKDOWN_LANG);
                            res.push('\n');
                        }
                        BufferBlockStyle::PlainText => (),
                    };

                    if !matches!(text_block.style, BufferBlockStyle::OrderedList { .. }) {
                        ordered_list_numbering.reset();
                    }

                    let block_escapes = !matches!(
                        self.style,
                        MarkdownStyle::Export {
                            should_not_escape_markdown_punctuation: true,
                            ..
                        }
                    ) && text_block.style.escape_markdown_punctuation();
                    Self::append_block_content(&text_block.block, block_escapes, &mut res);

                    // Push the suffix for the active block.
                    match text_block.style {
                        BufferBlockStyle::CodeBlock { .. } | BufferBlockStyle::Table { .. } => {
                            res.push_str("```\n")
                        }
                        BufferBlockStyle::Header { .. }
                        | BufferBlockStyle::PlainText
                        | BufferBlockStyle::UnorderedList { .. }
                        | BufferBlockStyle::OrderedList { .. }
                        | BufferBlockStyle::TaskList { .. } => (),
                    }
                }
            }
        }

        res
    }

    fn serialize_table_to_gfm_markdown(table: &FormattedTable, buf: &mut String) {
        let mut column_count = table.headers.len();
        for row in &table.rows {
            column_count = column_count.max(row.len());
        }

        if column_count == 0 {
            return;
        }

        let header_cells = (0..column_count)
            .map(|index| {
                table
                    .headers
                    .get(index)
                    .map(inline_to_markdown)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        append_gfm_table_row(&header_cells, buf);

        let separator_cells: Vec<String> = (0..column_count)
            .map(|index| {
                alignment_to_gfm_separator(
                    table
                        .alignments
                        .get(index)
                        .copied()
                        .unwrap_or(FormattedTableAlignment::Left),
                )
            })
            .collect();
        append_gfm_table_row(&separator_cells, buf);

        for row in &table.rows {
            let row_cells = (0..column_count)
                .map(|index| row.get(index).map(inline_to_markdown).unwrap_or_default())
                .collect::<Vec<_>>();
            append_gfm_table_row(&row_cells, buf);
        }
    }

    /// Emits Markdown formatting markers for changing from `prev_styles` to `next_styles`.
    ///
    /// This will omit formatting for unchanged styles, which is required to produce valid Markdown
    /// (for example, according to [this](https://spec.commonmark.org/0.30/#example-411),
    /// `*abc**def*`, parses to `<i_s>abc**def<i_e>`, not `<i_s>abcdef<i_e>`).
    ///
    /// If the text that `next_styles` applies to (`next_content`) starts with whitespace,
    /// the formatting markers might be reordered after the first character to comply
    /// with [these rules](https://spec.commonmark.org/0.30/#left-flanking-delimiter-run).
    /// Likewise, if the last character of `buf` is whitespace, the formatting markers may be
    /// reordered before it.
    ///
    /// This alters the user content slightly, but will parse back to what they expect, rather than
    /// something un-representable. In this case, the returned `&str` is the shifted
    /// `next_content`.
    fn append_formatting<'b>(
        prev_styles: &TextStylesWithMetadata,
        next_styles: &TextStylesWithMetadata,
        mut next_content: &'b str,
        buf: &mut String,
    ) -> &'b str {
        // Because of how links start a new formatting scope, we have to change styles in a particular
        // order, from the inside out:
        // 1. End inline code (since other style markers aren't interpreted within code spans)
        // 2. End other inline styles
        // 3. End and then start links
        // 4. Start inline styles
        // 5. Start inline code

        if prev_styles.is_inline_code() && !next_styles.is_inline_code() {
            buf.push('`')
        }

        let end_bold = prev_styles.is_at_least_bold() && !next_styles.is_at_least_bold();
        let end_italic = prev_styles.is_italic() && !next_styles.is_italic();
        let end_strikethrough = prev_styles.is_strikethrough() && !next_styles.is_strikethrough();
        let end_underline = prev_styles.is_underlined() && !next_styles.is_underlined();

        // This, and the parallel logic below, is an unfortunate workaround for the CommonMark
        // rules about space before/after bold and italic formatting markers
        // (https://spec.commonmark.org/0.30/#left-flanking-delimiter-run). Like Notion, we
        // rearrange the user content slightly to match the spec, in order to be compatible with
        // GitHub and other parsers.
        // In this case, if the current buffer ends in whitespace, we remove it, write the
        // end formatting markers, and then re-add it.
        let swapped_content = buf
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_whitespace())
            .last()
            // Panic safety: iterating over `char_indices` means that `idx` must be in-bounds and a
            // code point boundary.
            .map(|(idx, _)| buf.split_off(idx));

        if end_underline {
            buf.push_str("</u>")
        }

        if end_strikethrough {
            buf.push_str("~~");
        }

        if end_bold {
            buf.push_str("**");
        }

        if end_italic {
            buf.push('*');
        }

        buf.extend(swapped_content);

        let prev_link = prev_styles.link_content();
        let next_link = next_styles.link_content();

        // Open/close links if:
        // * A link is beginning or ending.
        // * There are two adjacent, different, links.
        if prev_link != next_link {
            if let Some(closing_url) = prev_link {
                // The `String` implementation of `fmt::Write` never returns `Err`.
                let _ = write!(buf, "]({closing_url})");
            }

            if next_link.is_some() {
                buf.push('[');
            }
        }

        let start_bold = !prev_styles.is_at_least_bold() && next_styles.is_at_least_bold();
        let start_italic = !prev_styles.is_italic() && next_styles.is_italic();
        let start_strikethrough = !prev_styles.is_strikethrough() && next_styles.is_strikethrough();
        let start_underline = !prev_styles.is_underlined() && next_styles.is_underlined();

        // In order to parse correctly, Markdown emphasis markers must be after whitespace. If the
        // next run of content starts with whitespace, we write that and then the markers, and then
        // let the outer loop write the rest of the content.
        if (start_bold || start_italic || start_strikethrough || start_underline)
            && let Some(idx) = next_content.find(|c: char| !c.is_whitespace())
        {
            // Panic safety: if the pattern matches, `find` returns the byte index of the first
            // matching character, so it will be a code point boundary.
            let (whitespace, rest) = next_content.split_at(idx);
            buf.push_str(whitespace);
            next_content = rest;
        }

        if start_bold {
            buf.push_str("**");
        }

        if start_italic {
            buf.push('*');
        }

        if start_strikethrough {
            buf.push_str("~~");
        }

        if start_underline {
            buf.push_str("<u>");
        }

        if !prev_styles.is_inline_code() && next_styles.is_inline_code() {
            buf.push('`');
        }

        next_content
    }

    /// Appends text content to a Markdown buffer, escaping any special characters as needed.
    fn append_content(text: &str, should_escape: bool, buf: &mut String) {
        if should_escape {
            buf.reserve(text.len());
            for ch in text.chars() {
                // Only escape punctuation characters that have special meaning in Markdown
                if Self::is_markdown_special_char(ch) {
                    buf.push('\\');
                }
                buf.push(ch);
            }
        } else {
            buf.push_str(text);
        }
    }

    fn styled_runs_to_markdown_text(runs: &[StyledBufferRun]) -> String {
        let mut res = String::new();
        Self::append_block_content(runs, false, &mut res);
        res
    }

    fn append_block_content(runs: &[StyledBufferRun], block_escapes: bool, buf: &mut String) {
        let mut prev_styles = TextStylesWithMetadata::default();
        for run in runs {
            if run.run.is_empty() || run.text_styles.is_placeholder() {
                continue;
            }
            let (content, has_trailing_newline) = match run.run.strip_suffix('\n') {
                Some(without_newline) => (without_newline, true),
                _ => (run.run.as_str(), false),
            };
            let content = Self::append_formatting(&prev_styles, &run.text_styles, content, buf);
            prev_styles.clone_from(&run.text_styles);
            let should_escape = block_escapes && run.text_styles.escape_markdown_punctuation();
            Self::append_content(content, should_escape, buf);
            if has_trailing_newline {
                Self::append_formatting(
                    &run.text_styles,
                    &TextStylesWithMetadata::default(),
                    "",
                    buf,
                );
                prev_styles = TextStylesWithMetadata::default();
                buf.push('\n');
            }
        }
        Self::append_formatting(&prev_styles, &TextStylesWithMetadata::default(), "", buf);
    }

    /// Returns true if the character has special meaning in Markdown and should be escaped.
    fn is_markdown_special_char(ch: char) -> bool {
        matches!(
            ch,
            '\\' | // Backslash - used for escaping other characters
            '`' | // Backtick - used for inline code
            '*' | // Asterisk - used for emphasis and unordered lists
            '_' | // Underscore - used for emphasis
            '{' | // Left curly brace - used in some Markdown extensions
            '}' | // Right curly brace - used in some Markdown extensions
            '[' | // Left square bracket - used for links and images
            ']' | // Right square bracket - used for links and images
            '(' | // Left parenthesis - used for links and images
            ')' | // Right parenthesis - used for links and images
            '#' | // Hash - used for headings
            '+' | // Plus - used for unordered lists
            '-' | // Minus - used for unordered lists and horizontal rules
            '.' | // Period - used for ordered lists
            '!' // Exclamation mark - used for images
        )
    }
}

/// Convert a slice of buffer model in StyledBufferBlocks into FormattedText.
pub struct BufferToFormattedText<'a> {
    blocks: StyledBufferBlocks<'a>,
}

impl<'a> BufferToFormattedText<'a> {
    pub fn new(blocks: StyledBufferBlocks<'a>) -> Self {
        Self { blocks }
    }

    pub fn to_formatted_text(self) -> FormattedText {
        let mut lines = VecDeque::new();
        let mut trailing_new_line = false;
        for item in self.blocks {
            match item {
                StyledBufferBlock::Item(item) => {
                    trailing_new_line = false;
                    lines.push_back(item.to_formatted_text_line());
                }
                StyledBufferBlock::Text(text_block) => {
                    lines.push_back(match text_block.style.clone() {
                        BufferBlockStyle::Header { header_size } => {
                            trailing_new_line = false;
                            FormattedTextLine::Heading(FormattedTextHeader {
                                heading_size: header_size.into(),
                                text: text_block
                                    .block
                                    .into_iter()
                                    .map(|run| run.to_formatted_text_fragment())
                                    .collect(),
                            })
                        }
                        BufferBlockStyle::TaskList {
                            indent_level,
                            complete,
                        } => {
                            trailing_new_line = false;
                            FormattedTextLine::TaskList(FormattedTaskList {
                                complete,
                                indent_level: indent_level.as_usize(),
                                text: text_block
                                    .block
                                    .into_iter()
                                    .map(|run| run.to_formatted_text_fragment())
                                    .collect(),
                            })
                        }
                        BufferBlockStyle::UnorderedList { indent_level } => {
                            trailing_new_line = false;
                            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                                indent_level: indent_level.as_usize(),
                                text: text_block
                                    .block
                                    .into_iter()
                                    .map(|run| run.to_formatted_text_fragment())
                                    .collect(),
                            })
                        }
                        BufferBlockStyle::OrderedList {
                            indent_level,
                            number,
                        } => {
                            trailing_new_line = false;
                            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                                number,
                                indented_text: FormattedIndentTextInline {
                                    indent_level: indent_level.as_usize(),
                                    text: text_block
                                        .block
                                        .into_iter()
                                        .map(|run| run.to_formatted_text_fragment())
                                        .collect(),
                                },
                            })
                        }
                        BufferBlockStyle::CodeBlock { code_block_type } => {
                            trailing_new_line = false;
                            let text = text_block.block.into_iter().map(|run| run.run).collect();
                            FormattedTextLine::CodeBlock(CodeBlockText {
                                lang: code_block_type.to_string(),
                                code: text,
                            })
                        }
                        BufferBlockStyle::Table { alignments, .. } => {
                            trailing_new_line = false;
                            let text: String =
                                text_block.block.into_iter().map(|run| run.run).collect();
                            FormattedTextLine::Table(
                                markdown_parser::FormattedTable::from_internal_format_with_alignments(&text, alignments),
                            )
                        }
                        BufferBlockStyle::PlainText => {
                            trailing_new_line = false;
                            FormattedTextLine::Line(
                                text_block
                                    .block
                                    .into_iter()
                                    .map(|run| {
                                        trailing_new_line = run.run.ends_with('\n');
                                        run.to_formatted_text_fragment()
                                    })
                                    .collect(),
                            )
                        }
                    });
                }
            }
        }

        // Make sure trailing newline gets its own linebreak.
        if trailing_new_line {
            lines.push_back(FormattedTextLine::LineBreak);
        }

        FormattedText { lines }
    }
}

impl StyledBufferRun {
    fn to_formatted_text_fragment(&self) -> FormattedTextFragment {
        let text = match self.run.strip_suffix('\n') {
            Some(without_newline) => without_newline.to_string(),
            _ => self.run.clone(),
        };

        FormattedTextFragment {
            text,
            styles: self.text_styles.clone().into(),
        }
    }
}

impl BufferBlockStyle {
    /// Whether or not punctuation that may be misinterpreted as Markdown formatting should be
    /// escaped. This is true for _most_ block types, but some do not allow escapes.
    fn escape_markdown_punctuation(&self) -> bool {
        !matches!(
            self,
            BufferBlockStyle::CodeBlock { .. } | BufferBlockStyle::Table { .. }
        )
    }
}

impl TextStylesWithMetadata {
    fn escape_markdown_punctuation(&self) -> bool {
        !self.is_inline_code()
    }
}

use markdown_parser::FormattedTextDelta;

use crate::content::buffer::{BufferEvent, ShouldAutoscroll, ToBufferCharOffset};

impl Buffer {
    pub(super) fn replace_with_formatted_text(
        &mut self,
        range: Range<CharOffset>,
        text: FormattedText,
        origin: EditOrigin,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        selection_model.update(ctx, |selection_model, _| {
            selection_model.truncate();
        });
        let edit = self.edits_for_formatted_text(range, text, origin);
        self.modify_first_selection(selection_model, edit, ctx)
    }

    pub(super) fn insert_formatted_text_at_selections(
        &mut self,
        text: FormattedText,
        origin: EditOrigin,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.modify_each_selection(
            selection_model,
            |buffer, selection, selection_model, _index| {
                let range = selection_model.selection_to_offset_range(selection);
                let text = text.clone();
                buffer.edits_for_formatted_text(range, text, origin)
            },
            ctx,
        )
    }

    pub(super) fn edits_for_formatted_text(
        &mut self,
        range: Range<CharOffset>,
        text: FormattedText,
        origin: EditOrigin,
    ) -> ActionWithSelectionDelta {
        let mut editor_action_set = vec![];
        // When replacing text in-place for a subset of the document, we may
        // need to adjust the block style based on the first line of the
        // replacement. However, for full-buffer replacements (the entire
        // document body), we rely on the inserted formatted text alone and do
        // not emit a separate StyleBlock action, to avoid introducing extra
        // empty headings or markers.
        let is_full_buffer_replacement =
            range.start == CharOffset::from(1) && range.end == self.max_charoffset();

        if range.start != CharOffset::zero()
            && !is_full_buffer_replacement
            && range.start == self.containing_line_start(range.start)
            && range.end == self.containing_line_end(range.end) - 1
        {
            let new_block_style = text
                .lines
                .front()
                .and_then(formatted_text_line_to_block_style);

            let active_block_type = self.block_type_at_point(range.start);

            if let BlockType::Text(active_block_style) = active_block_type {
                match new_block_style {
                    Some(block_style)
                        if block_style != active_block_style
                            && active_block_style == BufferBlockStyle::PlainText =>
                    {
                        editor_action_set.push(CoreEditorAction::new(
                            range.clone(),
                            CoreEditorActionType::StyleBlock(block_style),
                        ));
                    }
                    _ => (),
                }
            }
        }

        editor_action_set.push(CoreEditorAction::new(
            range.clone(),
            CoreEditorActionType::Insert {
                text,
                source: origin,
                override_next_style: false,
                insert_on_selection: true,
            },
        ));
        editor_action_set.push(self.update_buffer_end(range.end));
        ActionWithSelectionDelta::new_with_offsets(
            editor_action_set,
            &mut self.internal_anchors,
            range.end,
            range.end,
            0,
            AnchorSide::Right,
        )
    }

    /// Compute the character range corresponding to the suffix of the
    /// document that should be replaced based on the given formatted-text
    /// delta.
    ///
    /// The Point is at the start of the line after the common prefix lines.
    fn formatted_text_suffix_range(&self, delta: &FormattedTextDelta) -> Range<CharOffset> {
        let row = delta.common_prefix_lines + 1;
        let mut suffix_start = Point::new(row as u32, 0).to_buffer_char_offset(self);

        // By default the buffer has an empty plain text. If we don't replace the initial marker and we're inserting
        // a different formatted text line at the beginning, we will end up with an incorrect initial line.
        // If we're replacing the whole content, include the zeroth offset so we set the initial block styling correctly.
        if suffix_start == CharOffset::from(1) {
            suffix_start = CharOffset::zero();
        }
        suffix_start = suffix_start.clamp(CharOffset::zero(), self.max_charoffset());

        suffix_start..self.max_charoffset()
    }

    pub fn apply_formatted_text_delta(
        &mut self,
        delta: &FormattedTextDelta,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        if delta.is_noop() {
            return;
        }

        let suffix_range = self.formatted_text_suffix_range(delta);
        let selection_model_id = selection_model.id();

        // Build the new suffix as formatted text. This will replace the old
        // suffix range and effectively delete the old suffix while inserting
        // the new suffix, leaving the prefix untouched.
        let new_suffix_text = FormattedText::new(delta.new_suffix.clone());
        let edit_result = self.replace_with_formatted_text(
            suffix_range,
            new_suffix_text,
            // We need user initiated here as it impacts the editing behavior. Specifically,
            // we want to insert inline and not trigger the block specific linebreak suffix.
            EditOrigin::UserInitiated,
            selection_model,
            ctx,
        );

        self.version = BufferVersion::new();

        let Some(delta) = edit_result.delta else {
            log::debug!("Editor action was no-op");
            return;
        };
        ctx.emit(BufferEvent::ContentChanged {
            delta,
            origin: EditOrigin::SystemEdit,
            should_autoscroll: ShouldAutoscroll::No,
            buffer_version: self.version,
            selection_model_id: Some(selection_model_id),
        });
    }
}

pub struct ExportedBufferBlocks<'a> {
    pub blocks: Vec<StyledBufferBlock>,
    pub context: &'a AppContext,
}

impl ExportedBufferBlocks<'_> {
    // Serialize a styled block into HTML string.
    pub fn serialize_styled_blocks(self) -> Result<String> {
        let mut result = vec![];
        serialize(&mut result, &self, Default::default())?;
        String::from_utf8(result).context("Failed to parse HTML bytes")
    }
}

impl Serialize for ExportedBufferBlocks<'_> {
    fn serialize<S>(&self, serializer: &mut S, _: TraversalScope) -> io::Result<()>
    where
        S: Serializer,
    {
        let mut previous_block_type = None;
        let mut active_indent_level = 0;
        for block in self.blocks.iter() {
            match block {
                StyledBufferBlock::Item(item) => {
                    if let Some(end_container) = previous_block_type
                        .take()
                        .and_then(|style| container_element(&style))
                    {
                        end_list_with_indent(serializer, end_container, active_indent_level)?
                    }

                    match item {
                        BufferBlockItem::HorizontalRule => {
                            let element_name = "hr";

                            let tag_name = QualName::new(None, ns!(html), element_name.into());
                            serializer.start_elem(tag_name.clone(), iter::empty())?;
                            serializer.end_elem(tag_name)?;
                        }
                        BufferBlockItem::Embedded { item } => {
                            let rich_copy_format = item.to_rich_format(self.context);

                            let html_format = rich_copy_format.html;
                            let tag_name =
                                QualName::new(None, ns!(html), html_format.element_name.into());

                            let attributes = html_format
                                .attributes
                                .iter()
                                .map(|(key, value)| {
                                    (QualName::new(None, ns!(), (*key).into()), *value)
                                })
                                .collect_vec();
                            serializer.start_elem(
                                tag_name.clone(),
                                attributes.iter().map(|(name, value)| (name, *value)),
                            )?;

                            serializer.write_text(&html_format.content)?;
                            serializer.end_elem(tag_name)?;
                        }
                        BufferBlockItem::Image {
                            alt_text,
                            source,
                            title,
                        } => {
                            let tag_name = QualName::new(None, ns!(html), "img".into());
                            let src_attr = QualName::new(None, ns!(), "src".into());
                            let alt_attr = QualName::new(None, ns!(), "alt".into());
                            let title_attr = QualName::new(None, ns!(), "title".into());
                            let mut attrs =
                                vec![(&src_attr, source.as_str()), (&alt_attr, alt_text.as_str())];
                            // Only emit `title="..."` when the parsed title is non-empty so
                            // the default form matches the pre-title behavior.
                            if let Some(title) = title.as_deref().filter(|t| !t.is_empty()) {
                                attrs.push((&title_attr, title));
                            }
                            serializer.start_elem(tag_name.clone(), attrs.into_iter())?;
                            serializer.end_elem(tag_name)?;
                        }
                    };

                    active_indent_level = 0;
                }
                StyledBufferBlock::Text(text_block) => {
                    let (new_indent_level, number) = match &text_block.style {
                        BufferBlockStyle::OrderedList {
                            indent_level,
                            number,
                        } => (indent_level.as_usize() + 1, *number),
                        BufferBlockStyle::UnorderedList { indent_level }
                        | BufferBlockStyle::TaskList { indent_level, .. } => {
                            (indent_level.as_usize() + 1, None)
                        }
                        _ => (0, None),
                    };

                    // Start or end the active list element.
                    let current_container = container_element(&text_block.style);
                    let previous_container =
                        previous_block_type.as_ref().and_then(container_element);

                    // Right now the only container element is lists. In the future when we support more container types we will
                    // need to update the logic here.
                    //
                    // For lists, note that we don't start and close <li> tag within the same iteration. This is because the syntax
                    // of nested list is we would need to start the new <ul> tag within the open <li> tag. For example, <ul0>first line<ul1>second line
                    // will look like <ul><li>first line<ul><li>second line</li></ul></li></ul> in HTML.
                    match (previous_container, current_container) {
                        // If we are starting a new list from a non-list block, just start the <ul> or <ol> tag with the new indent level.
                        (None, Some(current_tag)) => start_list_with_indent(
                            serializer,
                            current_tag,
                            new_indent_level,
                            number,
                        )?,
                        // If we are ending a list into a non-list block, end the <ul> or <ol> tag with the active indent level.
                        (Some(prev_tag), None) => {
                            end_list_with_indent(serializer, prev_tag, active_indent_level)?
                        }
                        (Some(prev_tag), Some(current_tag)) => {
                            // If we are converting from one type of list to another, end the active list and start the new list.
                            if prev_tag != current_tag {
                                end_list_with_indent(serializer, prev_tag, active_indent_level)?;
                                start_list_with_indent(
                                    serializer,
                                    current_tag,
                                    new_indent_level,
                                    number,
                                )?
                            // If the list type is the same and we have less indent level on the new list, we need to end the number of tags
                            // matching the difference in indent level and start a new <li> element.
                            } else if active_indent_level > new_indent_level {
                                end_list_with_indent(
                                    serializer,
                                    prev_tag,
                                    active_indent_level - new_indent_level,
                                )?;

                                serializer.end_elem(QualName::new(None, ns!(html), "li".into()))?;
                                serializer.start_elem(
                                    QualName::new(None, ns!(html), "li".into()),
                                    iter::empty(),
                                )?;
                            // If the list type is the same and we have more indent level on the new list, we need to start the number of tags
                            // matching the difference in indent level.
                            } else if new_indent_level > active_indent_level {
                                start_list_with_indent(
                                    serializer,
                                    current_tag,
                                    new_indent_level - active_indent_level,
                                    number,
                                )?
                            // If the list type is the same and the indent level is the same, we just need to start a new <li> element.
                            } else if new_indent_level == active_indent_level {
                                serializer.end_elem(QualName::new(None, ns!(html), "li".into()))?;
                                serializer.start_elem(
                                    QualName::new(None, ns!(html), "li".into()),
                                    iter::empty(),
                                )?;
                            }
                        }
                        (None, None) => (),
                    }

                    active_indent_level = new_indent_level;
                    if let BufferBlockStyle::Table { alignments, .. } = &text_block.style {
                        let markdown_text =
                            BufferMarkdownParser::styled_runs_to_markdown_text(&text_block.block);
                        let table = super::text::table_from_internal_format_with_inline_markdown(
                            &markdown_text,
                            alignments.clone(),
                        );
                        serialize_table_to_html(serializer, &table)?;
                        previous_block_type = Some(text_block.style.clone());
                        continue;
                    }
                    let name = match text_block.style {
                        BufferBlockStyle::CodeBlock { .. } => Some("pre".to_string()),
                        BufferBlockStyle::UnorderedList { .. }
                        | BufferBlockStyle::OrderedList { .. }
                        | BufferBlockStyle::TaskList { .. } => None,
                        BufferBlockStyle::Header { header_size } => {
                            Some(format!("h{}", Into::<usize>::into(header_size)))
                        }
                        BufferBlockStyle::PlainText => Some("p".to_string()),
                        BufferBlockStyle::Table { .. } => None,
                    };
                    let tag_name = name.map(|name| QualName::new(None, ns!(html), name.into()));

                    previous_block_type = Some(text_block.style.clone());

                    if let Some(tag_name) = tag_name.clone() {
                        serializer.start_elem(tag_name, iter::empty())?;

                        // Push an additional <code> element here so other apps could recognize the included language info.
                        if let BufferBlockStyle::CodeBlock { code_block_type } = &text_block.style {
                            let language_name = QualName::new(None, ns!(), "class".into());
                            let language_value = format!(
                                "language-{}",
                                code_block_type
                                    .to_markdown_representation(MarkdownStyle::Export {
                                        app_context: Some(self.context),
                                        should_not_escape_markdown_punctuation: false,
                                    })
                                    .to_lowercase()
                            );
                            let attrs = [(&language_name, language_value.as_str())];

                            let code_block_name = QualName::new(None, ns!(html), "code".into());
                            serializer.start_elem(code_block_name.clone(), attrs.into_iter())?;
                        }
                    } else if let BufferBlockStyle::TaskList { complete, .. } = text_block.style {
                        // For tasklists, the formatting is similar to unordered lists but with an additional
                        // <input type="checkbox"></input> before the text fragments.
                        let type_name = QualName::new(None, ns!(), "type".into());
                        let checked_name = QualName::new(None, ns!(), "checked".into());

                        let mut checkbox_attrs = vec![(&type_name, "checkbox")];

                        if complete {
                            checkbox_attrs.push((&checked_name, ""));
                        }

                        let tag_name = QualName::new(None, ns!(html), "input".into());
                        serializer.start_elem(tag_name.clone(), checkbox_attrs.into_iter())?;
                        serializer.end_elem(tag_name)?;
                    }

                    for (idx, runs) in text_block.block.iter().enumerate() {
                        let mut text = runs.run.as_str();
                        // Strip trailing newline since <p> or <code> already includes a linebreak.
                        if idx == text_block.block.len() - 1
                            && let Some(removed_trailing_newline) = runs.run.strip_suffix('\n')
                        {
                            text = removed_trailing_newline;
                        }

                        if let Some(link) = runs.text_styles.link_content() {
                            let tag_name = QualName::new(None, ns!(html), "a".into());
                            let attribute_name = QualName::new(None, ns!(), "href".into());
                            serializer.start_elem(
                                tag_name,
                                vec![(&attribute_name, link.as_str())].into_iter(),
                            )?;
                        }

                        if runs.text_styles.is_at_least_bold() {
                            let tag_name = QualName::new(None, ns!(html), "strong".into());
                            serializer.start_elem(tag_name, iter::empty())?;
                        }

                        if runs.text_styles.is_italic() {
                            let tag_name = QualName::new(None, ns!(html), "em".into());
                            serializer.start_elem(tag_name, iter::empty())?;
                        }

                        if runs.text_styles.is_strikethrough() {
                            let tag_name = QualName::new(None, ns!(html), "s".into());
                            serializer.start_elem(tag_name, iter::empty())?;
                        }

                        if runs.text_styles.is_underlined() {
                            let tag_name = QualName::new(None, ns!(html), "u".into());
                            serializer.start_elem(tag_name, iter::empty())?;
                        }

                        if runs.text_styles.is_inline_code() {
                            let tag_name = QualName::new(None, ns!(html), "code".into());
                            serializer.start_elem(tag_name, iter::empty())?;
                        }

                        serializer.write_text(text)?;

                        if runs.text_styles.is_inline_code() {
                            serializer.end_elem(QualName::new(None, ns!(html), "code".into()))?;
                        }

                        if runs.text_styles.is_underlined() {
                            serializer.end_elem(QualName::new(None, ns!(html), "u".into()))?;
                        }

                        if runs.text_styles.is_strikethrough() {
                            serializer.end_elem(QualName::new(None, ns!(html), "s".into()))?;
                        }

                        if runs.text_styles.is_italic() {
                            serializer.end_elem(QualName::new(None, ns!(html), "em".into()))?;
                        }

                        if runs.text_styles.is_at_least_bold() {
                            serializer.end_elem(QualName::new(None, ns!(html), "strong".into()))?;
                        }

                        if runs.text_styles.is_link() {
                            serializer.end_elem(QualName::new(None, ns!(html), "a".into()))?;
                        }
                    }

                    if let Some(tag_name) = tag_name {
                        if matches!(text_block.style, BufferBlockStyle::CodeBlock { .. }) {
                            let code_block_name = QualName::new(None, ns!(html), "code".into());
                            serializer.end_elem(code_block_name)?;
                        }
                        serializer.end_elem(tag_name)?;
                    }
                }
            }
        }

        // If the copied region ended on a list element, close the parent list.
        if let Some(end_container) = previous_block_type.and_then(|style| container_element(&style))
        {
            end_list_with_indent(serializer, end_container, active_indent_level)?
        }

        Ok(())
    }
}

fn start_list_with_indent<S>(
    serializer: &mut S,
    tag: &str,
    indent_level: usize,
    start: Option<usize>,
) -> io::Result<()>
where
    S: Serializer,
{
    let start_attr = start.map(|start| {
        (
            QualName::new(None, ns![], "start".into()),
            start.to_string(),
        )
    });

    for i in 0..indent_level {
        // If an ordered list item had a custom number, we make it the start of the immediate parent
        // list. As a result, custom numbers on items within a list are ignored, matching the
        // behavior for Markdown.
        let list_attrs = start_attr.as_ref().and_then(|(attr, value)| {
            if i == indent_level - 1 {
                Some((attr, value.as_str()))
            } else {
                None
            }
        });
        serializer.start_elem(
            QualName::new(None, ns!(html), tag.into()),
            list_attrs.into_iter(),
        )?;
        serializer.start_elem(QualName::new(None, ns!(html), "li".into()), iter::empty())?;
    }

    Ok(())
}

fn end_list_with_indent<S>(serializer: &mut S, tag: &str, indent_level: usize) -> io::Result<()>
where
    S: Serializer,
{
    for _ in 0..indent_level {
        serializer.end_elem(QualName::new(None, ns!(html), "li".into()))?;
        serializer.end_elem(QualName::new(None, ns!(html), tag.into()))?;
    }

    Ok(())
}

/// The parent element wrapping consecutive blocks of a given style.
///
/// This only applies to list blocks.
fn container_element(style: &BufferBlockStyle) -> Option<&'static str> {
    match style {
        BufferBlockStyle::OrderedList { .. } => Some("ol"),
        BufferBlockStyle::UnorderedList { .. } | BufferBlockStyle::TaskList { .. } => Some("ul"),
        _ => None,
    }
}

fn formatted_text_line_to_block_style(line: &FormattedTextLine) -> Option<BufferBlockStyle> {
    match line {
        FormattedTextLine::Heading(header) => Some(BufferBlockStyle::Header {
            header_size: header
                .heading_size
                .try_into()
                .unwrap_or(BlockHeaderSize::Header1),
        }),
        FormattedTextLine::CodeBlock(text) => Some(BufferBlockStyle::CodeBlock {
            code_block_type: text.into(),
        }),
        FormattedTextLine::Table(table) => Some(BufferBlockStyle::table(table.alignments.clone())),
        FormattedTextLine::UnorderedList(list) => Some(BufferBlockStyle::UnorderedList {
            indent_level: ListIndentLevel::from_usize(list.indent_level),
        }),
        FormattedTextLine::TaskList(list) => Some(BufferBlockStyle::TaskList {
            indent_level: ListIndentLevel::from_usize(list.indent_level),
            complete: list.complete,
        }),
        FormattedTextLine::OrderedList(list) => Some(BufferBlockStyle::OrderedList {
            indent_level: ListIndentLevel::from_usize(list.indented_text.indent_level),
            number: list.number,
        }),
        FormattedTextLine::Line(_) => Some(BufferBlockStyle::PlainText),
        FormattedTextLine::LineBreak => None,
        FormattedTextLine::HorizontalRule => Some(BufferBlockStyle::PlainText),
        // TODO(kevin): handle embedded objects in buffer.
        FormattedTextLine::Embedded(_) => None,
        // Images are block items, not block styles
        FormattedTextLine::Image(_) => None,
    }
}

fn alignment_to_html_value(alignment: &FormattedTableAlignment) -> &'static str {
    match alignment {
        FormattedTableAlignment::Left => "left",
        FormattedTableAlignment::Center => "center",
        FormattedTableAlignment::Right => "right",
    }
}

fn serialize_table_cell_inline<S>(
    serializer: &mut S,
    inline: &[FormattedTextFragment],
) -> io::Result<()>
where
    S: Serializer,
{
    for fragment in inline {
        let styles = TextStylesWithMetadata::from(fragment.styles.clone());

        if let Some(link) = styles.link_content() {
            let tag = QualName::new(None, ns!(html), "a".into());
            let href = QualName::new(None, ns!(), "href".into());
            serializer.start_elem(tag, vec![(&href, link.as_str())].into_iter())?;
        }
        if styles.is_at_least_bold() {
            serializer.start_elem(
                QualName::new(None, ns!(html), "strong".into()),
                iter::empty(),
            )?;
        }
        if styles.is_italic() {
            serializer.start_elem(QualName::new(None, ns!(html), "em".into()), iter::empty())?;
        }
        if styles.is_strikethrough() {
            serializer.start_elem(QualName::new(None, ns!(html), "s".into()), iter::empty())?;
        }
        if styles.is_underlined() {
            serializer.start_elem(QualName::new(None, ns!(html), "u".into()), iter::empty())?;
        }
        if styles.is_inline_code() {
            serializer.start_elem(QualName::new(None, ns!(html), "code".into()), iter::empty())?;
        }

        serializer.write_text(&fragment.text)?;

        if styles.is_inline_code() {
            serializer.end_elem(QualName::new(None, ns!(html), "code".into()))?;
        }
        if styles.is_underlined() {
            serializer.end_elem(QualName::new(None, ns!(html), "u".into()))?;
        }
        if styles.is_strikethrough() {
            serializer.end_elem(QualName::new(None, ns!(html), "s".into()))?;
        }
        if styles.is_italic() {
            serializer.end_elem(QualName::new(None, ns!(html), "em".into()))?;
        }
        if styles.is_at_least_bold() {
            serializer.end_elem(QualName::new(None, ns!(html), "strong".into()))?;
        }
        if styles.link_content().is_some() {
            serializer.end_elem(QualName::new(None, ns!(html), "a".into()))?;
        }
    }
    Ok(())
}

fn serialize_table_to_html<S>(serializer: &mut S, table: &FormattedTable) -> io::Result<()>
where
    S: Serializer,
{
    let table_tag = QualName::new(None, ns!(html), "table".into());
    serializer.start_elem(table_tag.clone(), iter::empty())?;
    let thead_tag = QualName::new(None, ns!(html), "thead".into());
    serializer.start_elem(thead_tag.clone(), iter::empty())?;
    let tr_tag = QualName::new(None, ns!(html), "tr".into());
    serializer.start_elem(tr_tag.clone(), iter::empty())?;
    let align_attr = QualName::new(None, ns!(), "align".into());
    for (i, header) in table.headers.iter().enumerate() {
        let alignment = table
            .alignments
            .get(i)
            .unwrap_or(&FormattedTableAlignment::Left);
        let align_value = alignment_to_html_value(alignment);
        let th_tag = QualName::new(None, ns!(html), "th".into());
        serializer.start_elem(th_tag.clone(), vec![(&align_attr, align_value)].into_iter())?;
        serialize_table_cell_inline(serializer, header)?;
        serializer.end_elem(th_tag)?;
    }
    serializer.end_elem(tr_tag)?;
    serializer.end_elem(thead_tag)?;
    let tbody_tag = QualName::new(None, ns!(html), "tbody".into());
    serializer.start_elem(tbody_tag.clone(), iter::empty())?;
    for row in &table.rows {
        let tr_tag = QualName::new(None, ns!(html), "tr".into());
        serializer.start_elem(tr_tag.clone(), iter::empty())?;
        for (i, cell) in row.iter().enumerate() {
            let alignment = table
                .alignments
                .get(i)
                .unwrap_or(&FormattedTableAlignment::Left);
            let align_value = alignment_to_html_value(alignment);
            let td_tag = QualName::new(None, ns!(html), "td".into());
            serializer.start_elem(td_tag.clone(), vec![(&align_attr, align_value)].into_iter())?;
            serialize_table_cell_inline(serializer, cell)?;
            serializer.end_elem(td_tag)?;
        }
        serializer.end_elem(tr_tag)?;
    }
    serializer.end_elem(tbody_tag)?;
    serializer.end_elem(table_tag)?;

    Ok(())
}

fn inline_to_markdown(inline: &FormattedTextInline) -> String {
    let mut markdown = String::new();
    let mut previous_styles = TextStylesWithMetadata::default();
    for fragment in inline {
        let next_styles = TextStylesWithMetadata::from(fragment.styles.clone());
        let content = BufferMarkdownParser::append_formatting(
            &previous_styles,
            &next_styles,
            fragment.text.as_str(),
            &mut markdown,
        );
        previous_styles = next_styles;
        BufferMarkdownParser::append_content(content, true, &mut markdown);
    }
    BufferMarkdownParser::append_formatting(
        &previous_styles,
        &TextStylesWithMetadata::default(),
        "",
        &mut markdown,
    );
    markdown
}

fn append_gfm_table_row(cells: &[String], buf: &mut String) {
    buf.push('|');
    for cell in cells {
        buf.push(' ');
        buf.push_str(&escape_gfm_table_cell(cell));
        buf.push(' ');
        buf.push('|');
    }
    buf.push('\n');
}

fn alignment_to_gfm_separator(alignment: FormattedTableAlignment) -> String {
    match alignment {
        FormattedTableAlignment::Left => "---".to_string(),
        FormattedTableAlignment::Center => ":---:".to_string(),
        FormattedTableAlignment::Right => "---:".to_string(),
    }
}

fn escape_gfm_table_cell(cell: &str) -> String {
    cell.replace('|', "\\|")
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
