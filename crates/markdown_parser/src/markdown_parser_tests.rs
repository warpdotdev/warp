use nom::{
    Finish,
    error::{VerboseError, convert_error},
};
use serde_yaml::Mapping;

use crate::{
    CustomWeight, FormattedTable, FormattedTextStyles, LineCount, compute_formatted_text_delta,
};

use super::*;

// Simple transformer to make testing easier.
fn test_parse_markdown(source: &str) -> Vec<FormattedTextLine> {
    parse_all(source, |input| parse_markdown_internal(input, false))
}

fn test_parse_markdown_with_gfm_tables(source: &str) -> Vec<FormattedTextLine> {
    parse_all(source, |input| parse_markdown_internal(input, true))
}

#[test]
fn test_parse_empty() {
    assert_eq!(test_parse_markdown(""), vec![]);
}

#[test]
fn test_parse_line_endings() {
    // This is a regression test for CORE-1958.
    let parsed = parse_markdown("First\rSecond\nThird\r\nFourth").expect("Markdown should parse");
    assert_eq!(
        parsed.lines,
        &[
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("First")]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("Second")]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("Third")]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("Fourth")]),
        ]
    )
}

#[test]
fn test_parse_single_line() {
    // Ensure we can parse without a trailing newline.
    assert_eq!(
        test_parse_markdown("A single **line** of ~~text~~! <u>Hooray!</u>"),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("A single "),
            FormattedTextFragment::bold("line"),
            FormattedTextFragment::plain_text(" of "),
            FormattedTextFragment::strikethrough("text"),
            FormattedTextFragment::plain_text("! "),
            FormattedTextFragment::underline("Hooray!"),
        ])]
    )
}

#[test]
fn test_parse_headers() {
    let source = "# This is a header";
    assert_eq!(
        parse_all(source, parse_header),
        FormattedTextHeader {
            heading_size: 1,
            text: vec![FormattedTextFragment::plain_text("This is a header")]
        }
    );
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Heading(FormattedTextHeader {
            heading_size: 1,
            text: vec![FormattedTextFragment::plain_text("This is a header")]
        })]
    );

    let source = "### This is a header";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Heading(FormattedTextHeader {
            heading_size: 3,
            text: vec![FormattedTextFragment::plain_text("This is a header")]
        })]
    );

    let source = "### This is a __header__";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Heading(FormattedTextHeader {
            heading_size: 3,
            text: vec![
                FormattedTextFragment::plain_text("This is a "),
                FormattedTextFragment::bold("header")
            ]
        })]
    );

    let source = "#### This is a header";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Heading(FormattedTextHeader {
            heading_size: 4,
            text: vec![FormattedTextFragment::plain_text("This is a header")]
        })]
    );

    let source = "##### This is a header";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Heading(FormattedTextHeader {
            heading_size: 5,
            text: vec![FormattedTextFragment::plain_text("This is a header")]
        })]
    );

    let source = "###### This is a header";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Heading(FormattedTextHeader {
            heading_size: 6,
            text: vec![FormattedTextFragment::plain_text("This is a header")]
        })]
    );

    let source = "####### This is a header";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("####### This is a header")
        ])]
    );

    let source = "### This is a [lin*ked*](http://example.com) header";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Heading(FormattedTextHeader {
            heading_size: 3,
            text: vec![
                FormattedTextFragment::plain_text("This is a "),
                FormattedTextFragment::hyperlink("lin", "http://example.com"),
                FormattedTextFragment {
                    text: "ked".to_string(),
                    styles: FormattedTextStyles {
                        italic: true,
                        hyperlink: Some(Hyperlink::Url("http://example.com".to_string())),
                        ..Default::default()
                    }
                },
                FormattedTextFragment::plain_text(" header")
            ]
        })]
    )
}

#[test]
fn test_parse_horizontal_rule() {
    let source = "***\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::HorizontalRule]
    );

    // One to three spaces indent are allowed.
    let source = "   ---\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::HorizontalRule]
    );

    // Four spaces is too many.
    let source = "    ---\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("    ---")
        ])]
    );

    // More than three characters may be used.
    let source = "*************\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::HorizontalRule]
    );

    // Spaces are allowed between the characters and at the end.
    let source = " _  __     ___ \n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::HorizontalRule]
    );

    // It is required that all of the non-space characters be the same.
    let source = " _  __     --_ \n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(" _  __     --_ ")
        ])]
    );

    // However, no other characters may occur in the line.
    let source = " _  _a     ___ \n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(" _  _a     ___ ")
        ])]
    );

    // When both a horizontal rule and a list item are possible interpretations of a line, the horizontal rule takes precedence.
    let source = " * * *\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::HorizontalRule]
    );
}

#[test]
fn test_parse_image_prefix_for_inline_runs() {
    let input = "![Alt](image.png \"caption\") trailing";
    let (rest, image) = parse_image_prefix(input).expect("expected image prefix");
    assert_eq!(rest, " trailing");
    assert_eq!(
        image,
        FormattedImage {
            alt_text: "Alt".to_string(),
            source: "image.png".to_string(),
            title: Some("caption".to_string()),
        }
    );
}

#[test]
fn test_parse_image_run_line_single_image() {
    assert_eq!(
        parse_image_run_line("   ![Alt](image.png \"caption\")\n"),
        Some(vec![FormattedImage {
            alt_text: "Alt".to_string(),
            source: "image.png".to_string(),
            title: Some("caption".to_string()),
        }])
    );
}

#[test]
fn test_parse_image_run_line_multiple_images() {
    assert_eq!(
        parse_image_run_line("![One](one.png)   ![Two](two.png \"caption\")\n"),
        Some(vec![
            FormattedImage {
                alt_text: "One".to_string(),
                source: "one.png".to_string(),
                title: None,
            },
            FormattedImage {
                alt_text: "Two".to_string(),
                source: "two.png".to_string(),
                title: Some("caption".to_string()),
            },
        ])
    );
}

#[test]
fn test_parse_image_run_line_rejects_mixed_text() {
    assert_eq!(
        parse_image_run_line("Intro ![One](one.png) ![Two](two.png)\n"),
        None
    );
}
#[test]
fn test_parse_task_list() {
    let source = "- [ ] This is a list";
    let ctx = std::cell::RefCell::new(ListIndentationContext::new());
    assert_eq!(
        parse_all(source, |input| parse_task_list(input, &ctx)),
        FormattedTaskList {
            complete: false,
            indent_level: 0,
            text: vec![FormattedTextFragment::plain_text("This is a list")]
        }
    );
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::TaskList(FormattedTaskList {
            complete: false,
            indent_level: 0,
            text: vec![FormattedTextFragment::plain_text("This is a list")]
        })]
    );

    let source = "- [x] This is a list";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::TaskList(FormattedTaskList {
            complete: true,
            indent_level: 0,
            text: vec![FormattedTextFragment::plain_text("This is a list")]
        })]
    );

    let source = "        - [X] This is a list";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::TaskList(FormattedTaskList {
            complete: true,
            indent_level: 0,
            text: vec![FormattedTextFragment::plain_text("This is a list")]
        })]
    );

    let source = "- [x] This is a __list__";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::TaskList(FormattedTaskList {
            complete: true,
            indent_level: 0,
            text: vec![
                FormattedTextFragment::plain_text("This is a "),
                FormattedTextFragment::bold("list")
            ]
        })]
    );

    let source = "- [] This is a list";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::UnorderedList(
            FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text("[] This is a list")]
            }
        )]
    );
}

#[test]
fn test_parse_ordered_list() {
    let source = "5. First\n6. Second\n    2. A\n    3. B\n7. Third";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: Some(5),
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("First")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                // This is not the first item at its indent level, so the number is discarded.
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("Second")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                // This is the first item at its indent level, so the number is preserved.
                number: Some(2),
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("A")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                // This is not the first item at its indent level, so the number is discarded.
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("B")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                // This is a continuation of the list, so the number is discarded.
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("Third")]
                }
            }),
        ]
    );

    // Only Arabic numerals are allowed for list markers.
    let source = "1. First\nb. Second";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: Some(1),
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("First")]
                }
            }),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("b. Second")])
        ]
    )
}

#[test]
fn test_formatted_text() {
    let source = "This is _body_";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::italic("body")
        ])]
    );

    let source = "This is _body";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is _body"),
        ])]
    );

    let source = "This is _body _body_";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is _body "),
            FormattedTextFragment::italic("body"),
        ])]
    );

    let source = "This is *italic****body***";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::italic("italic"),
            FormattedTextFragment::bold_italic("body")
        ])]
    );
}

#[test]
fn test_multi_line() {
    let source = "# This is a header\nThis is body";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 1,
                text: vec![FormattedTextFragment::plain_text("This is a header")]
            }),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("This is body")])
        ]
    );

    let source = "# This is a header\nThis is _italic_ body\n## This is **bold** subheader";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 1,
                text: vec![FormattedTextFragment::plain_text("This is a header")]
            }),
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("This is "),
                FormattedTextFragment::italic("italic"),
                FormattedTextFragment::plain_text(" body")
            ]),
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 2,
                text: vec![
                    FormattedTextFragment::plain_text("This is "),
                    FormattedTextFragment::bold("bold"),
                    FormattedTextFragment::plain_text(" subheader")
                ]
            })
        ]
    );
}

