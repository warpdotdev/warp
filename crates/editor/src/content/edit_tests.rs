use super::{
    BlockLocation, LayOutArgs, layout_mermaid_diagram_block, layout_table_block, layout_text_block,
};
use crate::{
    content::{
        buffer::{StyledBufferRun, StyledTextBlock},
        edit::{
            ParsedUrl, highlight_urls, layout_mermaid_block_for_test,
            resolve_asset_source_relative_to_directory,
        },
        mermaid_diagram::{mermaid_asset_source, mermaid_diagram_layout},
        text::{BufferBlockStyle, CodeBlockType, TextStylesWithMetadata},
    },
    render::{
        layout::{TextLayout, add_link_to_style_and_font, markdown_inline_to_text_and_style_runs},
        model::{BlockItem, RenderLayoutOptions, test_utils::TEST_STYLES},
    },
};
use std::path::Path;
use string_offset::CharOffset;
use warp_core::features::FeatureFlag;
use warpui::{
    App, SingletonEntity,
    assets::asset_cache::{AssetCache, AssetSource, AssetState},
    fonts::{Properties, Style, Weight},
    image_cache::ImageType,
    text_layout::{LayoutCache, StyleAndFont, TextStyle},
};

#[test]
fn test_highlight_urls() {
    let mut test_styled_buffer_runs = vec![
        StyledBufferRun {
            run: "https:".to_string(),
            text_styles: TextStylesWithMetadata::default().bold(),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "//".to_string(),
            text_styles: TextStylesWithMetadata::default().italic(),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "google.com".to_string(),
            text_styles: TextStylesWithMetadata::default(),
            block_style: BufferBlockStyle::PlainText,
        },
    ];

    assert_eq!(
        highlight_urls(&test_styled_buffer_runs),
        [ParsedUrl {
            url_range: 0..18,
            link: "https://google.com".to_string()
        },]
    );

    test_styled_buffer_runs.extend(vec![
        StyledBufferRun {
            run: " abc ".to_string(),
            text_styles: TextStylesWithMetadata::default().bold(),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "https://warp.dev".to_string(),
            text_styles: TextStylesWithMetadata::default(),
            block_style: BufferBlockStyle::PlainText,
        },
    ]);

    assert_eq!(
        highlight_urls(&test_styled_buffer_runs),
        [
            ParsedUrl {
                url_range: 0..18,
                link: "https://google.com".to_string()
            },
            ParsedUrl {
                url_range: 23..39,
                link: "https://warp.dev".to_string()
            }
        ]
    );
}

#[test]
fn test_highlight_urls_unicode() {
    let test_runs = vec![StyledBufferRun {
        run: "This (not https://example.com) is a 🔥 link about a 🇨🇦 🏡:\u{a0}https://warp.dev"
            .to_string(),
        text_styles: Default::default(),
        block_style: BufferBlockStyle::PlainText,
    }];
    assert_eq!(
        highlight_urls(&test_runs),
        [
            ParsedUrl {
                url_range: 10..29,
                link: "https://example.com".to_string()
            },
            ParsedUrl {
                url_range: 57..73,
                link: "https://warp.dev".to_string()
            }
        ]
    )
}

#[test]
fn test_highlight_incomplete_url() {
    // Tests that we can highlight the valid range of a URL that's still being typed.
    // URLs can't end in a `.`, so the detector stops at `www`.
    let test_runs = vec![StyledBufferRun {
        run: "Word https://www. later".to_string(),
        text_styles: Default::default(),
        block_style: BufferBlockStyle::PlainText,
    }];
    assert_eq!(
        highlight_urls(&test_runs),
        [ParsedUrl {
            url_range: 5..16,
            link: "https://www".to_string()
        },]
    )
}

