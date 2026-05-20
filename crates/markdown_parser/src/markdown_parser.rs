use anyhow::Result;
use itertools::Itertools;
use nom::{
    FindToken, IResult, InputIter, InputLength, Parser, Slice,
    branch::alt,
    bytes::complete::{
        is_a, tag, tag_no_case, take, take_till1, take_until, take_while, take_while_m_n,
        take_while1,
    },
    character::{
        complete::{char, one_of, satisfy, space0, space1},
        is_digit,
    },
    combinator::{
        all_consuming, consumed, eof, fail, flat_map, map, map_parser, recognize, value, verify,
    },
    error::{ContextError, ErrorKind, ParseError, context, make_error},
    multi::{fold_many_m_n, fold_many1, many_m_n, many0},
    sequence::{delimited, pair, preceded, terminated, tuple},
};
use serde_yaml::Value;
use std::cell::RefCell;

use crate::{
    CodeBlockText, FormattedImage, FormattedIndentTextInline, FormattedTable, FormattedTaskList,
    FormattedText, FormattedTextFragment, FormattedTextHeader, FormattedTextInline,
    FormattedTextLine, Hyperlink, OrderedFormattedIndentTextInline, TableAlignment,
};
use crate::{CustomWeight, FormattedTextStyles};

const HEADER_TAG_MIN_COUNT: usize = 1;
const HEADER_TAG_MAX_COUNT: usize = 6;

const HORIZONTALRULE_TAG_MIN_COUNT: usize = 3;

pub const INDENT_MAX_LEVEL: usize = 5;
pub const NUM_SPACE_PER_INDENT_LEVEL: usize = 4;

pub const EMBED_BLOCK_MARKDOWN_LANG: &str = "warp-embedded-object";

pub const RUNNABLE_BLOCK_MARKDOWN_LANG: &str = "warp-runnable-command";
pub const CODE_BLOCK_DEFAULT_MARKDOWN_LANG: &str = "text";
pub const TABLE_BLOCK_MARKDOWN_LANG: &str = "warp-markdown-table";

const INDENT_TAG_MIN_COUNT: usize = 0;
const INDENT_TAG_MAX_COUNT: usize = INDENT_MAX_LEVEL * NUM_SPACE_PER_INDENT_LEVEL;

/// Formatting delimiter characters used for emphasis/strikethrough in Markdown.
/// These are stripped from trailing URLs and used to detect valid autolink boundaries.
const FORMATTING_DELIMITERS: &str = "*_~";

/// Tracks indentation context during list parsing to enable relative indentation calculation.
#[derive(Debug, Clone)]
struct ListIndentationContext {
    /// Stack of (space_count, indent_level) for lines we've seen
    /// This allows us to find the most recent line with fewer spaces than this line
    indentation_stack: Vec<(usize, usize)>,
}

impl ListIndentationContext {
    fn new() -> Self {
        Self {
            indentation_stack: Vec::new(),
        }
    }

    /// Clear the indentation context (reset to initial state)
    fn clear(&mut self) {
        self.indentation_stack.clear();
    }

    /// Calculate indent level using proper relative indentation logic:
    /// We are indented if we are >= 2 spaces further indented relative to the most recent line that does not have more spaces.
    fn get_and_register_indent_level(&mut self, space_count: usize) -> usize {
        // Pop off entries with more spaces than current until we find the most recent line
        // with equal or fewer spaces
        while let Some(&(spaces, _)) = self.indentation_stack.last() {
            if spaces <= space_count {
                break;
            }
            self.indentation_stack.pop();
        }

        let (reference_space_count, reference_indentation_level) = self
            .indentation_stack
            .last()
            .copied()
            .unwrap_or((space_count, 0));

        let space_difference = space_count - reference_space_count;
        // >= 2 spaces should indent one level
        // TODO: most markdown parsers stop treating this as a list item after some number of spaces > 4
        let new_level = if space_difference >= 2 {
            reference_indentation_level + 1
        } else {
            reference_indentation_level
        };

        // Add current line to stack
        self.indentation_stack.push((space_count, new_level));

        new_level
    }
}

pub fn parse_markdown(markdown: &str) -> Result<FormattedText> {
    parse_markdown_impl(markdown, false)
}

pub fn parse_markdown_with_gfm_tables(markdown: &str) -> Result<FormattedText> {
    parse_markdown_impl(markdown, true)
}

fn parse_markdown_impl(markdown: &str, parse_gfm_tables: bool) -> Result<FormattedText> {
    parse_markdown_internal::<'_, nom::error::Error<_>>(markdown, parse_gfm_tables)
        .map(|(_, mut res)| {
            if let Some(FormattedTextLine::LineBreak) = res.last() {
                res.pop();
            }
            FormattedText { lines: res.into() }
        })
        .map_err(|err| {
            if cfg!(debug_assertions) {
                anyhow::anyhow!("Failed to parse Markdown: {err}")
            } else {
                anyhow::anyhow!("Failed to parse Markdown")
            }
        })
}

pub fn parse_markdown_to_raw_text(markdown: &str) -> Result<String> {
    let formatted_text = parse_markdown(markdown)?;
    Ok(formatted_text.raw_text())
}

fn parse_markdown_internal<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
    parse_gfm_tables: bool,
) -> IResult<&'a str, Vec<FormattedTextLine>, E> {
    let indentation_context = RefCell::new(ListIndentationContext::new());

    let mut block = context(
        "block",
        alt((
            parse_blank_line,
            parse_horizontal_rule,
            map(parse_code_block, |(lang, content)| {
                if lang == EMBED_BLOCK_MARKDOWN_LANG
                    && let Ok(Value::Mapping(mapping)) = serde_yaml::from_str(&content)
                {
                    return FormattedTextLine::Embedded(mapping);
                }
                if lang == TABLE_BLOCK_MARKDOWN_LANG {
                    return FormattedTextLine::Table(FormattedTable::from_internal_format(
                        &content,
                    ));
                }

                FormattedTextLine::CodeBlock(CodeBlockText {
                    lang: lang.to_string(),
                    code: content,
                })
            }),
            map(parse_header, FormattedTextLine::Heading),
            map(parse_image, FormattedTextLine::Image),
            |i| {
                parse_task_list(i, &indentation_context)
                    .map(|(s, t)| (s, FormattedTextLine::TaskList(t)))
            },
            |i| {
                parse_ordered_list(i, &indentation_context)
                    .map(|(s, o)| (s, FormattedTextLine::OrderedList(o)))
            },
            |i| {
                parse_unordered_list(i, &indentation_context)
                    .map(|(s, u)| (s, FormattedTextLine::UnorderedList(u)))
            },
            |i| {
                if !parse_gfm_tables {
                    return Err(nom::Err::Error(E::from_error_kind(i, ErrorKind::Alt)));
                }
                map(parse_table, FormattedTextLine::Table)(i)
            },
            parse_paragraph,
        )),
    );

    let mut remaining = markdown;
    let mut lines = Vec::new();
    while !remaining.is_empty() {
        let (remaining_after_block, mut line) = block(remaining)?;
        remaining = remaining_after_block;

        // Clear indentation context for non-list content and handle ordered list numbering
        match &mut line {
            FormattedTextLine::LineBreak => {
                // Line breaks don't reset context
            }
            FormattedTextLine::UnorderedList(_) | FormattedTextLine::TaskList(_) => {
                // List items already update indentation context during parsing
            }
            FormattedTextLine::OrderedList(list_item) => {
                // For ordered lists, only the starting item's number is applied. We reset the number for
                // subsequent items here because, in isolation, we don't know if a given list item starts a
                // list or not.
                if let Some(FormattedTextLine::OrderedList(prev_list_item)) = lines.last()
                    && prev_list_item.indented_text.indent_level
                        >= list_item.indented_text.indent_level
                {
                    list_item.number = None;
                }
            }
            _ => {
                // Non-list content resets indentation context
                indentation_context.borrow_mut().clear();
            }
        }
        lines.push(line);
    }

    Ok((remaining, lines))
}

