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
use warpui_core::color::ColorU;
use warpui_core::elements::{Border, Fill};
use warpui_core::fonts::{FamilyId, Weight};
use warpui_core::text_layout::LayoutCache;
use warpui_core::units::IntoPixels;
use warpui_core::App;

const BLOCK_COUNT: usize = 4_096;
const BLOCK_TEXT: &str =
    "fn layout_parallel_editor_block(value: usize) -> usize { value.saturating_add(1) }\n";

fn benchmark_styles() -> RichTextStyles {
    let white = ColorU::white();
    let paragraph = ParagraphStyles {
        font_family: FamilyId(0),
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
            font_family: FamilyId(0),
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
            font_family: FamilyId(0),
            font_size: 13.,
            cell_padding: 8.,
            outer_border: true,
            column_dividers: true,
            row_dividers: true,
        },
    }
}

fn benchmark_delta() -> EditDelta {
    let block = StyledBufferBlock::Text(StyledTextBlock {
        block: vec![StyledBufferRun {
            run: BLOCK_TEXT.to_owned(),
            text_styles: TextStylesWithMetadata::default(),
            block_style: BufferBlockStyle::PlainText,
        }],
        style: BufferBlockStyle::PlainText,
        content_length: CharOffset::from(BLOCK_TEXT.chars().count()),
    });
    EditDelta {
        old_offset: CharOffset::from(1)
            ..CharOffset::from(1 + BLOCK_COUNT * BLOCK_TEXT.chars().count()),
        new_lines: Arc::new(vec![block; BLOCK_COUNT]),
        ..Default::default()
    }
}

fn text_layout_benchmark(criterion: &mut Criterion) {
    let delta = benchmark_delta();
    let styles = benchmark_styles();
    let layout_options = RenderLayoutOptions::default();
    let chars = BLOCK_COUNT * BLOCK_TEXT.chars().count();
    let mut criterion = std::mem::take(criterion);

    App::test((), move |app| async move {
        app.read(|ctx| {
            let mut group = criterion.benchmark_group("editor_text_layout");
            group.throughput(Throughput::Elements(chars as u64));
            group.bench_function("layout_delta_4096_blocks", |bench| {
                bench.iter(|| {
                    let layout_cache = LayoutCache::new();
                    let text_layout = TextLayout::new(
                        &layout_cache,
                        ctx.font_cache().text_layout_system(),
                        &styles,
                        f32::MAX,
                    );
                    black_box(delta.layout_delta(
                        &text_layout,
                        None,
                        &layout_options,
                        None,
                        ctx,
                    ))
                })
            });
            group.finish();
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5));
    targets = text_layout_benchmark
}
criterion_main!(benches);