#[test]
fn test_links_not_auto_highlighted() {
    // Test that links whose tags look like URLs aren't auto-linked, but also that they don't
    // prevent auto-linking other URLs.
    let runs = &[
        StyledBufferRun {
            run: "first link is https://warp.dev ".to_string(),
            text_styles: Default::default(),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "http://example.com".to_string(),
            text_styles: TextStylesWithMetadata::default().link("https://warp.dev".to_string()),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: " second is https://google.com".to_string(),
            text_styles: Default::default(),
            block_style: BufferBlockStyle::PlainText,
        },
    ];

    assert_eq!(
        highlight_urls(runs),
        &[
            ParsedUrl {
                url_range: 14..30,
                link: "https://warp.dev".to_string()
            },
            ParsedUrl {
                url_range: 60..78,
                link: "https://google.com".to_string()
            }
        ]
    )
}

#[test]
fn test_highlight_url_before_link() {
    // Test that a URL right before an actual hyperlink is still highlighted.
    let runs = &[
        StyledBufferRun {
            run: "https://example.com".to_string(),
            text_styles: Default::default(),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "hyperlink".to_string(),
            text_styles: TextStylesWithMetadata::default().link("https://example.com".to_string()),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "https://warp.dev".to_string(),
            text_styles: Default::default(),
            block_style: BufferBlockStyle::PlainText,
        },
    ];

    assert_eq!(
        highlight_urls(runs),
        vec![
            ParsedUrl {
                url_range: 0..19,
                link: "https://example.com".to_string()
            },
            ParsedUrl {
                url_range: 28..44,
                link: "https://warp.dev".to_string()
            }
        ]
    )
}

#[test]
fn test_text_around_link_not_auto_highlighted() {
    // Test that text which, without the link in the middle, would be a URL is not auto-linked.
    let runs = &[
        StyledBufferRun {
            run: "ht".to_string(),
            text_styles: Default::default(),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "alink".to_string(),
            text_styles: TextStylesWithMetadata::default().link("https://warp.dev".to_string()),
            block_style: BufferBlockStyle::PlainText,
        },
        StyledBufferRun {
            run: "tps://example.com".to_string(),
            text_styles: Default::default(),
            block_style: BufferBlockStyle::PlainText,
        },
    ];

    assert!(highlight_urls(runs).is_empty());
}

#[test]
fn test_layout_partial_url() {
    // Regression test for laying out a partially-styled autodetected URL (CLD-871).
    App::test((), |app| async move {
        let layout_cache = LayoutCache::new();

        let runs = vec![
            StyledBufferRun {
                run: "A link: https://www.".to_string(),
                text_styles: Default::default(),
                block_style: BufferBlockStyle::PlainText,
            },
            StyledBufferRun {
                run: "example.com".to_string(),
                text_styles: TextStylesWithMetadata::default().bold(),
                block_style: BufferBlockStyle::PlainText,
            },
            StyledBufferRun {
                run: "/path text".to_string(),
                text_styles: Default::default(),
                block_style: BufferBlockStyle::PlainText,
            },
        ];

        app.read(|ctx| {
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );

            let mut line = LayOutArgs::new();
            line.highlighted_urls = highlight_urls(&runs);
            line.next_url_index = 0;

            for run in runs.iter() {
                line.layout_run(
                    &text_layout,
                    run,
                    &text_layout.paragraph_styles(&BufferBlockStyle::PlainText),
                );
            }

            let family_id = TEST_STYLES.base_text.font_family;
            let base_styles =
                StyleAndFont::new(family_id, Properties::default(), TextStyle::default());

            assert_eq!(&line.text, "A link: https://www.example.com/path text");
            assert_eq!(
                &line.style_runs,
                &[
                    (0..8, base_styles),
                    (8..20, add_link_to_style_and_font(base_styles)),
                    (
                        20..31,
                        add_link_to_style_and_font(StyleAndFont::new(
                            family_id,
                            Properties::default().weight(Weight::Bold),
                            TextStyle::default()
                        ))
                    ),
                    (31..36, add_link_to_style_and_font(base_styles)),
                    (36..41, base_styles)
                ]
            )
        });
    })
}