/// Parse a single paragraph of Markdown text.
fn parse_paragraph<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, FormattedTextLine, E> {
    context(
        "paragraph",
        map(parse_markdown_line, FormattedTextLine::Line),
    )(markdown)
}

/// Parse a blank line.
fn parse_blank_line<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, FormattedTextLine, E> {
    context(
        "blank_line",
        value(
            FormattedTextLine::LineBreak,
            pair(space0, parse_line_ending),
        ),
    )(markdown)
}

/// Parse a horizontal rule.
fn parse_horizontal_rule<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, FormattedTextLine, E> {
    context(
        "horizontal_rule",
        value(
            FormattedTextLine::HorizontalRule,
            delimited(
                parse_block_leading_spaces,
                alt((
                    many_m_n(
                        HORIZONTALRULE_TAG_MIN_COUNT,
                        usize::MAX,
                        parse_horizontal_rule_asterisk,
                    ),
                    many_m_n(
                        HORIZONTALRULE_TAG_MIN_COUNT,
                        usize::MAX,
                        parse_horizontal_rule_dash,
                    ),
                    many_m_n(
                        HORIZONTALRULE_TAG_MIN_COUNT,
                        usize::MAX,
                        parse_horizontal_rule_underline,
                    ),
                )),
                parse_line_ending,
            ),
        ),
    )(markdown)
}

/// Parse to the end of the line or input as inline Markdown.
fn parse_markdown_line<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, Vec<FormattedTextFragment>, E> {
    map_parser(parse_line, all_consuming(parse_inline))(markdown)
}

fn parse_header<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, FormattedTextHeader, E> {
    context(
        "header",
        map(
            pair(parse_header_tag, parse_markdown_line),
            |(heading_size, text)| FormattedTextHeader { heading_size, text },
        ),
    )(markdown)
}

/// Parse markdown image syntax: `![alt text](source)` or `![alt text](source "title")`.
///
/// Images must be on their own line (optionally with leading whitespace).
/// Inline images (e.g., `text ![img](path) more text`) are not supported and
/// will be rendered as plain text, providing graceful degradation.
///
/// The title may be delimited by `".."`, `'..'`, or `(..)` per CommonMark.
fn parse_image<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, FormattedImage, E> {
    context("image", |input| {
        let (input, _) = parse_block_leading_spaces(input)?;
        let (input, image) = parse_image_prefix_internal(input)?;
        let (input, _) = alt((value((), parse_line_ending), value((), eof)))(input)?;
        Ok((input, image))
    })(markdown)
}
/// Parse a line consisting entirely of one or more Markdown images separated
/// by whitespace.
///
/// Returns `None` for mixed-content lines such as `text ![img](path)`.
pub fn parse_image_run_line(line: &str) -> Option<Vec<FormattedImage>> {
    let mut remaining = line.trim_start();
    let mut images = Vec::new();

    loop {
        let (rest, image) = parse_image_prefix(remaining)?;
        images.push(image);

        if rest.trim().is_empty() {
            return Some(images);
        }

        let next = rest.trim_start();
        if next.len() == rest.len() {
            return None;
        }
        remaining = next;
    }
}

pub fn parse_image_prefix(input: &str) -> Option<(&str, FormattedImage)> {
    parse_image_prefix_internal::<nom::error::Error<&str>>(input).ok()
}

fn parse_image_prefix_internal<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, FormattedImage, E> {
    let (input, _) = tag("![")(input)?;
    let (input, alt_text) = take_until("]")(input)?;
    let (input, _) = tag("]")(input)?;
    let (input, (source, title)) = parse_image_target(input)?;
    let title = title.filter(|t| !t.is_empty());
    Ok((
        input,
        FormattedImage {
            alt_text: alt_text.to_string(),
            source,
            title,
        },
    ))
}

/// Parse an image's `(source [title])` target, including an optional
/// CommonMark title.
///
/// This is intentionally image-specific so that the link parser
/// (`parse_link_target`) keeps its current behavior. CommonMark link titles
/// are explicitly out of scope for this change.
fn parse_image_target<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, (String, Option<String>), E> {
    let (input, _) = tag("(")(input)?;
    let (input, _) = space0(input)?;
    let (input, source) = parse_image_destination(input)?;
    let (input, saw_whitespace) = parse_image_title_leading_whitespace(input)?;
    let (input, title) = if saw_whitespace {
        match parse_image_title::<E>(input) {
            Ok((rest, title)) => {
                let (rest, _) = space0(rest)?;
                (rest, Some(title))
            }
            Err(_) => (input, None),
        }
    } else {
        (input, None)
    };
    let (input, _) = tag(")")(input)?;
    Ok((input, (source, title)))
}

/// Parse the destination portion of an image target, stopping before any
/// title or closing `)`. Supports angle-bracket delimited destinations
/// (`<...>`) as well as the non-delimited form with balanced parens.
fn parse_image_destination<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, String, E> {
    let (in_brackets, mut remaining) = match tag::<_, _, ()>("<")(input) {
        Ok((rest, _)) => (true, rest),
        Err(_) => (false, input),
    };

    let mut target = String::new();
    let mut paren_depth = 0usize;

    loop {
        // Detect line endings as a hard error; destinations never span lines.
        if let Ok((_, _)) = parse_line_ending::<()>(remaining) {
            return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag)));
        }
        if remaining.is_empty() {
            return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag)));
        }

        // An escape is always valid inside a destination.
        if let Ok((rest, ch)) = parse_escape::<()>(remaining) {
            target.push(ch);
            remaining = rest;
            continue;
        }

        let mut chars = remaining.chars();
        let ch = match chars.next() {
            Some(ch) => ch,
            None => return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag))),
        };
        let ch_len = ch.len_utf8();

        match ch {
            '>' if in_brackets => {
                return Ok((&remaining[ch_len..], target));
            }
            '<' if in_brackets => {
                return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag)));
            }
            _ if in_brackets => {
                target.push(ch);
                remaining = &remaining[ch_len..];
            }
            c if c.is_whitespace() => {
                // Non-bracket destinations end at the first whitespace.
                return Ok((remaining, target));
            }
            '(' => {
                paren_depth += 1;
                target.push('(');
                remaining = &remaining[ch_len..];
            }
            ')' => {
                if paren_depth == 0 {
                    // An unbalanced `)` ends the destination.
                    return Ok((remaining, target));
                }
                paren_depth -= 1;
                target.push(')');
                remaining = &remaining[ch_len..];
            }
            c if c.is_control() => {
                return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag)));
            }
            c => {
                target.push(c);
                remaining = &remaining[ch_len..];
            }
        }
    }
}

/// Consume the whitespace that may separate a destination from an optional
/// title, returning `true` iff any whitespace was consumed. CommonMark
/// requires at least one whitespace character between destination and title.
fn parse_image_title_leading_whitespace<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, bool, E> {
    match space1::<_, ()>(input) {
        Ok((rest, _)) => Ok((rest, true)),
        Err(_) => Ok((input, false)),
    }
}

/// Parse a CommonMark image title with one of `".."`, `'..'`, or `(..)`.
/// The matching closing delimiter may be escaped with a backslash.
/// Titles that cross a line ending before closing cause the whole image to
/// fall back to plain text (invariant 5).
fn parse_image_title<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, String, E> {
    let mut chars = input.chars();
    let opening = chars
        .next()
        .ok_or_else(|| nom::Err::Error(make_error(input, ErrorKind::Tag)))?;
    let closing = match opening {
        '"' => '"',
        '\'' => '\'',
        '(' => ')',
        _ => return Err(nom::Err::Error(make_error(input, ErrorKind::Tag))),
    };

    let mut remaining = &input[opening.len_utf8()..];
    let mut title = String::new();

    loop {
        if remaining.is_empty() {
            return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag)));
        }

        // Reject titles that span a line ending without closing first.
        if let Ok((_, _)) = parse_line_ending::<()>(remaining) {
            return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag)));
        }

        if let Ok((rest, ch)) = parse_escape::<()>(remaining) {
            title.push(ch);
            remaining = rest;
            continue;
        }

        let mut title_chars = remaining.chars();
        let ch = match title_chars.next() {
            Some(ch) => ch,
            None => return Err(nom::Err::Error(make_error(remaining, ErrorKind::Tag))),
        };
        let ch_len = ch.len_utf8();

        if ch == closing {
            return Ok((&remaining[ch_len..], title));
        }

        title.push(ch);
        remaining = &remaining[ch_len..];
    }
}

