use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use string_offset::CharOffset;
use warp_editor::content::buffer::{StyledBufferBlock, StyledBufferRun, StyledTextBlock};
use warp_editor::content::edit::EditDelta;
use warp_editor::content::text::{BufferBlockStyle, TextStylesWithMetadata};
use warp_editor::render::layout::TextLayout;
use warp_editor::render::model::{
    BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle, ParagraphStyles,
    RenderLayoutOptions, RichTextStyles, TableStyle,
};
use warpui_core::App;
use warpui_core::color::ColorU;
use warpui_core::elements::{Border, Fill};
use warpui_core::fonts::{FamilyId, Weight};
use warpui_core::units::IntoPixels;
#[cfg(target_os = "macos")]
use {warpui::platform::mac::FontDB as MacFontDB, warpui_core::fonts::Cache as FontCache};

const BLOCK_COUNT: usize = 4_096;
const BLOCK_TEXT: &str =
    "fn layout_parallel_editor_block(value: usize) -> usize { value.saturating_add(1) }\n";
fn benchmark_block(text: String) -> StyledBufferBlock {
    let content_length = text.chars().count();
    StyledBufferBlock::Text(StyledTextBlock {
        block: vec![StyledBufferRun {
            run: text,
            text_styles: TextStylesWithMetadata::default(),
            block_style: BufferBlockStyle::PlainText,
        }],
        style: BufferBlockStyle::PlainText,
        content_length: CharOffset::from(content_length),
    })
}

fn benchmark_styles(font_family: FamilyId) -> RichTextStyles {
    let white = ColorU::white();
    let paragraph = ParagraphStyles {
        font_family,
        font_size: 13.,
        font_weight: Weight::Normal,
        line_height_ratio: 1.2,
        text_color: white,
        baseline_ratio: 0.8,
        fixed_width_tab_size: None,
    };
    RichTextStyles {
        base_text: paragraph,
        code_text: ParagraphStyles {
            fixed_width_tab_size: Some(4),
            ..paragraph
        },
        code_background: Fill::None,
        embedding_background: Fill::None,
        embedding_text: paragraph,
        code_border: Border::new(0.),
        placeholder_color: white,
        selection_fill: Fill::None,
        cursor_fill: Fill::None,
        inline_code_style: InlineCodeStyle {
            font_family,
            background: white,
            font_color: white,
        },
        check_box_style: CheckBoxStyle {
            border_width: 2.,
            border_color: white,
            icon_path: "bundled/svg/check-thick.svg",
            background: white,
            hover_background: white,
        },
        horizontal_rule_style: HorizontalRuleStyle {
            rule_height: 2.,
            color: white,
        },
        broken_link_style: BrokenLinkStyle {
            icon_path: "bundled/svg/link-broken-02.svg",
            icon_color: white,
        },
        block_spacings: Default::default(),
        minimum_paragraph_height: Some(24.0.into_pixels()),
        show_placeholder_text_on_empty_block: false,
        cursor_width: 1.,
        highlight_urls: false,
        table_style: TableStyle {
            border_color: white,
            header_background: white,
            cell_background: white,
            alternate_row_background: None,
            text_color: white,
            header_text_color: white,
            scrollbar_nonactive_thumb_color: white,
            scrollbar_active_thumb_color: white,
            font_family,
            font_size: 13.,
            cell_padding: 8.,
            outer_border: true,
            column_dividers: true,
            row_dividers: true,
        },
    }
}

fn benchmark_delta(texts: impl IntoIterator<Item = String>) -> (EditDelta, usize) {
    let blocks: Vec<_> = texts.into_iter().map(benchmark_block).collect();
    let chars = blocks
        .iter()
        .map(StyledBufferBlock::content_length)
        .map(CharOffset::as_usize)
        .sum();
    (
        EditDelta {
            old_offset: CharOffset::from(1)..CharOffset::from(1 + chars),
            new_lines: Arc::new(blocks),
            ..Default::default()
        },
        chars,
    )
}

fn test_backend_text_layout_benchmark(criterion: &mut Criterion) {
    let (delta, chars) = benchmark_delta(std::iter::repeat_n(BLOCK_TEXT.to_owned(), BLOCK_COUNT));
    let styles = benchmark_styles(FamilyId(0));
    let layout_options = RenderLayoutOptions::default();
    let mut criterion = std::mem::take(criterion);

    App::test((), move |app| async move {
        app.read(|ctx| {
            let mut group = criterion.benchmark_group("editor_text_layout/test_backend");
            group.throughput(Throughput::Elements(chars as u64));
            group.bench_function("layout_delta_4096_identical_blocks", |bench| {
                bench.iter(|| {
                    let text_layout =
                        TextLayout::new(ctx.font_cache().text_layout_system(), &styles, f32::MAX);
                    black_box(delta.layout_delta(&text_layout, None, &layout_options, None, ctx))
                })
            });
            group.finish();
        });
    });
}
#[cfg(target_os = "macos")]
fn core_text_layout_benchmark(criterion: &mut Criterion) {
    let mut font_cache = FontCache::new(Box::new(MacFontDB::new()));
    let font_family = font_cache
        .load_system_font("Menlo")
        .expect("Menlo should be available on macOS");
    let (delta, chars) = benchmark_delta((0..BLOCK_COUNT).map(|index| {
        format!(
            "fn layout_parallel_editor_block_{index}(value: usize) -> usize {{ value.saturating_add(1) }}\n"
        )
    }));
    let styles = benchmark_styles(font_family);
    let layout_options = RenderLayoutOptions::default();
    let mut criterion = std::mem::take(criterion);

    App::test((), move |app| async move {
        app.read(|ctx| {
            let mut group = criterion.benchmark_group("editor_text_layout/core_text");
            group.throughput(Throughput::Elements(chars as u64));
            group.bench_function("layout_delta_4096_unique_blocks", |bench| {
                bench.iter(|| {
                    let text_layout =
                        TextLayout::new(font_cache.text_layout_system(), &styles, f32::MAX);
                    black_box(delta.layout_delta(&text_layout, None, &layout_options, None, ctx))
                })
            });
            group.finish();
        });
    });
}

#[cfg(target_os = "macos")]

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5));
    targets = test_backend_text_layout_benchmark, core_text_layout_benchmark
}
#[cfg(not(target_os = "macos"))]
criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5));
    targets = test_backend_text_layout_benchmark
}
criterion_main!(benches);