#[test]
fn test_layout_mermaid_block_uses_loaded_svg_aspect_ratio() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let content = "graph TD\nA[Start] --> B[Finish]\n";
        let asset_source = mermaid_asset_source(content);

        let mermaid_load = app.read(|ctx| {
            let asset_cache = AssetCache::as_ref(ctx);
            match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loading { handle } => handle.when_loaded(asset_cache),
                AssetState::Loaded { .. } => None,
                AssetState::Evicted => panic!("Mermaid asset should not be evicted during test"),
                AssetState::FailedToLoad(err) => {
                    panic!("Mermaid asset should load successfully: {err}")
                }
            }
        });
        if let Some(future) = mermaid_load {
            future.await;
        }

        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let block_style = BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Mermaid,
            };
            let block = StyledTextBlock {
                block: vec![StyledBufferRun {
                    run: content.to_string(),
                    text_styles: TextStylesWithMetadata::default(),
                    block_style: block_style.clone(),
                }],
                style: block_style.clone(),
                content_length: CharOffset::from(content.chars().count()),
            };
            let spacing = TEST_STYLES.block_spacings.from_block_style(&block_style);
            let mermaid_diagram = mermaid_diagram_layout(content, &text_layout, spacing, ctx);

            let (item, _has_trailing_newline) = layout_mermaid_diagram_block(
                block,
                mermaid_diagram.0,
                mermaid_diagram.1,
                BlockLocation::Middle,
                false,
            )
            .expect("Mermaid layout should succeed");

            let asset_cache = AssetCache::as_ref(ctx);
            let svg = match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loaded { data } => match data.as_ref() {
                    ImageType::Svg { svg } => svg.clone(),
                    _ => panic!("expected loaded svg asset"),
                },
                AssetState::Loading { .. } => panic!("Mermaid asset should already be loaded"),
                AssetState::Evicted => panic!("Mermaid asset should not be evicted during test"),
                AssetState::FailedToLoad(err) => {
                    panic!("Mermaid asset should load successfully: {err}")
                }
            };

            match &item {
                BlockItem::MermaidDiagram {
                    content_length,
                    config,
                    ..
                } => {
                    let intrinsic_size = svg.size();
                    let expected_width = 800.
                        - TEST_STYLES
                            .block_spacings
                            .from_block_style(&block_style)
                            .x_axis_offset()
                            .as_f32();
                    let expected_height =
                        expected_width * intrinsic_size.height() / intrinsic_size.width();
                    assert_eq!(*content_length, CharOffset::from(content.chars().count()));
                    assert!((config.width.as_f32() - expected_width).abs() < 0.5);
                    assert!((config.height.as_f32() - expected_height).abs() < 0.5);
                    assert!((item.content_height().as_f32() - config.height.as_f32()).abs() < 0.5);
                    assert_eq!(item.lines(), 1.into());
                    assert_eq!(item.first_line_height(), config.height.as_f32());
                }
                item => panic!("expected MermaidDiagram block, got {item:?}"),
            }
        });
    })
}

#[test]
fn test_unloaded_mermaid_diagram_uses_stable_full_width_placeholder_height() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let contents = "graph TD\nA[Unloaded] --> B[Placeholder]\n";
            let block_style = BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Mermaid,
            };
            let spacing = TEST_STYLES.block_spacings.from_block_style(&block_style);
            let (_asset_source, config) =
                mermaid_diagram_layout(contents, &text_layout, spacing, ctx);
            let expected_width = 800. - spacing.x_axis_offset().as_f32();
            let expected_height = TEST_STYLES.base_line_height().as_f32() * 10.;

            assert!(
                (config.width.as_f32() - expected_width).abs() < 0.5,
                "expected unloaded Mermaid diagram width {} to use full available width {}",
                config.width.as_f32(),
                expected_width,
            );
            assert!(
                (config.height.as_f32() - expected_height).abs() < 0.5,
                "expected unloaded Mermaid diagram height {} to use stable placeholder height {}",
                config.height.as_f32(),
                expected_height,
            );
        });
    })
}

