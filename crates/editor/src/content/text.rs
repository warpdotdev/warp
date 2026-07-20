use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::{self, Display};
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, BitXor, BitXorAssign, Range, Sub, SubAssign};
use std::sync::{Arc, OnceLock};

use arrayvec::ArrayString;
use enum_iterator::Sequence;
use lazy_static::lazy_static;
pub use markdown_parser::markdown_parser::TABLE_BLOCK_MARKDOWN_LANG;
use markdown_parser::markdown_parser::{
    CODE_BLOCK_DEFAULT_MARKDOWN_LANG, EMBED_BLOCK_MARKDOWN_LANG, RUNNABLE_BLOCK_MARKDOWN_LANG,
};
use markdown_parser::weight::CustomWeight;
use markdown_parser::{
    CodeBlockText, FormattedImage, FormattedTextLine, FormattedTextStyles, Hyperlink,
    VerticalAlign, parse_markdown,
};
pub use markdown_parser::{
    FormattedTable, FormattedTableAlignment, FormattedTextFragment, FormattedTextInline,
};
use pathfinder_color::ColorU;
use rand::Rng;
use serde::{Deserialize, Serialize};
use string_offset::{ByteOffset, CharOffset, impl_offset};
use sum_tree::{Cursor, SeekBias, SumTree};
use warp_core::features::FeatureFlag;
use warpui_core::AppContext;
use warpui_core::elements::ListIndentLevel;
use warpui_core::fonts::{Properties, Style, Weight};
use warpui_core::text::BlockHeaderSize as HeaderSize;
use warpui_core::text::point::Point;

use super::buffer::Buffer;
use super::core::CursorType;
use super::markdown::MarkdownStyle;
use crate::render::model::EmbeddedItem;
use crate::render::model::table_offset_map::{TableCellOffsetMap, TableOffsetMap};

/// Collect the plain text from a `FormattedTextInline` (a slice of fragments).
pub fn inline_to_text(inline: &[FormattedTextFragment]) -> String {
    inline
        .iter()
        .map(|fragment| fragment.text.as_str())
        .collect()
}

/// Build per-cell source↔rendered offset maps for every cell in `table`, using the raw
/// tab-and-newline-separated `source` text as the source of truth for each cell's pre-parse bytes.
///
/// We pass `source` in rather than deriving it from `table` because the parser has already stripped
/// escape backslashes, consolidated fragments, and potentially normalized table shape. Walking the
/// raw source alongside each cell's parsed fragments gives exact source spans without having to
/// reproduce marker syntax in this crate.
pub fn table_cell_offset_maps(
    table: &FormattedTable,
    source: &str,
) -> Vec<Vec<TableCellOffsetMap>> {
    let source_rows: Vec<Vec<&str>> = source
        .lines()
        .map(|row| row.split('\t').collect())
        .collect();

    std::iter::once(&table.headers)
        .chain(table.rows.iter())
        .enumerate()
        .map(|(row_idx, row)| {
            row.iter()
                .enumerate()
                .map(|(col_idx, cell)| {
                    let cell_source = source_rows
                        .get(row_idx)
                        .and_then(|row_cells| row_cells.get(col_idx))
                        .copied()
                        .unwrap_or("");
                    TableCellOffsetMap::from_inline_and_source(cell_source, cell)
                })
                .collect()
        })
        .collect()
}

/// Cached parse of a Markdown table block: the parsed `FormattedTable`, per-cell source↔rendered
/// offset maps, and the linear `TableOffsetMap` that maps character offsets to cell coordinates.
///
/// Storing these together lets consumers that already know the block's plain text (clipboard copy,
/// table layout) avoid re-running `table_from_internal_format_with_inline_markdown` and
/// `table_cell_offset_maps` on every access.
pub struct TableBlockCache {
    pub table: FormattedTable,
    pub cell_offset_maps: Vec<Vec<TableCellOffsetMap>>,
    pub offset_map: TableOffsetMap,
}

impl TableBlockCache {
    pub(super) fn build(text: &str, alignments: Vec<FormattedTableAlignment>) -> Self {
        let mut table = table_from_internal_format_with_inline_markdown(text, alignments);
        table.normalize_shape();
        let cell_offset_maps = table_cell_offset_maps(&table, text);
        let cell_lengths = cell_offset_maps
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.source_length().as_usize())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let offset_map = TableOffsetMap::new(cell_lengths);
        Self {
            table,
            cell_offset_maps,
            offset_map,
        }
    }
}

/// Lazy wrapper around a shared [`TableBlockCache`] stored on [`BufferBlockStyle::Table`].
///
/// The cache is populated on first access via [`TableCache::get_or_populate`]. Clones share the
/// same lock, so populating the cache through one clone makes the result visible to all clones.
///
/// `TableCache` intentionally compares equal to every other `TableCache` and hashes to the same
/// value, so the cache field does not affect `BufferBlockStyle` equality or hashing — two table
/// markers with the same alignments are considered equal regardless of cache state.
#[derive(Clone, Default)]
pub struct TableCache(Arc<OnceLock<TableBlockCache>>);

impl TableCache {
    /// Returns the cached parse of the table block, populating it from `text` and `alignments`
    /// on the first call.
    pub fn get_or_populate(
        &self,
        text: &str,
        alignments: &[FormattedTableAlignment],
    ) -> &TableBlockCache {
        self.0
            .get_or_init(|| TableBlockCache::build(text, alignments.to_vec()))
    }
}

impl PartialEq for TableCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for TableCache {}

impl Hash for TableCache {
    fn hash<H: Hasher>(&self, _state: &mut H) {}
}

impl fmt::Debug for TableCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TableCache").finish()
    }
}

pub const CODE_BLOCK_DEFAULT_DISPLAY_LANG: &str = "Code";
pub const CODE_BLOCK_SHELL_DISPLAY_LANG: &str = "Shell";

#[cfg(test)]
pub const TEXT_FRAGMENT_SIZE: usize = 64;
#[cfg(not(test))]
pub const TEXT_FRAGMENT_SIZE: usize = 128;

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;

/// A summary of text locations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextSummary {
    pub chars: CharOffset,
    pub bytes: ByteOffset,
    pub lines: Point,
    pub first_line_len: u32,
    pub rightmost_point: Point,
}

impl<'a> std::ops::AddAssign<&'a Self> for TextSummary {
    fn add_assign(&mut self, other: &'a Self) {
        let joined_line_len = self.lines.column + other.first_line_len;
        if joined_line_len > self.rightmost_point.column {
            self.rightmost_point = Point::new(self.lines.row, joined_line_len);
        }
        if other.rightmost_point.column > self.rightmost_point.column {
            self.rightmost_point = self.lines + other.rightmost_point;
        }

        if self.lines.row == 0 {
            self.first_line_len += other.first_line_len;
        }

        self.chars += other.chars;
        self.bytes += other.bytes;
        self.lines += other.lines;
    }
}

fn parse_table_cell_markdown_inline(cell: &str) -> FormattedTextInline {
    let Ok(parsed) = parse_markdown(cell) else {
        return vec![FormattedTextFragment::plain_text(cell)];
    };

    let mut inline = Vec::new();
    for line in parsed.lines {
        match line {
            FormattedTextLine::Line(fragments) => inline.extend(fragments),
            FormattedTextLine::Heading(header) => inline.extend(header.text),
            FormattedTextLine::OrderedList(item) => inline.extend(item.indented_text.text),
            FormattedTextLine::UnorderedList(item) => inline.extend(item.text),
            FormattedTextLine::TaskList(item) => inline.extend(item.text),
            FormattedTextLine::CodeBlock(block) => {
                inline.push(FormattedTextFragment::plain_text(block.code));
            }
            FormattedTextLine::Table(table) => {
                inline.push(FormattedTextFragment::plain_text(
                    table.to_internal_format(),
                ));
            }
            FormattedTextLine::Image(image) => {
                inline.push(FormattedTextFragment::plain_text(image.alt_text));
            }
            FormattedTextLine::LineBreak => {
                inline.push(FormattedTextFragment::plain_text("\n"));
            }
            FormattedTextLine::HorizontalRule => {
                inline.push(FormattedTextFragment::plain_text("---"));
            }
            FormattedTextLine::Embedded(_) => {}
        }
    }

    if inline.is_empty() {
        vec![FormattedTextFragment::plain_text(cell)]
    } else {
        inline
    }
}

