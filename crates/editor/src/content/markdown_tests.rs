use std::sync::Arc;

use markdown_parser::{compute_formatted_text_delta, parse_markdown};
use serde_yaml::Value;
use string_offset::CharOffset;
use vec1::Vec1;
use warpui_core::{App, ReadModel};

use super::MarkdownStyle;
use crate::content::buffer::tests::TestEmbeddedItem;
use crate::content::buffer::{Buffer, BufferEditAction, EditOrigin, StyledBlockBoundaryBehavior};
use crate::content::text::{IndentBehavior, TABLE_BLOCK_MARKDOWN_LANG};

#[test]
fn test_export_normalizes_code_languages() {
    let formatted = parse_markdown(
        r#"
```JavaScript
console.log("Hello, World");
```
```Rust
println!("Hello, World");
```
```ocaml
print_endline "Hello, World!"
```
"#,
    )
    .unwrap();
    let exported = Buffer::export_to_markdown(
        formatted,
        None,
        MarkdownStyle::Export {
            app_context: None,
            should_not_escape_markdown_punctuation: false,
        },
    );

    // Exporting should use external code languages.
    assert_eq!(
        exported,
        r#"
```js
console.log("Hello, World");
```
```rust
println!("Hello, World");
```
```ocaml
print_endline "Hello, World!"
```
"#
    );
}

#[test]
fn test_mermaid_markdown_round_trip() {
    App::test((), |mut app| async move {
        let _flag = warp_core::features::FeatureFlag::MarkdownMermaid.override_enabled(true);
        let markdown = "```mermaid\ngraph TD\nA --> B\n```\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let internal_markdown = app.read_model(&buffer, |buffer, _| buffer.markdown());
        assert_eq!(internal_markdown, markdown);

        let exported_markdown = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert_eq!(exported_markdown, markdown);
    });
}

#[test]
fn test_export_expands_embeds() {
    // This tests styled block for the edge case of querying just the
    // leading block item (0..1).
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            r#"
```warp-embedded-object
id: embed-123
```
```warp-embedded-object
id: embed-456
ignored: value
```"#,
            Some(|mut mapping| match mapping.remove(&"id".into()) {
                Some(Value::String(id)) => Some(Arc::new(TestEmbeddedItem { id })),
                _ => None,
            }),
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let exported = app.read_model(&buffer, |buffer, _| {
            buffer.to_markdown(MarkdownStyle::Export {
                app_context: None,
                should_not_escape_markdown_punctuation: false,
            })
        });

        // Exporting should expand the embedded objects.
        assert_eq!(
            exported,
            r#"
```warp-embedded-object
---
id: embed-123
export: true

```
```warp-embedded-object
---
id: embed-456
export: true

```
"#
        );
    });
}

#[test]
fn test_table_html_serialization() {
    App::test((), |mut app| async move {
        let markdown = format!(
            "```{}\nheader 1\theader 2\nvalue 1\tvalue 2\n```\n",
            TABLE_BLOCK_MARKDOWN_LANG
        );
        let (buffer, _selection) = Buffer::mock_from_markdown(
            &markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let html = app.read_model(&buffer, |buffer, ctx| {
            let range = CharOffset::from(1)..buffer.max_charoffset();
            buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
        });

        assert!(html.is_some());
        let html = html.unwrap();
        assert!(html.contains(
            "<table><thead><tr><th align=\"left\">header 1</th><th align=\"left\">header 2</th></tr></thead><tbody><tr><td align=\"left\">value 1</td><td align=\"left\">value 2</td></tr></tbody></table>"
        ));
    });
}

#[test]
fn test_gfm_table_html_serialization() {
    App::test((), |mut app| async move {
        let _flag = warp_core::features::FeatureFlag::MarkdownTables.override_enabled(true);
        let markdown = "\
| header 1 | header 2 |\n\
| --- | --- |\n\
| value 1 | value 2 |\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let html = app.read_model(&buffer, |buffer, ctx| {
            let range = CharOffset::from(1)..buffer.max_charoffset();
            buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
        });

        assert!(html.is_some());
        let html = html.unwrap();
        assert!(html.contains(
            "<table><thead><tr><th align=\"left\">header 1</th><th align=\"left\">header 2</th></tr></thead><tbody><tr><td align=\"left\">value 1</td><td align=\"left\">value 2</td></tr></tbody></table>"
        ));
    });
}

