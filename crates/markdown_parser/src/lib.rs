use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::fmt::Debug;
use std::ops::Range;
use std::sync::Arc;

pub mod html_parser;
pub mod markdown_parser;
pub mod weight;
pub use html_parser::parse_html;
use itertools::Itertools;
pub use markdown_parser::{
    parse_image_prefix, parse_image_run_line, parse_inline_markdown, parse_markdown,
    parse_markdown_with_gfm_tables,
};
use serde_yaml::Mapping;
use weight::CustomWeight;

/// Trait for an "action" that can be dispatched via a hyperlink click handler.
/// This purposefully shadows the `Action` trait from `warpui`.
///
/// Since `warpui` depends on this crate, we can't depend on the `warpui_core::Action` trait directly.
/// Instead, we create a new trait with a blanket implementation that implicitly results
/// in any `warpui_core::Action` implementing this `Action`.
pub trait Action: Any + Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

impl<T> Action for T
where
    T: Any + Debug + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub trait LineCount {
    fn num_lines(&self) -> usize;
}

/// A simple line-based delta between two [`FormattedText`] values.
///
/// `common_prefix_lines` is the number of leading lines that are identical
/// between the old and new formatted text. `new_suffix`
/// contains the replacement lines from the new value to replace from after common_prefix_lines
/// to the end of the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedTextDelta {
    /// The number of actual lines in the common prefix (corresponding to a row in Point)
    /// Note that a FormattedTextLine can have multiple lines - this refers to the actual line count,
    /// not the FormattedTextLine count.
    pub common_prefix_lines: usize,
    /// The number of existing formatted text lines to be replaced
    pub old_suffix_formatted_text_lines: usize,
    pub new_suffix: VecDeque<FormattedTextLine>,
}

impl FormattedTextDelta {
    pub fn is_noop(&self) -> bool {
        self.old_suffix_formatted_text_lines == 0 && self.new_suffix.is_empty()
    }
}