#[test]
fn test_line_break() {
    let source = "\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::LineBreak]
    );

    let source = "\nThis is body";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::LineBreak,
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("This is body")])
        ]
    );

    let source = "\nThis is body\n\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::LineBreak,
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("This is body")]),
            FormattedTextLine::LineBreak,
        ]
    );

    let source = "\n\nThis is body";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::LineBreak,
            FormattedTextLine::LineBreak,
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("This is body")]),
        ]
    );

    let source = "abc\n\n\ndef";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("abc")]),
            // The first newline ends the "abc" paragraph, and the next 2 are line breaks.
            FormattedTextLine::LineBreak,
            FormattedTextLine::LineBreak,
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("def")]),
        ]
    );
}

#[test]
fn test_special_chars() {
    // This tests that we can parse:
    // - A non-escaping literal backslash
    // - ASCII punctuation that isn't formatting (:)
    // - Emoji
    assert_eq!(
        test_parse_markdown("multi\\byte: **💙**"),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("multi\\byte: "),
            FormattedTextFragment::bold("💙")
        ])]
    )
}

#[test]
fn test_parse_ordered_list_tag() {
    let source = "     1. This is a list\n";
    assert_eq!(
        parse(source, parse_ordered_list_tag),
        ("This is a list\n", (5, "1"))
    );
}

#[test]
fn test_parse_unordered_list_tag() {
    let source = "    * List item\n";
    assert_eq!(parse(source, parse_unordered_list_tag), ("List item\n", 4))
}

#[test]
fn test_parse_code_block_lang() {
    let source = "```sample_lang\nsome code\n```";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "sample_lang".to_string(),
            code: "some code\n".to_string()
        }),]
    );
}

#[test]
fn test_parse_table_block_lang() {
    let source = format!("```{TABLE_BLOCK_MARKDOWN_LANG}\nname\tage\nalice\t30\n```");
    assert_eq!(
        test_parse_markdown(source.as_str()),
        vec![FormattedTextLine::Table(
            FormattedTable::from_internal_format("name\tage\nalice\t30\n")
        )]
    );
}

#[test]
fn test_parse_code_block_missing_lang() {
    let source = "```\nsome code\n```";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: RUNNABLE_BLOCK_MARKDOWN_LANG.to_string(),
            code: "some code\n".to_string()
        }),]
    );
}

#[test]
fn test_parse_code_block_whitespace_lang() {
    let source = "```   \nsome code\n```";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: RUNNABLE_BLOCK_MARKDOWN_LANG.to_string(),
            code: "some code\n".to_string()
        }),]
    );
}

#[test]
fn test_parse_code_block_with_preceding_whitespace() {
    let source = "   ```rust\nsome code\n```";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "rust".to_string(),
            code: "some code\n".to_string()
        }),]
    );
}

#[test]
fn test_parse_code_block_with_trailing_whitespace() {
    let source = "   ```rust\nsome code\n``` ";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "rust".to_string(),
            code: "some code\n".to_string()
        }),]
    );
}

#[test]
fn test_parse_code_block_with_backticks() {
    let source = "```md\nan `inner` backtick\n```";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "md".to_string(),
            code: "an `inner` backtick\n".to_string()
        })]
    )
}

#[test]
fn test_parse_code_block_body() {
    let source = "```json
        {
          firstName: John,
          lastName: Smith,
          age: 25
        }
```
        ";
    assert_eq!(
        parse(source, parse_code_block),
        (
            "        ",
            (
                "json",
                "        {\n          firstName: John,\n          lastName: Smith,\n          age: 25\n        }\n"
                    .to_string()
            )
        ),
    );
}

#[test]
fn test_consec_code_blocks() {
    let source = "```\n hello\n```\n```\nwhat\n```\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::CodeBlock(CodeBlockText {
                lang: RUNNABLE_BLOCK_MARKDOWN_LANG.to_string(),
                code: " hello\n".to_string()
            }),
            FormattedTextLine::CodeBlock(CodeBlockText {
                lang: RUNNABLE_BLOCK_MARKDOWN_LANG.to_string(),
                code: "what\n".to_string()
            })
        ]
    );
}

#[test]
fn test_parse_italics_text_with_underscore() {
    let source = "first_thing second_thing";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("first_thing second_thing"),
        ])]
    );
}

#[test]
fn test_parse_bold_text_with_underscore() {
    let source = "first__thing second__thing";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("first__thing second__thing"),
        ])]
    );
}

#[test]
fn test_parse_italics_text_with_one_space_and_underscore() {
    let source = "first_thing second_ thing";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("first_thing second_ thing"),
        ])]
    );
}

#[test]
fn test_parse_italics_text_with_two_spaces_and_underscore() {
    let source = "first _thing second_ thing";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("first "),
            FormattedTextFragment::italic("thing second"),
            FormattedTextFragment::plain_text(" thing")
        ])]
    );
}

#[test]
fn test_parse_italics_on_new_line() {
    let source = "_thing\n _hello";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("_thing"),]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(" _hello")])
        ]
    );
}

#[test]
fn test_parse_bold_on_new_line() {
    let source = "__thing\n __hello";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("__thing"),]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(" __hello")])
        ]
    );
}

#[test]
fn test_parse_italics_star_on_new_line() {
    let source = "*thing\n *hello";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("*thing"),]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(" *hello")])
        ]
    );
}

#[test]
fn test_italics_quotes() {
    let source = "_```_";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::italic("```"),
        ])]
    );
}

#[test]
fn test_inline_code() {
    let source = "`hi`";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::inline_code("hi"),
        ])]
    );
}

#[test]
fn test_interior_inline_code() {
    assert_eq!(
        test_parse_markdown("word`code`word"),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("word"),
            FormattedTextFragment::inline_code("code"),
            FormattedTextFragment::plain_text("word"),
        ])]
    )
}

#[test]
fn test_code_block_with_underscore() {
    let source = "```\n hello_there\n```\n```\nwhat_is\n```\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::CodeBlock(CodeBlockText {
                lang: RUNNABLE_BLOCK_MARKDOWN_LANG.to_string(),
                code: " hello_there\n".to_string()
            }),
            FormattedTextLine::CodeBlock(CodeBlockText {
                lang: RUNNABLE_BLOCK_MARKDOWN_LANG.to_string(),
                code: "what_is\n".to_string()
            })
        ]
    );
}

#[test]
fn test_basic_parse_hyperlink() {
    let source = "This is [body](url)";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::hyperlink("body", "url"),
        ])]
    );
}

#[test]
fn test_multi_parse_hyperlink() {
    let source = "This is [body1](url1) and [body2](url2)";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::hyperlink("body1", "url1"),
            FormattedTextFragment::plain_text(" and "),
            FormattedTextFragment::hyperlink("body2", "url2"),
        ])]
    );
}

#[test]
fn test_parse_link_in_tag() {
    let source = "a [https://google.com](https://google.com) link";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("a "),
            FormattedTextFragment::hyperlink("https://google.com", "https://google.com"),
            FormattedTextFragment::plain_text(" link")
        ])]
    );

    let source = "a [https://google.com](https://warp.dev) link";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("a "),
            FormattedTextFragment::hyperlink("https://google.com", "https://warp.dev"),
            FormattedTextFragment::plain_text(" link")
        ])]
    );
}

#[test]
fn test_basic_parse_url() {
    let source = "This is www.google.com";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::hyperlink("www.google.com", "www.google.com"),
        ])]
    );
}

#[test]
fn test_multi_parse_url() {
    let source = "This is www.google.com and https://www.google.com and http://www.google.com";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::hyperlink("www.google.com", "www.google.com"),
            FormattedTextFragment::plain_text(" and "),
            FormattedTextFragment::hyperlink("https://www.google.com", "https://www.google.com"),
            FormattedTextFragment::plain_text(" and "),
            FormattedTextFragment::hyperlink("http://www.google.com", "http://www.google.com"),
        ])]
    );
}

#[test]
fn test_autolink_with_escaped_dot() {
    // After a save cycle, `parse_url` may see `https://google\.com` (with `\.` from
    // the serializer's escape of `.`). The URL should be unescaped so the buffer
    // stores the clean string and the round-trip is stable.
    assert_eq!(
        test_parse_markdown("https://google\\.com"),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::hyperlink("https://google.com", "https://google.com"),
        ])]
    );
}

#[test]
fn test_autolink_with_escaped_dash() {
    assert_eq!(
        test_parse_markdown("https://warp\\-dev.com"),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::hyperlink("https://warp-dev.com", "https://warp-dev.com"),
        ])]
    );
}

#[test]
fn test_autolink_with_multiple_escapes() {
    // Multiple escaped characters (`.`, `-`, `#`) should all be unescaped.
    assert_eq!(
        test_parse_markdown("https://github\\.com/foo\\-bar\\#anchor"),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::hyperlink(
                "https://github.com/foo-bar#anchor",
                "https://github.com/foo-bar#anchor"
            ),
        ])]
    );
}

#[test]
fn test_autolink_in_link_label_with_escapes() {
    // When an escaped URL appears as the display text of an explicit link
    // (the `can_autolink = false` path), it should also be unescaped.
    let source = "[https://google\\.com](https://google.com)";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::hyperlink("https://google.com", "https://google.com"),
        ])]
    );
}

#[test]
fn test_autolink_escape_roundtrip_is_stable() {
    // Parsing a URL with no escapes should produce the same result as
    // parsing the same URL with `\.` in place of `.` (simulating a save cycle).
    // Both should yield the clean, unescaped URL.
    let clean = test_parse_markdown("https://google.com");
    let escaped = test_parse_markdown("https://google\\.com");
    assert_eq!(clean, escaped);
}