impl std::ops::AddAssign<Self> for TextSummary {
    fn add_assign(&mut self, other: Self) {
        *self += &other;
    }
}

impl sum_tree::Dimension<'_, TextSummary> for TextSummary {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary;
    }
}

impl sum_tree::Dimension<'_, TextSummary> for Point {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary.lines;
    }
}

impl sum_tree::Dimension<'_, TextSummary> for ByteOffset {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary.bytes
    }
}

impl sum_tree::Dimension<'_, TextSummary> for CharOffset {
    fn add_summary(&mut self, summary: &TextSummary) {
        *self += summary.chars;
    }
}

pub struct Bytes<'a> {
    cursor: Cursor<'a, BufferText, ByteOffset, ByteOffset>,
    start: ByteOffset,
    end: ByteOffset,
}

impl<'a> Bytes<'a> {
    pub fn new(buffer: &'a Buffer, start: ByteOffset, end: ByteOffset) -> Self {
        Self::from_sum_tree(&buffer.content, start, end)
    }

    pub fn from_sum_tree(
        content: &'a SumTree<BufferText>,
        start: ByteOffset,
        end: ByteOffset,
    ) -> Self {
        let mut cursor = content.cursor();
        cursor.seek(&start, SeekBias::Right);
        Self { cursor, start, end }
    }

    /// Re-seek the iterator to a new starting position, keeping the same end bound.
    pub fn seek(&mut self, offset: ByteOffset) {
        self.start = offset;
        self.cursor.seek(&offset, SeekBias::Right);
    }
}

impl<'a> Iterator for Bytes<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(item) = self.cursor.item() {
            let start = *self.cursor.start();
            let end = self.cursor.end();

            if start >= self.end {
                break;
            }

            self.cursor.next();
            match item {
                BufferText::BlockMarker { .. } | BufferText::Newline => return Some(b"\n"),
                BufferText::Text { fragment, .. } => {
                    let mut slice_start = 0;
                    let mut slice_end = fragment.len();
                    if self.start > start {
                        slice_start = (self.start - start).as_usize();
                    }

                    if self.end < end {
                        slice_end = slice_end.saturating_sub((end - self.end).as_usize());
                    }
                    return fragment.as_bytes().get(slice_start..slice_end);
                }
                _ => (),
            };
        }

        None
    }
}

#[derive(Eq, PartialEq, Clone, Debug)]
pub enum ColorMarker {
    Start(ColorU),
    End,
}

impl ColorMarker {
    fn to_counter_delta(&self) -> i32 {
        match &self {
            ColorMarker::Start(_) => 1,
            ColorMarker::End => -1,
        }
    }
}

#[derive(Eq, PartialEq, Clone, Debug)]
pub enum LinkMarker {
    Start(String),
    End,
}

impl LinkMarker {
    fn to_counter_delta(&self) -> i32 {
        match &self {
            LinkMarker::Start(_) => 1,
            LinkMarker::End => -1,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum BlockType {
    Item(BufferBlockItem),
    Text(BufferBlockStyle),
}

/// Parse the internal tab-separated format
/// `parse_markdown` to each cell so that inline formatting is preserved.
pub fn table_from_internal_format_with_inline_markdown(
    content: &str,
    mut alignments: Vec<FormattedTableAlignment>,
) -> FormattedTable {
    let parse_line = |line: &str| -> Vec<FormattedTextInline> {
        line.split('\t')
            .map(parse_table_cell_markdown_inline)
            .collect()
    };

    let mut lines = content.lines().peekable();
    let headers = lines.next().map(parse_line).unwrap_or_default();
    let rows: Vec<Vec<FormattedTextInline>> = lines.map(parse_line).collect();
    let col_count = headers.len();
    alignments.resize(col_count, FormattedTableAlignment::default());

    FormattedTable {
        headers,
        alignments,
        rows,
    }
}

#[derive(Clone, Debug)]
pub enum BufferBlockItem {
    HorizontalRule,
    Embedded {
        item: Arc<dyn EmbeddedItem>,
    },
    Image {
        alt_text: String,
        source: String,
        /// Optional CommonMark image title, preserved so buffer round-trips
        /// and exports do not drop the `title` suffix.
        title: Option<String>,
    },
}

impl Eq for BufferBlockItem {}

impl PartialEq for BufferBlockItem {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Embedded { item: left }, Self::Embedded { item: right }) => {
                left.hashed_id() == right.hashed_id()
            }
            (Self::HorizontalRule, Self::HorizontalRule) => true,
            (
                Self::Image {
                    alt_text: alt_left,
                    source: src_left,
                    title: title_left,
                },
                Self::Image {
                    alt_text: alt_right,
                    source: src_right,
                    title: title_right,
                },
            ) => alt_left == alt_right && src_left == src_right && title_left == title_right,
            _ => false,
        }
    }
}

impl BufferBlockItem {
    pub fn content_length(&self) -> usize {
        match self {
            Self::HorizontalRule | Self::Embedded { .. } | Self::Image { .. } => 1,
        }
    }

    pub fn line_count(&self) -> u32 {
        match self {
            Self::HorizontalRule | Self::Embedded { .. } | Self::Image { .. } => 1,
        }
    }

    pub fn as_markdown(&self, style: MarkdownStyle) -> Cow<'_, str> {
        match &self {
            Self::HorizontalRule => "***".into(),
            Self::Embedded { item } => {
                let mapping = item.to_mapping(style);
                format!(
                    "```{}\n{}\n```",
                    EMBED_BLOCK_MARKDOWN_LANG,
                    serde_yaml::to_string(&mapping)
                        .expect("Serde YAML mapping should convert to string")
                )
                .into()
            }
            Self::Image {
                alt_text,
                source,
                title,
            } => format_image_markdown(alt_text, source, title.as_deref()).into(),
        }
    }

    pub fn as_rich_format_text(&self, app: &AppContext) -> Cow<'_, str> {
        match &self {
            Self::HorizontalRule => "***\n".into(),
            Self::Embedded { item } => {
                format!("```\n{}\n```\n", item.to_rich_format(app).plain_text).into()
            }
            Self::Image {
                alt_text,
                source,
                title,
            } => format!(
                "{}\n",
                format_image_markdown(alt_text, source, title.as_deref())
            )
            .into(),
        }
    }

    pub fn to_formatted_text_line(&self) -> FormattedTextLine {
        match self {
            BufferBlockItem::HorizontalRule => FormattedTextLine::HorizontalRule,
            BufferBlockItem::Embedded { item } => {
                FormattedTextLine::Embedded(item.to_mapping(MarkdownStyle::Internal))
            }
            BufferBlockItem::Image {
                alt_text,
                source,
                title,
            } => FormattedTextLine::Image(FormattedImage {
                alt_text: alt_text.clone(),
                source: source.clone(),
                title: title.clone(),
            }),
        }
    }

    pub fn as_plain_text(&self) -> String {
        match self {
            Self::HorizontalRule | Self::Embedded { .. } | Self::Image { .. } => "\n".to_string(),
        }
    }

    pub fn as_plain_text_sliced(&self, start: usize, end: usize) -> String {
        let full_text = self.as_plain_text();
        full_text
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }
}

/// Serialize a CommonMark image to `![alt](source)` or `![alt](source "title")`.
///
/// Titles are canonically re-serialized with double quotes to match the HTML
/// export and CommonMark's canonical output. Any literal `"` in the title is
/// escaped with a backslash so the round-trip remains lossless.
pub fn format_image_markdown(alt_text: &str, source: &str, title: Option<&str>) -> String {
    match title.filter(|t| !t.is_empty()) {
        Some(title) => {
            let escaped = title.replace('\\', "\\\\").replace('"', "\\\"");
            format!("![{alt_text}]({source} \"{escaped}\")")
        }
        None => format!("![{alt_text}]({source})"),
    }
}

