//! Converts `.ipynb` (Jupyter) notebooks into Markdown that Warp's existing
//! rich-text/notebook renderer can display.
//!
//! This is **render-only**: it produces a read-only Markdown representation of
//! the notebook's existing content (markdown cells, code cells, and saved
//! outputs). It does not execute cells or round-trip edits back to the file.
//!
//! Only nbformat v4 is supported. Anything that fails to parse as a v4 notebook
//! returns an [`IpynbError`] so callers can fall back to showing the raw file
//! contents instead of a blank view.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The only nbformat major version this converter understands.
const SUPPORTED_NBFORMAT: i64 = 4;

/// Maximum number of characters rendered for a single text output before it is
/// truncated. Prevents a single pathological output from bloating the buffer.
const MAX_TEXT_OUTPUT_CHARS: usize = 100_000;

/// Maximum size (in base64 characters) of an embedded image we will render.
/// Larger images are replaced with a placeholder so the rest of the notebook
/// still renders without freezing the UI.
const MAX_IMAGE_DATA_CHARS: usize = 8 * 1024 * 1024;

/// Error produced when the input cannot be rendered as a supported notebook.
#[derive(Debug, thiserror::Error)]
pub enum IpynbError {
    /// The input was not valid notebook JSON.
    #[error("failed to parse notebook JSON: {0}")]
    Parse(#[from] serde_json::Error),
    /// The notebook used an unsupported nbformat version (only v4 is supported).
    #[error("unsupported notebook format: nbformat={nbformat:?} (only v{SUPPORTED_NBFORMAT} is supported)")]
    UnsupportedFormat { nbformat: Option<i64> },
}

/// Convert the JSON contents of a `.ipynb` file into Markdown.
///
/// Returns an [`IpynbError`] if the input is not a parseable nbformat v4
/// notebook; callers should fall back to displaying the raw contents in that
/// case (never a blank view).
pub fn ipynb_to_markdown(json: &str) -> Result<String, IpynbError> {
    let notebook: Notebook = serde_json::from_str(json)?;

    // Guard against arbitrary JSON that happens to deserialize into an empty
    // notebook: require an explicit, supported nbformat version.
    if notebook.nbformat != Some(SUPPORTED_NBFORMAT) {
        return Err(IpynbError::UnsupportedFormat {
            nbformat: notebook.nbformat,
        });
    }

    let language = notebook.language();
    let mut out = String::new();

    for cell in &notebook.cells {
        match cell.cell_type.as_str() {
            "markdown" => {
                let source = cell.source.to_text();
                push_block(&mut out, source.trim_end_matches('\n'));
            }
            "code" => {
                let source = cell.source.to_text();
                push_code_block(&mut out, &language, source.trim_end_matches('\n'));
                for output in &cell.outputs {
                    push_output(&mut out, output);
                }
            }
            // Raw cells are passed through verbatim in Jupyter; render them as a
            // plain (unhighlighted) code block so their contents can't inject
            // unexpected markdown.
            "raw" => {
                let source = cell.source.to_text();
                push_code_block(&mut out, "", source.trim_end_matches('\n'));
            }
            // Unknown / future cell types are skipped rather than rendered raw.
            _ => {}
        }
    }

    Ok(out.trim_end().to_string())
}

/// Wrap raw file contents in a fenced code block so a notebook that fails to
/// parse (malformed JSON, unsupported nbformat version, etc.) is shown verbatim
/// as a fallback, never re-interpreted as Markdown/HTML. The fence length
/// adapts so content containing backtick runs cannot break out of the block.
pub fn raw_fallback_markdown(content: &str) -> String {
    let mut out = String::new();
    push_code_block(&mut out, "json", content.trim_end_matches('\n'));
    out.trim_end().to_string()
}

/// Append a block of already-formatted Markdown, separated from surrounding
/// content by a blank line. Empty blocks are skipped.
fn push_block(out: &mut String, content: &str) {
    if content.is_empty() {
        return;
    }
    out.push_str(content);
    out.push_str("\n\n");
}

/// Append a fenced code block with the given language tag (empty for none).
/// The fence length adapts so content containing backtick runs is not broken.
fn push_code_block(out: &mut String, language: &str, content: &str) {
    let fence = backtick_fence(content);
    out.push_str(&fence);
    out.push_str(language);
    out.push('\n');
    out.push_str(content);
    if !content.is_empty() && !content.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&fence);
    out.push_str("\n\n");
}