fn mermaid_code_block(contents: &str) -> StyledTextBlock {
    let block_style = BufferBlockStyle::CodeBlock {
        code_block_type: CodeBlockType::Mermaid,
    };
    StyledTextBlock {
        block: vec![StyledBufferRun {
            run: contents.to_string(),
            text_styles: TextStylesWithMetadata::default(),
            block_style: block_style.clone(),
        }],
        style: block_style,
        content_length: CharOffset::from(contents.chars().count()),
    }
}

fn mermaid_layout_options() -> RenderLayoutOptions {
    RenderLayoutOptions {
        render_mermaid_diagrams: true,
        ..Default::default()
    }
}

#[test]
fn test_empty_mermaid_block_lays_out_as_code_block() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let block = mermaid_code_block("\n");
            let (item, _has_trailing_newline) =
                layout_mermaid_block_for_test(block, &text_layout, mermaid_layout_options(), ctx)
                    .expect("layout should succeed");

            match item {
                BlockItem::RunnableCodeBlock {
                    code_block_type,
                    pending_mermaid_asset,
                    ..
                } => {
                    assert_eq!(code_block_type, CodeBlockType::Mermaid);
                    assert!(
                        pending_mermaid_asset.is_none(),
                        "empty Mermaid blocks should not trigger a render"
                    );
                }
                other => panic!("expected code block, got {other:?}"),
            }
        });
    })
}

#[test]
fn test_non_parseable_mermaid_block_lays_out_as_code_block() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let block = mermaid_code_block("echo hi\n");
            let (item, _) =
                layout_mermaid_block_for_test(block, &text_layout, mermaid_layout_options(), ctx)
                    .expect("layout should succeed");

            // On first sight, the asset is still loading. We defer the Mermaid-diagram
            // path until the render either succeeds or fails, so the block should lay out
            // as a code block with the asset source attached so the view layer can watch it.
            match item {
                BlockItem::RunnableCodeBlock {
                    code_block_type,
                    pending_mermaid_asset,
                    ..
                } => {
                    assert_eq!(code_block_type, CodeBlockType::Mermaid);
                    assert!(
                        pending_mermaid_asset.is_some(),
                        "a freshly-seen Mermaid source should schedule an asset load"
                    );
                }
                other => panic!("expected code block, got {other:?}"),
            }
        });
    })
}

#[test]
fn test_invalid_mermaid_block_stays_as_code_block_after_load_fails() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let contents = "echo hi\n";
        let asset_source = mermaid_asset_source(contents);

        // Drive the asset load to completion (it should fail, since `echo hi` isn't
        // valid Mermaid).
        let pending = app.read(|ctx| {
            let asset_cache = AssetCache::as_ref(ctx);
            match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loading { handle } => handle.when_loaded(asset_cache),
                _ => None,
            }
        });
        if let Some(future) = pending {
            future.await;
        }

        app.read(|ctx| {
            let asset_cache = AssetCache::as_ref(ctx);
            assert!(
                matches!(
                    asset_cache.load_asset::<ImageType>(asset_source.clone()),
                    AssetState::FailedToLoad(_)
                ),
                "Mermaid render for invalid source should have failed"
            );

            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let block = mermaid_code_block(contents);
            let (item, _) =
                layout_mermaid_block_for_test(block, &text_layout, mermaid_layout_options(), ctx)
                    .expect("layout should succeed");

            match item {
                BlockItem::RunnableCodeBlock {
                    code_block_type,
                    pending_mermaid_asset,
                    ..
                } => {
                    assert_eq!(code_block_type, CodeBlockType::Mermaid);
                    // Once the asset load has failed we don't need to keep watching the
                    // asset handle; the state won't flip again until the source changes.
                    assert!(
                        pending_mermaid_asset.is_none(),
                        "no watcher is needed after a Mermaid render failure"
                    );
                }
                other => panic!("expected code block, got {other:?}"),
            }
        });
    })
}

