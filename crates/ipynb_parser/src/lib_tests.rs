use markdown_parser::{CodeBlockText, FormattedImage, FormattedText, FormattedTextLine};

use super::*;

/// Convert a notebook, asserting it parses successfully (GFM tables off).
fn convert(json: &str) -> FormattedText {
    ipynb_to_formatted_text(json, false).expect("should convert")
}

/// All code blocks in the formatted text, in order.
fn code_blocks(ft: &FormattedText) -> Vec<&CodeBlockText> {
    ft.lines
        .iter()
        .filter_map(|line| match line {
            FormattedTextLine::CodeBlock(block) => Some(block),
            _ => None,
        })
        .collect()
}

/// All images in the formatted text, in order.
fn images(ft: &FormattedText) -> Vec<&FormattedImage> {
    ft.lines
        .iter()
        .filter_map(|line| match line {
            FormattedTextLine::Image(image) => Some(image),
            _ => None,
        })
        .collect()
}

#[test]
fn test_markdown_and_code_cells() {
    let json = r##"{
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {"language_info": {"name": "python"}},
        "cells": [
            {"cell_type": "markdown", "source": ["# Title\n", "Some text"]},
            {"cell_type": "code", "source": "print('hi')", "outputs": []}
        ]
    }"##;

    let ft = convert(json);
    // The markdown cell is parsed once into formatted text...
    let raw = ft.raw_text();
    assert!(raw.contains("Title"), "got: {raw:?}");
    assert!(raw.contains("Some text"), "got: {raw:?}");
    // ...and the code cell becomes a structured code block (no fence/re-parse).
    let blocks = code_blocks(&ft);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].lang, "python");
    assert_eq!(blocks[0].code, "print('hi')");
}

#[test]
fn test_code_cell_without_language() {
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "x = 1"}
        ]
    }"#;

    let blocks_owned = convert(json);
    let blocks = code_blocks(&blocks_owned);
    assert_eq!(blocks.len(), 1);
    // No language tag when the notebook does not declare one.
    assert_eq!(blocks[0].lang, "");
    assert_eq!(blocks[0].code, "x = 1");
}

#[test]
fn test_language_falls_back_to_kernelspec() {
    let json = r#"{
        "nbformat": 4,
        "metadata": {"kernelspec": {"language": "julia"}},
        "cells": [{"cell_type": "code", "source": "1 + 1"}]
    }"#;

    let ft = convert(json);
    assert_eq!(code_blocks(&ft)[0].lang, "julia");
}

#[test]
fn test_language_with_backticks_is_sanitized() {
    // A hostile language tag containing backticks (and a newline) must never end
    // up as a code block's language. Because the language is now a struct field
    // (not a fence), this can't break rendering regardless; we still drop it.
    let json = r#"{
        "nbformat": 4,
        "metadata": {"language_info": {"name": "py```\ninjected"}},
        "cells": [{"cell_type": "code", "source": "x = 1"}]
    }"#;

    let ft = convert(json);
    let blocks = code_blocks(&ft);
    assert_eq!(blocks[0].lang, "");
    assert_eq!(blocks[0].code, "x = 1");
}

#[test]
fn test_language_with_special_chars_is_preserved() {
    // Legitimate language names containing `+`/`#`/`-` are kept verbatim.
    let json = r#"{
        "nbformat": 4,
        "metadata": {"language_info": {"name": "c++"}},
        "cells": [{"cell_type": "code", "source": "int x;"}]
    }"#;

    let ft = convert(json);
    assert_eq!(code_blocks(&ft)[0].lang, "c++");
}

#[test]
fn test_sanitize_language_accepts_and_rejects() {
    // Identifier-like tokens (including the punctuation real language names use)
    // are accepted and trimmed.
    assert_eq!(sanitize_language("python"), "python");
    assert_eq!(sanitize_language("C++"), "C++");
    assert_eq!(sanitize_language("objective-c"), "objective-c");
    assert_eq!(sanitize_language("  rust  "), "rust");
    // Backticks, whitespace, other info-string syntax, and oversized values are
    // rejected, yielding an empty (safe) tag.
    assert_eq!(sanitize_language("py`thon"), "");
    assert_eq!(sanitize_language("two words"), "");
    assert_eq!(sanitize_language(""), "");
    assert_eq!(
        sanitize_language(&"a".repeat(MAX_LANGUAGE_TAG_CHARS + 1)),
        ""
    );
}

