use warp_core::ui::theme::ColorScheme;
use warpui_core::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui_core::image_cache::ImageType;
use warpui_core::text_layout::LayoutCache;
use warpui_core::{App, SingletonEntity};

use super::*;
use crate::render::layout::TextLayout;
use crate::render::model::test_utils::TEST_STYLES;

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
            let (_asset_source, config) = mermaid_diagram_layout(
                source,
                &text_layout,
                mermaid_block_spacing(),
                ColorScheme::LightOnDark,
                ctx,
            );
            let expected_height = TEST_STYLES.base_line_height()
                * DEFAULT_MERMAID_HEIGHT_LINE_MULTIPLIER.into_pixels();

            assert!((config.height.as_f32() - expected_height.as_f32()).abs() < 0.5);
        });
    })
}

#[test]
fn mermaid_asset_source_renders_frontmatter_formatting_directives() {
    let source = r##"---
config:
  theme: base
  themeVariables:
    primaryColor: "#ff0000"
  fontFamily: Inter
  fontSize: 18px
  flowchart:
    curve: linear
    nodeSpacing: 80
---
flowchart TD
  A[Start] --> B[Done]
"##;

    let AssetSource::Async { fetch, .. } = mermaid_asset_source(source, ColorScheme::LightOnDark)
    else {
        panic!("expected Mermaid diagrams to be async assets");
    };
    let bytes = match futures_lite::future::block_on(fetch()) {
        Ok(bytes) => bytes,
        Err(error) => panic!("expected frontmatter directives to render: {error:#}"),
    };
    let svg = match String::from_utf8(bytes.to_vec()) {
        Ok(svg) => svg,
        Err(error) => panic!("expected Mermaid SVG to be valid UTF-8: {error}"),
    };

    assert!(svg.contains("<svg "));
    assert!(svg.contains("background-color: #ffffff"));
    assert!(svg.contains(r##"fill="#ff0000""##));
    assert!(svg.contains(r#"font-family="Inter""#));
}

#[test]
fn mermaid_asset_source_uses_active_color_scheme() {
    let source = "graph TD\nA[Start] --> B[Finish]\n";
    let AssetSource::Async {
        fetch: fetch_dark, ..
    } = mermaid_asset_source(source, ColorScheme::LightOnDark)
    else {
        panic!("expected Mermaid diagrams to be async assets");
    };
    let AssetSource::Async {
        fetch: fetch_light, ..
    } = mermaid_asset_source(source, ColorScheme::DarkOnLight)
    else {
        panic!("expected Mermaid diagrams to be async assets");
    };

    let dark_svg = match futures_lite::future::block_on(fetch_dark()) {
        Ok(bytes) => String::from_utf8(bytes.to_vec()).expect("expected valid UTF-8"),
        Err(error) => panic!("expected dark Mermaid SVG to render: {error:#}"),
    };
    let light_svg = match futures_lite::future::block_on(fetch_light()) {
        Ok(bytes) => String::from_utf8(bytes.to_vec()).expect("expected valid UTF-8"),
        Err(error) => panic!("expected light Mermaid SVG to render: {error:#}"),
    };

    assert!(dark_svg.contains("background-color: #1e1e1e"));
    assert!(light_svg.contains("background-color: #ffffff"));
}

#[test]
fn mermaid_asset_source_cache_key_includes_color_scheme() {
    let source = "graph TD\nA[Start] --> B[Finish]\n";
    let dark_asset = mermaid_asset_source(source, ColorScheme::LightOnDark);
    let light_asset = mermaid_asset_source(source, ColorScheme::DarkOnLight);

    assert_ne!(dark_asset, light_asset);
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
