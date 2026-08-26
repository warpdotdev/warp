use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use rangemap::RangeSet;
use string_offset::CharOffset;
use warp_core::features::FeatureFlag;
use warpui_core::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui_core::fonts::{Properties, Style, Weight};
use warpui_core::image_cache::ImageType;
use warpui_core::text_layout::{LayoutCache, StyleAndFont, TextStyle};
use warpui_core::{App, SingletonEntity};

use super::{
    BlockLocation, LayOutArgs, LayoutTask, MAX_LAYOUT_CONTENT_CHARS_PER_PARALLEL_CHUNK,
    MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK, chunk_layout_tasks, layout_mermaid_diagram_block,
    layout_table_block, layout_temporary_blocks, layout_text_block,
};
use crate::content::buffer::{StyledBufferBlock, StyledBufferRun, StyledTextBlock};
use crate::content::edit::{
    EditDelta, ParsedUrl, TemporaryBlock, highlight_urls, layout_mermaid_block_for_test,
    resolve_asset_source, resolve_asset_source_relative_to_directory,
};
use crate::content::mermaid_diagram::{mermaid_asset_source, mermaid_diagram_layout};
use crate::content::text::{BufferBlockStyle, CodeBlockType, TextStylesWithMetadata};
use crate::render::layout::{
    TextLayout, add_link_to_style_and_font, markdown_inline_to_text_and_style_runs,
};
use crate::render::model::test_utils::TEST_STYLES;
use crate::render::model::{
    BlockItem, CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES, LineCount, RenderLayoutOptions,
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
fn test_layout_delta_never_takes_ownership_of_new_lines_with_multiple_owners() {
    // Regression test for APP-4844: `EditDelta::new_lines` is wrapped in an `Arc` so that
    // cloning a delta (e.g. to stash it in `DelayRendering::edits`, or because multiple editors
    // share the same underlying buffer) is O(1) instead of O(file size). `layout_delta` must not
    // depend on `new_lines` having a single owner to stay cheap: it takes `&self` and only ever
    // borrows through the `Arc`, so laying out a delta can never fall back to cloning the whole
    // (potentially file-sized) block list, no matter how many clones of the delta are alive.
    //
    // This exercises `layout_delta` itself (not just raw `Arc` semantics) with two live clones of
    // the same delta -- the exact shape of two `CodeEditorModel`s sharing one buffer, each
    // holding their own clone of the `ContentChanged` event's delta -- and confirms neither
    // layout call touches the `Arc`'s strong count or invalidates the other clone.
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );

            let block = StyledBufferBlock::Text(StyledTextBlock {
                block: vec![StyledBufferRun {
                    run: "hello\n".to_string(),
                    text_styles: Default::default(),
                    block_style: BufferBlockStyle::PlainText,
                }],
                style: BufferBlockStyle::PlainText,
                content_length: CharOffset::from(6),
            });

            let delta = EditDelta {
                new_lines: Arc::new(vec![block]),
                old_offset: CharOffset::from(1)..CharOffset::from(1),
                ..EditDelta::default()
            };

            // Simulate two editors sharing the same buffer, each holding their own clone of the
            // delta emitted by the shared `ContentChanged` event.
            let editor_a_delta = delta.clone();
            let editor_b_delta = delta.clone();
            drop(delta);
            assert_eq!(Arc::strong_count(&editor_a_delta.new_lines), 2);

            let laid_out_a = editor_a_delta.layout_delta(
                &text_layout,
                None,
                &RenderLayoutOptions::default(),
                None,
                ctx,
            );
            assert_eq!(laid_out_a.laid_out_line.len(), 1);
            assert_eq!(
                Arc::strong_count(&editor_a_delta.new_lines),
                2,
                "layout_delta must not take ownership of new_lines"
            );

            let laid_out_b = editor_b_delta.layout_delta(
                &text_layout,
                None,
                &RenderLayoutOptions::default(),
                None,
                ctx,
            );
            assert_eq!(laid_out_b.laid_out_line.len(), 1);
            assert_eq!(
                Arc::strong_count(&editor_a_delta.new_lines),
                2,
                "both clones of the delta must remain valid, sharing the same allocation, after layout"
            );
            assert!(Arc::ptr_eq(&editor_a_delta.new_lines, &editor_b_delta.new_lines));
        });
    })
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
                &block,
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
        AssetSource::LocalFile { path, .. } => {
            assert_eq!(Path::new(&path), Path::new("/tmp/session/diagram.png"));
        }
        source => panic!("expected local file asset source, got {source:?}"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn unique_markdown_image_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "warp_editor_markdown_image_{}_{nonce}.png",
        std::process::id()
    ))
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_resolve_asset_source_versions_local_files_for_markdown_layout() {
    let image_path = unique_markdown_image_path();
    std::fs::write(&image_path, b"old").expect("write initial image contents");
    let document_path = image_path.with_file_name("document.md");
    let image_name = image_path
        .file_name()
        .expect("image path should have a file name")
        .to_string_lossy();

    let initial = resolve_asset_source(&image_name, Some(&document_path));
    assert!(matches!(
        &initial,
        AssetSource::LocalFile {
            content_version: Some(_),
            ..
        }
    ));

    std::fs::write(&image_path, b"updated image contents").expect("update image contents");
    let updated = resolve_asset_source(&image_name, Some(&document_path));
    assert_ne!(
        initial, updated,
        "an updated Markdown image should produce a new cache key"
    );

    let _ = std::fs::remove_file(image_path);
}