#[test]
fn test_stream_output_renders_as_text_block() {
    let json = r#"{
        "nbformat": 4,
        "metadata": {"language_info": {"name": "python"}},
        "cells": [
            {"cell_type": "code", "source": "print('hello')", "outputs": [
                {"output_type": "stream", "name": "stdout", "text": ["hello\n"]}
            ]}
        ]
    }"#;

    let ft = convert(json);
    let blocks = code_blocks(&ft);
    // Code cell, then its (unhighlighted) text output.
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].lang, "python");
    assert_eq!(blocks[0].code, "print('hello')");
    assert_eq!(blocks[1].lang, "");
    assert_eq!(blocks[1].code, "hello");
}

#[test]
fn test_execute_result_text_plain() {
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "2 + 2", "outputs": [
                {"output_type": "execute_result", "data": {"text/plain": "4"}, "metadata": {}}
            ]}
        ]
    }"#;

    let ft = convert(json);
    let blocks = code_blocks(&ft);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[1].code, "4");
}

#[test]
fn test_error_traceback_strips_ansi() {
    // Traceback lines containing SGR color escape sequences.
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "boom", "outputs": [
                {"output_type": "error", "ename": "NameError", "evalue": "boom",
                 "traceback": ["\u001b[0;31mNameError\u001b[0m: name 'boom'", "is not defined"]}
            ]}
        ]
    }"#;

    let ft = convert(json);
    let raw = ft.raw_text();
    assert!(
        !raw.contains('\u{1b}'),
        "ANSI escapes should be stripped: {raw:?}"
    );
    assert!(raw.contains("NameError: name 'boom'"), "got: {raw:?}");
    assert!(raw.contains("is not defined"), "got: {raw:?}");
}

#[test]
fn test_png_output_renders_as_data_uri_image() {
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "plot()", "outputs": [
                {"output_type": "display_data", "data": {"image/png": "iVBORw0KGgo=\n"}, "metadata": {}}
            ]}
        ]
    }"#;

    let ft = convert(json);
    let images = images(&ft);
    assert_eq!(images.len(), 1);
    // Whitespace in the embedded base64 is stripped.
    assert_eq!(images[0].source, "data:image/png;base64,iVBORw0KGgo=");
    assert_eq!(images[0].alt_text, "output");
}

#[test]
fn test_image_payload_is_not_validated_by_parser() {
    // The parser no longer size-checks or validates base64 payloads: it emits
    // the `data:` URI verbatim and defers decoding and size limits to the shared
    // asset layer (mirroring how Markdown `data:` images are handled). An
    // undecodable payload is still emitted as an image here and simply fails to
    // load at render time (silent omission).
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "plot()", "outputs": [
                {"output_type": "display_data", "data": {"image/png": "not-valid-base64"}, "metadata": {}}
            ]}
        ]
    }"#;

    let ft = convert(json);
    let images = images(&ft);
    assert_eq!(
        images.len(),
        1,
        "payload should be emitted without parser-side validation"
    );
    assert_eq!(images[0].source, "data:image/png;base64,not-valid-base64");
}

#[test]
fn test_image_preferred_over_text_plain() {
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "plot()", "outputs": [
                {"output_type": "execute_result",
                 "data": {"image/png": "AAAA", "text/plain": "<Figure>"}, "metadata": {}}
            ]}
        ]
    }"#;

    let ft = convert(json);
    assert_eq!(images(&ft).len(), 1);
    assert_eq!(images(&ft)[0].source, "data:image/png;base64,AAAA");
    assert!(
        !ft.raw_text().contains("<Figure>"),
        "text/plain should be skipped when an image exists"
    );
}

#[test]
fn test_unsupported_mime_is_skipped() {
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "html()", "outputs": [
                {"output_type": "execute_result", "data": {"text/html": "<b>hi</b>"}, "metadata": {}}
            ]}
        ]
    }"#;

    let ft = convert(json);
    // The code still renders; the unsupported HTML output is dropped.
    let blocks = code_blocks(&ft);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].code, "html()");
    assert!(images(&ft).is_empty());
}

#[test]
fn test_backticks_in_code_are_stored_verbatim() {
    // Source containing a triple-backtick run is stored verbatim in the code
    // block; there is no fence to escape, so it cannot break out.
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "s = \"\"\"```\""}
        ]
    }"#;

    let ft = convert(json);
    let blocks = code_blocks(&ft);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].code, "s = \"\"\"```\"");
}