pub fn compute_formatted_text_delta(old: FormattedText, new: FormattedText) -> FormattedTextDelta {
    let mut common_prefix_formatted_text_lines = 0usize;
    let mut common_prefix_lines = 0usize;
    let old_len = old.lines.len();
    let new_len = new.lines.len();
    let shared_len = old_len.min(new_len);

    while common_prefix_formatted_text_lines < shared_len {
        let old_line = &old.lines[common_prefix_formatted_text_lines];
        let new_line = &new.lines[common_prefix_formatted_text_lines];

        // Special handling for code blocks: only compare the code, not the language
        // This is because the lang string in our internal buffer representation may not match
        // the lang string in the parsed markdown exactly (e.g. "Python" vs "python path=/path/to/file.py start=1")
        let lines_equal = match (old_line, new_line) {
            (FormattedTextLine::CodeBlock(old_block), FormattedTextLine::CodeBlock(new_block)) => {
                old_block.code == new_block.code
            }
            _ => old_line == new_line,
        };

        if !lines_equal {
            break;
        }

        common_prefix_formatted_text_lines += 1;
        common_prefix_lines += old_line.num_lines();
    }

    let old_suffix_formatted_text_lines =
        old_len.saturating_sub(common_prefix_formatted_text_lines);
    let new_suffix = new
        .lines
        .iter()
        .skip(common_prefix_formatted_text_lines)
        .cloned()
        .collect();

    FormattedTextDelta {
        common_prefix_lines,
        old_suffix_formatted_text_lines,
        new_suffix,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedText {
    pub lines: VecDeque<FormattedTextLine>,
}

impl FormattedText {
    pub fn new(lines: impl Into<VecDeque<FormattedTextLine>>) -> Self {
        Self {
            lines: lines.into(),
        }
    }

    /// Creates a new FormattedText where the first and last line breaks are removed, if any.
    pub fn new_trimmed(lines: impl Into<VecDeque<FormattedTextLine>>) -> Self {
        let mut new = Self::new(lines);
        new.trim();
        new
    }

    fn trim(&mut self) {
        // Since we exhaust contiguous new lines into a single line break,
        // there won't be multiple contiguous line breaks; there's at most one to remove.
        if let Some(FormattedTextLine::LineBreak) = self.lines.front() {
            self.lines.pop_front();
        }

        // Similarly for the end.
        if let Some(FormattedTextLine::LineBreak) = self.lines.back() {
            self.lines.pop_back();
        }
    }

    /// Returns the raw text of the markdown, without any of the markdown
    /// markers.
    pub fn raw_text(&self) -> String {
        self.lines.iter().map(|line| line.raw_text()).join("")
    }

    pub fn append_line(mut self, line: FormattedTextLine) -> Self {
        self.lines.push_back(line);
        self
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FormattedTextLine {
    Heading(FormattedTextHeader),
    Line(FormattedTextInline),
    OrderedList(OrderedFormattedIndentTextInline),
    UnorderedList(FormattedIndentTextInline),
    CodeBlock(CodeBlockText),
    TaskList(FormattedTaskList),
    LineBreak,
    HorizontalRule,
    Embedded(Mapping),
    Image(FormattedImage),
    Table(FormattedTable),
}

impl FormattedTextLine {
    pub fn raw_text(&self) -> String {
        let mut text = match self {
            Self::CodeBlock(text) => text.code.clone(),
            Self::Heading(header) => header
                .text
                .iter()
                .map(|fragment| fragment.raw_text())
                .join(""),
            Self::Line(line) => line.iter().map(|fragment| fragment.raw_text()).join(""),
            Self::TaskList(line) => line
                .text
                .iter()
                .map(|fragment| fragment.raw_text())
                .join(""),
            Self::OrderedList(list) => list
                .indented_text
                .text
                .iter()
                .map(|fragment| fragment.raw_text())
                .join(""),
            Self::UnorderedList(list) => list
                .text
                .iter()
                .map(|fragment| fragment.raw_text())
                .join(""),
            Self::LineBreak | Self::HorizontalRule | Self::Embedded(_) => "\n".to_string(),
            Self::Image(image) => format!("{}\n", image.alt_text),
            Self::Table(table) => table.to_internal_format(),
        };
        // Each `FormattedTextLine` unit represents a complete line. If it doesn't already end in
        // a newline, add one.
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    pub fn set_weight(&mut self, weight: Option<CustomWeight>) -> &Self {
        match self {
            Self::Heading(header) => {
                for fragment in &mut header.text {
                    fragment.styles.weight = weight;
                }
            }
            Self::Line(line) => {
                for fragment in line {
                    fragment.styles.weight = weight;
                }
            }
            Self::OrderedList(list) => {
                for fragment in &mut list.indented_text.text {
                    fragment.styles.weight = weight;
                }
            }
            Self::UnorderedList(list) => {
                for fragment in &mut list.text {
                    fragment.styles.weight = weight;
                }
            }
            Self::TaskList(list) => {
                for fragment in &mut list.text {
                    fragment.styles.weight = weight;
                }
            }
            Self::Table(_)
            | Self::CodeBlock(_)
            | Self::LineBreak
            | Self::HorizontalRule
            | Self::Embedded(_)
            | Self::Image(_) => {}
        }
        self
    }

    fn inline_fragments(&self) -> Option<&FormattedTextInline> {
        match &self {
            FormattedTextLine::Heading(header) => Some(&header.text),
            FormattedTextLine::Line(texts) => Some(texts),
            FormattedTextLine::OrderedList(texts) => Some(&texts.indented_text.text),
            FormattedTextLine::UnorderedList(texts) => Some(&texts.text),
            FormattedTextLine::TaskList(list) => Some(&list.text),
            FormattedTextLine::CodeBlock(_)
            | FormattedTextLine::LineBreak
            | FormattedTextLine::HorizontalRule
            | FormattedTextLine::Embedded(_)
            | FormattedTextLine::Image(_)
            | FormattedTextLine::Table(_) => None,
        }
    }

    pub fn hyperlinks(&self, skip_raw_links: bool) -> Vec<(Range<usize>, Hyperlink)> {
        let mut hyperlinks: Vec<(Range<usize>, Hyperlink)> = Vec::new();
        if let Some(inline_fragments) = self.inline_fragments() {
            let mut char_count = 0;
            for fragment in inline_fragments {
                let range_start = char_count;
                char_count += fragment.text.chars().count();
                if let Some(link) = &fragment.styles.hyperlink
                    && (!skip_raw_links
                        || !matches!(&link, Hyperlink::Url(url) if url == &fragment.text))
                {
                    hyperlinks.push((range_start..char_count, link.clone()));
                }
            }
        }
        hyperlinks
    }

    pub fn is_empty_line(&self) -> bool {
        matches!(self, Self::Line(line) if line.iter().all(|fragment| fragment.text.is_empty()))
    }
}

impl LineCount for FormattedTextLine {
    fn num_lines(&self) -> usize {
        match self {
            Self::CodeBlock(text) => text.code.matches('\n').count(),
            Self::Heading(_) => 1,
            Self::Line(_) => 1,
            Self::OrderedList(_) => 1,
            Self::UnorderedList(_) => 1,
            Self::TaskList(_) => 1,
            Self::LineBreak => 0,
            Self::HorizontalRule => 0,
            Self::Embedded(_) => 1,
            Self::Image(_) => 1,
            Self::Table(table) => 1 + table.rows.len(), // Header + data rows (separator not counted as a line)
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FormattedTextHeader {
    pub heading_size: usize,
    pub text: FormattedTextInline,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FormattedTaskList {
    pub complete: bool,
    pub indent_level: usize,
    pub text: FormattedTextInline,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FormattedIndentTextInline {
    pub indent_level: usize,
    pub text: FormattedTextInline,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CodeBlockText {
    pub lang: String,
    pub code: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrderedFormattedIndentTextInline {
    /// The number of this item, which may be `None` if it was unspecified or invalid in the source
    /// document.
    pub number: Option<usize>,
    pub indented_text: FormattedIndentTextInline,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FormattedImage {
    pub alt_text: String,
    pub source: String,
    /// Optional CommonMark image title, e.g. the `title` in `![alt](src "title")`.
    /// Empty titles are normalized to `None` by the parser.
    pub title: Option<String>,
}

/// Column alignment for table cells
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default, Hash)]
pub enum TableAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// A formatted table with headers, alignments, and rows
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FormattedTable {
    pub headers: Vec<FormattedTextInline>,
    pub alignments: Vec<TableAlignment>,
    pub rows: Vec<Vec<FormattedTextInline>>,
}

impl FormattedTable {
    /// Parse from the internal tab-separated format used in `warp-markdown-table` code blocks.
    pub fn from_internal_format(content: &str) -> Self {
        let parse_line = |line: &str| -> Vec<FormattedTextInline> {
            line.split('\t')
                .map(|cell| vec![FormattedTextFragment::plain_text(cell)])
                .collect()
        };

        let mut lines = content.lines().peekable();
        let headers = lines.next().map(parse_line).unwrap_or_default();
        let rows: Vec<Vec<FormattedTextInline>> = lines.map(parse_line).collect();
        let col_count = headers.len();

        Self {
            headers,
            alignments: vec![TableAlignment::default(); col_count],
            rows,
        }
    }

    pub fn from_internal_format_with_alignments(
        content: &str,
        mut alignments: Vec<TableAlignment>,
    ) -> Self {
        let mut table = Self::from_internal_format(content);
        let col_count = table.headers.len();
        alignments.resize(col_count, TableAlignment::default());
        alignments.truncate(col_count);
        table.alignments = alignments;
        table
    }

    /// Serialize to the internal tab-separated format used in `warp-markdown-table` code blocks.
    /// Inline formatting is preserved as markdown syntax so it survives the buffer round-trip.
    pub fn to_internal_format(&self) -> String {
        if self.headers.is_empty() && self.rows.is_empty() {
            return String::new();
        }

        let mut result = String::new();
        let headers: Vec<String> = self.headers.iter().map(inline_to_markdown).collect();
        result.push_str(&headers.join("\t"));
        result.push('\n');
        for row in &self.rows {
            let cells: Vec<String> = row.iter().map(inline_to_markdown).collect();
            result.push_str(&cells.join("\t"));
            result.push('\n');
        }
        result
    }

    /// Pad ragged rows/headers to a uniform column count.
    pub fn normalize_shape(&mut self) {
        let mut column_count = self
            .headers
            .len()
            .max(self.rows.iter().map(Vec::len).max().unwrap_or(0));
        if column_count == 0 {
            column_count = 1;
        }

        self.headers.resize_with(column_count, Vec::new);
        self.alignments
            .resize(column_count, TableAlignment::default());
        for row in &mut self.rows {
            row.resize_with(column_count, Vec::new);
        }
    }

    /// Serialize to GFM pipe-table markdown.
    pub fn to_plain_text(&self) -> String {
        fn inline_to_text(inline: &FormattedTextInline) -> String {
            inline.iter().map(|f| f.text.as_str()).collect()
        }

        let mut lines = Vec::new();
        let headers: Vec<String> = self.headers.iter().map(inline_to_text).collect();
        lines.push(format!("| {} |", headers.join(" | ")));
        let separator: Vec<String> = self
            .alignments
            .iter()
            .map(|alignment| match alignment {
                TableAlignment::Left => "---".to_string(),
                TableAlignment::Center => ":---:".to_string(),
                TableAlignment::Right => "---:".to_string(),
            })
            .collect();
        lines.push(format!("| {} |", separator.join(" | ")));
        for row in &self.rows {
            let cells: Vec<String> = row.iter().map(inline_to_text).collect();
            lines.push(format!("| {} |", cells.join(" | ")));
        }
        lines.join("\n")
    }
}

/// Convert a `FormattedTextInline` back to markdown syntax.
fn inline_to_markdown(inline: &FormattedTextInline) -> String {
    let mut result = String::new();
    for fragment in inline {
        let mut text = fragment.text.clone();
        if text.is_empty() {
            continue;
        }

        if fragment.styles.inline_code {
            result.push('`');
            result.push_str(&text);
            result.push('`');
            continue;
        }

        if let Some(Hyperlink::Url(url)) = &fragment.styles.hyperlink {
            text = format!("[{text}]({url})");
        }
        if fragment.styles.strikethrough {
            text = format!("~~{text}~~");
        }
        if fragment.styles.underline {
            text = format!("<u>{text}</u>");
        }
        let is_bold = fragment
            .styles
            .weight
            .is_some_and(|w| matches!(w, CustomWeight::Bold));
        if is_bold && fragment.styles.italic {
            text = format!("***{text}***");
        } else if is_bold {
            text = format!("**{text}**");
        } else if fragment.styles.italic {
            text = format!("*{text}*");
        }

        result.push_str(&text);
    }
    result
}
pub type FormattedTableAlignment = TableAlignment;

pub type FormattedTextInline = Vec<FormattedTextFragment>;

/// A fragment of formatted text, containing the text itself and formatting flags/metadata.
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct FormattedTextFragment {
    pub text: String,
    pub styles: FormattedTextStyles,
}

#[derive(Debug, Clone)]
pub enum Hyperlink {
    Url(String),
    Action(Arc<dyn Action>),
}

impl Hyperlink {
    /// Returns the URL if this is a URL, or `None` otherwise.
    pub fn url(self) -> Option<String> {
        match self {
            Hyperlink::Url(url) => Some(url),
            Hyperlink::Action(_) => None,
        }
    }
}

impl PartialEq for Hyperlink {
    // Stub implementation for [`Hyperlink`] that only compares URLs and not Actions.
    // This is an unfortunate byproduct of the fact that an [`Action`] does not implement [`PartialEq`]
    // but we require [`PartialEq`] to consolidate [`FormattedTextStyles`].
    // To get around this, we only compare URLs, which works for style consolidation since this is only
    // needed when generating formatted text from markdown, which provably does not support URLs that dispatch
    // actions
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Url(left), Self::Url(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for Hyperlink {}

/// Formatted text styling, with no attached content.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct FormattedTextStyles {
    pub weight: Option<CustomWeight>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub inline_code: bool,
    pub hyperlink: Option<Hyperlink>,
}

impl FormattedTextFragment {
    pub fn plain_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            styles: Default::default(),
        }
    }

    pub fn weighted(text: impl Into<String>, weight: Option<CustomWeight>) -> Self {
        Self {
            text: text.into(),
            styles: FormattedTextStyles {
                weight,
                ..Default::default()
            },
        }
    }

    pub fn with_weight(&mut self, weight: Option<CustomWeight>) -> &Self {
        self.styles.weight = weight;
        self
    }

    pub fn bold(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            styles: FormattedTextStyles {
                weight: Some(CustomWeight::Bold),
                ..Default::default()
            },
        }
    }

    pub fn italic(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            styles: FormattedTextStyles {
                italic: true,
                ..Default::default()
            },
        }
    }

    pub fn bold_italic(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            styles: FormattedTextStyles {
                weight: Some(CustomWeight::Bold),
                italic: true,
                ..Default::default()
            },
        }
    }

    pub fn hyperlink(tag: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: tag.into(),
            styles: FormattedTextStyles {
                hyperlink: Some(Hyperlink::Url(url.into())),
                ..Default::default()
            },
        }
    }

    /// Constructs a new hyperlink that dispatches an action when clicked.
    pub fn hyperlink_action<A: Action>(tag: impl Into<String>, action: A) -> Self {
        Self {
            text: tag.into(),
            styles: FormattedTextStyles {
                hyperlink: Some(Hyperlink::Action(Arc::new(action))),
                ..Default::default()
            },
        }
    }

    pub fn inline_code(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            styles: FormattedTextStyles {
                inline_code: true,
                ..Default::default()
            },
        }
    }

    pub fn strikethrough(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            styles: FormattedTextStyles {
                strikethrough: true,
                ..Default::default()
            },
        }
    }

    pub fn underline(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            styles: FormattedTextStyles {
                underline: true,
                ..Default::default()
            },
        }
    }

    pub fn raw_text(&self) -> &String {
        &self.text
    }
}

impl fmt::Debug for FormattedTextStyles {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // For readability, only show active styles.
        let mut first = true;

        if let Some(weight) = self.weight {
            if !first {
                f.write_str(" | ")?;
            }
            write!(f, "{weight:?}")?;
            first = false;
        }

        if self.italic {
            if !first {
                f.write_str(" | ")?;
            }
            f.write_str("Italic")?;
            first = false;
        }

        if self.strikethrough {
            if !first {
                f.write_str(" | ")?;
            }
            f.write_str("Strikethrough")?;
            first = false;
        }

        if self.inline_code {
            if !first {
                f.write_str(" | ")?;
            }
            f.write_str("InlineCode")?;
            first = false;
        }

        if let Some(link) = &self.hyperlink {
            if !first {
                f.write_str(" | ")?;
            }

            write!(f, "Hyperlink({link:?})")?;
            first = false;
        }

        if first {
            // No styles are active, so this is plain text.
            f.write_str("PlainText")?;
        }

        Ok(())
    }
}