#[test]
fn test_parse_url_preserves_non_escaped_backslash() {
    // A backslash followed by a non-punctuation character is not a markdown escape
    // sequence and should be left as-is.
    let source = "https://example.com/path\\nname";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::hyperlink(
                "https://example.com/path\\nname",
                "https://example.com/path\\nname"
            ),
        ])]
    );
}

#[test]
fn test_parse_url_embedded() {
    let source = "This iswww.google.comabc";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This iswww.google.comabc"),
        ])]
    );
}

#[test]
fn test_basic_parse_strikethrough() {
    let source = "~~This is some text~~";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::strikethrough("This is some text"),
        ])]
    );
}

#[test]
fn test_mixed_parse_strikethrough() {
    let source = "This is ~~test~~ **with** *text*";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::strikethrough("test"),
            FormattedTextFragment::plain_text(" "),
            FormattedTextFragment::bold("with"),
            FormattedTextFragment::plain_text(" "),
            FormattedTextFragment::italic("text"),
        ])]
    );
}

#[test]
fn test_multi_parse_strikethrough() {
    let source = "~~test1~~\n~~test2~~";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::strikethrough("test1"),]),
            FormattedTextLine::Line(vec![FormattedTextFragment::strikethrough("test2"),])
        ]
    );
}

#[test]
fn test_parse_unclosed_strikethrough() {
    let source = "~~**test1**";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("~~"),
            FormattedTextFragment::bold("test1")
        ])]
    );
}

#[test]
fn test_parse_escapes() {
    let source = "This is \\*not\\* italic. *This* is marked by \\* though";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is *not* italic. "),
            FormattedTextFragment::italic("This"),
            FormattedTextFragment::plain_text(" is marked by * though")
        ])]
    );
}

#[test]
fn test_parse_escape_in_style() {
    let source = "Some *styled \\* escaped* [te\\]xt\\^](https://warp.dev)";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("Some "),
            FormattedTextFragment::italic("styled * escaped"),
            FormattedTextFragment::plain_text(" "),
            FormattedTextFragment::hyperlink("te]xt^", "https://warp.dev")
        ])]
    );
}

#[test]
fn test_parser_raw_text_headers() {
    let parsed_text = parse_markdown("## New Header\n### Subheader").expect("Should parse");
    assert_eq!(parsed_text.raw_text(), "New Header\nSubheader\n");
}

#[test]
fn test_parser_raw_text_italic_bold() {
    let parsed_text = parse_markdown("This is ~~test~~ **with** *text*").expect("should parse");
    assert_eq!(parsed_text.raw_text(), "This is test with text\n");
}

#[test]
fn test_parser_raw_text_escaped_chars() {
    let parsed_text = parse_markdown("This is \\*not\\* italic. *This* is marked by \\* though")
        .expect("should parse");
    assert_eq!(
        parsed_text.raw_text(),
        "This is *not* italic. This is marked by * though\n"
    );
}

#[test]
fn test_parser_raw_text_url_hyperlinks() {
    let parsed_text =
        parse_markdown("This is [body1](url1) and [body2](url2)").expect("should parse");
    assert_eq!(parsed_text.raw_text(), "This is body1 and body2\n");
}

#[test]
fn test_parse_hash_in_code() {
    let parsed_text =
        parse_markdown("```bash\nls -l\n# a comment\npwd\n```\n").expect("should parse");

    assert_eq!(
        &parsed_text.lines,
        &[FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "bash".to_string(),
            code: "ls -l\n# a comment\npwd\n".to_string()
        })]
    );
}

/// Parse `input` completely using `parser`.
fn parse_all<'a, O>(
    input: &'a str,
    parser: impl FnMut(&'a str) -> IResult<&'a str, O, VerboseError<&'a str>>,
) -> O {
    let (remaining, result) = parse(input, parser);
    assert_eq!(remaining, "", "Parser should consume all input");
    result
}

/// Parse `input` using `parser`.
fn parse<'a, O>(
    input: &'a str,
    mut parser: impl FnMut(&'a str) -> IResult<&'a str, O, VerboseError<&'a str>>,
) -> (&'a str, O) {
    match parser(input).finish() {
        Ok(result) => result,
        Err(err) => {
            let trace = convert_error(input, err);
            panic!("Parse failed:\n{trace}");
        }
    }
}

#[test]
fn test_indented_code_block() {
    let source = "    - Item\n    ```\n    code\n      indented\n    ```";
    let result = test_parse_markdown(source);

    assert_eq!(result.len(), 2);
    match &result[0] {
        FormattedTextLine::UnorderedList(_) => {} // OK
        _ => panic!("Expected UnorderedList, got {:?}", result[0]),
    }

    match &result[1] {
        FormattedTextLine::CodeBlock(block) => {
            assert_eq!(block.code, "code\n  indented\n");
        }
        _ => panic!("Expected CodeBlock, got {:?}", result[1]),
    }
}

#[test]
fn test_code_block_indentation_stripping() {
    let source = "    ```\n    line1\n      line2\n   line3\n    ```";
    let result = test_parse_markdown(source);
    assert_eq!(result.len(), 1);
    if let FormattedTextLine::CodeBlock(block) = &result[0] {
        assert_eq!(block.code, "line1\n  line2\nline3\n");
    } else {
        panic!("Expected CodeBlock, got {:?}", result[0]);
    }
}

#[test]
fn test_deeply_indented_code_block() {
    // 8 spaces indentation
    let source = "        ```
        line1
        line2
        ```";
    let result = test_parse_markdown(source);
    assert_eq!(result.len(), 1);
    if let FormattedTextLine::CodeBlock(block) = &result[0] {
        assert_eq!(block.code, "line1\nline2\n");
    } else {
        panic!("Expected CodeBlock, got {:?}", result[0]);
    }
}

#[test]
fn test_variable_indentation_code_block() {
    // Opening fence indented 4 spaces
    // Content has varying indentation (4, 6, 2 spaces)
    // Closing fence indented 4 spaces (matching)
    let source = "    ```
    line1
      line2
  line3
    ```";
    let result = test_parse_markdown(source);
    assert_eq!(result.len(), 1);
    if let FormattedTextLine::CodeBlock(block) = &result[0] {
        // line1: 4 spaces -> stripped (0 indent)
        // line2: 6 spaces -> 4 stripped (2 indent)
        // line3: 2 spaces -> 2 stripped (0 indent) - Wait, min(4, 2) = 2 stripped.
        assert_eq!(block.code, "line1\n  line2\nline3\n");
    } else {
        panic!("Expected CodeBlock, got {:?}", result[0]);
    }
}

#[test]
fn test_closing_fence_less_indented() {
    let source = "    ```
    content
```";
    let result = test_parse_markdown(source);
    assert_eq!(result.len(), 1);
    if let FormattedTextLine::CodeBlock(block) = &result[0] {
        assert_eq!(block.code, "content\n");
    } else {
        panic!("Expected CodeBlock, got {:?}", result[0]);
    }
}

#[test]
fn test_closing_fence_indented_3_extra_spaces() {
    // Opening fence: 4 spaces
    // Content: "content" (4 spaces)
    // Closing fence: 7 spaces (4 + 3). Should close.
    let source = "    ```
    content
       ```";
    let result = test_parse_markdown(source);
    assert_eq!(result.len(), 1);
    if let FormattedTextLine::CodeBlock(block) = &result[0] {
        assert_eq!(block.code, "content\n");
    } else {
        panic!("Expected CodeBlock, got {:?}", result[0]);
    }
}

#[test]
fn test_closing_fence_indented_4_extra_spaces() {
    // Opening fence: 4 spaces
    // Content line: "    ```" (8 spaces). Should NOT close.
    // Real closing fence: 4 spaces.
    let source = "    ```
    text
        ```
    ```";
    let result = test_parse_markdown(source);
    assert_eq!(result.len(), 1);
    if let FormattedTextLine::CodeBlock(block) = &result[0] {
        // The middle line has 8 spaces. 4 are stripped.
        // Remaining content is "    ```\n"
        assert_eq!(block.code, "text\n    ```\n");
    } else {
        panic!("Expected CodeBlock, got {:?}", result[0]);
    }
}

#[test]
fn test_code_span() {
    assert_eq!(parse_all("`foo \\bar`", parse_code_span), "foo \\bar");
    assert_eq!(parse_all("``an inner ` ``", parse_code_span), "an inner ` ");
}

