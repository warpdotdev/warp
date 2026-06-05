use super::*;

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

    let md = ipynb_to_markdown(json).expect("should convert");
    assert_eq!(md, "# Title\nSome text\n\n```python\nprint('hi')\n```");
}

#[test]
fn test_code_cell_without_language() {
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "x = 1"}
        ]
    }"#;

    let md = ipynb_to_markdown(json).expect("should convert");
    // No language tag follows the opening fence.
    assert_eq!(md, "```\nx = 1\n```");
}

#[test]
fn test_language_falls_back_to_kernelspec() {
    let json = r#"{
        "nbformat": 4,
        "metadata": {"kernelspec": {"language": "julia"}},
        "cells": [{"cell_type": "code", "source": "1 + 1"}]
    }"#;

    let md = ipynb_to_markdown(json).expect("should convert");
    assert!(md.starts_with("```julia\n"), "got: {md}");
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

    let md = ipynb_to_markdown(json).expect("should convert");
    assert_eq!(md, "```python\nprint('hello')\n```\n\n```\nhello\n```");
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

    let md = ipynb_to_markdown(json).expect("should convert");
    assert!(md.contains("```\n4\n```"), "got: {md}");
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

    let md = ipynb_to_markdown(json).expect("should convert");
    assert!(
        !md.contains('\u{1b}'),
        "ANSI escapes should be stripped: {md:?}"
    );
    assert!(md.contains("NameError: name 'boom'"), "got: {md}");
    assert!(md.contains("is not defined"), "got: {md}");
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

    let md = ipynb_to_markdown(json).expect("should convert");
    // Whitespace in the embedded base64 is stripped.
    assert!(
        md.contains("![output](data:image/png;base64,iVBORw0KGgo=)"),
        "got: {md}"
    );
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

    let md = ipynb_to_markdown(json).expect("should convert");
    assert!(md.contains("data:image/png;base64,AAAA"), "got: {md}");
    assert!(
        !md.contains("<Figure>"),
        "text/plain should be skipped when an image exists: {md}"
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

    let md = ipynb_to_markdown(json).expect("should convert");
    // The code still renders; the unsupported HTML output is dropped.
    assert_eq!(md, "```\nhtml()\n```");
}

#[test]
fn test_backtick_fence_in_code_is_escaped() {
    // Source contains a triple-backtick run, so the fence must be longer.
    let json = r#"{
        "nbformat": 4,
        "cells": [
            {"cell_type": "code", "source": "s = \"\"\"```\""}
        ]
    }"#;

    let md = ipynb_to_markdown(json).expect("should convert");
    assert!(
        md.starts_with("````\n"),
        "fence should be 4 backticks: {md:?}"
    );
    assert!(
        md.ends_with("\n````"),
        "closing fence should be 4 backticks: {md:?}"
    );
}

#[test]
fn test_empty_notebook_is_ok() {
    let json = r#"{"nbformat": 4, "cells": []}"#;
    let md = ipynb_to_markdown(json).expect("should convert");
    assert_eq!(md, "");
}

#[test]
fn test_malformed_json_is_error() {
    let result = ipynb_to_markdown("{ not valid json");
    assert!(matches!(result, Err(IpynbError::Parse(_))));
}

#[test]
fn test_non_v4_notebook_is_error() {
    let json = r#"{"nbformat": 3, "cells": []}"#;
    let result = ipynb_to_markdown(json);
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
    let result = ipynb_to_markdown(json);
    assert!(matches!(
        result,
        Err(IpynbError::UnsupportedFormat { nbformat: None })
    ));
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

    let md = ipynb_to_markdown(json).expect("should convert");
    // Raw cells are not tagged with the kernel language.
    assert_eq!(md, "```\nraw content\n```");
}

#[test]
fn test_strip_ansi_handles_csi_and_osc() {
    assert_eq!(strip_ansi("\u{1b}[0;31mred\u{1b}[0m"), "red");
    assert_eq!(strip_ansi("plain text"), "plain text");
    // OSC sequence terminated by BEL.
    assert_eq!(strip_ansi("\u{1b}]0;title\u{07}body"), "body");
}

#[test]
fn test_truncate_chars_respects_char_boundary() {
    // Multi-byte characters must not be split mid-codepoint.
    assert_eq!(truncate_chars("héllo", 2), "hé");
    assert_eq!(truncate_chars("abc", 10), "abc");
}

#[test]
fn test_oversized_text_output_is_truncated() {
    // A pathologically large stream output is truncated rather than rendered in
    // full, so a single cell can't bloat the buffer (PRODUCT invariant 12).
    let big = "a".repeat(MAX_TEXT_OUTPUT_CHARS + 100);
    let json = format!(
        r#"{{"nbformat": 4, "cells": [{{"cell_type": "code", "source": "x", "outputs": [{{"output_type": "stream", "name": "stdout", "text": "{big}"}}]}}]}}"#
    );

    let md = ipynb_to_markdown(&json).expect("should convert");
    assert!(
        md.contains("[output truncated]"),
        "expected truncation marker"
    );
    // The rendered output is bounded near the limit, not the full oversized size.
    assert!(
        md.chars().count() <= MAX_TEXT_OUTPUT_CHARS + 200,
        "output should be truncated near the limit, got {} chars",
        md.chars().count()
    );
}

#[test]
fn test_oversized_image_is_omitted_with_placeholder() {
    // An embedded image larger than the cap is replaced with a visible
    // placeholder rather than embedded (PRODUCT invariant 12).
    let big = "A".repeat(MAX_IMAGE_DATA_CHARS + 1);
    let json = format!(
        r#"{{"nbformat": 4, "cells": [{{"cell_type": "code", "source": "plot()", "outputs": [{{"output_type": "display_data", "data": {{"image/png": "{big}"}}, "metadata": {{}}}}]}}]}}"#
    );

    let md = ipynb_to_markdown(&json).expect("should convert");
    assert!(
        md.contains("[output image omitted: exceeds size limit]"),
        "expected placeholder for oversized image"
    );
    assert!(
        !md.contains("data:image/png;base64,"),
        "oversized image should not be embedded as a data URI"
    );
}

#[test]
fn test_raw_fallback_fences_content_verbatim() {
    // Content that is not a parseable notebook is wrapped in a fenced block so
    // any Markdown/HTML inside it is shown verbatim, not interpreted.
    let raw = "{ \"nbformat\": 4, # Heading <b>bold</b>";
    let md = raw_fallback_markdown(raw);
    assert!(md.starts_with("```json\n"), "should open a json fence: {md:?}");
    assert!(md.ends_with("\n```"), "should close the fence: {md:?}");
    assert!(
        md.contains("# Heading <b>bold</b>"),
        "raw content should be preserved verbatim: {md:?}"
    );
}

#[test]
fn test_raw_fallback_adapts_fence_for_backticks() {
    // Raw content containing a triple-backtick run gets a longer fence so it
    // cannot break out of the code block.
    let md = raw_fallback_markdown("```");
    assert!(
        md.starts_with("````json\n"),
        "fence should be 4 backticks: {md:?}"
    );
}