/// Building units of a buffer.
#[derive(Eq, PartialEq, Clone, Debug)]
pub enum BufferText {
    /// A character.
    Text {
        fragment: ArrayString<TEXT_FRAGMENT_SIZE>,
        char_count: u8,
    },
    /// A style marker. The markers come in pairs. If there is a starting marker,
    /// it has to be closed by an ending marker.
    Marker {
        /// The style type the marker represents.
        marker_type: BufferTextStyle,
        /// Whether this is a start or an end marker.
        dir: MarkerDir,
    },
    Link(LinkMarker),
    Color(ColorMarker),
    /// A newline.
    Newline,
    /// Block-level item that takes up an entire line.
    BlockItem {
        item_type: BufferBlockItem,
    },
    /// Styling marker that decorates an entire paragraph.
    BlockMarker {
        marker_type: BufferBlockStyle,
    },
    /// Ghosted text, such as an autosuggestion or zero-state placeholder for a newly-inserted block.
    Placeholder {
        content: String,
    },
}

impl Display for BufferText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text { fragment, .. } => write!(f, "{fragment}"),
            Self::Newline => f.write_str("\\n"),
            Self::Marker { marker_type, dir } => {
                let start = match marker_type {
                    BufferTextStyle::Weight(_) => "b",
                    BufferTextStyle::Italic => "i",
                    BufferTextStyle::Underline => "u",
                    BufferTextStyle::InlineCode => "c",
                    BufferTextStyle::StrikeThrough => "s",
                    BufferTextStyle::Subscript => "sub",
                    BufferTextStyle::Superscript => "sup",
                };

                let end = match dir {
                    MarkerDir::Start => "s",
                    MarkerDir::End => "e",
                };

                write!(f, "<{start}_{end}>")
            }
            Self::Color(marker) => {
                let name = match marker {
                    ColorMarker::Start(color) => format!("c_{:?}", *color),
                    ColorMarker::End => "c".to_string(),
                };

                write!(f, "<{name}>")
            }
            Self::Link(marker) => {
                let name = match marker {
                    LinkMarker::Start(url) => format!("a_{url}"),
                    LinkMarker::End => "a".to_string(),
                };

                write!(f, "<{name}>")
            }
            Self::BlockItem { item_type } => {
                let name = match item_type {
                    BufferBlockItem::HorizontalRule => "hr".to_string(),
                    BufferBlockItem::Embedded { item } => format!("embed_{}", item.hashed_id()),
                    BufferBlockItem::Image {
                        alt_text,
                        source,
                        title,
                    } => match title {
                        Some(title) => format!("img_{alt_text}_{source}_{title}"),
                        None => format!("img_{alt_text}_{source}"),
                    },
                };

                write!(f, "<{name}>")
            }
            Self::BlockMarker { marker_type } => {
                f.write_str("<")?;
                match marker_type {
                    BufferBlockStyle::CodeBlock { code_block_type } => {
                        write!(f, "code:{code_block_type}")?;
                    }
                    BufferBlockStyle::PlainText => f.write_str("text")?,
                    BufferBlockStyle::Header { header_size } => {
                        write!(f, "header{}", Into::<usize>::into(*header_size))?;
                    }
                    BufferBlockStyle::TaskList {
                        indent_level,
                        complete,
                    } => {
                        write!(f, "cl{}:{}", indent_level.as_usize(), *complete)?;
                    }
                    BufferBlockStyle::UnorderedList { indent_level } => {
                        write!(f, "ul{}", indent_level.as_usize())?;
                    }
                    BufferBlockStyle::OrderedList {
                        indent_level,
                        number,
                    } => {
                        write!(f, "ol{}", indent_level.as_usize())?;
                        if let Some(number) = number {
                            write!(f, "@{number}")?;
                        }
                    }
                    BufferBlockStyle::Table { .. } => f.write_str("table")?,
                };
                f.write_str(">")
            }
            Self::Placeholder { content } => write!(f, "<placeholder_s>{content}<placeholder_e>"),
        }
    }
}

/// Identify whether it is a starting or ending marker.
#[derive(Eq, PartialEq, Clone, Debug, Copy)]
pub enum MarkerDir {
    Start,
    End,
}

impl MarkerDir {
    fn to_counter_delta(self) -> i32 {
        match self {
            MarkerDir::Start => 1,
            MarkerDir::End => -1,
        }
    }
}

impl fmt::Display for CodeBlockType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeBlockType::Shell => write!(f, "{CODE_BLOCK_SHELL_DISPLAY_LANG}"),
            CodeBlockType::Mermaid => write!(f, "Mermaid"),
            CodeBlockType::Code { lang } if lang == "text" => {
                write!(f, "{CODE_BLOCK_DEFAULT_DISPLAY_LANG}")
            }
            CodeBlockType::Code { lang } => write!(f, "{lang}"),
        }
    }
}

#[derive(Eq, PartialEq, Clone, Debug, Hash, Default)]
pub enum CodeBlockType {
    #[default]
    Shell,
    Mermaid,
    Code {
        lang: String,
    },
}

lazy_static! {
        /// Markdown languages that we consider shell commands
    static ref MARKDOWN_SHELL_LANGUAGES: HashSet<&'static str> = HashSet::from([
        RUNNABLE_BLOCK_MARKDOWN_LANG,
        "sh",
        "shell",
        "bash",
        "fish",
        "zsh",
    ]);
}

impl From<&CodeBlockText> for CodeBlockType {
    fn from(code_block_text: &CodeBlockText) -> Self {
        // Markdown blocks can contain metadata after the language, e.g.:
        // ```rust path=/foo start=1
        // Only use the first token as the language identifier.
        let lang = code_block_text
            .lang
            .as_str()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_lowercase();

        if MARKDOWN_SHELL_LANGUAGES.contains(lang.as_str()) {
            CodeBlockType::Shell
        } else if FeatureFlag::MarkdownMermaid.is_enabled()
            && mermaid_to_svg::is_mermaid_diagram(code_block_text.lang.as_str())
        {
            CodeBlockType::Mermaid
        } else {
            // Parse all the recognized languages supported by the code block.
            let recognized_lang = match lang.as_str() {
                "go" | "golang" => "Go",
                "c++" | "cpp" => "C++",
                "c#" | "csharp" | "cs" => "C#",
                "java" | "groovy" => "Java",
                "javascript" | "jsx" | "js" => "JavaScript",
                "python" | "py" => "Python",
                "ruby on rails" | "ruby" => "Ruby on Rails",
                "rust" => "Rust",
                "sql" => "SQL",
                "yaml" => "YAML",
                "json" => "JSON",
                "php" => "PHP",
                "toml" => "TOML",
                "swift" => "Swift",
                "kotlin" => "Kotlin",
                "powershell" => "PowerShell",
                text => text,
            };
            CodeBlockType::Code {
                lang: recognized_lang.to_string(),
            }
        }
    }
}

impl CodeBlockType {
    pub fn all() -> impl Iterator<Item = Self> {
        // TODO: This should include all supported languages
        [
            CodeBlockType::Shell,
            CodeBlockType::Mermaid,
            CodeBlockType::Code {
                lang: "PowerShell".to_owned(),
            },
            CodeBlockType::Code {
                lang: "C++".to_owned(),
            },
            CodeBlockType::Code {
                lang: "C#".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Go".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Java".to_owned(),
            },
            CodeBlockType::Code {
                lang: "JavaScript".to_owned(),
            },
            CodeBlockType::Code {
                lang: "JSON".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Kotlin".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Lua".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Python".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Ruby".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Ruby on Rails".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Rust".to_owned(),
            },
            CodeBlockType::Code {
                lang: "SQL".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Swift".to_owned(),
            },
            CodeBlockType::Code {
                lang: "YAML".to_owned(),
            },
            CodeBlockType::Code {
                lang: "PHP".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Elixir".to_owned(),
            },
            CodeBlockType::Code {
                lang: "Scala".to_owned(),
            },
            CodeBlockType::Code {
                lang: CODE_BLOCK_DEFAULT_MARKDOWN_LANG.to_owned(),
            },
        ]
        .into_iter()
    }