#[test]
fn test_parse_inline() {
    assert_eq!(
        parse_all("*emphasis*", parse_inline),
        vec![FormattedTextFragment::italic("emphasis")]
    );

    assert_eq!(
        parse_all("em*pha*sis", parse_inline),
        vec![
            FormattedTextFragment::plain_text("em"),
            FormattedTextFragment::italic("pha"),
            FormattedTextFragment::plain_text("sis")
        ]
    );

    assert_eq!(
        parse_all("***nested** stuff*", parse_inline),
        vec![
            FormattedTextFragment::bold_italic("nested"),
            FormattedTextFragment::italic(" stuff")
        ]
    );

    assert_eq!(
        parse_all("**strong**", parse_inline),
        vec![FormattedTextFragment::bold("strong")]
    );

    // https://spec.commonmark.org/0.30/#example-351
    assert_eq!(
        parse_all("a * foo bar*", parse_inline),
        vec![FormattedTextFragment::plain_text("a * foo bar*")]
    );

    // https://spec.commonmark.org/0.30/#example-352
    assert_eq!(
        parse_all("a*\"foo\"*", parse_inline),
        vec![FormattedTextFragment::plain_text("a*\"foo\"*")]
    );

    // Rule 2: // https://spec.commonmark.org/0.30/#example-356 - 363
    assert_eq!(
        parse_all("_foo bar_", parse_inline),
        vec![FormattedTextFragment::italic("foo bar")]
    );
    assert_eq!(
        parse_all("_ foo bar_", parse_inline),
        vec![FormattedTextFragment::plain_text("_ foo bar_")]
    );
    assert_eq!(
        parse_all("a_\"foo\"_", parse_inline),
        vec![FormattedTextFragment::plain_text("a_\"foo\"_")]
    );
    assert_eq!(
        parse_all("foo_bar_", parse_inline),
        vec![FormattedTextFragment::plain_text("foo_bar_")]
    );

    // Rule 3
    assert_eq!(
        parse_all("_foo*", parse_inline),
        vec![FormattedTextFragment::plain_text("_foo*")]
    );

    // Bold/italic/combined examples
    assert_eq!(
        parse_all("***strong emph***", parse_inline),
        vec![FormattedTextFragment::bold_italic("strong emph")]
    );
    assert_eq!(
        parse_all("***strong** in emph*", parse_inline),
        vec![
            FormattedTextFragment::bold_italic("strong"),
            FormattedTextFragment::italic(" in emph")
        ]
    );
    assert_eq!(
        parse_all("***emph* in strong**", parse_inline),
        vec![
            FormattedTextFragment::bold_italic("emph"),
            FormattedTextFragment::bold(" in strong")
        ]
    );
    assert_eq!(
        parse_all("**in strong *emph***", parse_inline),
        vec![
            FormattedTextFragment::bold("in strong "),
            FormattedTextFragment::bold_italic("emph")
        ]
    );
    assert_eq!(
        parse_all("*in emph **strong***", parse_inline),
        vec![
            FormattedTextFragment::italic("in emph "),
            FormattedTextFragment::bold_italic("strong")
        ]
    );

    assert_eq!(
        parse_all("*Complicated **text*** with *nest**ing***", parse_inline),
        vec![
            FormattedTextFragment::italic("Complicated "),
            FormattedTextFragment::bold_italic("text"),
            FormattedTextFragment::plain_text(" with "),
            FormattedTextFragment::italic("nest"),
            FormattedTextFragment::bold_italic("ing")
        ]
    );

    // https://spec.commonmark.org/0.30/#example-363
    assert_eq!(
        parse_all("foo-_(bar)_", parse_inline),
        vec![
            FormattedTextFragment::plain_text("foo-"),
            FormattedTextFragment::italic("(bar)")
        ]
    );

    // https://spec.commonmark.org/0.30/#example-367
    assert_eq!(
        parse_all("*(*foo)", parse_inline),
        vec![FormattedTextFragment::plain_text("*(*foo)")]
    );

    // https://spec.commonmark.org/0.30/#example-372
    assert_eq!(
        parse_all("_(_foo_)_", parse_inline),
        vec![FormattedTextFragment::italic("(foo)")]
    );

    // https://spec.commonmark.org/0.30/#example-382
    assert_eq!(
        parse_all("__ foo bar__", parse_inline),
        vec![FormattedTextFragment::plain_text("__ foo bar__")]
    );

    // https://spec.commonmark.org/0.30/#example-410
    assert_eq!(
        parse_all("*foo**bar**baz*", parse_inline),
        vec![
            FormattedTextFragment::italic("foo"),
            FormattedTextFragment::bold_italic("bar"),
            FormattedTextFragment::italic("baz")
        ]
    );
    assert_eq!(
        parse_all("*foo**bar*", parse_inline),
        vec![FormattedTextFragment::italic("foo**bar")]
    );

    assert_eq!(
        parse_all("**foo* bar*", parse_inline),
        vec![FormattedTextFragment::italic("foo bar")]
    );
}

#[test]
fn test_parse_inline_link() {
    assert_eq!(
        parse_all("[basic](https://warp.dev)", parse_inline),
        vec![FormattedTextFragment::hyperlink(
            "basic",
            "https://warp.dev"
        )]
    );

    assert_eq!(
        parse_all("**Bold [link](http://example.com) here**", parse_inline),
        vec![
            FormattedTextFragment::bold("Bold "),
            FormattedTextFragment {
                text: "link".to_string(),
                styles: FormattedTextStyles {
                    weight: Some(CustomWeight::Bold),
                    hyperlink: Some(Hyperlink::Url("http://example.com".to_string())),
                    ..Default::default()
                }
            },
            FormattedTextFragment::bold(" here"),
        ]
    );

    assert_eq!(
        parse_all(
            "A [styl*ed* link](https://example.com) is **nice**.",
            parse_inline
        ),
        vec![
            FormattedTextFragment::plain_text("A "),
            FormattedTextFragment::hyperlink("styl", "https://example.com"),
            FormattedTextFragment {
                text: "ed".to_string(),
                styles: FormattedTextStyles {
                    italic: true,
                    hyperlink: Some(Hyperlink::Url("https://example.com".to_string())),
                    ..Default::default()
                },
            },
            FormattedTextFragment::hyperlink(" link", "https://example.com"),
            FormattedTextFragment::plain_text(" is "),
            FormattedTextFragment::bold("nice"),
            FormattedTextFragment::plain_text(".")
        ]
    );

    // Nested links are not allowed.
    assert_eq!(
        parse_all("[A [inner](place) link](target)", parse_inline),
        vec![
            FormattedTextFragment::plain_text("[A "),
            FormattedTextFragment::hyperlink("inner", "place"),
            FormattedTextFragment::plain_text(" link](target)")
        ]
    );

    // Link tags can only use formatting markers within the tag.
    assert_eq!(
        parse_all("_outside [a_ link](target)", parse_inline),
        vec![
            FormattedTextFragment::plain_text("_outside "),
            FormattedTextFragment::hyperlink("a_ link", "target")
        ]
    );
}

#[test]
fn test_parse_unclosed_link() {
    assert_eq!(
        parse_all("this *[link](target is unclosed*", parse_inline),
        vec![
            FormattedTextFragment::plain_text("this "),
            FormattedTextFragment::italic("[link](target is unclosed")
        ]
    );

    assert_eq!(
        parse_all("unbalanced [link]((parens)", parse_inline),
        vec![FormattedTextFragment::plain_text(
            "unbalanced [link]((parens)"
        )]
    );
}

#[test]
fn test_parse_inline_link_spec() {
    // This tests some of the inline link examples in the CommonMark spec.
    // See https://spec.commonmark.org/0.31.2/#example-483.

    // Example 483.
    assert_eq!(
        parse_all("[link](/uri)", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "/uri")]
    );

    // Example 484.
    assert_eq!(
        parse_all("[](./target.md)", parse_inline),
        vec![FormattedTextFragment::hyperlink("", "./target.md")]
    );

    // Example 485.
    assert_eq!(
        parse_all("[link]()", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "")]
    );

    // Example 486.
    assert_eq!(
        parse_all("[link](<>)", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "")]
    );

    // Example 487.
    assert_eq!(
        parse_all("[]()", parse_inline),
        vec![FormattedTextFragment::hyperlink("", "")]
    );

    // Example 488.
    assert_eq!(
        parse_all("[link](/my uri)", parse_inline),
        vec![FormattedTextFragment::plain_text("[link](/my uri)")]
    );

    // Example 489.
    assert_eq!(
        parse_all("[link](</my uri>)", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "/my uri")]
    );

    // Example 490.
    assert_eq!(
        test_parse_markdown("[link](foo\nbar)"),
        vec![
            FormattedTextLine::Line(vec!(FormattedTextFragment::plain_text("[link](foo"))),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("bar)")])
        ]
    );

    // Example 491.
    assert_eq!(
        test_parse_markdown("[link](<foo\nbar>)"),
        vec![
            FormattedTextLine::Line(vec!(FormattedTextFragment::plain_text("[link](<foo"))),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("bar>)")])
        ]
    );

    // Example 492.
    assert_eq!(
        parse_all("[a](<b)c>)", parse_inline),
        vec![FormattedTextFragment::hyperlink("a", "b)c")]
    );

    // Example 493.
    assert_eq!(
        parse_all("[link](<foo\\>)", parse_inline),
        vec![FormattedTextFragment::plain_text("[link](<foo>)")]
    );

    // Example 494.
    assert_eq!(
        parse_all("[a](<b)c", parse_inline),
        vec![FormattedTextFragment::plain_text("[a](<b)c")]
    );
    assert_eq!(
        parse_all("[a](<b)c>", parse_inline),
        vec![FormattedTextFragment::plain_text("[a](<b)c>")]
    );
    assert_eq!(
        parse_all("[a](<b>c)", parse_inline),
        vec![FormattedTextFragment::plain_text("[a](<b>c)")]
    );

    // Example 495.
    assert_eq!(
        parse_all("[link](\\(foo\\))", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "(foo)")]
    );

    // Example 496.
    assert_eq!(
        parse_all("[link](foo(and(bar)))", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "foo(and(bar))")]
    );

    // Example 497.
    assert_eq!(
        parse_all("[link](foo(and(bar))", parse_inline),
        vec![FormattedTextFragment::plain_text("[link](foo(and(bar))")]
    );

    // Example 498.
    assert_eq!(
        parse_all("[link](foo\\(and\\(bar\\))", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "foo(and(bar)")]
    );

    // Example 499.
    assert_eq!(
        parse_all("[link](<foo(and(bar)>)", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "foo(and(bar)")]
    );

    // Example 500.
    assert_eq!(
        parse_all("[link](foo\\)\\:)", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "foo):")]
    );

    // Example 501.
    assert_eq!(
        parse_all("[link](#fragment)", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "#fragment")]
    );
    assert_eq!(
        parse_all("[link](https://example.com#fragment)", parse_inline),
        vec![FormattedTextFragment::hyperlink(
            "link",
            "https://example.com#fragment"
        )]
    );
    assert_eq!(
        parse_all("[link](https://example.com?foo=3#frag)", parse_inline),
        vec![FormattedTextFragment::hyperlink(
            "link",
            "https://example.com?foo=3#frag"
        )]
    );

    // Example 502.
    assert_eq!(
        parse_all("[link](foo\\bar)", parse_inline),
        vec![FormattedTextFragment::hyperlink("link", "foo\\bar")]
    );
}