/// Append a single saved cell output.
fn push_output(out: &mut String, output: &Output) {
    match output.output_type.as_str() {
        "stream" => {
            if let Some(text) = &output.text {
                push_text_output(out, &text.to_text());
            }
        }
        "execute_result" | "display_data" => {
            let Some(data) = &output.data else {
                return;
            };
            // Prefer images, then plain text. Other MIME types (text/html,
            // LaTeX, widgets, ...) are intentionally skipped in v1.
            if let Some(value) = data.get("image/png") {
                push_image(out, "image/png", value);
            } else if let Some(value) = data.get("image/jpeg") {
                push_image(out, "image/jpeg", value);
            } else if let Some(value) = data.get("text/plain") {
                push_text_output(out, &value_to_text(value));
            }
        }
        "error" => {
            let traceback = output
                .traceback
                .as_ref()
                .map(|lines| lines.join("\n"))
                .unwrap_or_default();
            push_text_output(out, &strip_ansi(&traceback));
        }
        // Unknown output types are skipped.
        _ => {}
    }
}

/// Append a text output as a plain (unhighlighted) fenced block, truncating
/// oversized output. Empty output is skipped.
fn push_text_output(out: &mut String, text: &str) {
    let text = text.trim_end_matches('\n');
    if text.is_empty() {
        return;
    }
    if text.chars().count() > MAX_TEXT_OUTPUT_CHARS {
        let truncated = format!(
            "{}\n[output truncated]",
            truncate_chars(text, MAX_TEXT_OUTPUT_CHARS)
        );
        push_code_block(out, "", &truncated);
    } else {
        push_code_block(out, "", text);
    }
}

/// Append an embedded image output as a base64 data-URI image. The image data
/// is already base64 in the notebook, so we only strip whitespace and bound the
/// size.
fn push_image(out: &mut String, mime: &str, value: &serde_json::Value) {
    let base64: String = value_to_text(value)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if base64.is_empty() {
        return;
    }
    if base64.len() > MAX_IMAGE_DATA_CHARS {
        push_text_output(out, "[output image omitted: exceeds size limit]");
        return;
    }
    out.push_str(&format!("![output](data:{mime};base64,{base64})\n\n"));
}

/// Returns a backtick fence (at least three backticks) longer than any backtick
/// run in `content`, so fenced content containing backticks is not terminated
/// early.
fn backtick_fence(content: &str) -> String {
    let mut longest_run = 0;
    let mut current_run = 0;
    for ch in content.chars() {
        if ch == '`' {
            current_run += 1;
            longest_run = longest_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    "`".repeat(longest_run.max(2) + 1)
}

/// Truncate a string to at most `max_chars` characters on a char boundary.
fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
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

/// Top-level notebook structure (nbformat v4, only the fields we render).
#[derive(Debug, Deserialize)]
struct Notebook {
    #[serde(default)]
    nbformat: Option<i64>,
    #[serde(default)]
    cells: Vec<Cell>,
    #[serde(default)]
    metadata: Metadata,
}

impl Notebook {
    /// The code-fence language, derived from notebook metadata. Empty if the
    /// notebook does not declare a language.
    fn language(&self) -> String {
        self.metadata
            .language_info
            .as_ref()
            .and_then(|info| info.name.clone())
            .or_else(|| {
                self.metadata
                    .kernelspec
                    .as_ref()
                    .and_then(|spec| spec.language.clone())
            })
            .unwrap_or_default()
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
#[path = "ipynb_tests.rs"]
mod tests;