    pub fn to_markdown_representation(&self, style: MarkdownStyle) -> &str {
        match self {
            CodeBlockType::Shell => RUNNABLE_BLOCK_MARKDOWN_LANG,
            CodeBlockType::Mermaid => "mermaid",
            CodeBlockType::Code { lang } => match style {
                MarkdownStyle::Internal => lang,
                MarkdownStyle::Export { .. } => {
                    // Undo the language parsing done by From<&CodeBlockText>.
                    match lang.as_str() {
                        "Go" => "go",
                        "C++" => "cpp",
                        "C#" => "csharp",
                        "Java" => "java",
                        "JavaScript" => "js",
                        "Python" => "python",
                        "Ruby on Rails" => "ruby",
                        "Ruby" => "ruby",
                        "Rust" => "rust",
                        "YAML" => "yaml",
                        "JSON" => "json",
                        "PHP" => "php",
                        "TOML" => "toml",
                        "Swift" => "swift",
                        "Kotlin" => "kotlin",
                        "PowerShell" => "powershell",
                        "Elixir" => "elixir",
                        "Scala" => "scala",
                        text => text,
                    }
                }
            },
        }
    }
}

#[derive(Eq, PartialEq, Clone, Debug, Hash)]
pub enum BufferBlockStyle {
    CodeBlock {
        code_block_type: CodeBlockType,
    },
    TaskList {
        indent_level: ListIndentLevel,
        complete: bool,
    },
    PlainText,
    Header {
        header_size: BlockHeaderSize,
    },
    UnorderedList {
        indent_level: ListIndentLevel,
    },
    OrderedList {
        number: Option<usize>,
        indent_level: ListIndentLevel,
    },
    Table {
        alignments: Vec<FormattedTableAlignment>,
        /// Lazy cache of the parsed table, per-cell offset maps, and linear offset map for this
        /// block. Does not participate in equality or hashing.
        #[allow(dead_code)]
        cache: TableCache,
    },
}

impl BufferBlockStyle {
    /// Construct a new `Table` block style with an empty cache.
    pub fn table(alignments: Vec<FormattedTableAlignment>) -> Self {
        Self::Table {
            alignments,
            cache: TableCache::default(),
        }
    }

    pub(super) fn line_break_behavior(&self) -> BlockLineBreakBehavior {
        match self {
            Self::Header { .. } => BlockLineBreakBehavior::BlockMarker(BufferBlockStyle::PlainText),
            Self::TaskList { indent_level, .. } => {
                BlockLineBreakBehavior::BlockMarker(BufferBlockStyle::TaskList {
                    indent_level: *indent_level,
                    complete: false,
                })
            }
            Self::UnorderedList { indent_level } => {
                BlockLineBreakBehavior::BlockMarker(BufferBlockStyle::UnorderedList {
                    indent_level: *indent_level,
                })
            }
            Self::OrderedList { indent_level, .. } => {
                BlockLineBreakBehavior::BlockMarker(BufferBlockStyle::OrderedList {
                    indent_level: *indent_level,
                    number: None,
                })
            }
            Self::PlainText | Self::CodeBlock { .. } | Self::Table { .. } => {
                BlockLineBreakBehavior::NewLine
            }
        }
    }

    /// This function is used to determine whether a newly inserted block should inherit the previous block's
    /// style based on the current cursor position and previous block style type.
    pub(super) fn should_inherit_style(
        &self,
        edit_cursor: CursorType,
        previous_block_style: BufferBlockStyle,
    ) -> bool {
        match self {
            // For plain text and runnable code blocks, always inherit the previous block's styling if
            // the cursor is not at buffer start.
            Self::PlainText | Self::CodeBlock { .. } | Self::Table { .. } => {
                edit_cursor != CursorType::BufferStart
            }
            // For other non-plain text blocks, inherit the previous block's styling if
            // 1) The block styling is different.
            // 2) The cursor is either inline or in a runnable code block.
            Self::Header { .. }
            | Self::OrderedList { .. }
            | Self::UnorderedList { .. }
            | Self::TaskList { .. } => {
                previous_block_style != *self
                    && (edit_cursor == CursorType::Inline
                        || (edit_cursor == CursorType::NewLineStart
                            && matches!(previous_block_style, BufferBlockStyle::CodeBlock { .. })))
            }
        }
    }

    /// Whether or not this block type allows user-defined formatting (e.g. bold, hyperlinks).
    pub fn allows_formatting(&self) -> bool {
        match self {
            Self::PlainText
            | Self::Header { .. }
            | Self::UnorderedList { .. }
            | Self::OrderedList { .. }
            | Self::TaskList { .. }
            | Self::Table { .. } => true,
            Self::CodeBlock { .. } => false,
        }
    }

    /// Construct an auto-numbered ordered list style.
    pub fn ordered_list(indent_level: ListIndentLevel) -> Self {
        Self::OrderedList {
            number: None,
            indent_level,
        }
    }
}

/// This describes different block's behavior when encountering a line break.
/// For multi-line blocks like code block, line break should insert a newline.
/// For single-line blocks like lists, line break should insert a marker for a new block.
/// Note that there are differences within single-line blocks as well. Lists should create
/// a new list item on linebreak while headers should switch to plain text.
#[derive(Eq, PartialEq, Debug)]
pub(super) enum BlockLineBreakBehavior {
    NewLine,
    BlockMarker(BufferBlockStyle),
}

/// The unit that should be applied on each indent.
#[derive(Eq, PartialEq, Clone, Copy, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum IndentUnit {
    Tab,
    Space(usize),
}

impl Default for IndentUnit {
    fn default() -> Self {
        Self::Space(4)
    }
}

impl IndentUnit {
    pub fn width(&self) -> usize {
        match self {
            Self::Tab => 1,
            Self::Space(num) => *num,
        }
    }

    pub fn char_unit(&self) -> String {
        match self {
            Self::Tab => "\t".to_string(),
            Self::Space(_) => " ".to_string(),
        }
    }

    pub fn text_with_num_tab_stops(&self, count: usize) -> String {
        match self {
            Self::Tab => "\t".repeat(count),
            Self::Space(num) => " ".repeat(*num * count),
        }
    }
}

/// This describes behavior when indenting or unindenting a line.
#[derive(Eq, PartialEq)]
pub enum IndentBehavior {
    /// Restyle the block to the new style (for example, to change a list indent level).
    Restyle(BufferBlockStyle),
    /// Insert or remove one tab's worth of spaces at the start of the line.
    TabIndent(IndentUnit),
    /// Do nothing - this block cannot be (un)indented.
    Ignore,
}

/// To keep this PR more reasonably sized, I'm going to add this alias.
/// I'll remove the alias in a follow-up PR.
pub type BlockHeaderSize = HeaderSize;

/// Supported text styles in the buffer.
#[derive(Eq, PartialEq, Clone, Copy, Debug, Hash, Sequence)]
pub enum BufferTextStyle {
    Weight(CustomWeight),
    Italic,
    Underline,
    InlineCode,
    StrikeThrough,
    Subscript,
    Superscript,
}

impl BufferTextStyle {
    pub fn bold() -> Self {
        Self::Weight(CustomWeight::Bold)
    }

    pub fn custom_weight(&self) -> Option<Weight> {
        match self {
            Self::Weight(weight) => Some(Weight::from_custom_weight(Some(*weight))),
            _ => None,
        }
    }

    /// Returns true if the style is [`BufferTextStyle::Weight`] and the weight is non-Normal.
    pub fn has_custom_weight(&self) -> bool {
        matches!(self, Self::Weight(_))
    }

    pub fn random<R: Rng>(rng: &mut R) -> Self {
        let r = rng.gen_range(0..7);
        match r {
            0 => Self::Weight(CustomWeight::Bold),
            1 => Self::Italic,
            2 => Self::Underline,
            3 => Self::InlineCode,
            4 => Self::StrikeThrough,
            5 => Self::Subscript,
            6 => Self::Superscript,
            _ => unreachable!(),
        }
    }
}

#[derive(Eq, PartialEq, Debug, Default, Clone)]
pub struct TextStylesWithMetadata {
    weight: Option<CustomWeight>,
    italic: bool,
    underline: bool,
    inline_code: bool,
    placeholder: bool,
    strikethrough: bool,
    link: Option<String>,
    color: Option<ColorU>,
    vertical_align: Option<VerticalAlign>,
}

impl TextStylesWithMetadata {
    pub fn bold(mut self) -> Self {
        self.weight = Some(CustomWeight::Bold);
        self
    }