/// Parse a GFM-style markdown table.
///
/// Tables have:
/// - Header row: `| Header 1 | Header 2 |`
/// - Separator row: `| --- | :---: |` (determines alignment)
/// - Data rows: `| Cell 1 | Cell 2 |`
///
fn parse_table<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, FormattedTable, E> {
    context("table", |input| {
        let (input, _) = parse_block_leading_spaces(input)?;

        let (input, headers) = parse_table_row(input)?;

        let (input, alignments) = parse_table_separator(input)?;

        if alignments.len() != headers.len() {
            return Err(nom::Err::Error(make_error(input, ErrorKind::Verify)));
        }

        let mut rows = Vec::new();
        let mut remaining = input;

        while let Ok((next_input, cells)) = parse_table_row::<E>(remaining) {
            rows.push(cells);
            remaining = next_input;
        }

        Ok((
            remaining,
            FormattedTable {
                headers,
                alignments,
                rows,
            },
        ))
    })(markdown)
}

/// Parse a single table row, returning each cell's content as parsed inline markdown
fn parse_table_row<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Vec<FormattedTextInline>, E> {
    let (input, _) = space0(input)?;
    let (input, _) = tag("|")(input)?;

    let (input, cells) = many0(parse_table_cell)(input)?;

    let (input, _) = alt((value((), parse_line_ending), value((), eof)))(input)?;

    let parsed_cells = cells.iter().map(|cell| parse_cell_content(cell)).collect();
    Ok((input, parsed_cells))
}

/// Parse a single table cell, handling escaped pipes (`\|`) as literal pipe characters.
/// Stops at newlines since table rows are line-based.
fn parse_table_cell<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, String, E> {
    let mut content = String::new();
    let mut chars = input.char_indices().peekable();
    let mut end_index = 0;

    while let Some((i, c)) = chars.next() {
        if c == '|' {
            end_index = i;
            break;
        } else if c == '\n' || c == '\r' {
            return Err(nom::Err::Error(make_error(input, ErrorKind::TakeUntil)));
        } else if c == '\\' {
            if let Some(&(_, next_c)) = chars.peek()
                && next_c == '|'
            {
                content.push('|');
                chars.next();
                continue;
            }
            content.push(c);
        } else {
            content.push(c);
        }
        end_index = i + c.len_utf8();
    }

    if end_index >= input.len() || !input[end_index..].starts_with('|') {
        return Err(nom::Err::Error(make_error(input, ErrorKind::TakeUntil)));
    }

    let remaining = &input[end_index + 1..];
    Ok((remaining, content.trim().to_string()))
}

/// Parse the table separator row to determine column alignments
fn parse_table_separator<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Vec<TableAlignment>, E> {
    let (input, _) = space0(input)?;
    let (input, _) = tag("|")(input)?;

    let (input, alignments) = many0(parse_separator_cell)(input)?;

    // Must have at least one column
    if alignments.is_empty() {
        return Err(nom::Err::Error(make_error(input, ErrorKind::Verify)));
    }

    let (input, _) = alt((value((), parse_line_ending), value((), eof)))(input)?;

    Ok((input, alignments))
}

/// Parse a single separator cell to determine alignment
fn parse_separator_cell<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, TableAlignment, E> {
    let (input, content) = take_until("|")(input)?;
    let (input, _) = tag("|")(input)?;

    let trimmed = content.trim();

    // Verify it's a valid separator (contains only :, -, and whitespace)
    let is_valid = trimmed
        .chars()
        .all(|c: char| c == '-' || c == ':' || c.is_whitespace());
    if !is_valid || trimmed.is_empty() {
        return Err(nom::Err::Error(make_error(input, ErrorKind::Verify)));
    }

    let alignment = if trimmed.starts_with(':') && trimmed.ends_with(':') {
        TableAlignment::Center
    } else if trimmed.ends_with(':') {
        TableAlignment::Right
    } else {
        TableAlignment::Left
    };

    Ok((input, alignment))
}

/// Parse cell content as inline markdown
fn parse_cell_content(cell: &str) -> Vec<FormattedTextFragment> {
    match all_consuming(parse_inline::<nom::error::Error<_>>)(cell) {
        Ok((_, fragments)) => fragments,
        Err(_) => vec![FormattedTextFragment::plain_text(cell)],
    }
}

/// Parse a string as inline markdown.
/// This is useful for parsing table cell content or other inline text that may contain
/// formatting like **bold**, *italic*, `code`, [links](url), etc.
pub fn parse_inline_markdown(text: &str) -> Vec<FormattedTextFragment> {
    parse_cell_content(text)
}

fn parse_header_tag<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, usize, E> {
    context(
        "header_tag",
        map(
            delimited(
                parse_block_leading_spaces,
                take_while_m_n(HEADER_TAG_MIN_COUNT, HEADER_TAG_MAX_COUNT, |c| c == '#'),
                space1,
            ),
            |s: &str| s.len(),
        ),
    )(markdown)
}

/// Parses unordered list and gets the indent number.
fn parse_unordered_list<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
    indentation_context: &RefCell<ListIndentationContext>,
) -> IResult<&'a str, FormattedIndentTextInline, E> {
    context(
        "unordered_list",
        pair(parse_unordered_list_tag, parse_markdown_line),
    )(markdown)
    .map(|(s, (raw_space_count, text))| {
        let indent_level = indentation_context
            .borrow_mut()
            .get_and_register_indent_level(raw_space_count);
        (s, FormattedIndentTextInline { indent_level, text })
    })
}
fn parse_unordered_list_tag<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, usize, E> {
    context(
        "unordered_list_tag",
        map(
            terminated(
                take_while_m_n(INDENT_TAG_MIN_COUNT, INDENT_TAG_MAX_COUNT, |c| c == ' '),
                terminated(one_of("*-"), tag(" ")),
            ),
            |s: &str| s.len(), // Return raw space count
        ),
    )(markdown)
}

/// Parses ordered list and gets the indent number.
fn parse_ordered_list<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
    indentation_context: &RefCell<ListIndentationContext>,
) -> IResult<&'a str, OrderedFormattedIndentTextInline, E> {
    context(
        "ordered_list",
        pair(parse_ordered_list_tag, parse_markdown_line),
    )(markdown)
    .map(|(s, ((raw_space_count, number), text))| {
        let indent_level = indentation_context
            .borrow_mut()
            .get_and_register_indent_level(raw_space_count);
        (
            s,
            OrderedFormattedIndentTextInline {
                number: number.parse().ok(),
                indented_text: FormattedIndentTextInline { indent_level, text },
            },
        )
    })
}

/// Parse ordered list tag returning raw space count for main parser.
fn parse_ordered_list_tag<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, (usize, &'a str), E> {
    context(
        "ordered_list_tag",
        pair(
            take_while_m_n(INDENT_TAG_MIN_COUNT, INDENT_TAG_MAX_COUNT, |c| c == ' '),
            terminated(take_while1(|d| is_digit(d as u8)), tag(". ")),
        ),
    )(markdown)
    .map(|(s, (spaces, number))| (s, (spaces.len(), number)))
}

