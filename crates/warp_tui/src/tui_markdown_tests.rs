use std::cell::Cell;

use futures::channel::oneshot;
use markdown_parser::{
    parse_markdown, parse_markdown_with_gfm_tables, FormattedText, FormattedTextFragment,
    FormattedTextLine,
};
use warp::tui_export::Appearance;
use warpui::platform::WindowStyle;
use warpui::AddWindowOptions;
use warpui_core::elements::tui::{
    Modifier, TuiBufferExt, TuiChildView, TuiElement, TuiRect, TuiText,
};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, ViewHandle, WindowInvalidation};

use super::{render_formatted_text, TuiMarkdownBlockHooks, TuiMarkdownPalette};
use crate::test_fixtures::TestHostView;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_code_block_view::{TuiCodeBlockPayload, TuiCodeBlockView, TuiCodeBlockViewEvent};

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
fn continues_ordered_list_numbering_at_each_indent_level() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let formatted = parse_markdown(
                "5. first\n6. second\n  2. nested\n  3. nested continuation\n7. parent continuation",
            )
            .expect("Markdown should parse");
            let (lines, _) = render(&formatted, 40, ctx);
            assert_eq!(
                lines,
                vec![
                    "5. first",
                    "6. second",
                    "  2. nested",
                    "  3. nested continuation",
                    "7. parent continuation",
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
fn image_fallback_uses_title_then_source_when_alt_text_is_missing() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let formatted =
                parse_markdown("![](diagram.png \"System diagram\")\n\n![](fallback.png)")
                    .expect("Markdown should parse");
            let (lines, _) = render(&formatted, 80, ctx);
            assert_eq!(
                lines,
                vec![
                    "Image: System diagram (diagram.png)",
                    "",
                    "Image: fallback.png",
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

#[test]
fn fenced_markdown_code_block_applies_syntax_colors_to_exact_cells() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let formatted =
            parse_markdown("```rust\nfn main() {}\n```").expect("Markdown should parse");
        let code = formatted
            .lines
            .iter()
            .find_map(|line| match line {
                FormattedTextLine::CodeBlock(code) => Some(code.clone()),
                FormattedTextLine::Line(_)
                | FormattedTextLine::Heading(_)
                | FormattedTextLine::OrderedList(_)
                | FormattedTextLine::UnorderedList(_)
                | FormattedTextLine::TaskList(_)
                | FormattedTextLine::Table(_)
                | FormattedTextLine::Image(_)
                | FormattedTextLine::Embedded(_)
                | FormattedTextLine::LineBreak
                | FormattedTextLine::HorizontalRule => None,
            })
            .expect("fenced Markdown should contain a code block");
        let code_view = add_code_view(&mut app);
        let (tx, rx) = oneshot::channel();
        app.update(|ctx| {
            let mut tx = Some(tx);
            ctx.subscribe_to_view(&code_view, move |_, event, _| {
                if matches!(event, TuiCodeBlockViewEvent::SyntaxUpdated) {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(());
                    }
                }
            });
            code_view.update(ctx, |view, ctx| {
                view.sync(TuiCodeBlockPayload::new(code.code, Some(code.lang)), ctx);
            });
        });
        rx.await.expect("syntax parse should complete");

        app.update(|ctx| {
            let render_code = |index: usize, _: &markdown_parser::CodeBlockText| {
                assert_eq!(index, 0);
                Some(TuiChildView::new(&code_view).finish())
            };
            let hooks = TuiMarkdownBlockHooks {
                render_code: Some(&render_code),
            };
            let palette = TuiMarkdownPalette::from_builder(&TuiUiBuilder::from_app(ctx));
            let mut presenter = TuiPresenter::new();
            let mut invalidation = WindowInvalidation::default();
            invalidation.updated.insert(code_view.id());
            presenter.invalidate(&invalidation, ctx, code_view.window_id(ctx));
            let frame = presenter.present_element(
                render_formatted_text(&formatted, palette, &hooks),
                TuiRect::new(0, 0, 40, 40),
                ctx,
            );
            let lines = frame
                .buffer
                .to_lines()
                .into_iter()
                .map(|line| line.trim_end().to_owned())
                .collect::<Vec<_>>();
            let buffer = frame.buffer;
            let code_row = lines
                .iter()
                .position(|line| line.contains("fn main()"))
                .expect("rendered Markdown should contain the code line");
            let keyword_start_byte = lines[code_row]
                .find("fn main()")
                .expect("rendered code should preserve the Rust keyword");
            let keyword_start = lines[code_row][..keyword_start_byte].chars().count();
            let first_keyword_cell = &buffer[(keyword_start as u16, code_row as u16)];
            let second_keyword_cell = &buffer[((keyword_start + 1) as u16, code_row as u16)];
            let following_space_cell = &buffer[((keyword_start + 2) as u16, code_row as u16)];

            assert_eq!(first_keyword_cell.symbol(), "f");
            assert_eq!(second_keyword_cell.symbol(), "n");
            assert_eq!(first_keyword_cell.fg, second_keyword_cell.fg);
            assert_ne!(first_keyword_cell.fg, following_space_cell.fg);
        });
    });
}

fn add_code_view(app: &mut App) -> ViewHandle<TuiCodeBlockView> {
    app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_tui_view(window_id, |ctx| {
            TuiCodeBlockView::new(TuiCodeBlockPayload::new("", None), ctx)
        })
    })
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