#[test]
fn test_apply_formatted_text_delta_append() {
    App::test((), |mut app| async move {
        let old_markdown = "hello world\n";
        let (buffer, selection) = Buffer::mock_from_markdown(
            old_markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        // Buffer::mock_from_markdown removes the trailing newline, so add it back.
        buffer.update(&mut app, |buffer, ctx| {
            let end_offset = buffer.max_charoffset();
            let edits =
                Vec1::try_from_vec(vec![("\n".to_string(), end_offset..end_offset)]).unwrap();
            buffer.update_content(
                BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                EditOrigin::SystemEdit,
                selection.clone(),
                ctx,
            );
        });

        let (exported, old_formatted) = app.read_model(&buffer, |buffer, _| {
            let old_formatted = buffer.range_to_formatted_text(
                CharOffset::from(1)..buffer.max_charoffset(),
                StyledBlockBoundaryBehavior::Exclusive,
            );
            (buffer.markdown_unescaped(), old_formatted)
        });

        assert_eq!(exported, "hello world\n");

        let new_markdown = "hello world\n#";
        let new_formatted = parse_markdown(new_markdown).unwrap();
        let delta = compute_formatted_text_delta(old_formatted, new_formatted.clone());
        // Should just be appending a new line
        assert_eq!(delta.common_prefix_lines, 1);
        // There's a trailing linebreak being replaced
        assert_eq!(delta.old_suffix_formatted_text_lines, 1);
        assert_eq!(delta.new_suffix.len(), 1);
        buffer.update(&mut app, |buffer, ctx| {
            buffer.apply_formatted_text_delta(&delta, selection.clone(), ctx);
        });

        let (exported, formatted_in_buffer) = app.read_model(&buffer, |buffer, _| {
            let new_formatted = buffer.range_to_formatted_text(
                CharOffset::from(1)..buffer.max_charoffset(),
                StyledBlockBoundaryBehavior::Exclusive,
            );
            (buffer.markdown_unescaped(), new_formatted)
        });

        assert_eq!(exported, new_markdown);
        assert_eq!(new_formatted, formatted_in_buffer);

        let new_markdown_2 = "hello world\n# This is a heading";
        let new_formatted_2 = parse_markdown(new_markdown_2).unwrap();
        let delta_2 = compute_formatted_text_delta(new_formatted, new_formatted_2.clone());
        // Should be replacing the # line while keeping the hello world line
        assert_eq!(delta_2.common_prefix_lines, 1);
        assert_eq!(delta_2.old_suffix_formatted_text_lines, 1);
        assert_eq!(delta_2.new_suffix.len(), 1);
        buffer.update(&mut app, |buffer, ctx| {
            buffer.apply_formatted_text_delta(&delta_2, selection.clone(), ctx);
        });

        let (exported, formatted_in_buffer) = app.read_model(&buffer, |buffer, _| {
            let new_formatted = buffer.range_to_formatted_text(
                CharOffset::from(1)..buffer.max_charoffset(),
                StyledBlockBoundaryBehavior::Exclusive,
            );
            (buffer.markdown_unescaped(), new_formatted)
        });

        // We add a trailing newline
        assert_eq!(exported.trim_end(), new_markdown_2);
        assert_eq!(new_formatted_2, formatted_in_buffer);
    });
}

#[test]
fn test_image_html_serialization() {
    App::test((), |mut app| async move {
        let markdown = "![Alt text](image.png)\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let html = app.read_model(&buffer, |buffer, ctx| {
            let range = CharOffset::from(1)..buffer.max_charoffset();
            buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
        });

        // Image should be serialized as <img src="image.png" alt="Alt text" />
        assert!(html.is_some());
        let html = html.unwrap();
        assert!(html.contains("<img"));
        assert!(html.contains("src=\"image.png\""));
        assert!(html.contains("alt=\"Alt text\""));
    });
}