#[test]
fn test_valid_mermaid_block_lays_out_as_diagram_after_load() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let contents = "graph TD\nA[Start] --> B[Finish]\n";
        let asset_source = mermaid_asset_source(contents);

        // Drive the async Mermaid render to completion.
        let pending = app.read(|ctx| {
            let asset_cache = AssetCache::as_ref(ctx);
            match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loading { handle } => handle.when_loaded(asset_cache),
                _ => None,
            }
        });
        if let Some(future) = pending {
            future.await;
        }

        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let block = mermaid_code_block(contents);
            let (item, _) =
                layout_mermaid_block_for_test(block, &text_layout, mermaid_layout_options(), ctx)
                    .expect("layout should succeed");

            assert!(
                matches!(item, BlockItem::MermaidDiagram { .. }),
                "expected MermaidDiagram block once the asset is loaded, got {item:?}"
            );
        });
    })
}

#[test]
fn test_mermaid_block_skipped_when_render_disabled() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                800.,
            );
            let block = mermaid_code_block("graph TD\nA --> B\n");
            let options = RenderLayoutOptions {
                render_mermaid_diagrams: false,
                ..Default::default()
            };
            let (item, _) = layout_mermaid_block_for_test(block, &text_layout, options, ctx)
                .expect("layout should succeed");

            // When Mermaid rendering is disabled globally, the block should lay out as a
            // plain code block with no asset source attached.
            match item {
                BlockItem::RunnableCodeBlock {
                    pending_mermaid_asset,
                    ..
                } => {
                    assert!(pending_mermaid_asset.is_none());
                }
                other => panic!("expected code block, got {other:?}"),
            }
        });
    })
}

#[test]
fn test_resolve_asset_source_relative_to_directory_uses_base_directory() {
    let asset_source =
        resolve_asset_source_relative_to_directory("diagram.png", Some(Path::new("/tmp/session")));

    match asset_source {
        AssetSource::LocalFile { path } => {
            assert_eq!(Path::new(&path), Path::new("/tmp/session/diagram.png"));
        }
        source => panic!("expected local file asset source, got {source:?}"),
    }
}

#[test]
fn test_layout_text_block_uses_rich_table_when_flag_enabled() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let _flag = FeatureFlag::MarkdownTables.override_enabled(true);
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );
            let content = "short\tmuch longer\ncell\trow\n";
            let block = StyledTextBlock {
                block: vec![StyledBufferRun {
                    run: content.to_string(),
                    text_styles: TextStylesWithMetadata::default(),
                    block_style: BufferBlockStyle::table(Vec::new()),
                }],
                style: BufferBlockStyle::table(Vec::new()),
                content_length: CharOffset::from(content.chars().count()),
            };

            let (item, has_trailing_newline) =
                layout_text_block(block, &text_layout, BlockLocation::Middle, false)
                    .expect("table layout should succeed");

            assert!(matches!(item, BlockItem::Table(_)));
            assert!(!has_trailing_newline);
        });
    })
}

#[test]
fn test_layout_text_block_uses_plain_text_when_flag_disabled() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let _flag = FeatureFlag::MarkdownTables.override_enabled(false);
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );
            let content = "short\tmuch longer\ncell\trow\n";
            let block = StyledTextBlock {
                block: vec![StyledBufferRun {
                    run: content.to_string(),
                    text_styles: TextStylesWithMetadata::default(),
                    block_style: BufferBlockStyle::table(Vec::new()),
                }],
                style: BufferBlockStyle::table(Vec::new()),
                content_length: CharOffset::from(content.chars().count()),
            };

            let (item, _has_trailing_newline) =
                layout_text_block(block, &text_layout, BlockLocation::Middle, false)
                    .expect("table layout should succeed");

            assert!(matches!(item, BlockItem::Paragraph(_)));
        });
    })
}

