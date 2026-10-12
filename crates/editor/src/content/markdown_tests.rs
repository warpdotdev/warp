use std::sync::Arc;

use markdown_parser::{compute_formatted_text_delta, parse_markdown};
use serde_yaml::Value;
use string_offset::CharOffset;
use vec1::Vec1;
use warpui_core::{App, ReadModel};

use super::MarkdownStyle;
use crate::content::buffer::tests::TestEmbeddedItem;
use crate::content::buffer::{Buffer, BufferEditAction, EditOrigin, StyledBlockBoundaryBehavior};
use crate::content::text::{
    BlockType, BufferBlockStyle, IndentBehavior, TABLE_BLOCK_MARKDOWN_LANG,
};

/// The rendered row a source line resolves to, as text. Asserted on instead of a literal offset so
/// the tests pin the row a reader lands on rather than the buffer's offset arithmetic.
fn landed_row(buffer: &Buffer, source_line: usize) -> Option<String> {
    let offset = buffer.markdown_offset_for_source_line(source_line)?;
    let end = buffer
        .containing_line_end(offset)
        .min(buffer.max_charoffset());
    let row = buffer.text_in_range(offset..end).into_string();
    Some(row.trim_end_matches('\n').to_string())
}

#[test]
fn markdown_source_lines_map_to_rendered_buffer_offsets() {
    App::test((), |mut app| async move {
        let markdown = "Before\n```text\n=>\n---\n```\n<!--\nhidden\n-->\nAfter\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            // Rendered as: "Before" / "=>" / "---" / "After".
            assert_eq!(landed_row(buffer, 1).as_deref(), Some("Before"));
            // Code punctuation and a rule-like code line each keep their own row.
            assert_eq!(landed_row(buffer, 3).as_deref(), Some("=>"));
            assert_eq!(landed_row(buffer, 4).as_deref(), Some("---"));
            assert_eq!(landed_row(buffer, 9).as_deref(), Some("After"));
            // The hidden comment renders nothing, so it has nowhere to scroll to.
            assert_eq!(buffer.markdown_offset_for_source_line(6), None);
            assert_eq!(buffer.markdown_offset_for_source_line(7), None);
            assert_eq!(buffer.markdown_offset_for_source_line(8), None);
        });
    });
}

#[test]
fn markdown_table_separator_maps_to_the_table_header() {
    App::test((), |mut app| async move {
        let _flag = warp_core::features::FeatureFlag::MarkdownTables.override_enabled(true);
        let markdown = "| Header |\n| --- |\n| => |\nAfter\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            let header = buffer.markdown_offset_for_source_line(1);
            let separator = buffer.markdown_offset_for_source_line(2);

            // The separator row renders nothing of its own, so it shares the header's row.
            assert_eq!(separator, header);
            assert_eq!(landed_row(buffer, 1).as_deref(), Some("Header"));
            assert_eq!(landed_row(buffer, 3).as_deref(), Some("=>"));
            assert_eq!(landed_row(buffer, 4).as_deref(), Some("After"));
        });
    });
}

#[test]
fn markdown_source_map_does_not_count_hidden_link_destinations() {
    App::test((), |mut app| async move {
        let markdown = "[Warp](https://example.com/needle)\n\nclicked needle\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            assert_eq!(landed_row(buffer, 1).as_deref(), Some("Warp"));
            assert_eq!(landed_row(buffer, 3).as_deref(), Some("clicked needle"));
        });
    });
}

/// A thematic break lowers to a block item that absorbs the blank line after it, so the buffer
/// renders fewer rows than the parsed line count.
#[test]
fn markdown_source_map_survives_thematic_breaks() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "A\n\n---\n\nB\n\nC\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            assert_eq!(landed_row(buffer, 1).as_deref(), Some("A"));
            assert_eq!(landed_row(buffer, 5).as_deref(), Some("B"));
            assert_eq!(landed_row(buffer, 7).as_deref(), Some("C"));
        });
    });
}

