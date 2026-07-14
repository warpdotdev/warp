use std::cell::Cell;

use markdown_parser::{
    parse_markdown, parse_markdown_with_gfm_tables, FormattedText, FormattedTextFragment,
    FormattedTextLine,
};
use warp::tui_export::Appearance;
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiElement, TuiRect, TuiText};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext};

use super::{render_formatted_text, TuiMarkdownBlockHooks, TuiMarkdownPalette};
use crate::tui_builder::TuiUiBuilder;

#[test]
fn renders_blocks_inline_styles_and_accessible_links_without_markers() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let formatted = parse_markdown(
                "# Overview\n\nA **bold**, *italic*, ~~old~~, `code`, and [link](https://warp.dev).",
            )
            .expect("Markdown should parse");
            let (lines, buffer) = render(&formatted, 80, ctx);
            assert_eq!(
                lines,
                vec![
                    "Overview",
                    "",
                    "A bold, italic, old, code, and link (https://warp.dev).",
                ]
            );
            assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            assert!(buffer[(2, 2)].modifier.contains(Modifier::BOLD));
            assert!(buffer[(8, 2)].modifier.contains(Modifier::ITALIC));
            assert!(buffer[(16, 2)].modifier.contains(Modifier::CROSSED_OUT));
            assert!(buffer[(32, 2)].modifier.contains(Modifier::UNDERLINED));
        });
    });
}

#[test]
fn wraps_nested_lists_with_a_hanging_indent() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let formatted =
                parse_markdown("- outer\n  - nested content that wraps across terminal rows")
                    .expect("Markdown should parse");
            let (lines, _) = render(&formatted, 18, ctx);
            assert_eq!(
                lines,
                vec![
                    "• outer",
                    "  • nested content",
                    "    that wraps",
                    "    across",
                    "    terminal rows",
                ]
            );
        });
    });
}

#[test]
fn tables_switch_from_columns_to_header_keyed_records() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let formatted = parse_markdown_with_gfm_tables(
                "| Name | Description |\n| --- | --- |\n| Alice | Builds terminals |",
            )
            .expect("GFM table should parse");
            let (wide, _) = render(&formatted, 50, ctx);
            assert_eq!(
                wide,
                vec![
                    "Name  │ Description",
                    "──────────────────────────────────────────────────",
                    "Alice │ Builds terminals",
                ]
            );

            let (narrow, _) = render(&formatted, 12, ctx);
            assert_eq!(
                narrow,
                vec!["Name: Alice", "Description:", "Builds", "terminals"]
            );
        });
    });
}

#[test]
fn renders_structural_and_specialized_fallbacks() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let formatted =
                parse_markdown("---\n\n![Architecture](diagram.png)\n\n```rust\nfn main() {}\n```")
                    .expect("Markdown should parse")
                    .append_line(FormattedTextLine::Embedded(Default::default()));
            let (lines, _) = render(&formatted, 24, ctx);
            assert_eq!(
                lines,
                vec![
                    "────────────────────────",
                    "",
                    "Image: Architecture",
                    "(diagram.png)",
                    "",
                    "┌──────────────────────┐",
                    "│ rust                 │",
                    "│ fn main() {}         │",
                    "│                      │",
                    "└──────────────────────┘",
                    "[Unsupported embedded",
                    "content]",
                ]
            );
        });
    });
}

#[test]
fn delegates_code_blocks_to_the_supplied_hook() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let formatted = FormattedText::new([
                FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("before")]),
                FormattedTextLine::CodeBlock(markdown_parser::CodeBlockText {
                    lang: "rust".to_owned(),
                    code: "fn main() {}".to_owned(),
                }),
            ]);
            let calls = Cell::new(0);
            let render_code = |index: usize, block: &markdown_parser::CodeBlockText| {
                calls.set(calls.get() + 1);
                Some(TuiText::new(format!("code {index}: {}", block.lang)).finish())
            };
            let hooks = TuiMarkdownBlockHooks {
                render_code: Some(&render_code),
            };
            let (lines, _) = render_with_hooks(&formatted, 40, &hooks, ctx);
            assert_eq!(lines, vec!["before", "code 0: rust"]);
            assert_eq!(calls.get(), 1);
        });
    });
}

fn render(
    formatted: &FormattedText,
    width: u16,
    ctx: &AppContext,
) -> (Vec<String>, warpui_core::elements::tui::TuiBuffer) {
    render_with_hooks(formatted, width, &TuiMarkdownBlockHooks::default(), ctx)
}

fn render_with_hooks(
    formatted: &FormattedText,
    width: u16,
    hooks: &TuiMarkdownBlockHooks<'_>,
    ctx: &AppContext,
) -> (Vec<String>, warpui_core::elements::tui::TuiBuffer) {
    let palette = TuiMarkdownPalette::from_builder(&TuiUiBuilder::from_app(ctx));
    let mut presenter = TuiPresenter::new();
    let frame = presenter.present_element(
        render_formatted_text(formatted, palette, hooks),
        TuiRect::new(0, 0, width, 40),
        ctx,
    );
    let mut lines = frame
        .buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect::<Vec<_>>();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    (lines, frame.buffer)
}