#[test]
fn test_parse_long_link_with_parens() {
    // This is a regression test for CLD-1604.
    let source = "This is [a link](https://console.cloud.google.com/traces/list?project=astral-field-294621&pageState=(%22traceIntervalPicker%22:(%22groupValue%22:%22P1D%22,%22customValue%22:null),%22traceFilter%22:(%22chips%22:%22%255B%257B_22k_22_3A_22%252Fhttp%252Furl_22_2C_22t_22_3A10_2C_22v_22_3A_22_5C_22https_3A%252F%252Fapp.warp.dev%252Fgraphql_5C_22_22_2C_22s_22_3Atrue_2C_22i_22_3A_22%252Fhttp%252Furl_22%257D%255D%22))&minl=0&maxl=53.33333333333334&tid=c3c3ffc64f74f16a1015e3f2d105c3d9&spanId=0ddd4991fda16831) with parentheses";
    assert_eq!(
        parse_all(source, parse_inline),
        vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::hyperlink(
                "a link",
                "https://console.cloud.google.com/traces/list?project=astral-field-294621&pageState=(%22traceIntervalPicker%22:(%22groupValue%22:%22P1D%22,%22customValue%22:null),%22traceFilter%22:(%22chips%22:%22%255B%257B_22k_22_3A_22%252Fhttp%252Furl_22_2C_22t_22_3A10_2C_22v_22_3A_22_5C_22https_3A%252F%252Fapp.warp.dev%252Fgraphql_5C_22_22_2C_22s_22_3Atrue_2C_22i_22_3A_22%252Fhttp%252Furl_22%257D%255D%22))&minl=0&maxl=53.33333333333334&tid=c3c3ffc64f74f16a1015e3f2d105c3d9&spanId=0ddd4991fda16831"
            ),
            FormattedTextFragment::plain_text(" with parentheses")
        ]
    );
}

#[test]
fn test_parse_inline_strikethrough() {
    assert_eq!(
        parse_all("a ~~stricken~~ string", parse_inline),
        vec![
            FormattedTextFragment::plain_text("a "),
            FormattedTextFragment::strikethrough("stricken"),
            FormattedTextFragment::plain_text(" string")
        ]
    );

    assert_eq!(
        parse_all("~single~", parse_inline),
        vec![FormattedTextFragment::strikethrough("single")]
    );

    assert_eq!(
        parse_all("~~unmatched~", parse_inline),
        vec![FormattedTextFragment::plain_text("~~unmatched~")]
    );

    assert_eq!(
        parse_all("~~~excessive~~~", parse_inline),
        vec![FormattedTextFragment::plain_text("~~~excessive~~~")]
    );
}

#[test]
fn test_parse_overlapping_strikethrough() {
    // This isn't valid Markdown, but did cause a panic.
    assert_eq!(
        parse_all("*~~asdfasdfasd*~~", parse_inline),
        vec![
            FormattedTextFragment::italic("~~asdfasdfasd"),
            FormattedTextFragment::plain_text("~~")
        ]
    );
}

#[test]
fn test_parse_autolinks_inline() {
    assert_eq!(
        parse_all(
            "Links: https://example.com and http://example.com and www.example.com/some-path andhttp://www.example.com",
            parse_inline
        ),
        vec![
            FormattedTextFragment::plain_text("Links: "),
            FormattedTextFragment::hyperlink("https://example.com", "https://example.com"),
            FormattedTextFragment::plain_text(" and "),
            FormattedTextFragment::hyperlink("http://example.com", "http://example.com"),
            FormattedTextFragment::plain_text(" and "),
            FormattedTextFragment::hyperlink(
                "www.example.com/some-path",
                "www.example.com/some-path"
            ),
            FormattedTextFragment::plain_text(" andhttp://www.example.com")
        ]
    );
}

#[test]
fn test_parse_autolinks_with_emphasis() {
    // Per GFM spec, autolinks can follow formatting delimiters (*, _, ~, ().
    // https://github.github.com/gfm/#autolinks-extension-

    // Bold autolink
    assert_eq!(
        parse_all("**https://example.com**", parse_inline),
        vec![FormattedTextFragment {
            text: "https://example.com".to_string(),
            styles: FormattedTextStyles {
                weight: Some(CustomWeight::Bold),
                hyperlink: Some(Hyperlink::Url("https://example.com".to_string())),
                ..Default::default()
            }
        }]
    );

    // Italic autolink
    assert_eq!(
        parse_all("*https://example.com*", parse_inline),
        vec![FormattedTextFragment {
            text: "https://example.com".to_string(),
            styles: FormattedTextStyles {
                italic: true,
                hyperlink: Some(Hyperlink::Url("https://example.com".to_string())),
                ..Default::default()
            }
        }]
    );

    // Underscore bold autolink
    assert_eq!(
        parse_all("__https://example.com__", parse_inline),
        vec![FormattedTextFragment {
            text: "https://example.com".to_string(),
            styles: FormattedTextStyles {
                weight: Some(CustomWeight::Bold),
                hyperlink: Some(Hyperlink::Url("https://example.com".to_string())),
                ..Default::default()
            }
        }]
    );

    // Strikethrough autolink
    assert_eq!(
        parse_all("~~https://example.com~~", parse_inline),
        vec![FormattedTextFragment {
            text: "https://example.com".to_string(),
            styles: FormattedTextStyles {
                strikethrough: true,
                hyperlink: Some(Hyperlink::Url("https://example.com".to_string())),
                ..Default::default()
            }
        }]
    );
}

#[test]
fn test_parse_escapes_inline() {
    assert_eq!(
        parse_all("\\*not emphasized*", parse_inline),
        vec![FormattedTextFragment::plain_text("*not emphasized*")]
    );
    assert_eq!(
        parse_all("\\\\*emphasis*", parse_inline),
        vec![
            FormattedTextFragment::plain_text("\\"),
            FormattedTextFragment::italic("emphasis")
        ]
    );
    assert_eq!(
        parse_all("a \\literal backslash", parse_inline),
        vec![FormattedTextFragment::plain_text("a \\literal backslash")]
    );
    assert_eq!(
        parse_all("also \\**not bold**", parse_inline),
        vec![
            FormattedTextFragment::plain_text("also *"),
            FormattedTextFragment::italic("not bold"),
            FormattedTextFragment::plain_text("*")
        ]
    );
}

#[test]
fn test_parse_embedded() {
    assert_eq!(
        test_parse_markdown("```warp-embedded-object\nid: workflow-123\n```"),
        vec![FormattedTextLine::Embedded(Mapping::from_iter([(
            Value::String("id".to_string()),
            Value::String("workflow-123".to_string())
        )]))]
    );

    assert_eq!(
        test_parse_markdown("```warp-embedded-object\nid: notebook-123\ntype: notebook\n```"),
        vec![FormattedTextLine::Embedded(Mapping::from_iter([
            (
                Value::String("id".to_string()),
                Value::String("notebook-123".to_string())
            ),
            (
                Value::String("type".to_string()),
                Value::String("notebook".to_string())
            )
        ]))]
    );

    // Fallback to code block.
    assert_eq!(
        test_parse_markdown("```warp-embedded-object\ncargo run --features abc\n```"),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "warp-embedded-object".to_string(),
            code: "cargo run --features abc\n".to_string()
        })]
    );
}

#[test]
fn test_basic_parse_underline() {
    let source = "This is <u>underlined</u>";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::underline("underlined"),
        ])]
    );
}

#[test]
fn test_mixed_parse_underline() {
    let source = "This is ~~test~~ **with** <u>text</u>";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("This is "),
            FormattedTextFragment::strikethrough("test"),
            FormattedTextFragment::plain_text(" "),
            FormattedTextFragment::bold("with"),
            FormattedTextFragment::plain_text(" "),
            FormattedTextFragment::underline("text"),
        ])]
    );
}

#[test]
fn test_multi_parse_underline() {
    let source = "<u>test1</u>\n<u>test2</u>";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::underline("test1"),]),
            FormattedTextLine::Line(vec![FormattedTextFragment::underline("test2"),])
        ]
    );
}

#[test]
fn test_parse_empty_underline() {
    assert_eq!(
        parse_all("some <u></u> text", parse_inline),
        vec![
            FormattedTextFragment::plain_text("some "),
            FormattedTextFragment::underline(""),
            FormattedTextFragment::plain_text(" text")
        ]
    )
}