    pub fn set_weight(&mut self, weight: Option<Weight>) {
        self.weight = weight.and_then(|w| w.to_custom_weight());
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn inline_code(mut self) -> Self {
        self.inline_code = true;
        self
    }

    pub fn subscript(mut self) -> Self {
        self.vertical_align = Some(VerticalAlign::Sub);
        self
    }

    pub fn superscript(mut self) -> Self {
        self.vertical_align = Some(VerticalAlign::Sup);
        self
    }

    pub fn link(mut self, link: String) -> Self {
        self.link = Some(link);
        self
    }

    pub fn with_color(mut self, color: ColorU) -> Self {
        self.color = Some(color);
        self
    }

    pub fn color_mut(&mut self) -> &mut Option<ColorU> {
        &mut self.color
    }

    pub fn inline_code_mut(&mut self) -> &mut bool {
        &mut self.inline_code
    }

    pub fn style_mut(&mut self, style: &BufferTextStyle) -> Option<&mut bool> {
        match style {
            BufferTextStyle::Italic => Some(&mut self.italic),
            BufferTextStyle::Underline => Some(&mut self.underline),
            BufferTextStyle::InlineCode => Some(&mut self.inline_code),
            BufferTextStyle::StrikeThrough => Some(&mut self.strikethrough),
            // Vertical alignment isn't a `bool` toggle (it's an `Option<VerticalAlign>`), so it
            // can't be mutated through this accessor; callers apply it via `set_vertical_align`.
            BufferTextStyle::Weight(_)
            | BufferTextStyle::Subscript
            | BufferTextStyle::Superscript => None,
        }
    }

    /// Applies or clears the vertical alignment implied by a sub/superscript marker.
    pub fn set_vertical_align(&mut self, style: &BufferTextStyle, active: bool) {
        let align = match style {
            BufferTextStyle::Subscript => VerticalAlign::Sub,
            BufferTextStyle::Superscript => VerticalAlign::Sup,
            _ => return,
        };
        self.vertical_align = active.then_some(align);
    }

    /// Sets the collapsed vertical alignment directly. Used by the render iterator, which tracks
    /// subscript/superscript nesting depth separately and resolves the overlap through
    /// [`resolve_vertical_align`] so an inner end-marker can restore the still-active outer alignment
    /// (issue #13734 finding 3).
    pub(super) fn set_vertical_align_value(&mut self, vertical_align: Option<VerticalAlign>) {
        self.vertical_align = vertical_align;
    }

    pub fn colliding_style(&self, style: &BufferTextStyle) -> bool {
        match style {
            BufferTextStyle::Italic => self.italic,
            BufferTextStyle::Underline => self.underline,
            BufferTextStyle::InlineCode => self.inline_code,
            BufferTextStyle::StrikeThrough => self.strikethrough,
            BufferTextStyle::Weight(_) => self.weight.is_some(),
            BufferTextStyle::Subscript => self.is_subscript(),
            BufferTextStyle::Superscript => self.is_superscript(),
        }
    }

    pub fn exact_match_style(&self, style: &BufferTextStyle) -> bool {
        match style {
            BufferTextStyle::Italic => self.italic,
            BufferTextStyle::Underline => self.underline,
            BufferTextStyle::InlineCode => self.inline_code,
            BufferTextStyle::StrikeThrough => self.strikethrough,
            BufferTextStyle::Weight(weight) => self.weight == Some(*weight),
            BufferTextStyle::Subscript => self.is_subscript(),
            BufferTextStyle::Superscript => self.is_superscript(),
        }
    }

    /// Applies the [`Properties`] encompassed by this style.
    pub fn apply_properties(&self, mut properties: Properties) -> Properties {
        if self.italic {
            properties = properties.style(Style::Italic);
        }
        // To ensure we respect the weight set by the block type,
        // we only set the weight if its explicitly set, rather than
        // allowing `None` to override the weight set by the block type.
        if let Some(custom_weight) = self.weight {
            properties = properties.weight(Weight::from_custom_weight(Some(custom_weight)));
        }

        properties
    }

    pub fn link_mut(&mut self) -> &mut Option<String> {
        &mut self.link
    }

    pub fn link_content(&self) -> Option<String> {
        self.link.clone()
    }

    /// Returns true if Weight is [`Weight::Normal`].
    pub fn is_normal_weight(&self) -> bool {
        self.weight.is_none()
    }

    /// Whether or not the weight is at least [`Weight::Bold`].
    pub fn is_at_least_bold(&self) -> bool {
        self.weight.is_some_and(|w| w.is_at_least_bold())
    }

    pub fn is_inline_code(&self) -> bool {
        self.inline_code
    }

    pub fn is_italic(&self) -> bool {
        self.italic
    }

    pub fn is_underlined(&self) -> bool {
        self.underline
    }

    pub fn is_subscript(&self) -> bool {
        self.vertical_align == Some(VerticalAlign::Sub)
    }

    pub fn is_superscript(&self) -> bool {
        self.vertical_align == Some(VerticalAlign::Sup)
    }

    /// The raw collapsed alignment, if any. Used to seed the render iterator's `last_opened`
    /// tracking at a block boundary, where at most one alignment can be carried over.
    pub(super) fn vertical_align(&self) -> Option<VerticalAlign> {
        self.vertical_align
    }

    pub fn is_link(&self) -> bool {
        self.link.is_some()
    }

    pub fn is_strikethrough(&self) -> bool {
        self.strikethrough
    }

    pub fn is_placeholder(&self) -> bool {
        self.placeholder
    }

    pub fn color(&self) -> Option<ColorU> {
        self.color
    }

    pub fn from_text_styles(
        text_styles: TextStyles,
        link: Option<String>,
        color: Option<ColorU>,
    ) -> Self {
        Self {
            weight: text_styles.weight,
            italic: text_styles.italic,
            underline: text_styles.underline,
            placeholder: text_styles.placeholder,
            inline_code: text_styles.inline_code,
            strikethrough: text_styles.strikethrough,
            link,
            color,
            vertical_align: text_styles.vertical_align,
        }
    }

    /// Inherited text style behavior after backspacing.
    pub fn text_style_override_after_deletion(self, active_style: Self) -> Self {
        Self {
            weight: self.weight,
            italic: self.italic,
            inline_code: active_style.inline_code,
            ..Default::default()
        }
    }

    /// Mark these as styles for placeholder text. Currently, in [`super::buffer::StyledBufferRun`],
    /// we report placeholders as inline styles.
    pub fn for_placeholder(mut self) -> Self {
        self.placeholder = true;
        self
    }

    /// Given two text styles, return the styles that are active for both.
    pub fn mutual_styles(self, other: Self) -> Self {
        let link = match (self.link, other.link) {
            (Some(link1), Some(link2)) if link1 == link2 => Some(link1),
            _ => None,
        };

        let color = match (self.color, other.color) {
            (Some(color1), Some(color2)) if color1 == color2 => Some(color1),
            _ => None,
        };

        // If the weights are different, use the default Normal weight.
        let weight = if self.weight.eq(&other.weight) {
            self.weight
        } else {
            Default::default()
        };

        // Only a shared vertical alignment survives; differing (or absent) alignment yields none.
        let vertical_align = if self.vertical_align == other.vertical_align {
            self.vertical_align
        } else {
            None
        };

        Self {
            weight,
            italic: self.italic && other.italic,
            underline: self.underline && other.underline,
            inline_code: self.inline_code && other.inline_code,
            strikethrough: self.strikethrough && other.strikethrough,
            link,
            color,
            placeholder: self.placeholder && other.placeholder,
            vertical_align,
        }
    }
}

impl From<FormattedTextStyles> for TextStylesWithMetadata {
    fn from(styles: FormattedTextStyles) -> Self {
        TextStylesWithMetadata {
            weight: styles.weight,
            italic: styles.italic,
            underline: styles.underline,
            inline_code: styles.inline_code,
            strikethrough: styles.strikethrough,
            placeholder: false,
            link: styles.hyperlink.and_then(Hyperlink::url),
            color: None, // TODO: Update this when adding strikethrough support.
            vertical_align: styles.vertical_align,
        }
    }
}

impl From<TextStylesWithMetadata> for FormattedTextStyles {
    fn from(styles: TextStylesWithMetadata) -> Self {
        FormattedTextStyles {
            weight: styles.weight,
            italic: styles.italic,
            underline: styles.underline,
            strikethrough: styles.strikethrough,
            hyperlink: styles.link.map(Hyperlink::Url),
            inline_code: styles.inline_code,
            vertical_align: styles.vertical_align,
        }
    }
}

#[derive(Eq, PartialEq, Debug, Default, Clone, Copy)]
pub struct TextStyles {
    weight: Option<CustomWeight>,
    italic: bool,
    underline: bool,
    inline_code: bool,
    strikethrough: bool,
    /// Whether or not this is placeholder text. Placeholders are sort of a pseudo-style - they're
    /// represented as their own kind of buffer content, but we render them as inline styled text.
    placeholder: bool,
    link: bool,
    colored: bool,
    /// Subscript and superscript are mutually exclusive, so a single `Option<VerticalAlign>` (rather
    /// than two independent flags) makes the "both set" state unrepresentable — mirroring the storage
    /// in `FormattedTextStyles`/`TextStylesWithMetadata` so the conversions stay lossless.
    vertical_align: Option<VerticalAlign>,
}

impl TextStyles {
    pub fn all() -> Self {
        Self {
            weight: Some(CustomWeight::Bold),
            italic: true,
            underline: true,
            inline_code: true,
            strikethrough: true,
            // Sub and sup are mutually exclusive, so `all()` can only carry one; pick subscript.
            vertical_align: Some(VerticalAlign::Sub),
            ..Default::default()
        }
    }