fn parse_task_list<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
    indentation_context: &RefCell<ListIndentationContext>,
) -> IResult<&'a str, FormattedTaskList, E> {
    context(
        "task_list",
        tuple((parse_indentation, parse_task_list_tag, parse_markdown_line)),
    )(markdown)
    .map(|(s, (raw_space_count, complete, text))| {
        let indent_level = indentation_context
            .borrow_mut()
            .get_and_register_indent_level(raw_space_count);
        (
            s,
            FormattedTaskList {
                complete,
                indent_level,
                text,
            },
        )
    })
}

/// Parse indentation returning raw space count for main parser.
fn parse_indentation<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, usize, E> {
    context(
        "indentation",
        map(
            take_while_m_n(INDENT_TAG_MIN_COUNT, INDENT_TAG_MAX_COUNT, |c| c == ' '),
            |s: &str| s.len(), // Return raw space count
        ),
    )(markdown)
}

fn parse_task_list_tag<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, bool, E> {
    context(
        "task_list_tag",
        map(
            delimited(
                pair(one_of("*-"), tag(" [")),
                alt((tag(" "), tag_no_case("x"))),
                tag("] "),
            ),
            |s: &str| s.to_lowercase() == "x",
        ),
    )(markdown)
}

fn parse_horizontal_rule_asterisk<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, &'a str, E> {
    context("horizontal_rule_asterisk", terminated(tag("*"), space0))(markdown)
}

fn parse_horizontal_rule_dash<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, &'a str, E> {
    context("horizontal_rule_dash", terminated(tag("-"), space0))(markdown)
}

fn parse_horizontal_rule_underline<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, &'a str, E> {
    context("horizontal_rule_underline", terminated(tag("_"), space0))(markdown)
}

/// Parse a single code block into its info string and contents.
fn parse_code_block<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, (&'a str, String), E> {
    context("code_block", |input| {
        let (input, indentation) = parse_indentation(input)?;
        let (input, _) = tag("```")(input)?;
        let (input, lang) = parse_code_block_lang(input)?;
        let (input, lines) = terminated(
            |i| parse_code_block_lines(i, indentation),
            |i| parse_closing_fence(i, indentation),
        )(input)?;

        let content = strip_indentation_from_lines(&lines, indentation);
        Ok((input, (lang, content)))
    })(markdown)
}

/// Strip the given indentation from each line and join with newlines.
fn strip_indentation_from_lines(lines: &[&str], indentation: usize) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut content = lines
        .iter()
        .map(|line| {
            let line_indent = line.chars().take_while(|c| *c == ' ').count();
            let spaces_to_strip = std::cmp::min(indentation, line_indent);
            if line.len() >= spaces_to_strip {
                &line[spaces_to_strip..]
            } else {
                ""
            }
        })
        .join("\n");

    // Add trailing newline if we have content
    content.push('\n');
    content
}

/// Parse all lines of code block content (not including the closing fence).
fn parse_code_block_lines<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
    opening_fence_indent: usize,
) -> IResult<&'a str, Vec<&'a str>, E> {
    many0(|i| parse_code_block_line(i, opening_fence_indent))(input)
}

/// Parse a single line of code block content (not a closing fence).
fn parse_code_block_line<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
    opening_fence_indent: usize,
) -> IResult<&'a str, &'a str, E> {
    // Parse a line that is not a closing fence
    verify(parse_line, move |line: &str| {
        let line_indent = line.chars().take_while(|c| *c == ' ').count();
        let trimmed_line = line.trim_start();

        // A line is a closing fence if it starts with ``` and contains only backticks
        // and is indented less than 4 spaces relative to the opening fence
        if trimmed_line.starts_with("```") {
            let fence_content = trimmed_line.trim_end();
            let is_fence = fence_content.chars().all(|c| c == '`') && fence_content.len() >= 3;
            let relative_indent = line_indent.saturating_sub(opening_fence_indent);
            let properly_indented = relative_indent < 4;
            !(is_fence && properly_indented)
        } else {
            true
        }
    })(input)
}

/// Parse the closing fence of a code block.
fn parse_closing_fence<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
    opening_fence_indent: usize,
) -> IResult<&'a str, (), E> {
    // Closing fence can be indented up to 3 spaces more than the opening fence
    let max_indent = opening_fence_indent + 3;
    value(
        (),
        tuple((
            take_while_m_n(0, max_indent, |c| c == ' '),
            tag("```"),
            take_while(|c| c == '`'),
            space0,
            alt((parse_line_ending, eof)),
        )),
    )(input)
}

/// Parse the language line of a code block.
fn parse_code_block_lang<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, &'a str, E> {
    // Ideally, we can find a valid (non-empty) language after the first set of ```.
    // If we can't, we still want to parse the code block so we'll just mark the language as default.
    map(parse_line, |line| {
        if line.trim().is_empty() {
            RUNNABLE_BLOCK_MARKDOWN_LANG
        } else {
            line
        }
    })(markdown)
}

/// Parse a single [line](https://spec.commonmark.org/0.30/#line).
fn parse_line<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    terminated(not_markdown_line_ending, alt((parse_line_ending, eof)))(input)
}

/// Parse a single [line ending](https://spec.commonmark.org/0.30/#line-ending)
fn parse_line_ending<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    context("line_ending", alt((tag("\r\n"), tag("\r"), tag("\n"))))(input)
}

/// Recognizes a string of any characters _except_ a Markdown line ending (`\r\n`, `\r`, or `\n`).
///
/// This is adapted from [`nom::character::complete::not_line_ending`], which does not accept a
/// lone `\r`.
fn not_markdown_line_ending<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    match input.position(|c| c == '\r' || c == '\n') {
        None => Ok((input.slice(input.input_len()..), input)),
        // This is simpler than not_line_ending, because we don't need to error on a lone carriage
        // return.
        Some(index) => Ok((input.slice(index..), input.slice(..index))),
    }
}

fn parse_inline<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    mut input: &'a str,
) -> IResult<&'a str, Vec<FormattedTextFragment>, E> {
    // We're parsing inline tokens "by hand" instead of using `fold_many0` to allow lookahead.
    let mut state = InlineState::default();
    while !input.is_empty() {
        let (remaining, token) = parse_inline_token(input)?;
        input = remaining;

        match token {
            InlineToken::CodeSpan(text) => {
                state.push_closed_node(FormattedTextFragment::inline_code(text));
            }
            InlineToken::Text(text) => {
                state.push_text(text);
            }
            InlineToken::AutoLink(url) => {
                // Per GFM spec, autolinks can follow whitespace, line beginning, or formatting
                // delimiters (`*`, `_`, `~`, `(`).
                // https://github.github.com/gfm/#autolinks-extension-
                let can_autolink = state
                    .nodes
                    .last()
                    .and_then(|fragment| fragment.text.chars().last())
                    .is_none_or(|c| {
                        c.is_whitespace() || FORMATTING_DELIMITERS.contains(c) || c == '('
                    });
                if can_autolink {
                    state.push_closed_node(FormattedTextFragment::hyperlink(url.clone(), url));
                } else {
                    state.push_text(url);
                }
            }
            InlineToken::BackslashEscape(ch) | InlineToken::HtmlEntity(ch) => {
                state.push_text(ch);
            }
            InlineToken::Delimiter { kind, count } => {
                let node_index = state.nodes.len();
                let preceding_char = state
                    .nodes
                    .last()
                    .and_then(|fragment| fragment.text.chars().last());
                let following_char = remaining.chars().next();

                let delimiter =
                    Delimiter::new(node_index, kind, count, preceding_char, following_char);
                state.push_closed_node(FormattedTextFragment::plain_text(delimiter.to_text()));
                state.delimiters.push(delimiter);
            }
            InlineToken::LinkEnd => {
                input = parse_link(&mut state, remaining);
            }
            InlineToken::UnderlineEnd => {
                input = parse_underline(&mut state, remaining);
            }
        }
    }

    process_emphasis(&mut state, None);

    assert!(state.delimiters.is_empty()); // All delimiters should have been processed.

    Ok((input, consolidate_fragments(state.nodes)))
}