#[test]
fn test_multiple_images_html_serialization() {
    App::test((), |mut app| async move {
        let markdown = "![First](./path/img1.jpg)\n![Second](https://example.com/img2.png)\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let html = app.read_model(&buffer, |buffer, ctx| {
            let range = CharOffset::from(1)..buffer.max_charoffset();
            buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
        });

        // Check both images are in the HTML
        assert!(html.is_some());
        let html = html.unwrap();
        assert!(html.contains("src=\"./path/img1.jpg\""));
        assert!(html.contains("alt=\"First\""));
        assert!(html.contains("src=\"https://example.com/img2.png\""));
        assert!(html.contains("alt=\"Second\""));
    });
}

#[test]
fn test_table_markdown_round_trip() {
    App::test((), |mut app| async move {
        let markdown = format!(
            "```{}\nheader 1\theader 2\nvalue 1\tvalue 2\n```\n",
            TABLE_BLOCK_MARKDOWN_LANG
        );
        let (buffer, _selection) = Buffer::mock_from_markdown(
            &markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        let internal_markdown = app.read_model(&buffer, |buffer, _| buffer.markdown());
        assert_eq!(internal_markdown, markdown);

        let exported_markdown = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert_eq!(
            exported_markdown,
            "| header 1 | header 2 |\n| --- | --- |\n| value 1 | value 2 |\n"
        );
    });
}

#[test]
fn test_table_markdown_export_escapes_pipe_characters() {
    App::test((), |mut app| async move {
        let markdown = format!(
            "```{}\nhead|er 1\theader 2\nvalue | 1\tvalue 2\n```\n",
            TABLE_BLOCK_MARKDOWN_LANG
        );
        let (buffer, _selection) = Buffer::mock_from_markdown(
            &markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let exported_markdown = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert_eq!(
            exported_markdown,
            "| head\\|er 1 | header 2 |\n| --- | --- |\n| value \\| 1 | value 2 |\n"
        );
    });
}

#[test]
fn test_url_link_display_text_round_trip_is_stable() {
    App::test((), |mut app| async move {
        let original =
            "[https://example.com/index.html#section](https://example.com/index.html#section)";
        // After the first save, `.` and `#` in the display text are escaped.
        // The URL in `(...)` is written verbatim — no escaping.
        let expected_escaped = "[https://example\\.com/index\\.html\\#section](https://example.com/index.html#section)";

        let (buffer, _) = Buffer::mock_from_markdown(
            original,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        let after_first = app.read_model(&buffer, |buffer, _| buffer.markdown());
        assert_eq!(
            after_first, expected_escaped,
            "first save should escape special chars in display text"
        );

        let (buffer2, _) = Buffer::mock_from_markdown(
            &after_first,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        let after_second = app.read_model(&buffer2, |buffer, _| buffer.markdown());
        assert_eq!(
            after_second, expected_escaped,
            "second round-trip should be stable"
        );

        let (buffer3, _) = Buffer::mock_from_markdown(
            &after_second,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        let after_third = app.read_model(&buffer3, |buffer, _| buffer.markdown());
        assert_eq!(
            after_third, expected_escaped,
            "third round-trip should be stable"
        );

        // Plain text should be the clean, unescaped URL — no backslashes.
        let plain_text = app.read_model(&buffer3, |buffer, _| buffer.text().as_str().to_string());
        assert_eq!(plain_text, "https://example.com/index.html#section");
    });
}

#[test]
fn test_markdown_escapes_punctuation() {
    App::test((), |mut app| async move {
        // markdown() escapes special chars.
        let markdown = "Here's a markdown comment.\n";
        let (buffer, _) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        let escaped = app.read_model(&buffer, |buffer, _| buffer.markdown());
        assert!(
            escaped.contains("\\."),
            "expected escaped periods, got: {escaped}"
        );
    });
}

#[test]
fn test_markdown_unescaped_does_not_escape_punctuation() {
    App::test((), |mut app| async move {
        // markdown_unescaped() should not add backslashes before periods.
        let markdown = "Here's a markdown comment.\n";
        let (buffer, _) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        let unescaped = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert!(
            !unescaped.contains("\\."),
            "expected no escaped periods, got: {unescaped}"
        );
        assert!(
            unescaped.contains("comment."),
            "expected unescaped period, got: {unescaped}"
        );
    });
}

#[test]
fn test_markdown_unescaped_preserves_urls() {
    App::test((), |mut app| async move {
        // markdown_unescaped() should not escape characters inside URLs.
        let markdown = "Check out https://www.example.com/path\n";
        let (buffer, _) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        let unescaped = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert!(
            !unescaped.contains("\\/"),
            "expected no escaped slashes, got: {unescaped}"
        );
        assert!(
            unescaped.contains("https://www.example.com/path"),
            "expected URL preserved, got: {unescaped}"
        );
    });
}

#[test]
fn test_image_with_content_html_serialization() {
    App::test((), |mut app| async move {
        let markdown = "# Header\n\n![Image](test.png)\n\nSome text\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let html = app.read_model(&buffer, |buffer, ctx| {
            let range = CharOffset::from(1)..buffer.max_charoffset();
            buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
        });

        // Check that header, image, and text are all present
        assert!(html.is_some());
        let html = html.unwrap();
        assert!(html.contains("<h1>"));
        assert!(html.contains("Header"));
        assert!(html.contains("<img"));
        assert!(html.contains("src=\"test.png\""));
        assert!(html.contains("Some text"));
    });
}

/// `<kbd>` in body text serializes to a `<kbd>` element on HTML export (issue #13733).
#[test]
fn test_kbd_html_serialization() {
    App::test((), |mut app| async move {
        let markdown = "Press <kbd>Cmd</kbd> to run\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let html = app.read_model(&buffer, |buffer, ctx| {
            let range = CharOffset::from(1)..buffer.max_charoffset();
            buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
        });

        let html = html.unwrap();
        assert!(html.contains("<kbd>Cmd</kbd>"), "html was: {html}");
    });
}

/// Serialization must always round-trip the AUTHORED `<kbd>` text, never a rendered key glyph
/// (issue #13733). Glyph substitution (e.g. `<kbd>Cmd</kbd>` displaying as ⌘) is a render-only
/// concern: it must never leak into the data model or any export path. This test pins that
/// invariant across every serializer so a future glyph-substitution change that accidentally
/// mutated the buffer text (instead of only the render layer) would fail here.
#[test]
fn test_kbd_serialization_preserves_authored_text_not_glyph() {
    App::test((), |mut app| async move {
        let markdown = "Press <kbd>Cmd</kbd> to run\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        // The glyph a renderer might substitute for "Cmd" on macOS.
        let key_glyph = "\u{2318}"; // ⌘

        // Every export path must emit the authored "Cmd", never the glyph.
        let exported_markdown = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert!(
            exported_markdown.contains("<kbd>Cmd</kbd>"),
            "markdown export must preserve authored text, was: {exported_markdown}"
        );
        assert!(
            !exported_markdown.contains(key_glyph),
            "markdown export must not contain the rendered key glyph, was: {exported_markdown}"
        );

        let html = app
            .read_model(&buffer, |buffer, ctx| {
                let range = CharOffset::from(1)..buffer.max_charoffset();
                buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
            })
            .unwrap();
        assert!(
            html.contains("<kbd>Cmd</kbd>"),
            "html export must preserve authored text, was: {html}"
        );
        assert!(
            !html.contains(key_glyph),
            "html export must not contain the rendered key glyph, was: {html}"
        );

        // The buffer's own plain-text content must also hold the authored text, not the glyph.
        let plain_text = app.read_model(&buffer, |buffer, _| buffer.text().as_str().to_string());
        assert!(
            plain_text.contains("Cmd"),
            "buffer text must preserve authored text, was: {plain_text}"
        );
        assert!(
            !plain_text.contains(key_glyph),
            "buffer text must not contain the rendered key glyph, was: {plain_text}"
        );
    });
}

/// Nested `<kbd>` flat-collapses in the parser (issue #13733), so it serializes to the CANONICAL
/// FLAT form `<kbd>Ctrl+N</kbd>` — a single keycap over the inner content — rather than round-
/// tripping the authored nested `<kbd><kbd>…</kbd></kbd>` markup. The nesting is discarded at parse
/// time (the buffer model has no depth), so flat is the only faithful serialization; preserving the
/// authored nesting is the depth-aware work deferred to issue #13912.
#[test]
fn test_nested_kbd_serializes_to_flat_form() {
    App::test((), |mut app| async move {
        let markdown = "Press <kbd><kbd>Ctrl</kbd>+<kbd>N</kbd></kbd> now\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let exported = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert!(
            exported.contains("<kbd>Ctrl+N</kbd>"),
            "nested kbd should serialize to the flat form, was: {exported}"
        );
        // The flattened form must not re-emit the nested inner tags.
        assert!(
            !exported.contains("<kbd><kbd>") && !exported.contains("</kbd></kbd>"),
            "serialization must not reproduce nested kbd tags, was: {exported}"
        );
    });
}