#[test]
fn test_empty_notebook_is_ok() {
    let json = r#"{"nbformat": 4, "cells": []}"#;
    let ft = convert(json);
    assert!(
        ft.lines.is_empty(),
        "expected no lines, got: {:?}",
        ft.lines
    );
}

#[test]
fn test_malformed_json_is_error() {
    let result = ipynb_to_formatted_text("{ not valid json", false);
    assert!(matches!(result, Err(IpynbError::Parse(_))));
}

#[test]
fn test_non_v4_notebook_is_error() {
    let json = r#"{"nbformat": 3, "cells": []}"#;
    let result = ipynb_to_formatted_text(json, false);
    assert!(matches!(
        result,
        Err(IpynbError::UnsupportedFormat { nbformat: Some(3) })
    ));
}

#[test]
fn test_missing_nbformat_is_error() {
    // Arbitrary JSON that lacks an nbformat field must not render as a blank
    // notebook; it should error so the caller falls back to raw content.
    let json = r#"{"some": "json", "cells": []}"#;
    let result = ipynb_to_formatted_text(json, false);
    assert!(matches!(
        result,
        Err(IpynbError::UnsupportedFormat { nbformat: None })
    ));
}

#[test]
fn test_missing_cells_is_error() {
    // A v4 notebook that omits the required `cells` field must error rather than
    // deserialize as an empty (blank-rendering) notebook, so the caller falls
    // back to raw content. An explicit `"cells": []` remains a valid empty
    // notebook (see `test_empty_notebook_is_ok`).
    let json = r#"{"nbformat": 4}"#;
    let result = ipynb_to_formatted_text(json, false);
    assert!(matches!(result, Err(IpynbError::Parse(_))));
}

#[test]
fn test_raw_cell_rendered_as_plain_block() {
    let json = r#"{
        "nbformat": 4,
        "metadata": {"language_info": {"name": "python"}},
        "cells": [
            {"cell_type": "raw", "source": "raw content"}
        ]
    }"#;

    let ft = convert(json);
    let blocks = code_blocks(&ft);
    assert_eq!(blocks.len(), 1);
    // Raw cells are not tagged with the kernel language.
    assert_eq!(blocks[0].lang, "");
    assert_eq!(blocks[0].code, "raw content");
}

#[test]
fn test_strip_ansi_handles_csi_and_osc() {
    assert_eq!(strip_ansi("\u{1b}[0;31mred\u{1b}[0m"), "red");
    assert_eq!(strip_ansi("plain text"), "plain text");
    // OSC sequence terminated by BEL.
    assert_eq!(strip_ansi("\u{1b}]0;title\u{07}body"), "body");
}

#[test]
fn test_large_text_output_is_preserved_verbatim() {
    // Large outputs are rendered in full (no arbitrary truncation or synthetic
    // placeholder text) so select-all/copy yields the canonical content.
    let big = "a".repeat(250_000);
    let json = format!(
        r#"{{"nbformat": 4, "cells": [{{"cell_type": "code", "source": "x", "outputs": [{{"output_type": "stream", "name": "stdout", "text": "{big}"}}]}}]}}"#
    );

    let ft = convert(&json);
    let blocks = code_blocks(&ft);
    let output = &blocks[1].code;
    assert!(
        !output.contains("[output truncated]"),
        "output must not contain a synthetic truncation marker"
    );
    assert_eq!(output.chars().count(), big.chars().count());
}

#[test]
fn test_raw_fallback_holds_content_verbatim() {
    // Content that is not a parseable notebook is placed in a single json code
    // block so any Markdown/HTML inside it is shown verbatim, not interpreted.
    let raw = "{ \"nbformat\": 4, # Heading <b>bold</b>";
    let ft = raw_fallback_formatted_text(raw);
    let blocks = code_blocks(&ft);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].lang, "json");
    assert_eq!(blocks[0].code, raw);
}

#[test]
fn test_raw_fallback_handles_backticks_verbatim() {
    // Raw content containing a triple-backtick run is stored verbatim; there is
    // no fence for it to break out of.
    let ft = raw_fallback_formatted_text("```");
    let blocks = code_blocks(&ft);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].code, "```");
}