/// State for parsing inline Markdown.
#[derive(Default)]
struct InlineState {
    /// Accumulator for fragments of formatted text.
    nodes: Vec<FormattedTextFragment>,
    /// Whether the last fragment in `nodes` is "open" or "closed". If the node is open, additional
    /// text can be appended to it. If it's closed, it's a unique chunk of formatting and shouldn't
    /// be further extended. Note: this is completely separate from the idea of open/closed delimiters.
    last_node_closed: bool,
    /// The stack of not-yet used formatting delimiters.
    /// See https://spec.commonmark.org/0.30/#delimiter-stack.
    delimiters: Vec<Delimiter>,
}

impl InlineState {
    /// Append plain text. This will reuse the previous fragment if possible
    fn push_text<S>(&mut self, text: S)
    where
        S: Into<String>,
        String: Extend<S>,
    {
        // We can only reuse the previous fragment if:
        // 1. It exists
        // 2. It wasn't marked as closed (it's a delimiter, autolink, etc.)
        if !self.last_node_closed
            && let Some(node) = self.nodes.last_mut()
        {
            node.text.extend(Some(text));
            return;
        }

        self.nodes.push(FormattedTextFragment::plain_text(text));
        self.last_node_closed = false;
    }

    /// Append a closed node of formatted text.
    fn push_closed_node(&mut self, node: FormattedTextFragment) {
        self.nodes.push(node);
        self.last_node_closed = true;
    }

    /// Remove the formatted text fragment at `index`, updating the delimiter stack accordingly.
    ///
    /// ### Panics
    /// Panics if `index` is out of bounds.
    fn remove_node(&mut self, index: usize) {
        self.nodes.remove(index);
        // Loop backwards so we can short-circuit.
        for delimiter in self.delimiters.iter_mut().rev() {
            if delimiter.node_index > index {
                delimiter.node_index -= 1;
            } else {
                break;
            }
        }
    }

    /// Apply styles after parsing an end marker. This calls `f` to style each node from `start` to
    /// the current parse location. It ensures that there's at least one fragment to style.
    ///
    /// ### Panics
    /// Panics if `start` is out of bounds.
    fn backtrack_styles(&mut self, start: usize, mut f: impl FnMut(&mut FormattedTextStyles)) {
        if start + 1 == self.nodes.len() {
            self.push_text("");
        }
        for fragment in &mut self.nodes[start..] {
            f(&mut fragment.styles);
        }
    }
}

/// Search forwards and backwards to parse a potential link. The parser must have processed up to,
/// but not including, the `]` character that potentially ends the link tag.
///
/// This is approximately equivalent to CommonMark's [look for link or image](https://spec.commonmark.org/0.30/#phase-2-inline-structure)
/// algorithm.
fn parse_link<'a>(state: &mut InlineState, remaining: &'a str) -> &'a str {
    // When we encounter a `]` character, we look backwards in the delimiter stack to find a
    // potential start to the link (or, eventually, the image). We then parse ahead to find the
    // link's target. If either step fails, we insert literal `]` text.

    let Some((link_start_index, link_start)) = state
        .delimiters
        .iter()
        .enumerate()
        .rev()
        .find(|(_, delimiter)| delimiter.kind == DelimiterKind::LinkStart)
    else {
        // If there's no link start, treat this as a literal `]`.
        state.push_text("]");
        return remaining;
    };

    if !link_start.active {
        // If the start is inactive, remove it - this prevents nested links.
        state.delimiters.remove(link_start_index);
        state.push_text("]");
        return remaining;
    }

    // Parse ahead to see if we have a valid link target.
    match parse_link_target::<nom::error::Error<&str>>(remaining) {
        Ok((new_remaining, url)) => {
            // Capture the node index before the link_start reference is invalidated.
            let link_start_node = link_start.node_index;

            // Apply link styling to all in-range fragments. At this point in parsing, that's all
            // fragments from the link start to the current parse location.
            state.backtrack_styles(link_start_node, |styles| {
                styles.hyperlink = Some(Hyperlink::Url(url.clone()))
            });

            // Now, process inline styling within the link tag. This is bounded by the starting `[`.
            process_emphasis(state, Some(link_start_index));

            // We've now used the `[` delimiter, so remove it and deactivate prior delimiters (to
            // prevent nested links).
            // Tracking active/inactive state seems unnecessary with only links (as opposed to just
            // removing the delimiters), but it's needed for image parsing.
            state.delimiters.remove(link_start_index);
            for delimiter in &mut state.delimiters[..link_start_index] {
                if delimiter.kind == DelimiterKind::LinkStart {
                    delimiter.active = false;
                }
            }

            state.remove_node(link_start_node);

            // This skips over the link target, so it won't be re-parsed as Markdown.
            state.last_node_closed = true;
            new_remaining
        }
        Err(_) => {
            // If there isn't a link target, treat this as a literal `]`.
            state.delimiters.remove(link_start_index);
            state.push_text("]");

            remaining
        }
    }
}

/// Parse a link target, a simplified form of [link destination](https://spec.commonmark.org/0.30/#link-destination).
fn parse_link_target<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, String, E> {
    /// Potential characters inside a link destination.
    #[derive(Clone, Copy)]
    enum DestinationComponent<'a> {
        Char(char),
        RightAngle,
        LeftParen,
        RightParen,
        Whitespace(&'a str),
    }

    // Parse the opening paren.
    let (input, _) = tag("(")(input)?;

    // Link destinations may be in pointy brackets, which allows whitespace.
    let (in_brackets, mut input) = match tag::<_, _, ()>("<")(input) {
        Ok((rest, _)) => (true, rest),
        Err(_) => (false, input),
    };

    let mut target = String::new();
    let mut paren_depth = 0usize;

    loop {
        let (rest, component) = alt((
            // Any escape is valid inside a link target. Mapping directly to `char` prevents
            // handling escaped brackets or parentheses incorrectly.
            parse_escape.map(DestinationComponent::Char),
            // Line endings are never allowed inside link targets.
            map_parser(parse_line_ending, fail),
            space1.map(DestinationComponent::Whitespace),
            // A left angle bracket is never valid in bracket-delimited destinations, and always
            // valid in non-delimited destinations.
            verify(char('<'), |_| !in_brackets).map(DestinationComponent::Char),
            value(DestinationComponent::RightAngle, char('>')),
            value(DestinationComponent::LeftParen, char('(')),
            value(DestinationComponent::RightParen, char(')')),
            // ASCII control characters are only allowed inside bracket-delimited destinations.
            satisfy(|ch| in_brackets || !ch.is_control()).map(DestinationComponent::Char),
        ))(input)?;

        match component {
            DestinationComponent::Whitespace(sp) => {
                if !in_brackets {
                    // Whitespace is only allowed in bracketed destinations.
                    return Err(nom::Err::Error(make_error(input, ErrorKind::Space)));
                }
                target.push_str(sp);
            }
            DestinationComponent::Char(ch) => {
                target.push(ch);
            }
            DestinationComponent::RightAngle => {
                if in_brackets {
                    (input, _) = char(')')(rest)?;
                    break;
                } else {
                    target.push('>');
                }
            }
            DestinationComponent::LeftParen => {
                // Balance only matters in un-delimited destinations.
                if !in_brackets {
                    paren_depth += 1;
                }
                target.push('(');
            }
            DestinationComponent::RightParen => {
                if !in_brackets {
                    match paren_depth.checked_sub(1) {
                        None => {
                            // An unbalanced right paren ends the destination.
                            input = rest;
                            break;
                        }
                        Some(depth) => paren_depth = depth,
                    }
                }
                target.push(')');
            }
        }

        input = rest;
    }

    Ok((input, target))
}