#[test]
fn test_unordered_list_indentation_level_relative() {
    // Test that both 2-space and 4-space relative indentation produce the same structure
    let source_2space = "- top level\n  - sublevel\n    - subsublevel";
    let source_4space = "- top level\n    - sublevel\n        - subsublevel";

    let result_2space = test_parse_markdown(source_2space);
    let result_4space = test_parse_markdown(source_4space);

    // Both should have exactly 3 lines
    assert_eq!(result_2space.len(), 3);
    assert_eq!(result_4space.len(), 3);

    // Verify expected structure for 2-space version
    if let FormattedTextLine::UnorderedList(item) = &result_2space[0] {
        assert_eq!(item.indent_level, 0, "Top level should have indent_level 0");
        assert_eq!(item.text[0].text, "top level");
    } else {
        panic!("First line should be UnorderedList");
    }

    if let FormattedTextLine::UnorderedList(item) = &result_2space[1] {
        assert_eq!(item.indent_level, 1, "Sublevel should have indent_level 1");
        assert_eq!(item.text[0].text, "sublevel");
    } else {
        panic!("Second line should be UnorderedList");
    }

    if let FormattedTextLine::UnorderedList(item) = &result_2space[2] {
        assert_eq!(
            item.indent_level, 2,
            "Subsublevel should have indent_level 2"
        );
        assert_eq!(item.text[0].text, "subsublevel");
    } else {
        panic!("Third line should be UnorderedList");
    }

    // Both versions should be identical
    assert_eq!(
        result_2space, result_4space,
        "2-space and 4-space versions should be structurally identical"
    );
}

#[test]
fn test_task_list_indentation_level_relative() {
    // Test that both 2-space and 4-space relative indentation produce the same structure for task lists
    let source_2space =
        "- [ ] top level task\n  - [x] nested completed task\n    - [ ] deeply nested task";
    let source_4space =
        "- [ ] top level task\n    - [x] nested completed task\n        - [ ] deeply nested task";

    let result_2space = test_parse_markdown(source_2space);
    let result_4space = test_parse_markdown(source_4space);

    // Both should have exactly 3 lines
    assert_eq!(result_2space.len(), 3);
    assert_eq!(result_4space.len(), 3);

    // Verify expected structure for 2-space version
    if let FormattedTextLine::TaskList(item) = &result_2space[0] {
        assert_eq!(
            item.indent_level, 0,
            "Top level task should have indent_level 0"
        );
        assert!(!item.complete, "First task should be unchecked");
        assert_eq!(item.text[0].text, "top level task");
    } else {
        panic!("First line should be TaskList");
    }

    if let FormattedTextLine::TaskList(item) = &result_2space[1] {
        assert_eq!(
            item.indent_level, 1,
            "Nested task should have indent_level 1"
        );
        assert!(item.complete, "Second task should be checked");
        assert_eq!(item.text[0].text, "nested completed task");
    } else {
        panic!("Second line should be TaskList");
    }

    if let FormattedTextLine::TaskList(item) = &result_2space[2] {
        assert_eq!(
            item.indent_level, 2,
            "Deeply nested task should have indent_level 2"
        );
        assert!(!item.complete, "Third task should be unchecked");
        assert_eq!(item.text[0].text, "deeply nested task");
    } else {
        panic!("Third line should be TaskList");
    }

    // Both versions should be identical
    assert_eq!(
        result_2space, result_4space,
        "2-space and 4-space task list versions should be structurally identical"
    );
}

#[test]
fn test_ordered_list_indentation_level_relative() {
    // Test that both 2-space and 4-space relative indentation produce the same structure for ordered lists
    let source_2space = "1. top level item\n  2. nested item\n    3. deeply nested item";
    let source_4space = "1. top level item\n    2. nested item\n        3. deeply nested item";

    let result_2space = test_parse_markdown(source_2space);
    let result_4space = test_parse_markdown(source_4space);

    // Both should have exactly 3 lines
    assert_eq!(result_2space.len(), 3);
    assert_eq!(result_4space.len(), 3);

    // Verify expected structure for 2-space version
    if let FormattedTextLine::OrderedList(item) = &result_2space[0] {
        assert_eq!(
            item.indented_text.indent_level, 0,
            "Top level ordered item should have indent_level 0"
        );
        assert_eq!(
            item.number,
            Some(1),
            "First ordered item should have number 1"
        );
        assert_eq!(item.indented_text.text[0].text, "top level item");
    } else {
        panic!("First line should be OrderedList");
    }

    if let FormattedTextLine::OrderedList(item) = &result_2space[1] {
        assert_eq!(
            item.indented_text.indent_level, 1,
            "Nested ordered item should have indent_level 1"
        );
        assert_eq!(
            item.number,
            Some(2),
            "Second ordered item should have number 2"
        );
        assert_eq!(item.indented_text.text[0].text, "nested item");
    } else {
        panic!("Second line should be OrderedList");
    }

    if let FormattedTextLine::OrderedList(item) = &result_2space[2] {
        assert_eq!(
            item.indented_text.indent_level, 2,
            "Deeply nested ordered item should have indent_level 2"
        );
        assert_eq!(
            item.number,
            Some(3),
            "Third ordered item should have number 3"
        );
        assert_eq!(item.indented_text.text[0].text, "deeply nested item");
    } else {
        panic!("Third line should be OrderedList");
    }

    // Both versions should be identical
    assert_eq!(
        result_2space, result_4space,
        "2-space and 4-space ordered list versions should be structurally identical"
    );
}

#[test]
fn test_mixed_list_types_indentation_level_relative() {
    // Test mixing different list types with both 2-space and 4-space indentation
    let source_2space = "1. ordered top\n  - unordered nested\n    - [ ] task nested deeper";
    let source_4space = "1. ordered top\n    - unordered nested\n        - [ ] task nested deeper";

    let result_2space = test_parse_markdown(source_2space);
    let result_4space = test_parse_markdown(source_4space);

    // Both should have exactly 3 lines
    assert_eq!(result_2space.len(), 3);
    assert_eq!(result_4space.len(), 3);

    // Verify expected structure for 2-space version

    // Check first item (OrderedList)
    if let FormattedTextLine::OrderedList(item) = &result_2space[0] {
        assert_eq!(
            item.indented_text.indent_level, 0,
            "Ordered top level should have indent_level 0"
        );
        assert_eq!(
            item.number,
            Some(1),
            "First ordered item should have number 1"
        );
        assert_eq!(item.indented_text.text[0].text, "ordered top");
    } else {
        panic!("First line should be OrderedList");
    }

    // Check second item (UnorderedList)
    if let FormattedTextLine::UnorderedList(item) = &result_2space[1] {
        assert_eq!(
            item.indent_level, 1,
            "Unordered nested should have indent_level 1"
        );
        assert_eq!(item.text[0].text, "unordered nested");
    } else {
        panic!("Second line should be UnorderedList");
    }

    // Check third item (TaskList)
    if let FormattedTextLine::TaskList(item) = &result_2space[2] {
        assert_eq!(
            item.indent_level, 2,
            "Task nested deeper should have indent_level 2"
        );
        assert!(!item.complete, "Task should be unchecked");
        assert_eq!(item.text[0].text, "task nested deeper");
    } else {
        panic!("Third line should be TaskList");
    }

    // Both versions should be identical
    assert_eq!(
        result_2space, result_4space,
        "2-space and 4-space mixed list versions should be structurally identical"
    );
}

#[test]
fn test_unordered_list_various_indentation_patterns() {
    // Test case for various indentation levels including edge cases
    // Testing the specific case from the user request:
    // - 0 space (0 indent)
    //  - 1 space (0 indent)
    //     - 4 space (1 indent)
    //         - 8 space (2 indent)
    //       - 6 space (2 indent)
    //    - 3 space (1 indent)
    //   - 2 space (0 indent)
    //  - 1 space (0 indent)
    //   - 2 space (0 indent)
    let source = "- 0 space (0 indent)\n - 1 space (0 indent)\n    - 4 space (1 indent)\n        - 8 space (2 indent)\n      - 6 space (2 indent)\n   - 3 space (1 indent)\n  - 2 space (0 indent)\n - 1 space (0 indent)\n  - 2 space (0 indent)";

    let result = test_parse_markdown(source);

    // Should have exactly 9 lines
    assert_eq!(result.len(), 9);

    // Test each line's indentation level
    let expected_indents = [0, 0, 1, 2, 2, 1, 0, 0, 0];
    let expected_texts = [
        "0 space (0 indent)",
        "1 space (0 indent)",
        "4 space (1 indent)",
        "8 space (2 indent)",
        "6 space (2 indent)",
        "3 space (1 indent)",
        "2 space (0 indent)",
        "1 space (0 indent)",
        "2 space (0 indent)",
    ];

    for (i, (expected_indent, expected_text)) in expected_indents
        .iter()
        .zip(expected_texts.iter())
        .enumerate()
    {
        if let FormattedTextLine::UnorderedList(item) = &result[i] {
            assert_eq!(
                item.indent_level,
                *expected_indent,
                "Line {} ('{}') should have indent_level {}, got {}",
                i + 1,
                expected_text,
                expected_indent,
                item.indent_level
            );
            assert_eq!(
                item.text[0].text,
                *expected_text,
                "Line {} should have text '{}', got '{}'",
                i + 1,
                expected_text,
                item.text[0].text
            );
        } else {
            panic!(
                "Line {} should be UnorderedList, got {:?}",
                i + 1,
                result[i]
            );
        }
    }
}

