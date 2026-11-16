//! Parses `.ipynb` (Jupyter) notebooks directly into the [`FormattedText`]
//! representation that Warp's rich-text/notebook renderer consumes.
//!
//! This is **render-only**: it produces a read-only view of the notebook's
//! existing content (markdown cells, code cells, and saved outputs). It does
//! not execute cells or round-trip edits back to the file.
//!
//! Only nbformat v4 is supported. Anything that fails to parse as a v4 notebook
//! returns an [`IpynbError`] so callers can fall back to showing the raw file
//! contents instead of a blank view.

use std::collections::BTreeMap;

use markdown_parser::{
    CodeBlockText, FormattedImage, FormattedText, FormattedTextFragment, FormattedTextLine,
    parse_markdown, parse_markdown_with_gfm_tables,
};
use serde::Deserialize;

/// The only nbformat major version this converter understands.
const SUPPORTED_NBFORMAT: i64 = 4;

/// Maximum length of a code-block language tag we will emit. Real language
/// names are short; a longer value is treated as untrusted junk and dropped so
/// it cannot bloat every code block.
const MAX_LANGUAGE_TAG_CHARS: usize = 32;

/// Error produced when the input cannot be rendered as a supported notebook.
#[derive(Debug, thiserror::Error)]
pub enum IpynbError {
    /// The input was not valid notebook JSON.
    #[error("failed to parse notebook JSON: {0}")]
    Parse(#[from] serde_json::Error),
    /// The notebook used an unsupported nbformat version (only v4 is supported).
    #[error(
        "unsupported notebook format: nbformat={nbformat:?} (only v{SUPPORTED_NBFORMAT} is supported)"
    )]
    UnsupportedFormat { nbformat: Option<i64> },
}

/// Convert the JSON contents of a `.ipynb` file into [`FormattedText`].
///
/// `gfm_tables` selects the GFM-table-aware Markdown parser for markdown cells,
/// mirroring the `Buffer::from_markdown` behavior (the caller passes the
/// `MarkdownTables` feature flag state).
///
/// Returns an [`IpynbError`] if the input is not a parseable nbformat v4
/// notebook; callers should fall back to [`raw_fallback_formatted_text`] in that
/// case so the contents are shown verbatim (never a blank view).
pub fn ipynb_to_formatted_text(json: &str, gfm_tables: bool) -> Result<FormattedText, IpynbError> {
    let notebook: Notebook = serde_json::from_str(json)?;

    // Guard against arbitrary JSON that happens to deserialize into an empty
    // notebook: require an explicit, supported nbformat version.
    if notebook.nbformat != Some(SUPPORTED_NBFORMAT) {
        return Err(IpynbError::UnsupportedFormat {
            nbformat: notebook.nbformat,
        });
    }

    let language = notebook.language();
    // Convert each cell independently (a 1:1 map with an exact size hint), then
    // size the final buffer from the per-cell line counts so it is allocated
    // exactly once instead of growing incrementally.
    let per_cell: Vec<Vec<FormattedTextLine>> = notebook
        .cells
        .iter()
        .map(|cell| cell_lines(cell, &language, gfm_tables))
        .collect();
    let total_lines = per_cell.iter().map(Vec::len).sum();
    let mut lines = Vec::with_capacity(total_lines);
    for lines_for_cell in per_cell {
        lines.extend(lines_for_cell);
    }

    Ok(FormattedText::new_trimmed(lines))
}

/// Build a verbatim [`FormattedText`] fallback for content that is not a
/// parseable notebook (malformed JSON, unsupported nbformat version, etc.). The
/// raw content is placed in a single code block so any Markdown/HTML inside it
/// is shown verbatim, never re-interpreted.
pub fn raw_fallback_formatted_text(content: &str) -> FormattedText {
    FormattedText::new_trimmed(vec![FormattedTextLine::CodeBlock(CodeBlockText {
        lang: "json".to_string(),
        code: content.trim_end_matches('\n').to_string(),
    })])
}