/// Parses underlined text using the same logic as parse_link.
fn parse_underline<'a>(state: &mut InlineState, remaining: &'a str) -> &'a str {
    let Some((underline_start_index, underline_start)) = state
        .delimiters
        .iter()
        .enumerate()
        .rev()
        .find(|(_, delimiter)| delimiter.kind == DelimiterKind::UnderlineStart)
    else {
        // If there's no underline start, treat this as a literal `</u>`.
        state.push_text("</u>");
        return remaining;
    };

    if !underline_start.active {
        // If the start is inactive, remove it - this prevents nested underlines.
        state.delimiters.remove(underline_start_index);
        state.push_text("</u>");
        return remaining;
    }

    let underline_start_node = underline_start.node_index;
    state.backtrack_styles(underline_start_node, |styles| styles.underline = true);
    process_emphasis(state, Some(underline_start_index));

    state.delimiters.remove(underline_start_index);
    for delimiter in &mut state.delimiters[..underline_start_index] {
        if delimiter.kind == DelimiterKind::UnderlineStart {
            delimiter.active = false;
        }
    }

    state.remove_node(underline_start_node);
    state.last_node_closed = true;
    remaining
}

/// Process emphasis delimiters on the state's delimiter stack, bounded by `stack_bottom`.
///
/// This is approximately equivalent to the CommonMark [process emphasis](https://spec.commonmark.org/0.30/#phase-2-inline-structure)
/// algorithm. However:
/// * It omits `openers_bottom`, which is purely a performance optimization.
/// * It uses a `Vec` rather than a linked list, which changes the structure a bit to work with lifetimes.
/// * It also parses [GFM strikethrough](https://github.github.com/gfm/#strikethrough-extension-).
fn process_emphasis(state: &mut InlineState, stack_bottom: Option<usize>) {
    if stack_bottom.is_some_and(|bottom| bottom >= state.delimiters.len()) {
        return;
    }

    // The lowest index in the delimiter stack that we can look at to find a closing delimiter. As we
    // apply delimiters, this moves forward so that we don't re-visit them.
    let mut closer_window_start = stack_bottom.map_or(0, |bottom| bottom + 1);
    // This is the minimum point in the delimiter stack that we can consider when looking for
    // opening delimiters. Unlike `window_start`, it doesn't change as we move through the stack.
    // At all times:
    // * stack_bottom < minimum_index
    // * minimum_index <= closer_window_start
    // Once we've found a delimiter pair to process:
    // * closer_index = closer_window_start
    // * minimum_index <= opener_index < closer_index
    let minimum_index = closer_window_start;

    while let Some(offset_from_window) = state
        .delimiters
        .iter()
        .skip(closer_window_start)
        .position(|delimiter| delimiter.can_close)
    {
        let closer_index = closer_window_start + offset_from_window;
        closer_window_start = closer_index;

        // We need to borrow both opener and closer mutably. To do so, split off the portion of the
        // stack with possible opening delimiters.
        let (opener_range, after) = state.delimiters.split_at_mut(closer_index);
        // We know that `closer_index` is in bounds, which means it must be the first element of the second half.
        let closer = &mut after[0];

        // Find the closest possible opening delimiter.
        match opener_range
            .iter_mut()
            .enumerate()
            .rev()
            .find(|(index, delimiter)| *index >= minimum_index && delimiter.can_open_for(closer))
        {
            Some((opener_index, opener)) => {
                // In each pass, process as many delimiters as possible. Paired `***` delimiters
                // will take 2 passes, one for the bold and one for the italics. This is necessary
                // so that we can pair part of a `***` with a `**` or `*` in the case of nested emphasis.
                let (bold, italic, strikethrough, underline, consumed_delimiters) =
                    if opener.kind == DelimiterKind::Strikethrough {
                        (false, false, true, false, opener.count)
                    } else if opener.kind == DelimiterKind::UnderlineStart {
                        (false, false, false, true, 1)
                    } else if opener.count >= 2 && closer.count >= 2 {
                        (true, false, false, false, 2)
                    } else {
                        (false, true, false, false, 1)
                    };

                for fragment in &mut state.nodes[opener.node_index..closer.node_index] {
                    fragment.styles.italic |= italic;
                    let is_not_bolded = !fragment
                        .styles
                        .weight
                        .is_some_and(|weight| weight.is_at_least_bold());
                    if bold && is_not_bolded {
                        fragment.styles.weight = Some(CustomWeight::Bold);
                    }
                    fragment.styles.strikethrough |= strikethrough;
                    fragment.styles.underline |= underline;
                }

                let mut used_delimiters_start = opener_index + 1; // This index is inclusive.
                let mut used_delimiters_end = closer_index; // This end index is exclusive.

                // To handle nested strong emphasis, we peel off delimiters from the opener and
                // closer. If either is completely used up, we remove it from both the stack and
                // the text fragments. The fragment removal must be equivalent to
                // `InlineState::remove_node`, but we don't use that here because the delimiters
                // are already mutably borrowed (we can also handle both the opener and the closer
                // in a single pass this way).

                closer.count -= consumed_delimiters;
                let removed_closer = if closer.count == 0 {
                    state.nodes.remove(closer.node_index);
                    used_delimiters_end += 1;

                    // If we removed the closing delimiter, we advance to the next delimiter in
                    // the stack. Since the closing delimiter will be accounted for in `used_delimiters`,
                    // add 1 here to counteract it.
                    closer_window_start += 1;

                    true
                } else {
                    truncate_delimiters(
                        &mut state.nodes[closer.node_index],
                        closer.kind,
                        consumed_delimiters,
                    );

                    false
                };

                opener.count -= consumed_delimiters;
                let removed_opener = if opener.count == 0 {
                    state.nodes.remove(opener.node_index);
                    used_delimiters_start -= 1;
                    true
                } else {
                    truncate_delimiters(
                        &mut state.nodes[opener.node_index],
                        opener.kind,
                        consumed_delimiters,
                    );
                    false
                };

                // If we used up and removed the opening or closing delimiters, update the node
                // indices of all subsequent delimiters.
                if removed_opener {
                    for delimiter in &mut state.delimiters[opener_index + 1..] {
                        delimiter.node_index -= 1;
                    }
                }

                if removed_closer {
                    for delimiter in &mut state.delimiters[closer_index + 1..] {
                        delimiter.node_index -= 1;
                    }
                }

                let used_delimiters = state
                    .delimiters
                    .drain(used_delimiters_start..used_delimiters_end)
                    .count();

                // We know that all removed delimiters are before the closer, so we need to shift
                // the window accordingly.
                closer_window_start -= used_delimiters;
            }
            None => {
                // This delimiter can't be used, so remove it.
                if !closer.can_open {
                    state.delimiters.remove(closer_index);
                } else {
                    closer_window_start += 1;
                }
            }
        }
    }

    // At this point, we've applied all usable emphasis/strong delimiters in the window from
    // stack_bottom to the end of the delimiter stack.
    match stack_bottom {
        Some(idx) => state.delimiters.truncate(idx + 1),
        None => state.delimiters.clear(),
    }
}

/// Helper for [`process_emphasis`] that removes `count` delimiters of `kind` from `node`.
///
/// In debug builds, this panics if `node` is not a run of `kind` delimiters.
fn truncate_delimiters(node: &mut FormattedTextFragment, kind: DelimiterKind, count: u8) {
    let delimiter = kind.as_str();
    if cfg!(debug_assertions) {
        let text = &node.text;
        assert_eq!(
            text,
            &delimiter.repeat(text.len() / delimiter.len()),
            "{node:?} is not a {kind:?} run"
        );
    }

    node.text
        .truncate(node.text.len() - count as usize * delimiter.len());
}

/// Helper to merge adjacent text fragments with the same styling. Such fragments might come from:
/// * Backslash escapes
/// * Unused styling delimiters
fn consolidate_fragments(
    fragments: impl IntoIterator<Item = FormattedTextFragment>,
) -> Vec<FormattedTextFragment> {
    fragments
        .into_iter()
        .coalesce(|mut prev, current| {
            if prev.styles == current.styles {
                prev.text.push_str(&current.text);
                Ok(prev)
            } else {
                Err((prev, current))
            }
        })
        .collect_vec()
}