#[test]
fn test_layout_table_block_caches_cell_text_frames() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );
            let content = "short\tmuch longer\ncell\trow\n";
            let block = StyledTextBlock {
                block: vec![StyledBufferRun {
                    run: content.to_string(),
                    text_styles: TextStylesWithMetadata::default(),
                    block_style: BufferBlockStyle::table(Vec::new()),
                }],
                style: BufferBlockStyle::table(Vec::new()),
                content_length: CharOffset::from(content.chars().count()),
            };

            let table = match layout_table_block(
                block,
                &text_layout,
                TEST_STYLES
                    .block_spacings
                    .from_block_style(&BufferBlockStyle::table(Vec::new())),
            )
            .expect("table layout should succeed")
            {
                BlockItem::Table(table) => table,
                item => panic!("expected table block, got {item:?}"),
            };

            assert_eq!(table.cell_text_frames.len(), 2);
            assert_eq!(table.cell_text_frames[0].len(), 2);
            assert_eq!(table.cell_text_frames[1].len(), 2);
            assert_eq!(table.cell_layouts.len(), 2);
            assert_eq!(table.cell_layouts[0].len(), 2);
            assert_eq!(table.cell_layouts[1].len(), 2);
            assert!(
                table.cell_text_frames[0][1].max_width()
                    <= table.column_widths[1].as_f32() - table.config.style.cell_padding * 2.0
            );
        });
    })
}

#[test]
fn test_layout_table_block_clamps_cell_width_to_max() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );
            // One long cell in the second column that would otherwise blow out the column
            // width. The paragraph has no natural break points within the first 500px, so the
            // cell must rely on the per-cell max width cap to keep the column size bounded.
            let long_content = "word ".repeat(400);
            let content = format!("short\t{long_content}\ncell\trow\n");
            let block = StyledTextBlock {
                block: vec![StyledBufferRun {
                    run: content.clone(),
                    text_styles: TextStylesWithMetadata::default(),
                    block_style: BufferBlockStyle::table(Vec::new()),
                }],
                style: BufferBlockStyle::table(Vec::new()),
                content_length: CharOffset::from(content.chars().count()),
            };

            let table = match layout_table_block(
                block,
                &text_layout,
                TEST_STYLES
                    .block_spacings
                    .from_block_style(&BufferBlockStyle::table(Vec::new())),
            )
            .expect("table layout should succeed")
            {
                BlockItem::Table(table) => table,
                item => panic!("expected table block, got {item:?}"),
            };

            let cell_padding = table.config.style.cell_padding;
            let expected_max_cell_width = cell_padding * 2.0 + 500.0;
            assert!(
                table.column_widths[1].as_f32() <= expected_max_cell_width + f32::EPSILON,
                "long cell column width {} should be clamped to {}",
                table.column_widths[1].as_f32(),
                expected_max_cell_width,
            );
            // The clamped cell frame must be laid out within the clamped column's content
            // width so soft-wrap can occur inside the cell at paint time.
            let max_content_width =
                table.column_widths[1].as_f32() - table.config.style.cell_padding * 2.0;
            assert!(
                table.cell_text_frames[0][1].max_width() <= max_content_width + f32::EPSILON,
                "long cell frame max width {} should fit within clamped content width {}",
                table.cell_text_frames[0][1].max_width(),
                max_content_width,
            );
        });
    })
}

#[test]
fn test_table_inline_style_runs_apply_header_bold_default() {
    App::test((), |app| async move {
        let layout_cache = LayoutCache::new();
        app.read(|ctx| {
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );
            let mut header_style =
                text_layout.paragraph_styles(&BufferBlockStyle::table(Vec::new()));
            header_style.font_weight = Weight::Bold;
            let table = crate::content::text::table_from_internal_format_with_inline_markdown(
                "Header\tValue\nText\tCell\n",
                Vec::new(),
            );

            let layout_input = markdown_inline_to_text_and_style_runs(
                &table.headers[0],
                &header_style,
                Some(header_style.text_color),
                Some(TEST_STYLES.table_style.header_background),
            );

            assert_eq!(layout_input.text, "Header");
            assert!(!layout_input.style_runs.is_empty());
            assert!(
                layout_input
                    .style_runs
                    .iter()
                    .all(|(_, style)| style.properties.weight == Weight::Bold)
            );
        });
    });
}