#[test]
fn test_list_with_blank_line_preserves_context() {
    // Test case where we have list items separated by a blank line
    // The indentation context should NOT be reset by blank lines
    let source = "- top level\n\n    - sublevel after a space";

    let result = test_parse_markdown(source);

    // Should have 3 lines: list item, line break, list item
    assert_eq!(result.len(), 3);

    // First list item: "top level" should be indent_level 0
    if let FormattedTextLine::UnorderedList(item) = &result[0] {
        assert_eq!(
            item.indent_level, 0,
            "'top level' should have indent_level 0"
        );
        assert_eq!(item.text[0].text, "top level");
    } else {
        panic!("First line should be UnorderedList, got {:?}", result[0]);
    }

    // Second should be a line break
    assert_eq!(
        result[1],
        FormattedTextLine::LineBreak,
        "Second line should be a line break"
    );

    // Third list item: "sublevel after a space" should be indent_level 1 (context preserved through blank line)
    if let FormattedTextLine::UnorderedList(item) = &result[2] {
        assert_eq!(
            item.indent_level, 1,
            "'sublevel after a space' should have indent_level 1 (context preserved)"
        );
        assert_eq!(item.text[0].text, "sublevel after a space");
    } else {
        panic!("Third line should be UnorderedList, got {:?}", result[2]);
    }
}

#[test]
fn test_indentation_reset_with_nonlist_content() {
    // Test case where we have list items separated by non-list content
    // The indentation context should reset appropriately
    let source = "- top level\n  - sub level\n\nheres a sentence\n  - top level";

    let result = test_parse_markdown(source);

    // Should have 5 lines: 2 list items, 1 line break, 1 paragraph, 1 list item
    assert_eq!(result.len(), 5);

    // First list item: "top level" should be indent_level 0
    if let FormattedTextLine::UnorderedList(item) = &result[0] {
        assert_eq!(
            item.indent_level, 0,
            "First 'top level' should have indent_level 0"
        );
        assert_eq!(item.text[0].text, "top level");
    } else {
        panic!("First line should be UnorderedList, got {:?}", result[0]);
    }

    // Second list item: "sub level" should be indent_level 1
    if let FormattedTextLine::UnorderedList(item) = &result[1] {
        assert_eq!(
            item.indent_level, 1,
            "'sub level' should have indent_level 1"
        );
        assert_eq!(item.text[0].text, "sub level");
    } else {
        panic!("Second line should be UnorderedList, got {:?}", result[1]);
    }

    // Third should be a line break
    assert_eq!(
        result[2],
        FormattedTextLine::LineBreak,
        "Third line should be a line break"
    );

    // Fourth should be the sentence
    if let FormattedTextLine::Line(fragments) = &result[3] {
        assert_eq!(fragments[0].text, "heres a sentence");
    } else {
        panic!("Fourth line should be a Line, got {:?}", result[3]);
    }

    // Fifth list item: second "top level" should be indent_level 0 (reset after non-list content)
    if let FormattedTextLine::UnorderedList(item) = &result[4] {
        assert_eq!(
            item.indent_level, 0,
            "Second 'top level' should have indent_level 0 (reset after non-list content)"
        );
        assert_eq!(item.text[0].text, "top level");
    } else {
        panic!("Fifth line should be UnorderedList, got {:?}", result[4]);
    }
}

#[test]
fn test_parse_basic_image() {
    let source = "![Alt text](image.png)\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Alt text".to_string(),
            source: "image.png".to_string(),
            title: None,
        })]
    );
}

#[test]
fn test_parse_image_without_trailing_newline() {
    let source = "![Alt text](image.png)";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Alt text".to_string(),
            source: "image.png".to_string(),
            title: None,
        })]
    );
}

#[test]
fn test_parse_image_relative_path() {
    let source = "![My screenshot](./screenshots/demo.png)\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "My screenshot".to_string(),
            source: "./screenshots/demo.png".to_string(),
            title: None,
        })]
    );

    let source = "![Logo](../assets/logo.jpg)\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Logo".to_string(),
            source: "../assets/logo.jpg".to_string(),
            title: None,
        })]
    );
}

#[test]
fn test_parse_image_absolute_path() {
    let source = "![Chart](/absolute/path/to/chart.svg)\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Chart".to_string(),
            source: "/absolute/path/to/chart.svg".to_string(),
            title: None,
        })]
    );
}

#[test]
fn test_parse_image_with_double_quoted_title() {
    let source = "![Alt text](image.png \"A helpful caption\")\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Alt text".to_string(),
            source: "image.png".to_string(),
            title: Some("A helpful caption".to_string()),
        })]
    );
}

#[test]
fn test_parse_image_with_single_quoted_title() {
    let source = "![Alt](image.png 'A helpful caption')\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Alt".to_string(),
            source: "image.png".to_string(),
            title: Some("A helpful caption".to_string()),
        })]
    );
}

#[test]
fn test_parse_image_with_paren_wrapped_title() {
    let source = "![Alt](image.png (A helpful caption))\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Alt".to_string(),
            source: "image.png".to_string(),
            title: Some("A helpful caption".to_string()),
        })]
    );
}

#[test]
fn test_parse_image_with_empty_title_normalizes_to_none() {
    // Empty titles in each delimiter form are equivalent to no title, per
    // product invariant 4.
    for source in [
        "![Alt](image.png \"\")\n",
        "![Alt](image.png '')\n",
        "![Alt](image.png ())\n",
    ] {
        assert_eq!(
            test_parse_markdown(source),
            vec![FormattedTextLine::Image(FormattedImage {
                alt_text: "Alt".to_string(),
                source: "image.png".to_string(),
                title: None,
            })],
            "source: {source:?}"
        );
    }
}

#[test]
fn test_parse_image_title_with_escaped_delimiter() {
    let source = "![Alt](image.png \"a \\\"quoted\\\" caption\")\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Alt".to_string(),
            source: "image.png".to_string(),
            title: Some("a \"quoted\" caption".to_string()),
        })]
    );
}

#[test]
fn test_parse_image_unclosed_title_falls_back_to_plain_text() {
    let source = "![Alt](image.png \"unterminated)\n";
    // Falls back to a plain-text paragraph, not an image.
    let parsed = test_parse_markdown(source);
    assert!(
        !parsed
            .iter()
            .any(|line| matches!(line, FormattedTextLine::Image(_))),
        "expected plain-text fallback for unclosed title, got {parsed:?}"
    );
}

#[test]
fn test_parse_image_title_with_embedded_newline_falls_back() {
    let source = "![Alt](image.png \"line one\nline two\")\n";
    let parsed = test_parse_markdown(source);
    assert!(
        !parsed
            .iter()
            .any(|line| matches!(line, FormattedTextLine::Image(_))),
        "expected plain-text fallback for title spanning a line ending, got {parsed:?}"
    );
}

#[test]
fn test_table_line_helpers() {
    let mut line = FormattedTextLine::Table(FormattedTable::from_internal_format(
        "name\tage\nalice\t30\n",
    ));
    let before = line.clone();

    assert_eq!(line.raw_text(), "name\tage\nalice\t30\n".to_string());
    assert_eq!(line.num_lines(), 2);
    assert!(line.hyperlinks(false).is_empty());

    line.set_weight(Some(CustomWeight::Bold));
    assert_eq!(line, before);
}

#[test]
fn test_compute_formatted_text_delta_with_unchanged_table_prefix() {
    let old = FormattedText::new([
        FormattedTextLine::Table(FormattedTable::from_internal_format("name\tage\n")),
        FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("first")]),
    ]);
    let new = FormattedText::new([
        FormattedTextLine::Table(FormattedTable::from_internal_format("name\tage\n")),
        FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("second")]),
    ]);

    let delta = compute_formatted_text_delta(old, new);
    assert_eq!(delta.common_prefix_lines, 1);
    assert_eq!(delta.old_suffix_formatted_text_lines, 1);
    assert_eq!(delta.new_suffix.len(), 1);
}

#[test]
fn test_compute_formatted_text_delta_with_changed_table_line() {
    let old = FormattedText::new([FormattedTextLine::Table(
        FormattedTable::from_internal_format("name\tage\n"),
    )]);
    let new = FormattedText::new([FormattedTextLine::Table(
        FormattedTable::from_internal_format("name\tyears\n"),
    )]);

    let delta = compute_formatted_text_delta(old, new);
    assert_eq!(delta.common_prefix_lines, 0);
    assert_eq!(delta.old_suffix_formatted_text_lines, 1);
    assert_eq!(delta.new_suffix.len(), 1);
}

#[test]
fn test_parse_image_url() {
    let source = "![Remote image](https://example.com/image.png)\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "Remote image".to_string(),
            source: "https://example.com/image.png".to_string(),
            title: None,
        })]
    );
}

#[test]
fn test_parse_image_empty_alt() {
    let source = "![](image.png)\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![FormattedTextLine::Image(FormattedImage {
            alt_text: "".to_string(),
            source: "image.png".to_string(),
            title: None,
        })]
    );
}

#[test]
fn test_parse_multiple_images() {
    let source = "![First](image1.png)\n![Second](image2.png)\n";
    assert_eq!(
        test_parse_markdown(source),
        vec![
            FormattedTextLine::Image(FormattedImage {
                alt_text: "First".to_string(),
                source: "image1.png".to_string(),
                title: None,
            }),
            FormattedTextLine::Image(FormattedImage {
                alt_text: "Second".to_string(),
                source: "image2.png".to_string(),
                title: None,
            }),
        ]
    );
}

#[test]
fn test_parse_image_with_other_content() {
    let source = "# Header\n\n![Image](test.png)\n\nSome text\n";
    let result = test_parse_markdown(source);
    assert_eq!(result.len(), 5);
    assert!(matches!(result[0], FormattedTextLine::Heading(_)));
    assert!(matches!(result[1], FormattedTextLine::LineBreak));
    assert!(matches!(result[2], FormattedTextLine::Image(_)));
    if let FormattedTextLine::Image(img) = &result[2] {
        assert_eq!(img.alt_text, "Image");
        assert_eq!(img.source, "test.png");
    }
    assert!(matches!(result[3], FormattedTextLine::LineBreak));
    assert!(matches!(result[4], FormattedTextLine::Line(_)));
}