/// Parse a single inline Markdown token.
fn parse_inline_token<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    let code_span = map(parse_code_span, InlineToken::CodeSpan);
    let backslash_escape = map(parse_escape, InlineToken::BackslashEscape);
    let html_entity = map(parse_html_entity, InlineToken::HtmlEntity);

    // Split text runs at whitespace and punctuation so that we attempt the other token parsers.
    // This makes sure we can detect formatting within words and autolinks. It also makes the
    // text parser more robust to new kinds of Markdown syntax, which should all be indicated with
    // ASCII punctuation.
    let text = map(
        take_while1(|c: char| !c.is_whitespace() && !c.is_ascii_punctuation()),
        InlineToken::Text,
    );
    // Since runs of whitespace are fairly common, match them specially instead of one-character-at-a-time.
    let whitespace = map(take_while1(|c: char| c.is_whitespace()), InlineToken::Text);
    let unmatched_char = map(take(1usize), InlineToken::Text);

    context(
        "inline_token",
        alt((
            backslash_escape,
            html_entity,
            code_span,
            parse_inline_token_link_start,
            parse_inline_token_link_end,
            parse_inline_token_asterisk,
            parse_inline_token_underscore,
            parse_inline_token_strikethrough,
            parse_inline_token_autolink,
            parse_inline_token_underline_start,
            parse_inline_token_underline_end,
            whitespace,
            text,
            // This _must_ be the last parser in the chain. It unconditionally consumes a single
            // character that did not match any other token (such as a non-escaping backslash or
            // a punctuation character that doesn't affect formatting).
            unmatched_char,
        )),
    )(input)
}

/// Parse a `*` delimiter run.
fn parse_inline_token_asterisk<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context(
        "asterisk_delimiter",
        parse_delimiter_run(DelimiterKind::Asterisk),
    )(input)
}

/// Parse a `_` delimiter run.
fn parse_inline_token_underscore<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context(
        "underscore_delimiter",
        parse_delimiter_run(DelimiterKind::Underscore),
    )(input)
}

/// Parse a `~` delimiter run.
fn parse_inline_token_strikethrough<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context(
        "strikethrough_delimiter",
        map(
            consumed(parse_delimiter_run(DelimiterKind::Strikethrough)),
            |(matched, delimiter)| {
                // Per the GFM spec, 3+ tildes do not create strikethrough.
                if matched.len() > 2 {
                    InlineToken::Text(matched)
                } else {
                    delimiter
                }
            },
        ),
    )(input)
}

/// Parse an inline [autolink](https://github.github.com/gfm/#autolinks-extension-) token.
/// Autolinks are URLs starting with `http://`, `https://`, or `www.` and automatically converted
/// to hyperlinks.
///
/// Long-term, we should implement URL autodetection at layout/render-time, rather than in the
/// parser, to not conflict with user-inserted links.
fn parse_inline_token_autolink<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context("autolink", map(parse_url, InlineToken::AutoLink))(input)
}

/// Parse a link-start delimiter.
fn parse_inline_token_link_start<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context(
        "link_start",
        map(tag("["), |_| InlineToken::Delimiter {
            kind: DelimiterKind::LinkStart,
            count: 1,
        }),
    )(input)
}

/// Parse a link-end delimiter.
fn parse_inline_token_link_end<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context("link_end", map(tag("]"), |_| InlineToken::LinkEnd))(input)
}

/// Parse an underline-start delimiter.
fn parse_inline_token_underline_start<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context(
        "underline_start",
        map(tag("<u>"), |_| InlineToken::Delimiter {
            kind: DelimiterKind::UnderlineStart,
            count: 1,
        }),
    )(input)
}

/// Parse an underline-end delimiter.
fn parse_inline_token_underline_end<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, InlineToken<'a>, E> {
    context(
        "underline_end",
        map(tag("</u>"), |_| InlineToken::UnderlineEnd),
    )(input)
}

/// Helper to parse a run of delimiters.
fn parse_delimiter_run<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    kind: DelimiterKind,
) -> impl FnMut(&'a str) -> IResult<&'a str, InlineToken<'a>, E> {
    map(
        fold_many1(tag(kind.as_str()), || 0, |counter, _| counter + 1),
        move |count| InlineToken::Delimiter { kind, count },
    )
}

/// Parse an inline code span.
/// See https://spec.commonmark.org/0.30/#code-spans.
fn parse_code_span<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    // Markdown allows framing a code span with N backticks so that you can use backticks within it.
    let backtick_string = is_a("`");
    context(
        "code_span",
        flat_map(backtick_string, |backticks| {
            // take_until doesn't consume the end tag, so we do here.
            terminated(take_until(backticks), tag(backticks))
        }),
    )(input)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InlineToken<'a> {
    /// A run of `count` delimiter characters of `kind`.
    Delimiter { kind: DelimiterKind, count: u8 },
    /// A run of non-delimiter text.
    Text(&'a str),
    /// A backslash-escaped character.
    BackslashEscape(char),
    /// An HTML character entity reference (e.g., &lt; -> '<').
    HtmlEntity(char),
    /// An entire code span. Code spans have higher precedence than all other inline constructs,
    /// so we parse them into discrete tokens.
    CodeSpan(&'a str),
    /// An autolink URL. Owned because backslash escapes are processed (e.g., `\.` → `.`),
    /// so the result may differ from the input slice.
    AutoLink(String),
    /// A closing `]` bracket, which triggers link parsing.
    LinkEnd,
    /// A closing </u>, which triggers underline parsing.
    UnderlineEnd,
}

/// An entry in the [delimiter stack](https://spec.commonmark.org/0.30/#delimiter-stack)
#[derive(Debug, Clone)]
struct Delimiter {
    /// The type of delimiter this is.
    kind: DelimiterKind,
    /// The count of repeated delimiter units. This is modified during parsing as delimiters are
    /// consumed.
    count: u8,
    /// The count at the time the delimiter was parsed.
    original_count: u8,
    /// Whether or not this delimiter is active (only applies to link delimiters).
    active: bool,
    /// The index of the [`FormattedTextFragment`] corresponding to this delimiter.
    node_index: usize,
    /// Whether or not this delimiter can open a strong/emphasis range.
    can_open: bool,
    /// Whether or not this delimiter can close a strong/emphasis range.
    can_close: bool,
}

impl Delimiter {
    /// Initialize a new `Delimiter` from its surrounding context.
    ///
    /// The delimiter run's opening and closing state is initialized according to the rules about
    /// left- and right-flanking delimiter runs [here](https://spec.commonmark.org/0.30/#delimiter-run).
    fn new(
        node_index: usize,
        kind: DelimiterKind,
        count: u8,
        preceding_char: Option<char>,
        following_char: Option<char>,
    ) -> Self {
        debug_assert!(
            kind.valid_count(count),
            "{count} {kind:?} delimiters are invalid"
        );

        let followed_by_whitespace = following_char.is_none_or(char::is_whitespace);
        let followed_by_punctuation = following_char.is_some_and(|c| c.is_ascii_punctuation());
        let preceded_by_whitespace = preceding_char.is_none_or(char::is_whitespace);
        let preceded_by_punctuation = preceding_char.is_some_and(|c| c.is_ascii_punctuation());

        let left_flanking = !followed_by_whitespace
            && (!followed_by_punctuation || (preceded_by_whitespace || preceded_by_punctuation));
        let right_flanking = !preceded_by_whitespace
            && (!preceded_by_punctuation || (followed_by_whitespace || followed_by_punctuation));

        let can_open = match kind {
            DelimiterKind::LinkStart => false,
            DelimiterKind::Asterisk => left_flanking,
            DelimiterKind::Underscore => {
                left_flanking && (!right_flanking || preceded_by_punctuation)
            }
            // The GFM spec doesn't fully specify how strikethrough works, so treat it like asterisks.
            DelimiterKind::Strikethrough => left_flanking,
            DelimiterKind::UnderlineStart => left_flanking,
        };

        let can_close = match kind {
            DelimiterKind::LinkStart => false,
            DelimiterKind::Asterisk => right_flanking,
            DelimiterKind::Underscore => {
                right_flanking && (!left_flanking || followed_by_punctuation)
            }
            DelimiterKind::Strikethrough => right_flanking,
            DelimiterKind::UnderlineStart => right_flanking,
        };

        Self {
            kind,
            count,
            original_count: count,
            can_close,
            can_open,
            active: true,
            node_index,
        }
    }

    /// Convert this delimiter to literal text.
    fn to_text(&self) -> String {
        self.kind.as_str().repeat(self.count as usize)
    }

    /// Whether or not this delimiter can open for the given closing delimiter.
    fn can_open_for(&self, other: &Delimiter) -> bool {
        // Base rules that apply to all styling.
        if !self.can_open || self.kind != other.kind {
            return false;
        }

        // For strikethrough, the delimiter counts must match.
        if self.kind == DelimiterKind::Strikethrough {
            return self.count == other.count;
        }

        // This check implements rules 9 and 10 from https://spec.commonmark.org/0.30/#can-open-emphasis.
        // It's odd, but results in the behavior you'd expect for overlapping cases like `*nest**ing***`.
        if (self.can_close || other.can_open)
            && (self.original_count + other.original_count).is_multiple_of(3)
            && (!self.original_count.is_multiple_of(3) || !other.original_count.is_multiple_of(3))
        {
            return false;
        }

        true
    }
}

/// A type of Markdown styling delimiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DelimiterKind {
    Asterisk,
    Underscore,
    LinkStart,
    Strikethrough,
    UnderlineStart,
}