/// Convert a single notebook cell into its formatted-text lines.
fn cell_lines(cell: &Cell, language: &str, gfm_tables: bool) -> Vec<FormattedTextLine> {
    match cell.cell_type.as_str() {
        "markdown" => {
            let source = cell.source.to_text();
            markdown_block_lines(source.trim_end_matches('\n'), gfm_tables)
        }
        "code" => {
            let source = cell.source.to_text();
            let mut lines = code_block_lines(language, source.trim_end_matches('\n'));
            lines.extend(cell.outputs.iter().flat_map(output_lines));
            lines
        }
        // Raw cells are passed through verbatim in Jupyter; render them as a
        // plain (unhighlighted) code block so their contents can't inject
        // unexpected markdown.
        "raw" => {
            let source = cell.source.to_text();
            code_block_lines("", source.trim_end_matches('\n'))
        }
        // Unknown / future cell types are skipped rather than rendered raw.
        _ => Vec::new(),
    }
}

/// A markdown cell, parsed into formatted text and separated from surrounding
/// blocks by a line break. Empty cells produce no lines.
fn markdown_block_lines(content: &str, gfm_tables: bool) -> Vec<FormattedTextLine> {
    if content.is_empty() {
        return Vec::new();
    }
    let parse_fn = if gfm_tables {
        parse_markdown_with_gfm_tables
    } else {
        parse_markdown
    };
    // A markdown cell is, by definition, Markdown; parse it once. If parsing
    // somehow fails, preserve the content verbatim as a plain line rather than
    // dropping it.
    let parsed = parse_fn(content).unwrap_or_else(|_| {
        FormattedText::new(vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(content),
        ])])
    });
    let mut lines = Vec::from(parsed.lines);
    lines.push(FormattedTextLine::LineBreak);
    lines
}

/// A code block with the given language tag (empty for none), separated from
/// surrounding blocks by a line break.
fn code_block_lines(language: &str, content: &str) -> Vec<FormattedTextLine> {
    vec![
        FormattedTextLine::CodeBlock(CodeBlockText {
            lang: language.to_string(),
            code: content.to_string(),
        }),
        FormattedTextLine::LineBreak,
    ]
}

/// The lines for a single saved cell output. Skipped outputs produce no lines.
fn output_lines(output: &Output) -> Vec<FormattedTextLine> {
    match output.output_type.as_str() {
        "stream" => match &output.text {
            Some(text) => text_output_lines(&text.to_text()),
            None => Vec::new(),
        },
        "execute_result" | "display_data" => {
            let Some(data) = &output.data else {
                return Vec::new();
            };
            // Prefer images, then plain text. Other MIME types (text/html,
            // LaTeX, widgets, ...) are intentionally skipped in v1.
            if let Some(value) = data.get("image/png") {
                image_lines("image/png", value)
            } else if let Some(value) = data.get("image/jpeg") {
                image_lines("image/jpeg", value)
            } else if let Some(value) = data.get("text/plain") {
                text_output_lines(&value_to_text(value))
            } else {
                Vec::new()
            }
        }
        "error" => {
            let traceback = output
                .traceback
                .as_ref()
                .map(|tb| tb.join("\n"))
                .unwrap_or_default();
            // ANSI escapes (common in colored tracebacks) are stripped centrally
            // by `text_output_lines`.
            text_output_lines(&traceback)
        }
        // Unknown output types are skipped.
        _ => Vec::new(),
    }
}

/// A text output as a plain (unhighlighted) code block. Empty output produces
/// no lines.
///
/// TODO: Bounding pathologically large outputs is left to
/// a future change that models truncation without polluting buffer content
/// (e.g. a "show more" affordance) rather than injecting placeholder text.
fn text_output_lines(text: &str) -> Vec<FormattedTextLine> {
    // TODO: Remove ANSI stripping once we support ANSI-color rendering (a color
    // attribute on `FormattedTextStyles` + SGR parsing)
    let stripped = strip_ansi(text);
    let text = stripped.trim_end_matches('\n');
    if text.is_empty() {
        return Vec::new();
    }
    code_block_lines("", text)
}