// Table parsing tests

#[test]
fn table_cell_escaped_pipe_is_literal() {
    // Parse just the data row to validate cell splitting respects \\|.
    // Start at the first cell content (after the leading '|').
    let row_after_bar = " A \\| B | Pipe |";
    let (rest, cell1) = parse(row_after_bar, parse_table_cell);
    assert_eq!(cell1, "A | B");
    let (_rest2, cell2) = parse(rest, parse_table_cell);
    assert_eq!(cell2, "Pipe");
}

#[test]
fn table_cell_html_entities_are_decoded() {
    let decode = |s: &str| {
        let fragments = parse_cell_content(s);
        fragments.into_iter().map(|f| f.text).collect::<String>()
    };

    assert_eq!(decode("&lt;"), "<");
    assert_eq!(decode("&gt;"), ">");
    assert_eq!(decode("&amp;"), "&");
    assert_eq!(decode("&vert;"), "|");
    assert_eq!(decode("&ast;"), "*");
    assert_eq!(decode("&lowbar;"), "_");
    assert_eq!(decode("&grave;"), "`");
    assert_eq!(decode("&bsol;"), "\\");
    assert_eq!(decode("&#92;"), "\\");
    assert_eq!(decode("&#96;"), "`");
}

#[test]
fn test_html_entity_all_named_entities() {
    let decode = |s: &str| {
        let fragments = parse_all(s, parse_inline);
        fragments.into_iter().map(|f| f.text).collect::<String>()
    };

    assert_eq!(decode("&lt;"), "<");
    assert_eq!(decode("&gt;"), ">");
    assert_eq!(decode("&amp;"), "&");
    assert_eq!(decode("&quot;"), "\"");
    assert_eq!(decode("&apos;"), "'");
    assert_eq!(decode("&vert;"), "|");
    assert_eq!(decode("&ast;"), "*");
    assert_eq!(decode("&lowbar;"), "_");
    assert_eq!(decode("&grave;"), "`");
    assert_eq!(decode("&bsol;"), "\\");
    assert_eq!(decode("&nbsp;"), "\u{00A0}");
    assert_eq!(decode("&copy;"), "\u{00A9}");
    assert_eq!(decode("&reg;"), "\u{00AE}");
    assert_eq!(decode("&trade;"), "\u{2122}");
    assert_eq!(decode("&mdash;"), "\u{2014}");
    assert_eq!(decode("&ndash;"), "\u{2013}");
    assert_eq!(decode("&hellip;"), "\u{2026}");
    assert_eq!(decode("&lsquo;"), "\u{2018}");
    assert_eq!(decode("&rsquo;"), "\u{2019}");
    assert_eq!(decode("&ldquo;"), "\u{201C}");
    assert_eq!(decode("&rdquo;"), "\u{201D}");
}

#[test]
fn test_html_entity_numeric_decimal() {
    let decode = |s: &str| {
        let fragments = parse_all(s, parse_inline);
        fragments.into_iter().map(|f| f.text).collect::<String>()
    };

    assert_eq!(decode("&#60;"), "<");
    assert_eq!(decode("&#62;"), ">");
    assert_eq!(decode("&#38;"), "&");
    assert_eq!(decode("&#34;"), "\"");
    assert_eq!(decode("&#39;"), "'");
    assert_eq!(decode("&#160;"), "\u{00A0}");
    assert_eq!(decode("&#169;"), "\u{00A9}");
    assert_eq!(decode("&#8212;"), "\u{2014}");
}

#[test]
fn test_html_entity_numeric_hex() {
    let decode = |s: &str| {
        let fragments = parse_all(s, parse_inline);
        fragments.into_iter().map(|f| f.text).collect::<String>()
    };

    assert_eq!(decode("&#x3c;"), "<");
    assert_eq!(decode("&#x3e;"), ">");
    assert_eq!(decode("&#x26;"), "&");
    assert_eq!(decode("&#X3C;"), "<");
    assert_eq!(decode("&#X3E;"), ">");
    assert_eq!(decode("&#xA0;"), "\u{00A0}");
    assert_eq!(decode("&#x2014;"), "\u{2014}");
}

#[test]
fn test_html_entity_in_inline_text() {
    let decode = |s: &str| {
        let fragments = parse_all(s, parse_inline);
        fragments.into_iter().map(|f| f.text).collect::<String>()
    };

    assert_eq!(decode("Use &lt;tag&gt; here"), "Use <tag> here");
    assert_eq!(decode("Tom &amp; Jerry"), "Tom & Jerry");
    assert_eq!(
        decode("He said &ldquo;Hello&rdquo;"),
        "He said \u{201C}Hello\u{201D}"
    );
    assert_eq!(decode("Wait&hellip;"), "Wait\u{2026}");
    assert_eq!(decode("Copyright &copy; 2024"), "Copyright \u{00A9} 2024");
}

#[test]
fn test_html_entity_invalid_not_decoded() {
    let decode = |s: &str| {
        let fragments = parse_all(s, parse_inline);
        fragments.into_iter().map(|f| f.text).collect::<String>()
    };

    assert_eq!(decode("&unknown;"), "&unknown;");
    assert_eq!(decode("&foo;"), "&foo;");
    assert_eq!(decode("& lt;"), "& lt;");
    assert_eq!(decode("&lt"), "&lt");
    assert_eq!(decode("&#;"), "&#;");
    assert_eq!(decode("&#x;"), "&#x;");
    assert_eq!(decode("&#99999999999;"), "&#99999999999;");
}

#[test]
fn test_parse_simple_table() {
    let source = "| Header 1 | Header 2 |\n| --- | --- |\n| Cell 1 | Cell 2 |\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    eprintln!("Parse result: {:?}", result);
    eprintln!("Result length: {}", result.len());
    assert_eq!(result.len(), 1);

    if let FormattedTextLine::Table(table) = &result[0] {
        assert_eq!(table.headers.len(), 2);
        assert_eq!(table.alignments.len(), 2);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].len(), 2);
    } else {
        panic!("Expected table, got {:?}", result[0]);
    }
}

#[test]
fn test_parse_table_with_alignments() {
    let source = "| Left | Center | Right |\n| :--- | :---: | ---: |\n| L | C | R |\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    assert_eq!(result.len(), 1);

    if let FormattedTextLine::Table(table) = &result[0] {
        assert_eq!(table.alignments[0], TableAlignment::Left);
        assert_eq!(table.alignments[1], TableAlignment::Center);
        assert_eq!(table.alignments[2], TableAlignment::Right);
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_table_with_inline_formatting() {
    let source = "| **Bold** | *Italic* |\n| --- | --- |\n| `code` | normal |\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    assert_eq!(result.len(), 1);

    if let FormattedTextLine::Table(table) = &result[0] {
        // Check bold header
        assert_eq!(table.headers[0].len(), 1);
        assert_eq!(table.headers[0][0].styles.weight, Some(CustomWeight::Bold));

        // Check italic header
        assert_eq!(table.headers[1].len(), 1);
        assert!(table.headers[1][0].styles.italic);

        // Check inline code in cell
        assert_eq!(table.rows[0][0].len(), 1);
        assert!(table.rows[0][0][0].styles.inline_code);
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_table_multiple_rows() {
    let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    assert_eq!(result.len(), 1);

    if let FormattedTextLine::Table(table) = &result[0] {
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0][0][0].text, "1");
        assert_eq!(table.rows[1][0][0].text, "3");
        assert_eq!(table.rows[2][0][0].text, "5");
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_table_with_empty_cells() {
    let source = "| A | B |\n| --- | --- |\n|  | filled |\n| filled |  |\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    assert_eq!(result.len(), 1);

    if let FormattedTextLine::Table(table) = &result[0] {
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0].len(), 0);
        assert_eq!(table.rows[1][1].len(), 0);
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_table_with_links() {
    let source = "| Link | Text |\n| --- | --- |\n| [Warp](https://warp.dev) | normal |\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    assert_eq!(result.len(), 1);

    if let FormattedTextLine::Table(table) = &result[0] {
        assert_eq!(table.rows.len(), 1);
        let link_cell = &table.rows[0][0];
        assert_eq!(link_cell.len(), 1);
        assert_eq!(link_cell[0].text, "Warp");
        assert!(matches!(
            &link_cell[0].styles.hyperlink,
            Some(Hyperlink::Url(url)) if url == "https://warp.dev"
        ));
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_table_followed_by_other_content() {
    let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n\nSome text after the table\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    assert_eq!(result.len(), 3);

    assert!(matches!(result[0], FormattedTextLine::Table(_)));
    assert!(matches!(result[1], FormattedTextLine::LineBreak));

    if let FormattedTextLine::Line(fragments) = &result[2] {
        assert_eq!(fragments[0].text, "Some text after the table");
    } else {
        panic!("Expected Line, got {:?}", result[2]);
    }
}

#[test]
fn test_parse_table_with_strikethrough() {
    let source = "| Format | Example |\n| --- | --- |\n| ~~Strikethrough~~ | text |\n";
    let result = test_parse_markdown_with_gfm_tables(source);
    assert_eq!(result.len(), 1);

    if let FormattedTextLine::Table(table) = &result[0] {
        assert_eq!(table.rows.len(), 1);
        let cell = &table.rows[0][0];
        assert_eq!(cell.len(), 1);
        assert!(cell[0].styles.strikethrough);
        assert_eq!(cell[0].text, "Strikethrough");
    } else {
        panic!("Expected table");
    }
}