impl DelimiterKind {
    /// Whether or not `count` is a valid run length for this delimiter.
    fn valid_count(self, count: u8) -> bool {
        match self {
            // Emphasis and strong emphasis may be repeated arbitrarily.
            DelimiterKind::Asterisk | DelimiterKind::Underscore => true,
            DelimiterKind::LinkStart => count == 1,
            // According to https://github.github.com/gfm/#strikethrough-extension-, 3 or more
            // tildes do not create strikethrough.
            DelimiterKind::Strikethrough => count <= 2,
            DelimiterKind::UnderlineStart => count == 1,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            DelimiterKind::Asterisk => "*",
            DelimiterKind::Underscore => "_",
            DelimiterKind::LinkStart => "[",
            DelimiterKind::Strikethrough => "~",
            DelimiterKind::UnderlineStart => "<u>",
        }
    }
}

fn parse_url_prefix<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    alt((tag("https://"), tag("http://"), tag("www.")))(input)
}

// This is NOT a great URL parser. For now, a URL is a string that
// - starts with "https://" or "http://" or "www."
// - has at least one alphanumeric char after the prefix
// - does not include trailing formatting characters (*, _, ~)
// - backslash escapes are processed (e.g., `\.` → `.`)
fn parse_url<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    i: &'a str,
) -> IResult<&'a str, String, E> {
    // TODO: Look into other autolink rules here: https://github.github.com/gfm/#autolinks-extension-
    let (_, raw_url) = recognize(tuple((
        parse_url_prefix,
        take_till1(|c: char| c.is_whitespace() || "[]<".find_token(c)),
    )))(i)?;

    // Strip trailing formatting characters (*, _, ~) from the URL.
    // Per GFM spec, autolinks should not include trailing punctuation that could be
    // markdown formatting delimiters.
    let trimmed_len = raw_url
        .trim_end_matches(|c| FORMATTING_DELIMITERS.contains(c))
        .len();

    // If we trimmed everything after the prefix, the URL is invalid
    let min_valid_len = match raw_url.find("://") {
        Some(pos) => pos + "://".len(),
        None => "www.".len(),
    };
    if trimmed_len <= min_valid_len {
        return Err(nom::Err::Error(make_error(i, ErrorKind::TakeWhile1)));
    }

    let trimmed_raw_url = &i[..trimmed_len];
    let new_remaining = &i[trimmed_len..];

    // Process backslash escapes within the URL using parse_escape (e.g., `\.` → `.`).
    let mut trimmed_url = String::with_capacity(trimmed_len);
    let mut remaining = trimmed_raw_url;
    while !remaining.is_empty() {
        if let Ok((rest, ch)) = parse_escape::<nom::error::Error<&str>>(remaining) {
            trimmed_url.push(ch);
            remaining = rest;
        } else if let Some(ch) = remaining.chars().next() {
            trimmed_url.push(ch);
            remaining = &remaining[ch.len_utf8()..];
        }
    }

    Ok((new_remaining, trimmed_url))
}

/// Parses escaped ASCII punctuation
fn parse_escape<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, char, E> {
    let ascii_punctuation = satisfy(|c| c.is_ascii_punctuation());
    context("backslash_escape", preceded(tag("\\"), ascii_punctuation))(markdown)
}

/// Parses HTML character entity references (e.g., &lt; -> '<', &#60; -> '<', &#x3c; -> '<').
fn parse_html_entity<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, char, E> {
    context("html_entity", |input| {
        let (input, _) = tag("&")(input)?;

        if let Ok((input, _)) = tag::<_, _, ()>("#")(input) {
            if let Ok((input, _)) = alt::<_, _, (), _>((tag("x"), tag("X")))(input) {
                let (input, hex_digits) = take_while1(|c: char| c.is_ascii_hexdigit())(input)?;
                let (input, _) = tag(";")(input)?;
                let code_point = u32::from_str_radix(hex_digits, 16)
                    .map_err(|_| nom::Err::Error(make_error(input, ErrorKind::Digit)))?;
                let ch = char::from_u32(code_point)
                    .ok_or_else(|| nom::Err::Error(make_error(input, ErrorKind::Char)))?;
                return Ok((input, ch));
            } else {
                let (input, decimal_digits) = take_while1(|c: char| c.is_ascii_digit())(input)?;
                let (input, _) = tag(";")(input)?;
                let code_point: u32 = decimal_digits
                    .parse()
                    .map_err(|_| nom::Err::Error(make_error(input, ErrorKind::Digit)))?;
                let ch = char::from_u32(code_point)
                    .ok_or_else(|| nom::Err::Error(make_error(input, ErrorKind::Char)))?;
                return Ok((input, ch));
            }
        }

        let (input, entity_name) = take_while1(|c: char| c.is_ascii_alphanumeric())(input)?;
        let (input, _) = tag(";")(input)?;

        let ch = match entity_name {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\x27',
            "vert" => '|',
            "ast" => '*',
            "lowbar" => '_',
            "grave" => '`',
            "bsol" => '\\',
            "nbsp" => '\u{00A0}',
            "copy" => '\u{00A9}',
            "reg" => '\u{00AE}',
            "trade" => '\u{2122}',
            "mdash" => '\u{2014}',
            "ndash" => '\u{2013}',
            "hellip" => '\u{2026}',
            "lsquo" => '\u{2018}',
            "rsquo" => '\u{2019}',
            "ldquo" => '\u{201C}',
            "rdquo" => '\u{201D}',
            _ => return Err(nom::Err::Error(make_error(input, ErrorKind::Tag))),
        };

        Ok((input, ch))
    })(input)
}

/// Many blocks are allowed to start with up to 3 spaces, which are ignored.
fn parse_block_leading_spaces<'a, E: ContextError<&'a str> + ParseError<&'a str>>(
    markdown: &'a str,
) -> IResult<&'a str, (), E> {
    fold_many_m_n(0, 3, char(' '), || (), |_, _| ())(markdown)
}

#[cfg(test)]
#[path = "markdown_parser_tests.rs"]
mod tests;