#[test]
fn test_resolve_asset_source_leaves_non_local_markdown_images_unchanged() {
    let document_path = Path::new("/tmp/document.md");
    let base_directory = document_path.parent();

    for source in [
        "https://example.com/image.png",
        "data:image/png;base64,iVBORw0KGgo=",
    ] {
        assert_eq!(
            resolve_asset_source(source, Some(document_path)),
            resolve_asset_source_relative_to_directory(source, base_directory),
            "non-local source should not be changed: {source}"
        );
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
                layout_text_block(&block, &text_layout, BlockLocation::Middle, false)
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
                layout_text_block(&block, &text_layout, BlockLocation::Middle, false)
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
                &block,
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
                &block,
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

#[test]
fn test_chunk_layout_tasks_bounds_by_task_count() {
    let make_tasks = |count: usize| -> Vec<(LayoutTask, bool, usize)> {
        (0..count)
            .map(|_| {
                (
                    LayoutTask::temporary_block(String::new(), None, vec![]),
                    false,
                    1,
                )
            })
            .collect()
    };

    let chunks = chunk_layout_tasks(make_tasks(MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK + 5));
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK);
    assert_eq!(chunks[1].len(), 5);

    // Exactly at the cap should still be a single chunk.
    let chunks = chunk_layout_tasks(make_tasks(MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK));
    assert_eq!(chunks.len(), 1);
}

#[test]
fn test_chunk_layout_tasks_bounds_by_content_length() {
    let make_tasks = |lengths: &[usize]| -> Vec<(LayoutTask, bool, usize)> {
        lengths
            .iter()
            .map(|&len| {
                (
                    LayoutTask::temporary_block(String::new(), None, vec![]),
                    false,
                    len,
                )
            })
            .collect()
    };

    // Each task is over half the content-length cap, so every task should start a new chunk
    // well before the task-count cap is reached.
    let oversized = MAX_LAYOUT_CONTENT_CHARS_PER_PARALLEL_CHUNK / 2 + 1;
    let chunks = chunk_layout_tasks(make_tasks(&[oversized, oversized, oversized]));
    assert_eq!(
        chunks.len(),
        3,
        "each oversized task should start a new chunk"
    );
    assert!(chunks.iter().all(|chunk| chunk.len() == 1));

    // A single task larger than the cap must still get its own chunk rather than stalling.
    let huge = MAX_LAYOUT_CONTENT_CHARS_PER_PARALLEL_CHUNK * 4;
    let chunks = chunk_layout_tasks(make_tasks(&[huge]));
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), 1);
}

/// Builds a single-line, single-run PlainText block whose content is exactly `content_len`
/// characters (including the trailing newline), so its laid-out `content_length` uniquely
/// identifies the block for order verification.
fn identifiable_text_block(content_len: usize) -> StyledBufferBlock {
    let run = "a".repeat(content_len - 1) + "\n";
    StyledBufferBlock::Text(StyledTextBlock {
        block: vec![StyledBufferRun {
            run,
            text_styles: TextStylesWithMetadata::default(),
            block_style: BufferBlockStyle::PlainText,
        }],
        style: BufferBlockStyle::PlainText,
        content_length: CharOffset::from(content_len),
    })
}

