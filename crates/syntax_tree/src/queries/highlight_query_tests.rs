use std::ops::Range;

use languages::language_by_filename;
use string_offset::CharOffset;
use warp_editor::content::buffer::Buffer;
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::IndentBehavior;
use warp_util::standardized_path::StandardizedPath;
use warpui_core::color::ColorU;
use warpui_core::App;

use super::{injected_highlights, ColorMap};
use crate::SyntaxTreeState;

/// A `ColorMap` with a distinct color per category so tests can assert which token kind a
/// highlight came from.
fn test_color_map() -> ColorMap {
    ColorMap {
        keyword_color: ColorU::new(255, 0, 0, 255),
        function_color: ColorU::new(0, 255, 0, 255),
        string_color: ColorU::new(0, 0, 255, 255),
        type_color: ColorU::new(255, 255, 0, 255),
        number_color: ColorU::new(0, 255, 255, 255),
        comment_color: ColorU::new(255, 0, 255, 255),
        property_color: ColorU::new(128, 128, 128, 255),
        tag_color: ColorU::new(255, 128, 0, 255),
    }
}

/// Parse `markdown` and return the injection highlights (in parent-buffer coordinates) along with
/// the buffer's character length. Shared by the injection tests below.
fn injected_colors(markdown: &str) -> (Vec<(Range<CharOffset>, ColorU)>, CharOffset) {
    let markdown = markdown.to_owned();
    App::test((), move |mut app| async move {
        let language =
            language_by_filename(&StandardizedPath::try_new("/sample.md").expect("absolute path"))
                .expect("markdown language should resolve");

        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
        buffer_handle.update(&mut app, |buffer, ctx| {
            *buffer = Buffer::from_plain_text(
                &markdown,
                None,
                Box::new(|_, _| IndentBehavior::Ignore),
                selection,
                ctx,
            );
        });

        let snapshot = buffer_handle.read(&app, |buffer, _| buffer.buffer_snapshot());
        let tree = warpui_core::r#async::block_on(async {
            SyntaxTreeState::parse_text(snapshot, None, &language).await
        })
        .expect("markdown should parse");

        let injections_query = language
            .injections_query
            .as_ref()
            .expect("markdown should have an injection query");
        let color_map = test_color_map();

        buffer_handle.read(&app, |buffer, _| {
            let highlights = injected_highlights(
                injections_query,
                &color_map,
                CharOffset::from(0)..buffer.len(),
                buffer,
                &tree,
            );
            let collected: Vec<(Range<CharOffset>, ColorU)> = highlights
                .iter()
                .map(|(range, color)| (range.clone(), *color))
                .collect();
            (collected, buffer.len())
        })
    })
}

/// Functional test for fenced-code injection: a fenced block must be highlighted in the language
/// named by its fence. Exercises the whole `injected_highlights` path end to end — resolving the
/// fence language, parsing the body with that grammar, translating the embedded grammar's token
/// spans back into the parent buffer's coordinates, and merging the colors in — so a regression in
/// any of those steps (e.g. from later caching work or an arborium bump) is caught.
#[test]
fn markdown_fenced_code_block_is_highlighted() {
    let color_map = test_color_map();
    let (highlights, _) =
        injected_colors("# Title\n\n```rust\nfn demo() {\n    let s = \"hi\";\n}\n```\n");
    let colors: Vec<ColorU> = highlights.iter().map(|(_, color)| *color).collect();

    assert!(
        !colors.is_empty(),
        "fenced rust block should produce injected highlights",
    );
    assert!(
        colors.contains(&color_map.keyword_color),
        "rust keywords (fn/let) should be keyword-colored inside the fence",
    );
    assert!(
        colors.contains(&color_map.string_color),
        "the rust string literal should be string-colored inside the fence",
    );
}

/// Each fenced block is highlighted by its own language, so adding a second block of a different
/// language contributes additional highlights. Guards per-block language resolution and the match
/// loop rather than only the first block.
#[test]
fn each_fenced_block_is_highlighted_by_its_own_language() {
    let (one_block, _) = injected_colors("```rust\nfn a() {}\n```\n");
    let (two_blocks, _) =
        injected_colors("```rust\nfn a() {}\n```\n\n```python\ndef b():\n    pass\n```\n");

    assert!(
        !one_block.is_empty(),
        "the rust block alone should produce highlights",
    );
    assert!(
        two_blocks.len() > one_block.len(),
        "adding a python block should contribute additional highlights (both blocks resolved)",
    );
}

/// A fence whose language is not recognized produces no injected highlights — the body stays the
/// default text color rather than being mis-highlighted. Encodes the "unknown language = default"
/// behavior.
#[test]
fn unknown_fence_language_is_not_injected() {
    let (highlights, _) = injected_colors("```definitely-not-a-language\nfn x() {}\n```\n");
    assert!(
        highlights.is_empty(),
        "an unrecognized fence language should produce no injected highlights",
    );
}

/// A fence with no info string (no language) has no language node to resolve, so it is skipped.
#[test]
fn fence_without_language_is_not_injected() {
    let (highlights, _) = injected_colors("```\nfn x() {}\n```\n");
    assert!(
        highlights.is_empty(),
        "a fence with no language should produce no injected highlights",
    );
}

/// Multi-byte content must not desync the byte-to-character offset translation: tokens around a
/// multi-byte character still highlight, and every highlight range stays within the buffer's
/// character length (a byte/char confusion would push ranges past the end).
#[test]
fn multibyte_fenced_content_highlights_within_bounds() {
    let color_map = test_color_map();
    let (highlights, buffer_len) =
        injected_colors("```rust\nlet s = \"café\";\nfn after() {}\n```\n");
    let colors: Vec<ColorU> = highlights.iter().map(|(_, color)| *color).collect();

    assert!(
        colors.contains(&color_map.string_color),
        "the string literal containing a multi-byte char should be string-colored",
    );
    assert!(
        colors.contains(&color_map.keyword_color),
        "keywords after multi-byte content should still be colored",
    );
    assert!(
        highlights.iter().all(|(range, _)| range.end <= buffer_len),
        "byte/char offset confusion would push highlight ranges past the buffer's char length",
    );
}