#[test]
fn test_table_inline_style_runs_preserve_markdown_cell_styles() {
    App::test((), |app| async move {
        let layout_cache = LayoutCache::new();
        app.read(|ctx| {
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );
            let body_style = text_layout.paragraph_styles(&BufferBlockStyle::table(Vec::new()));
            let table = crate::content::text::table_from_internal_format_with_inline_markdown(
                "Header\tValue\nText\t**Bold** *Italic* [Link](https://warp.dev) `code`\n",
                Vec::new(),
            );

            let layout_input = markdown_inline_to_text_and_style_runs(
                &table.rows[0][1],
                &body_style,
                Some(body_style.text_color),
                Some(TEST_STYLES.table_style.cell_background),
            );

            assert_eq!(layout_input.text, "Bold Italic Link code");
            assert_eq!(layout_input.style_runs.len(), 7);

            assert_eq!(layout_input.style_runs[0].0, 0..4);
            assert_eq!(layout_input.style_runs[0].1.properties.weight, Weight::Bold);

            assert_eq!(layout_input.style_runs[2].0, 5..11);
            assert_eq!(layout_input.style_runs[2].1.properties.style, Style::Italic);

            assert_eq!(layout_input.style_runs[4].0, 12..16);
            assert!(
                layout_input.style_runs[4]
                    .1
                    .style
                    .foreground_color
                    .is_some()
            );
            assert!(layout_input.style_runs[4].1.style.underline_color.is_some());

            assert_eq!(layout_input.style_runs[6].0, 17..21);
            assert!(
                layout_input.style_runs[6]
                    .1
                    .style
                    .background_color
                    .is_some()
            );
        });
    });
}

#[test]
fn test_layout_code_block_urls() {
    // Regression test for laying out URLs in a code block, which contains multiple lines.
    App::test((), |app| async move {
        let runs = vec![
            StyledBufferRun {
                run: "curl -o myfile.txt http://example.com/myfile.txt\n".to_string(),
                text_styles: Default::default(),
                block_style: BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
            },
            StyledBufferRun {
                run: "vim myfile.txt\n".to_string(),
                text_styles: Default::default(),
                block_style: BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
            },
            StyledBufferRun {
                run: "rsync myfile.txt ssh://user@server.com\n".to_string(),
                text_styles: Default::default(),
                block_style: BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
            },
        ];

        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );
            let paragraph_styles = text_layout.paragraph_styles(&BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            });
            let family_id = TEST_STYLES.code_text.font_family;
            let base_styles =
                StyleAndFont::new(family_id, Properties::default(), TextStyle::default());

            let mut line = LayOutArgs::new();
            line.highlighted_urls = highlight_urls(&runs);
            line.next_url_index = 0;

            // First, make sure that we detected the URLs correctly.
            assert_eq!(
                &line.highlighted_urls,
                &[
                    ParsedUrl {
                        url_range: 19..48,
                        link: "http://example.com/myfile.txt".to_string()
                    },
                    ParsedUrl {
                        // URL offsets count painted characters, not newlines.
                        url_range: 79..100,
                        link: "ssh://user@server.com".to_string()
                    }
                ]
            );

            // Lay out each line of code 1 by 1 to verify the intermediate state.

            assert!(line.layout_run(&text_layout, &runs[0], &paragraph_styles));
            assert_eq!(
                &line.text,
                "curl -o myfile.txt http://example.com/myfile.txt"
            );
            assert_eq!(
                &line.style_runs,
                &[
                    (0..19, base_styles),
                    (19..48, add_link_to_style_and_font(base_styles)),
                ]
            );

            line.reset_for_newline();
            assert!(line.layout_run(&text_layout, &runs[1], &paragraph_styles));
            assert_eq!(&line.text, "vim myfile.txt");
            assert_eq!(&line.style_runs, &[(0..14, base_styles)]);

            line.reset_for_newline();
            assert!(line.layout_run(&text_layout, &runs[2], &paragraph_styles));
            assert_eq!(&line.text, "rsync myfile.txt ssh://user@server.com");
            assert_eq!(
                &line.style_runs,
                &[
                    (0..17, base_styles),
                    (17..38, add_link_to_style_and_font(base_styles)),
                ]
            );
        });
    })
}