/// As above for block-level images, including the last source line.
#[test]
fn markdown_source_map_survives_block_images() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "A\n\n![x](y.png)\n\nB\n\nC\n\nD\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            assert_eq!(landed_row(buffer, 1).as_deref(), Some("A"));
            assert_eq!(landed_row(buffer, 5).as_deref(), Some("B"));
            assert_eq!(landed_row(buffer, 7).as_deref(), Some("C"));
            assert_eq!(landed_row(buffer, 9).as_deref(), Some("D"));
        });
    });
}

/// An embedded object whose conversion yields nothing renders no row at all.
#[test]
fn markdown_source_map_survives_dropped_embedded_objects() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "A\n\n```warp-embedded-object\nid: abc\n```\n\nB\n\nC\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            assert_eq!(landed_row(buffer, 1).as_deref(), Some("A"));
            assert_eq!(landed_row(buffer, 7).as_deref(), Some("B"));
            assert_eq!(landed_row(buffer, 9).as_deref(), Some("C"));
        });
    });
}

/// Several block-level items in one document, whose row differences would otherwise compound.
#[test]
fn markdown_source_map_survives_repeated_block_items() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "Intro\n\n![i](a.png)\n\n---\n\n![j](b.png)\n\nTail\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            assert_eq!(landed_row(buffer, 1).as_deref(), Some("Intro"));
            assert_eq!(landed_row(buffer, 9).as_deref(), Some("Tail"));
        });
    });
}

/// Source lines resolve to the first character of their row, not to the newline that ends the row
/// above it.
#[test]
fn markdown_source_offsets_point_at_the_start_of_their_row() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "First\n\nSecond\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            for (source_line, expected) in [(1, "First"), (3, "Second")] {
                let offset = buffer
                    .markdown_offset_for_source_line(source_line)
                    .expect("line should resolve");
                let end = buffer
                    .max_charoffset()
                    .min(offset + expected.chars().count());
                assert_eq!(buffer.text_in_range(offset..end).into_string(), expected);
            }
        });
    });
}

#[test]
fn markdown_source_map_is_invalidated_after_an_edit() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "First\nSecond\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        app.read_model(&buffer, |buffer, _| {
            assert!(buffer.markdown_offset_for_source_line(2).is_some());
        });

        buffer.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "changed",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                selection,
                ctx,
            );
            assert_eq!(buffer.markdown_offset_for_source_line(2), None);
        });
    });
}

/// Syntax highlighting only writes colors, which are never serialized back to Markdown, so it must
/// not discard the source map. It runs asynchronously after load.
#[test]
fn markdown_source_map_survives_code_block_highlighting() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "Intro\n\n```sh\necho a\n```\n\nTarget line\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let before = app.read_model(&buffer, |buffer, _| {
            buffer.markdown_offset_for_source_line(7)
        });
        assert!(
            before.is_some(),
            "target should resolve before highlighting"
        );

        buffer.update(&mut app, |buffer, ctx| {
            let code_block_start = buffer
                .outline_blocks()
                .into_iter()
                .find(|block| {
                    matches!(
                        &block.block_type,
                        BlockType::Text(BufferBlockStyle::CodeBlock { .. })
                    )
                })
                .expect("document has a code block")
                .start;
            buffer.color_code_block_ranges(code_block_start + 1, &[], selection.clone(), ctx);
        });

        app.read_model(&buffer, |buffer, _| {
            assert_eq!(buffer.markdown_offset_for_source_line(7), before);
            assert_eq!(landed_row(buffer, 7).as_deref(), Some("Target line"));
        });
    });
}

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
fn test_apply_formatted_text_delta_replaces_content_with_empty_document() {
    App::test((), |mut app| async move {
        let old_markdown = "hello world\n";
        let (buffer, selection) = Buffer::mock_from_markdown(
            old_markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        let old_formatted = app.read_model(&buffer, |buffer, _| {
            buffer.range_to_formatted_text(
                CharOffset::from(1)..buffer.max_charoffset(),
                StyledBlockBoundaryBehavior::Inclusive,
            )
        });
        let delta = compute_formatted_text_delta(old_formatted, parse_markdown("").unwrap());

        buffer.update(&mut app, |buffer, ctx| {
            buffer.apply_formatted_text_delta(&delta, selection, ctx);
        });

        assert_eq!(
            app.read_model(&buffer, |buffer, _| buffer.markdown_unescaped()),
            ""
        );
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