/// `<kbd>` inside a GFM table cell round-trips through both the Markdown and HTML export paths.
#[test]
fn test_kbd_in_table_cell_serialization() {
    App::test((), |mut app| async move {
        let _flag = warp_core::features::FeatureFlag::MarkdownTables.override_enabled(true);
        let markdown = "\
| shortcut | action |\n\
| --- | --- |\n\
| <kbd>Cmd</kbd> | open palette |\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let exported = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        assert!(
            exported.contains("<kbd>Cmd</kbd>"),
            "exported markdown was: {exported}"
        );

        let html = app.read_model(&buffer, |buffer, ctx| {
            let range = CharOffset::from(1)..buffer.max_charoffset();
            buffer.ranges_as_html(Vec1::try_from_vec(vec![range]).unwrap(), ctx)
        });
        let html = html.unwrap();
        assert!(html.contains("<kbd>Cmd</kbd>"), "html was: {html}");
    });
}

/// A run that is both `<kbd>` and underlined must export properly nested Markdown tags (issue
/// #13733). The Markdown serializer closes `</kbd>` before `</u>`, so it must also open `<u>`
/// before `<kbd>` — otherwise a crossed `<kbd><u>…</kbd></u>` is emitted, which is malformed and
/// does not re-import to the same styles. This pins proper nesting and round-trip stability.
#[test]
fn test_kbd_underline_serializes_properly_nested() {
    App::test((), |mut app| async move {
        let markdown = "Press <u><kbd>Esc</kbd></u> now\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let exported = app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped());
        // Tags must be properly nested, not crossed. The close order is `</kbd></u>`, so the open
        // order must be `<u><kbd>`.
        assert!(
            exported.contains("<u><kbd>Esc</kbd></u>"),
            "kbd+underline run must serialize as properly nested tags, was: {exported}"
        );
        assert!(
            !exported.contains("<kbd><u>"),
            "serialization must not emit crossed <kbd><u> tags, was: {exported}"
        );

        // Round-trip: re-importing the exported Markdown must yield the same kbd+underline styles.
        let reparsed = parse_markdown(&exported).unwrap();
        let has_kbd_underline_run = reparsed.lines.iter().any(|line| match line {
            markdown_parser::FormattedTextLine::Line(fragments) => fragments
                .iter()
                .any(|f| f.styles.kbd && f.styles.underline && f.text.contains("Esc")),
            _ => false,
        });
        assert!(
            has_kbd_underline_run,
            "exported markdown must re-import to a kbd+underline run, was: {exported}"
        );
    });
}