    pub fn bold(mut self) -> Self {
        self.weight = Some(CustomWeight::Bold);
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn inline_code(mut self) -> Self {
        self.inline_code = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    pub fn subscript(mut self) -> Self {
        self.vertical_align = Some(VerticalAlign::Sub);
        self
    }

    pub fn superscript(mut self) -> Self {
        self.vertical_align = Some(VerticalAlign::Sup);
        self
    }

    pub fn is_subscript(&self) -> bool {
        self.vertical_align == Some(VerticalAlign::Sub)
    }

    pub fn is_superscript(&self) -> bool {
        self.vertical_align == Some(VerticalAlign::Sup)
    }

    /// Whether this mask carries *any* vertical alignment. Unlike [`Self::is_subscript`] and
    /// [`Self::is_superscript`], this doesn't distinguish which one — it's for "strip all styles"
    /// callers (see [`Self::all`]) that need to treat `Subscript` and `Superscript` as a single
    /// family, since the underlying `Option<VerticalAlign>` can only ever name one of them at a time.
    pub fn has_any_vertical_align(&self) -> bool {
        self.vertical_align.is_some()
    }

    pub fn set_weight(&mut self, weight: Weight) {
        self.weight = weight.to_custom_weight();
    }

    /// Applies or clears the vertical alignment implied by a sub/superscript marker. Mirrors the
    /// `Weight` marker handling: `style_mut` returns `None` for these (they aren't plain toggles),
    /// so callers reach for this instead.
    pub fn set_vertical_align(&mut self, style: &BufferTextStyle, active: bool) {
        let align = match style {
            BufferTextStyle::Subscript => VerticalAlign::Sub,
            BufferTextStyle::Superscript => VerticalAlign::Sup,
            _ => return,
        };
        self.vertical_align = active.then_some(align);
    }

    pub fn style_mut(&mut self, style: &BufferTextStyle) -> Option<&mut bool> {
        match style {
            BufferTextStyle::Italic => Some(&mut self.italic),
            BufferTextStyle::Underline => Some(&mut self.underline),
            BufferTextStyle::InlineCode => Some(&mut self.inline_code),
            BufferTextStyle::StrikeThrough => Some(&mut self.strikethrough),
            // Vertical alignment is an `Option<VerticalAlign>`, not a `bool` toggle, so it's applied
            // through `set_vertical_align` rather than this accessor — same as `Weight`.
            BufferTextStyle::Weight(_)
            | BufferTextStyle::Subscript
            | BufferTextStyle::Superscript => None,
        }
    }

    pub fn colliding_style(&self, style: &BufferTextStyle) -> bool {
        match style {
            BufferTextStyle::Italic => self.italic,
            BufferTextStyle::Underline => self.underline,
            BufferTextStyle::InlineCode => self.inline_code,
            BufferTextStyle::StrikeThrough => self.strikethrough,
            BufferTextStyle::Subscript => self.is_subscript(),
            BufferTextStyle::Superscript => self.is_superscript(),
            BufferTextStyle::Weight(_) => self.weight.is_some(),
        }
    }

    pub fn exact_match_style(&self, style: &BufferTextStyle) -> bool {
        match style {
            BufferTextStyle::Italic => self.italic,
            BufferTextStyle::Underline => self.underline,
            BufferTextStyle::InlineCode => self.inline_code,
            BufferTextStyle::StrikeThrough => self.strikethrough,
            BufferTextStyle::Subscript => self.is_subscript(),
            BufferTextStyle::Superscript => self.is_superscript(),
            BufferTextStyle::Weight(weight) => Some(*weight) == self.weight,
        }
    }

    pub fn link_mut(&mut self) -> &mut bool {
        &mut self.link
    }

    pub fn is_inline_code(&self) -> bool {
        self.inline_code
    }

    pub fn is_strikethrough(&self) -> bool {
        self.strikethrough
    }

    pub fn get_weight(&self) -> Weight {
        Weight::from_custom_weight(self.weight)
    }

    pub fn get_custom_weight(&self) -> Option<CustomWeight> {
        self.weight
    }

    pub fn is_italic(&self) -> bool {
        self.italic
    }

    pub fn is_underlined(&self) -> bool {
        self.underline
    }

    pub fn is_link(&self) -> bool {
        self.link
    }

    pub fn is_placeholder(&self) -> bool {
        self.placeholder
    }

    pub fn is_colored(&self) -> bool {
        self.colored
    }

    /// Filters to only inheritable text styles. Keep in sync with [`BufferTextStyle::is_inheritable`].
    pub fn inheritable(self) -> Self {
        Self {
            weight: self.weight,
            italic: self.italic,
            underline: self.underline,
            inline_code: self.inline_code,
            strikethrough: self.strikethrough,
            vertical_align: self.vertical_align,
            placeholder: false,
            link: false,
            colored: false,
        }
    }
}

impl From<TextStylesWithMetadata> for TextStyles {
    fn from(styles: TextStylesWithMetadata) -> TextStyles {
        Self {
            weight: styles.weight,
            italic: styles.italic,
            underline: styles.underline,
            placeholder: styles.placeholder,
            inline_code: styles.inline_code,
            strikethrough: styles.strikethrough,
            link: styles.link.is_some(),
            colored: styles.color.is_some(),
            vertical_align: styles.vertical_align,
        }
    }
}

/// XOR-toggle delta for an enum-valued `Option` style (`weight`, `vertical_align`).
///
/// Unlike a `bool` toggle, these fields carry a *value* (e.g. `Sub` vs `Sup`, `Bold` vs `Light`),
/// so equal presence isn't enough to decide the delta: comparing only `is_some()` drops a direct
/// value switch like `Sub` → `Sup` (both present) to no delta, silently losing the transition
/// (issue #14029). Compare the actual values instead:
/// - both absent, or both the *same* value → cancel to `None`;
/// - exactly one present → that present value;
/// - both present but *different* → the `rhs` (target/incoming) value wins, so the switch survives.
fn xor_optional_value<T: PartialEq>(lhs: Option<T>, rhs: Option<T>) -> Option<T> {
    match (lhs, rhs) {
        (None, None) => None,
        (Some(l), None) => Some(l),
        (None, Some(r)) => Some(r),
        (Some(l), Some(r)) => {
            if l == r {
                None
            } else {
                Some(r)
            }
        }
    }
}

impl BitXor for TextStyles {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        Self {
            weight: xor_optional_value(self.weight, rhs.weight),
            italic: self.italic ^ rhs.italic,
            underline: self.underline ^ rhs.underline,
            placeholder: self.placeholder ^ rhs.placeholder,
            inline_code: self.inline_code ^ rhs.inline_code,
            strikethrough: self.strikethrough ^ rhs.strikethrough,
            link: self.link ^ rhs.link,
            colored: self.colored ^ rhs.colored,
            vertical_align: xor_optional_value(self.vertical_align, rhs.vertical_align),
        }
    }
}

impl BitXorAssign for TextStyles {
    fn bitxor_assign(&mut self, rhs: Self) {
        self.weight = xor_optional_value(self.weight, rhs.weight);
        self.vertical_align = xor_optional_value(self.vertical_align, rhs.vertical_align);
        self.italic ^= rhs.italic;
        self.underline ^= rhs.underline;
        self.placeholder ^= rhs.placeholder;
        self.inline_code ^= rhs.inline_code;
        self.strikethrough ^= rhs.strikethrough;
        self.link ^= rhs.link;
        self.colored ^= rhs.colored;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LineCount(usize);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlockCount(usize);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LinkCount(pub usize);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SyntaxColorId(pub usize);

impl_offset!(LineCount);
impl_offset!(BlockCount);
impl_offset!(LinkCount);
impl_offset!(SyntaxColorId);

#[derive(Eq, PartialEq, Clone, Copy, Debug, Default)]
pub struct BlockSummary {
    pub block: BlockCount,
}

impl AddAssign<&BlockSummary> for BlockSummary {
    fn add_assign(&mut self, other: &Self) {
        self.block += other.block;
    }
}

/// Summary of the style decorating the current cursor position. This should match the style enums in BufferTextStyle.
///
/// How it works: A range of text is decorated by a style if it is enclosed by a pair of style markers.
/// The starting marker increments the counter by 1 whereas a closing marker decrements it by 1.
/// If the current cursor position has a non-zero counter value, it means this position is decorated by the style.
#[derive(Eq, PartialEq, Clone, Copy, Debug, Default)]
pub struct StyleSummary {
    weight_counter: i32,
    // Today, we do not support nested weights. If we do, this should be a stack.
    weight: Option<CustomWeight>,
    italic_counter: i32,
    underline_counter: i32,
    inline_code_counter: i32,
    strikethrough_counter: i32,
    subscript_counter: i32,
    superscript_counter: i32,
    /// Which of subscript/superscript was most recently *opened* (never updated by a close) as of
    /// this summary's right edge, so the innermost-wins tie rule can be recovered from a purely
    /// additive, order-agnostic-looking `AddAssign`. A leaf's own summary sets this to `Some(_)` only
    /// when the leaf itself is a start marker; `AddAssign` then prefers the right-hand summary's value
    /// whenever it carries one, which is well-defined because the sum-tree cursor always folds leaf
    /// summaries left-to-right in document order (never re-associated out of order). See
    /// `resolve_vertical_align` for how this combines with the depth counters.
    last_opened_vertical_align: Option<VerticalAlign>,
    link_counter: i32,
    syntax_color_counter: i32,
    /// We need to keep track the total link marker count so we could index into a specific link marker
    /// to retrieve the url metadata.
    total_link_marker: i32,
    total_color_marker: i32,
}

impl AddAssign<&StyleSummary> for StyleSummary {
    fn add_assign(&mut self, other: &Self) {
        self.weight = CustomWeight::merge_weights(self.weight, other.weight);
        self.weight_counter += other.weight_counter;
        self.italic_counter += other.italic_counter;
        self.link_counter += other.link_counter;
        self.inline_code_counter += other.inline_code_counter;
        self.total_link_marker += other.total_link_marker;
        self.syntax_color_counter += other.syntax_color_counter;
        self.total_color_marker += other.total_color_marker;
        self.strikethrough_counter += other.strikethrough_counter;
        self.subscript_counter += other.subscript_counter;
        self.superscript_counter += other.superscript_counter;
        self.underline_counter += other.underline_counter;
        // `other` is always the right-hand (later-in-document-order) summary in a left-to-right
        // fold, so it wins whenever it recorded an open of its own.
        if other.last_opened_vertical_align.is_some() {
            self.last_opened_vertical_align = other.last_opened_vertical_align;
        }
    }
}

/// The single tie/nesting rule for overlapping subscript and superscript, shared by every consumer
/// that has to collapse the two independent markers into one `Option<VerticalAlign>`.
///
/// IMPORTANT — this function is internal editor-consistency plumbing only, and is NOT the product's
/// nesting behavior for parsed Markdown. The product rule (see `markdown_parser::parse_vertical_align`)
/// is a whole-formula literal bail: any nesting of `<sub>`/`<sup>` tags in Markdown source degrades
/// the entire outermost span to literal text *before the buffer ever sees it*, so imported/parsed
/// content never produces overlapping buffer markers in the first place — this function's overlap
/// branch is simply unreachable from that path. It's reachable only through *live interactive style
/// editing*: applying subscript over a selection that already has superscript active (or vice versa)
/// directly in the buffer, which has no equivalent "is this nested" concept and can't refuse the
/// operation the way the parser can refuse to render a nested tag. For that editor-only case, ties
/// resolve **innermost-wins** (whichever marker was applied most recently over the overlapping
/// region) purely so `StyleSummary` and the render iterator agree with each other — there is no
/// visible product behavior riding on which specific tie-break rule this picks, since it's never
/// reachable from Markdown source, but the two consumers disagreeing with each other would still be
/// a real bug (cursor/toolbar state vs. rendered runs), which is what issue #13734 finding 3
/// originally caught. `last_opened` names whichever of the two markers was most recently *opened*
/// (not closed) in document order; a close never updates it, so that once the inner marker's depth
/// returns to zero this function falls through to whichever marker is still active. Both the
/// style-summary reconstruction and the render-run iterator MUST route through this function so they
/// never disagree with each other.
pub(super) fn resolve_vertical_align(
    subscript_depth: i32,
    superscript_depth: i32,
    last_opened: Option<VerticalAlign>,
) -> Option<VerticalAlign> {
    match last_opened {
        // The innermost marker is still active: it wins outright, regardless of the other marker.
        Some(VerticalAlign::Sup) if superscript_depth > 0 => Some(VerticalAlign::Sup),
        Some(VerticalAlign::Sub) if subscript_depth > 0 => Some(VerticalAlign::Sub),
        // The innermost marker already closed (or neither marker was ever opened here): fall
        // back to whichever marker is still active. The properly-nested-markers invariant means
        // at most one of these can be true once `last_opened`'s marker has closed.
        _ if superscript_depth > 0 => Some(VerticalAlign::Sup),
        _ if subscript_depth > 0 => Some(VerticalAlign::Sub),
        _ => None,
    }
}

impl StyleSummary {
    pub(super) fn style_counter(&self, style: &BufferTextStyle) -> i32 {
        match style {
            BufferTextStyle::Weight(_) => self.weight_counter,
            BufferTextStyle::Italic => self.italic_counter,
            BufferTextStyle::Underline => self.underline_counter,
            BufferTextStyle::InlineCode => self.inline_code_counter,
            BufferTextStyle::StrikeThrough => self.strikethrough_counter,
            BufferTextStyle::Subscript => self.subscript_counter,
            BufferTextStyle::Superscript => self.superscript_counter,
        }
    }

    pub(super) fn total_link_counter(&self) -> i32 {
        self.total_link_marker
    }

    pub(super) fn syntax_link_counter(&self) -> i32 {
        self.total_color_marker
    }

    fn set_weight(&mut self, weight: Option<CustomWeight>) {
        self.weight = weight;
    }

    fn style_counter_mut(&mut self, style: &BufferTextStyle) -> &mut i32 {
        match style {
            BufferTextStyle::Weight(_) => &mut self.weight_counter,
            BufferTextStyle::Italic => &mut self.italic_counter,
            BufferTextStyle::Underline => &mut self.underline_counter,
            BufferTextStyle::InlineCode => &mut self.inline_code_counter,
            BufferTextStyle::StrikeThrough => &mut self.strikethrough_counter,
            BufferTextStyle::Subscript => &mut self.subscript_counter,
            BufferTextStyle::Superscript => &mut self.superscript_counter,
        }
    }

    /// The text-level styling in this summary.
    pub fn text_styles(&self) -> TextStyles {
        let weight = if self.weight_counter > 0 {
            self.weight
        } else {
            None
        };
        // Sub and sup are independent markers and can overlap; `resolve_vertical_align` is the shared
        // tie rule (innermost-wins) that the render iterator also uses, so the two agree.
        let vertical_align = resolve_vertical_align(
            self.subscript_counter,
            self.superscript_counter,
            self.last_opened_vertical_align,
        );
        TextStyles {
            weight,
            italic: self.italic_counter > 0,
            underline: self.underline_counter > 0,
            link: self.link_counter > 0,
            inline_code: self.inline_code_counter > 0,
            colored: self.syntax_color_counter > 0,
            strikethrough: self.strikethrough_counter > 0,
            vertical_align,
            placeholder: false,
        }
    }
}

impl From<TextStyles> for StyleSummary {
    fn from(styles: TextStyles) -> StyleSummary {
        Self {
            weight: styles.weight,
            weight_counter: styles.weight.is_some().into(),
            italic_counter: styles.italic.into(),
            underline_counter: styles.underline.into(),
            link_counter: styles.link.into(),
            inline_code_counter: styles.inline_code.into(),
            syntax_color_counter: styles.colored.into(),
            strikethrough_counter: styles.strikethrough.into(),
            subscript_counter: styles.is_subscript().into(),
            superscript_counter: styles.is_superscript().into(),
            // A `TextStyles` snapshot only ever carries one active alignment (it's a single
            // `Option<VerticalAlign>`, not independently-overlapping counters), so seeding
            // "last opened" directly from it is exact and unambiguous.
            last_opened_vertical_align: styles.vertical_align,
            total_color_marker: 0,
            total_link_marker: 0,
        }
    }
}

impl From<StyleSummary> for TextStyles {
    fn from(summary: StyleSummary) -> TextStyles {
        summary.text_styles()
    }
}

/// Summary of a fragment of buffer. It contains both the text and style metadata.
///
/// Style data is stored behind an `Option<Box<>>` so that plain-text buffers
/// (code editors) pay zero cost for style tracking — the box is only allocated
/// when the item actually carries style information (Marker, Link, Color).
#[derive(Eq, PartialEq, Clone, Debug, Default)]
pub struct BufferSummary {
    pub style: Option<Box<StyleSummary>>,
    pub text: TextSummary,
    pub block: BlockSummary,
}

impl BufferSummary {
    /// Returns the style summary, or a default (all-zero) summary if no style
    /// data is present. This is the common accessor for code that needs to
    /// read style counters.
    pub fn style_summary(&self) -> StyleSummary {
        self.style.as_deref().copied().unwrap_or_default()
    }
}

impl AddAssign<&BufferSummary> for BufferSummary {
    fn add_assign(&mut self, other: &Self) {
        match (&mut self.style, &other.style) {
            (_, None) => {} // nothing to add
            (self_style @ None, Some(other_style)) => {
                *self_style = Some(other_style.clone());
            }
            (Some(self_style), Some(other_style)) => {
                *self_style.as_mut() += other_style.as_ref();
            }
        }
        self.text += &other.text;
        self.block += &other.block;
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for BufferSummary {
    fn add_summary(&mut self, summary: &BufferSummary) {
        *self += summary;
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for TextSummary {
    fn add_summary(&mut self, summary: &BufferSummary) {
        *self += &summary.text;
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for StyleSummary {
    fn add_summary(&mut self, summary: &BufferSummary) {
        if let Some(style) = &summary.style {
            *self += style.as_ref();
        }
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for CharOffset {
    fn add_summary(&mut self, summary: &BufferSummary) {
        *self += summary.text.chars;
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for SyntaxColorId {
    fn add_summary(&mut self, summary: &BufferSummary) {
        if let Some(style) = &summary.style {
            *self += style.total_color_marker as usize;
        }
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for LineCount {
    fn add_summary(&mut self, summary: &BufferSummary) {
        *self += summary.text.lines.row as usize;
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for BlockCount {
    fn add_summary(&mut self, summary: &BufferSummary) {
        *self += summary.block.block;
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for LinkCount {
    fn add_summary(&mut self, summary: &BufferSummary) {
        if let Some(style) = &summary.style {
            *self += style.total_link_marker as usize;
        }
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for ByteOffset {
    fn add_summary(&mut self, summary: &BufferSummary) {
        *self += summary.text.bytes;
    }
}

impl sum_tree::Dimension<'_, BufferSummary> for Point {
    fn add_summary(&mut self, summary: &BufferSummary) {
        *self += summary.text.lines;
    }
}

impl sum_tree::Item for BufferText {
    type Summary = BufferSummary;

    fn summary(&self) -> Self::Summary {
        let text_summary = match &self {
            BufferText::Newline => TextSummary {
                chars: 1.into(),
                bytes: 1.into(),
                lines: Point::new(1, 0),
                first_line_len: 0,
                rightmost_point: Point::new(0, 0),
            },
            BufferText::BlockItem { item_type } => {
                let chars = item_type.content_length();
                let lines = item_type.line_count();
                TextSummary {
                    chars: chars.into(),
                    bytes: chars.into(),
                    lines: Point::new(lines, 0),
                    first_line_len: 0,
                    rightmost_point: Point::new(lines.saturating_sub(1), 0),
                }
            }
            BufferText::Text {
                fragment,
                char_count,
            } => TextSummary {
                chars: (*char_count as usize).into(),
                bytes: fragment.len().into(),
                lines: Point::new(0, (*char_count).into()),
                first_line_len: (*char_count).into(),
                rightmost_point: Point::new(0, (*char_count).into()),
            },
            BufferText::Marker { .. } | BufferText::Link(_) | BufferText::Color(_) => TextSummary {
                chars: 0.into(),
                bytes: 0.into(),
                lines: Point::new(0, 0),
                first_line_len: 0,
                rightmost_point: Point::new(0, 0),
            },
            BufferText::BlockMarker { .. } => TextSummary {
                chars: 1.into(),
                bytes: 1.into(),
                lines: Point::new(1, 0),
                first_line_len: 0,
                rightmost_point: Point::new(0, 0),
            },
            // Placeholders count as single special characters. This lets us distinguish between
            // the cursor being at a placeholder and at the character just after it.
            BufferText::Placeholder { .. } => TextSummary {
                chars: 1.into(),
                bytes: 1.into(),
                lines: Point::new(0, 1),
                first_line_len: 1,
                rightmost_point: Point::new(0, 1),
            },
        };

        // Only allocate a StyleSummary when the item actually carries style
        // data. For plain-text buffers (code editors) this is never the case,
        // so style is always None — zero heap allocations.
        let style_summary = match self {
            BufferText::Marker { marker_type, dir } => {
                let mut s = StyleSummary::default();
                let delta = dir.to_counter_delta();
                if let BufferTextStyle::Weight(weight) = marker_type {
                    s.set_weight(Some(*weight));
                }
                *s.style_counter_mut(marker_type) += delta;
                // Only a *start* marker records an "open" for the innermost-wins tie rule; a close
                // leaves `last_opened_vertical_align` as `None` here so `AddAssign` never lets a
                // close override a still-active outer marker recorded in an earlier chunk (see
                // `resolve_vertical_align`).
                if matches!(dir, MarkerDir::Start) {
                    s.last_opened_vertical_align = match marker_type {
                        BufferTextStyle::Subscript => Some(VerticalAlign::Sub),
                        BufferTextStyle::Superscript => Some(VerticalAlign::Sup),
                        _ => None,
                    };
                }
                Some(Box::new(s))
            }
            BufferText::Color(marker) => {
                let mut s = StyleSummary::default();
                let delta = marker.to_counter_delta();
                s.syntax_color_counter += delta;
                s.total_color_marker += 1;
                Some(Box::new(s))
            }
            BufferText::Link(marker) => {
                let mut s = StyleSummary::default();
                let delta = marker.to_counter_delta();
                s.link_counter += delta;
                s.total_link_marker += 1;
                Some(Box::new(s))
            }
            _ => None,
        };

        let block_summary = match &self {
            BufferText::BlockMarker { .. } | BufferText::BlockItem { .. } => {
                BlockSummary { block: 1.into() }
            }
            _ => Default::default(),
        };

        BufferSummary {
            style: style_summary,
            text: text_summary,
            block: block_summary,
        }
    }
}