#[test]
fn test_layout_delta_chunk_boundary_preserves_order_hidden_collapsing_and_trailing_newline() {
    // Regression test for APP-5392: bounding EditDelta::layout_delta's parallel fan-out into
    // chunks must not change its observable behavior. This delta spans multiple chunks (given
    // MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK), with a hidden run that straddles a chunk boundary.
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );

            const TOTAL_BLOCKS: usize = MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK * 2 + 12;
            // Straddles the boundary between the first and second chunks.
            const HIDDEN_START: usize = MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK - 4;
            const HIDDEN_END: usize = MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK + 11;

            let mut new_lines = Vec::with_capacity(TOTAL_BLOCKS);
            let mut block_starts = Vec::with_capacity(TOTAL_BLOCKS);
            let mut content_lengths = Vec::with_capacity(TOTAL_BLOCKS);
            let mut offset = CharOffset::from(1);

            for i in 0..TOTAL_BLOCKS {
                let content_len = i + 3;
                block_starts.push(offset);
                content_lengths.push(content_len);
                new_lines.push(identifiable_text_block(content_len));
                offset += content_len;
            }

            let mut hidden_ranges = RangeSet::new();
            hidden_ranges.insert(block_starts[HIDDEN_START]..block_starts[HIDDEN_END]);

            let delta = EditDelta {
                old_offset: CharOffset::from(1)..offset,
                new_lines: Arc::new(new_lines),
                ..Default::default()
            };

            let laid_out = delta.layout_delta(
                &text_layout,
                None,
                &RenderLayoutOptions::default(),
                Some(hidden_ranges),
                ctx,
            );

            // The contiguous hidden run should collapse into exactly one BlockItem::Hidden.
            let expected_len = HIDDEN_START + 1 + (TOTAL_BLOCKS - HIDDEN_END);
            assert_eq!(laid_out.laid_out_line.len(), expected_len);

            for (item, &expected_block_len) in laid_out.laid_out_line[0..HIDDEN_START]
                .iter()
                .zip(&content_lengths[0..HIDDEN_START])
            {
                assert!(
                    matches!(item, BlockItem::Paragraph(_)),
                    "expected a visible block, got {item:?}"
                );
                assert_eq!(item.content_length(), CharOffset::from(expected_block_len));
            }

            let hidden_item = &laid_out.laid_out_line[HIDDEN_START];
            assert!(
                matches!(hidden_item, BlockItem::Hidden(_)),
                "the hidden run should collapse to a single item, got {hidden_item:?}"
            );
            let expected_hidden_length: usize =
                content_lengths[HIDDEN_START..HIDDEN_END].iter().sum();
            assert_eq!(
                hidden_item.content_length(),
                CharOffset::from(expected_hidden_length)
            );

            for (item, &expected_block_len) in laid_out.laid_out_line[HIDDEN_START + 1..]
                .iter()
                .zip(&content_lengths[HIDDEN_END..TOTAL_BLOCKS])
            {
                assert!(
                    matches!(item, BlockItem::Paragraph(_)),
                    "expected a visible block, got {item:?}"
                );
                assert_eq!(item.content_length(), CharOffset::from(expected_block_len));
            }

            // The last block is visible and ends with a newline, so the delta should report a
            // trailing newline, matching what an unchunked single-pass layout would produce.
            assert!(laid_out.trailing_newline.is_some());
        });
    })
}

#[test]
fn test_layout_delta_single_chunk_matches_direct_layout() {
    // A delta that fits within a single chunk should behave identically to laying out each
    // block directly: no hidden collapsing, and a trailing newline exactly when the last block
    // ends in one.
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );

            let new_lines = vec![
                identifiable_text_block(3),
                identifiable_text_block(4),
                identifiable_text_block(5),
            ];
            let total_len: usize = new_lines
                .iter()
                .map(StyledBufferBlock::content_length)
                .map(CharOffset::as_usize)
                .sum();

            let delta = EditDelta {
                old_offset: CharOffset::from(1)..CharOffset::from(1 + total_len),
                new_lines: Arc::new(new_lines),
                ..Default::default()
            };

            let laid_out = delta.layout_delta(
                &text_layout,
                None,
                &RenderLayoutOptions::default(),
                None,
                ctx,
            );

            assert_eq!(laid_out.laid_out_line.len(), 3);
            assert_eq!(
                laid_out.laid_out_line[0].content_length(),
                CharOffset::from(3)
            );
            assert_eq!(
                laid_out.laid_out_line[1].content_length(),
                CharOffset::from(4)
            );
            assert_eq!(
                laid_out.laid_out_line[2].content_length(),
                CharOffset::from(5)
            );
            assert!(laid_out.trailing_newline.is_some());
        });
    })
}