/// An embedded image output as a base64 data-URI image. Empty payloads produce
/// no lines.
fn image_lines(mime: &str, value: &serde_json::Value) -> Vec<FormattedTextLine> {
    let base64: String = value_to_text(value)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if base64.is_empty() {
        return Vec::new();
    }
    vec![
        FormattedTextLine::Image(FormattedImage {
            alt_text: "output".to_string(),
            source: format!("data:{mime};base64,{base64}"),
            title: None,
        }),
        FormattedTextLine::LineBreak,
    ]
}

/// Strip ANSI escape sequences (CSI/SGR colors, OSC, and simple escapes) so
/// tracebacks render as readable plain text.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            // CSI sequence: ESC [ ... <final byte 0x40-0x7E>
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            // OSC sequence: ESC ] ... terminated by BEL or ST (ESC \)
            Some(']') => {
                chars.next();
                while let Some(&next) = chars.peek() {
                    if next == '\u{07}' {
                        chars.next();
                        break;
                    }
                    if next == '\u{1b}' {
                        chars.next();
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                    chars.next();
                }
            }
            // Other escapes (e.g. ESC ( B): drop the single following byte.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Convert a JSON value that is either a string or an array of strings into a
/// single string (notebook source and text fields use both forms).
fn value_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items.iter().filter_map(|v| v.as_str()).collect(),
        _ => String::new(),
    }
}

/// Sanitize a notebook-declared language into a safe code-block language tag.
fn sanitize_language(raw: &str) -> String {
    let trimmed = raw.trim();
    let is_safe = !trimmed.is_empty()
        && trimmed.chars().count() <= MAX_LANGUAGE_TAG_CHARS
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '-' | '_' | '.'));
    if is_safe {
        trimmed.to_string()
    } else {
        String::new()
    }
}

/// Top-level notebook structure (nbformat v4, only the fields we render).
#[derive(Debug, Deserialize)]
struct Notebook {
    #[serde(default)]
    nbformat: Option<i64>,
    cells: Vec<Cell>,
    #[serde(default)]
    metadata: Metadata,
}

impl Notebook {
    /// The code-block language, derived from notebook metadata and sanitized to
    /// a safe tag (see [`sanitize_language`]). Empty if the notebook does not
    /// declare a language, or declares one that is not a safe identifier.
    fn language(&self) -> String {
        let raw = self
            .metadata
            .language_info
            .as_ref()
            .and_then(|info| info.name.clone())
            .or_else(|| {
                self.metadata
                    .kernelspec
                    .as_ref()
                    .and_then(|spec| spec.language.clone())
            })
            .unwrap_or_default();
        sanitize_language(&raw)
    }
}

#[derive(Debug, Default, Deserialize)]
struct Metadata {
    #[serde(default)]
    language_info: Option<LanguageInfo>,
    #[serde(default)]
    kernelspec: Option<Kernelspec>,
}

#[derive(Debug, Deserialize)]
struct LanguageInfo {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Kernelspec {
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Cell {
    #[serde(default)]
    cell_type: String,
    #[serde(default)]
    source: Source,
    #[serde(default)]
    outputs: Vec<Output>,
}

#[derive(Debug, Deserialize)]
struct Output {
    #[serde(default)]
    output_type: String,
    /// Present for `stream` outputs.
    #[serde(default)]
    text: Option<Source>,
    /// Present for `execute_result` / `display_data` outputs (MIME -> value).
    #[serde(default)]
    data: Option<BTreeMap<String, serde_json::Value>>,
    /// Present for `error` outputs.
    #[serde(default)]
    traceback: Option<Vec<String>>,
}

/// A notebook `source`/`text` field, which may be a single string or a list of
/// strings (each typically including its trailing newline).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Source {
    Lines(Vec<String>),
    Text(String),
}

impl Default for Source {
    fn default() -> Self {
        Source::Text(String::new())
    }
}

impl Source {
    fn to_text(&self) -> String {
        match self {
            Source::Lines(lines) => lines.concat(),
            Source::Text(text) => text.clone(),
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
