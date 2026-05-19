use std::borrow::Cow;

use super::*;
use crate::render::{layout::TextLayout, model::test_utils::TEST_STYLES};
use warpui::{
    App, SingletonEntity,
    assets::asset_cache::{AssetCache, AssetSource, AssetState},
    image_cache::ImageType,
    text_layout::LayoutCache,
};

fn mermaid_block_spacing() -> BlockSpacing {
    TEST_STYLES.block_spacings.from_block_style(
        &crate::content::text::BufferBlockStyle::CodeBlock {
            code_block_type: crate::content::text::CodeBlockType::Mermaid,
        },
    )
}

#[test]
fn loading_mermaid_layout_uses_default_height() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let source = "graph TD\nA[Start] --> B[Finish]\n";
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let (_asset_source, config) =
                mermaid_diagram_layout(source, &text_layout, mermaid_block_spacing(), ctx);
            let expected_height = TEST_STYLES.base_line_height()
                * DEFAULT_MERMAID_HEIGHT_LINE_MULTIPLIER.into_pixels();

            assert!((config.height.as_f32() - expected_height.as_f32()).abs() < 0.5);
        });
    })
}

#[test]
fn strip_frontmatter_removes_leading_config_block() {
    // The exact sample from issue #10676.
    let source = "---\nconfig:\n  theme: default\n---\nxychart-beta\n  title \"x\"\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, "xychart-beta\n  title \"x\"\n");
    assert!(matches!(stripped, Cow::Owned(_)));
}

#[test]
fn strip_frontmatter_preserves_source_without_frontmatter() {
    let source = "graph TD\nA --> B\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, source);
    assert!(
        matches!(stripped, Cow::Borrowed(_)),
        "no-op stripping should return Borrowed to avoid allocation",
    );
}

#[test]
fn strip_frontmatter_skips_leading_blank_lines_before_open_delimiter() {
    let source = "\n\n   \n---\nconfig:\n  theme: dark\n---\npie\n  \"a\" : 1\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, "pie\n  \"a\" : 1\n");
}

#[test]
fn strip_frontmatter_handles_crlf_line_endings() {
    let source = "---\r\nconfig:\r\n  theme: default\r\n---\r\nflowchart TD\r\nA --> B\r\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, "flowchart TD\r\nA --> B\r\n");
}

#[test]
fn strip_frontmatter_leaves_text_starting_with_three_dashes_then_content() {
    // `--- something` (with content after the dashes) is not a frontmatter
    // delimiter — the trimmed line is `--- something`, not `---`.
    let source = "--- some weird title\nflowchart TD\nA --> B\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, source);
    assert!(matches!(stripped, Cow::Borrowed(_)));
}

#[test]
fn strip_frontmatter_leaves_unterminated_block_for_renderer_to_surface() {
    // No closing `---`: leave intact so mermaid_to_svg can surface its own
    // error instead of silently dropping the body.
    let source = "---\nconfig:\n  theme: default\nflowchart TD\nA --> B\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, source);
    assert!(matches!(stripped, Cow::Borrowed(_)));
}

#[test]
fn strip_frontmatter_handles_empty_input() {
    assert_eq!(strip_mermaid_frontmatter(""), "");
}

#[test]
fn strip_frontmatter_handles_only_frontmatter_no_body() {
    let source = "---\nconfig: {}\n---\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, "");
}

#[test]
fn strip_frontmatter_handles_frontmatter_without_trailing_newline() {
    // Closing `---` on the final line (no newline after).
    let source = "---\nfoo: bar\n---";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, "");
}

#[test]
fn strip_frontmatter_handles_open_delimiter_only_with_newline() {
    // `---\n` and nothing else: treated as unterminated, leave intact.
    let source = "---\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, source);
    assert!(matches!(stripped, Cow::Borrowed(_)));
}

#[test]
fn strip_frontmatter_handles_open_delimiter_only_without_newline() {
    // Single line `---` with no newline at all: must not panic / loop forever;
    // returned as-is for the renderer to surface.
    let source = "---";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, source);
    assert!(matches!(stripped, Cow::Borrowed(_)));
}

#[test]
fn strip_frontmatter_treats_indented_dashes_as_delimiter() {
    // Documents the intentional choice: a leading-whitespace `---` line is
    // trimmed before comparison and treated as a frontmatter delimiter. This
    // mirrors the renderer's own `first_diagram_type_token`, which trims each
    // line before matching the diagram token — being stricter here would
    // diverge from the renderer and leave a class of broken sources broken.
    let source = "\t---\n  config: x\n  ---\nflowchart TD\nA --> B\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, "flowchart TD\nA --> B\n");
}

#[test]
fn strip_frontmatter_does_not_treat_inner_dashes_line_as_delimiter() {
    // The `---` between `title` and `body` here is NOT a frontmatter open
    // because the first content line is `flowchart TD`, not `---`.
    let source = "flowchart TD\n---\nA --> B\n";
    let stripped = strip_mermaid_frontmatter(source);
    assert_eq!(stripped, source);
}

#[test]
fn mermaid_asset_source_hashes_post_strip_for_cache_key_stability() {
    // The asset cache key must be derived from the post-strip source so that
    // the same logical diagram with and without frontmatter doesn't churn
    // the cache (and so the bug-fix actually changes what gets rendered).
    let with_frontmatter = "---\nconfig:\n  theme: default\n---\nflowchart TD\nA --> B\n";
    let without_frontmatter = "flowchart TD\nA --> B\n";
    let with = mermaid_asset_source(with_frontmatter);
    let without = mermaid_asset_source(without_frontmatter);
    let (with_id, without_id) = match (&with, &without) {
        (AssetSource::Async { id: a, .. }, AssetSource::Async { id: b, .. }) => (a, b),
        _ => panic!("expected Async asset sources"),
    };
    assert_eq!(with_id, without_id);
}

#[test]
fn failed_mermaid_layout_uses_compact_height() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let asset_source = AssetSource::Raw {
                id: "missing-mermaid-test-asset".to_string(),
            };
            let asset_cache = AssetCache::as_ref(ctx);
            assert!(matches!(
                asset_cache.load_asset::<ImageType>(asset_source.clone()),
                AssetState::FailedToLoad(_)
            ));

            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let config =
                mermaid_diagram_config(&asset_source, &text_layout, mermaid_block_spacing(), ctx);
            let expected_height = TEST_STYLES.base_line_height()
                * FAILED_MERMAID_HEIGHT_LINE_MULTIPLIER.into_pixels();

            assert!((config.height.as_f32() - expected_height.as_f32()).abs() < 0.5);
        });
    })
}