/// Builds a hidden, isolated `CodeBlock`-styled block whose gutter-button count (and thus its
/// laid-out `line_count`) directly observes the `BlockLocation` it was laid out with: Start/End
/// always get one button, but a genuine Middle location with `run_count >=
/// CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES` gets two. `run_count` only matters for the Middle
/// case; the run contents are never read since the block is hidden.
fn isolated_hidden_code_block(run_count: usize, content_len: usize) -> StyledBufferBlock {
    let style = BufferBlockStyle::CodeBlock {
        code_block_type: CodeBlockType::Shell,
    };
    StyledBufferBlock::Text(StyledTextBlock {
        block: vec![
            StyledBufferRun {
                run: String::new(),
                text_styles: TextStylesWithMetadata::default(),
                block_style: style.clone(),
            };
            run_count
        ],
        style,
        content_length: CharOffset::from(content_len),
    })
}

#[test]
fn test_layout_delta_block_location_is_global_across_chunk_boundaries() {
    // Regression test for APP-5392: BlockLocation must be computed from the delta's global
    // index, not a chunk-local one. A hidden block's gutter-button count only depends on its
    // BlockLocation when it's genuinely Middle with a large enough hidden run (2 buttons) vs.
    // Start/End (always 1 button), so an isolated hidden block at a later chunk's first index
    // makes a chunk-local-index regression directly observable.
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );

            const TOTAL_BLOCKS: usize = MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK * 2 + 10;
            const HIDDEN_AT_START: usize = 0;
            const HIDDEN_AT_TRUE_MIDDLE: usize = 5;
            // The first index of the second chunk: local index 0, but not the global start.
            const HIDDEN_AT_CHUNK_BOUNDARY: usize = MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK;
            const HIDDEN_AT_END: usize = TOTAL_BLOCKS - 1;
            const RUN_COUNT: usize = CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES + 5;

            let hidden_indices = [
                HIDDEN_AT_START,
                HIDDEN_AT_TRUE_MIDDLE,
                HIDDEN_AT_CHUNK_BOUNDARY,
                HIDDEN_AT_END,
            ];

            let mut new_lines = Vec::with_capacity(TOTAL_BLOCKS);
            let mut block_starts = Vec::with_capacity(TOTAL_BLOCKS);
            let mut offset = CharOffset::from(1);

            for i in 0..TOTAL_BLOCKS {
                block_starts.push(offset);
                if hidden_indices.contains(&i) {
                    new_lines.push(isolated_hidden_code_block(RUN_COUNT, 1));
                    offset += 1;
                } else {
                    new_lines.push(identifiable_text_block(3));
                    offset += 3;
                }
            }

            // Each hidden index is isolated (its neighbors are visible), so none of them merge.
            let mut hidden_ranges = RangeSet::new();
            for &i in &hidden_indices {
                hidden_ranges.insert(block_starts[i]..block_starts[i] + CharOffset::from(1));
            }

            let delta = EditDelta {
                old_offset: CharOffset::from(1)..offset,
                new_lines: Arc::new(new_lines),
                ..Default::default()
            };

            let laid_out = delta.layout_delta(
                &text_layout,
                None,
                &RenderLayoutOptions::default(),
                Some(hidden_ranges),
                ctx,
            );

            // No collapsing occurred, so output indices line up with input indices.
            assert_eq!(laid_out.laid_out_line.len(), TOTAL_BLOCKS);

            let line_count_at = |global_idx: usize| match &laid_out.laid_out_line[global_idx] {
                BlockItem::Hidden(config) => config.line_count(),
                other => panic!("expected a Hidden item at index {global_idx}, got {other:?}"),
            };

            assert_eq!(
                line_count_at(HIDDEN_AT_START),
                LineCount::from(1),
                "genuine Start should always get a single gutter button"
            );
            assert_eq!(
                line_count_at(HIDDEN_AT_TRUE_MIDDLE),
                LineCount::from(2),
                "genuine Middle with a large hidden block should get two gutter buttons"
            );
            assert_eq!(
                line_count_at(HIDDEN_AT_CHUNK_BOUNDARY),
                LineCount::from(2),
                "a later chunk's first task is still Middle (global index), not Start (chunk-local index)"
            );
            assert_eq!(
                line_count_at(HIDDEN_AT_END),
                LineCount::from(1),
                "genuine End should always get a single gutter button"
            );
        });
    })
}

#[test]
fn test_layout_delta_trailing_newline_carries_over_when_final_chunk_fully_fails() {
    // Regression test for APP-5392: when every task in the final chunk fails, the
    // trailing-newline result must still come from the last *successful* task in an earlier
    // chunk, matching the old single-pass find_last() semantics over the whole (possibly
    // filtered) sequence, rather than resetting to the default because the last chunk
    // contributed nothing.
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );

            const CHUNK_SIZE: usize = MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK;
            const FAILING_TASKS: usize = 5;
            const TOTAL_BLOCKS: usize = CHUNK_SIZE * 2 + FAILING_TASKS;
            // The last successful task, at the end of the second chunk.
            const LAST_SUCCESSFUL_INDEX: usize = CHUNK_SIZE * 2 - 1;

            let mut new_lines = Vec::with_capacity(TOTAL_BLOCKS);
            let mut offset = CharOffset::from(1);

            for i in 0..TOTAL_BLOCKS {
                if i >= CHUNK_SIZE * 2 {
                    // The entire final chunk fails: an empty CodeBlock has no runs, so no
                    // paragraph is ever pushed and layout_text_block errors instead of
                    // producing a trailing-newline value.
                    new_lines.push(StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: CodeBlockType::Shell,
                        },
                        content_length: CharOffset::from(3),
                    }));
                    offset += 3;
                } else if i == LAST_SUCCESSFUL_INDEX {
                    // No trailing newline, so this is distinguishable from the `true` default.
                    new_lines.push(StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ab".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText,
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2),
                    }));
                    offset += 2;
                } else {
                    new_lines.push(identifiable_text_block(3));
                    offset += 3;
                }
            }

            let delta = EditDelta {
                old_offset: CharOffset::from(1)..offset,
                new_lines: Arc::new(new_lines),
                ..Default::default()
            };

            let laid_out = delta.layout_delta(
                &text_layout,
                None,
                &RenderLayoutOptions::default(),
                None,
                ctx,
            );

            // Every task in the final chunk failed and was dropped.
            assert_eq!(laid_out.laid_out_line.len(), TOTAL_BLOCKS - FAILING_TASKS);
            assert!(
                laid_out.trailing_newline.is_none(),
                "trailing newline should come from the last successful task (none), not the default that would result from losing an earlier chunk's result"
            );
        });
    })
}

#[test]
fn test_layout_temporary_blocks_preserves_order_across_chunk_boundary() {
    // layout_temporary_blocks shares chunk_layout_tasks with EditDelta::layout_delta (APP-5392);
    // verify a batch spanning multiple chunks still groups its blocks by destination line in
    // their original order.
    App::test((), |app| async move {
        app.read(|ctx| {
            let layout_cache = LayoutCache::new();
            let text_layout = TextLayout::new(
                &layout_cache,
                ctx.font_cache().text_layout_system(),
                &TEST_STYLES,
                f32::MAX,
            );

            const TOTAL_BLOCKS: usize = MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK * 2 + 3;
            let insert_before = LineCount::from(5);

            let blocks: Vec<_> = (0..TOTAL_BLOCKS)
                .map(|i| TemporaryBlock {
                    content: format!("line-{i}\n"),
                    insert_before,
                    line_decoration: None,
                    inline_text_decorations: Vec::new(),
                })
                .collect();

            let mut result = layout_temporary_blocks(blocks, &text_layout);
            let items = result
                .remove(&insert_before)
                .expect("all blocks share the same destination line");

            assert_eq!(items.len(), TOTAL_BLOCKS);
            for item in &items {
                assert!(matches!(item, BlockItem::TemporaryBlock { .. }));
            }
        });
    })
}
