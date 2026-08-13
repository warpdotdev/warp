//! Test helpers for all render model tests.

use std::mem;
use std::sync::Arc;

use ordered_float::OrderedFloat;
use parking_lot::Once;
use vec1::{Vec1, vec1};
use warpui_core::color::ColorU;
use warpui_core::elements::{Border, Fill, ListIndentLevel};
use warpui_core::fonts::{FamilyId, Weight};
use warpui_core::geometry::vector::vec2f;
use warpui_core::text_layout::{CaretPosition, Glyph, Line, Run, TextFrame};
use warpui_core::units::{IntoPixels, Pixels};

use super::{
    BlockItem, BrokenLinkStyle, CheckBoxStyle, DEFAULT_BLOCK_SPACINGS, HorizontalRuleStyle,
    InlineCodeStyle, OffsetMap, PARAGRAPH_MIN_HEIGHT, Paragraph, ParagraphStyles, RichTextStyles,
    TEXT_SPACING, TableStyle,
};
use crate::content::text::BufferBlockStyle;

pub const TEST_BASELINE_OFFSET: f32 = 0.7;

/// Create a new paragraph that occupies the given space but has no content.
pub fn mock_paragraph(height: f32, width: f32, content_length: usize) -> BlockItem {
    let frame = TextFrame::new(
        vec1![Line {
            width,
            trailing_whitespace_width: 0.,
            runs: Vec::new(),
            // The line's effective height is determined by its font size and line height
            // ratio, so set those to produce the expected height,
            font_size: height,
            line_height_ratio: 1.,
            baseline_ratio: TEST_BASELINE_OFFSET,
            ascent: height * TEST_BASELINE_OFFSET,
            descent: height * (1. - TEST_BASELINE_OFFSET),
            clip_config: None,
            caret_positions: Vec::new(),
            chars_with_missing_glyphs: Vec::new(),
        }],
        width,
        Default::default(),
    );

    BlockItem::paragraph(
        Arc::new(frame),
        OffsetMap::direct(content_length),
        content_length.into(),
        TEXT_SPACING,
        Some(PARAGRAPH_MIN_HEIGHT),
    )
}

/// Create a new paragraph block laid out via [`layout`].
pub fn laid_out_paragraph(
    text: &str,
    styles: &RichTextStyles,
    max_width: impl IntoPixels,
) -> BlockItem {
    BlockItem::Paragraph(layout_paragraph(
        text,
        styles,
        &BufferBlockStyle::PlainText,
        max_width,
    ))
}

/// Create a new paragraph block laid out via [`layout`].
pub fn laid_out_unordered_lists(
    text: &str,
    styles: &RichTextStyles,
    max_width: impl IntoPixels,
) -> BlockItem {
    BlockItem::UnorderedList {
        indent_level: ListIndentLevel::One,
        paragraph: layout_paragraph(
            text,
            styles,
            &BufferBlockStyle::UnorderedList {
                indent_level: ListIndentLevel::One,
            },
            max_width,
        ),
    }
}

/// Lays out a single paragraph of text.
pub fn layout_paragraph(
    text: &str,
    styles: &RichTextStyles,
    block_style: &BufferBlockStyle,
    max_width: impl IntoPixels,
) -> Paragraph {
    let Some(text) = text.strip_suffix('\n') else {
        panic!("Laid out paragraph should end with newline");
    };
    let content_length = text.chars().count() + 1; // Add back 1 for the newline.
    Paragraph::new(
        Arc::new(layout(text, styles, max_width)),
        OffsetMap::direct(content_length),
        content_length.into(),
        vec![],
        styles.block_spacings.from_block_style(block_style),
        Some(PARAGRAPH_MIN_HEIGHT),
    )
}

/// Lays out each hard-wrapped paragraph in `text`
pub fn layout_paragraphs(
    text: &str,
    styles: &RichTextStyles,
    block_style: &BufferBlockStyle,
    max_width: impl IntoPixels + Copy,
) -> Vec1<Paragraph> {
    Vec1::try_from_vec(
        text.split('\n')
            .map(|line| {
                let frame = Arc::new(layout(line, styles, max_width));
                let content_length = line.chars().count() + 1; // Add back 1 for the newline.
                Paragraph::new(
                    frame,
                    OffsetMap::direct(content_length),
                    content_length.into(),
                    vec![],
                    styles.block_spacings.from_block_style(block_style),
                    Some(PARAGRAPH_MIN_HEIGHT),
                )
            })
            .collect::<Vec<_>>(),
    )
    .expect("Should have at least one paragraph")
}

/// Static default color, since [`ColorU`] constructors aren't `const`.
const WHITE: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

pub const TEST_STYLES: RichTextStyles = RichTextStyles {
    base_text: ParagraphStyles {
        font_family: FamilyId(0),
        font_size: 10.,
        font_weight: Weight::Normal,
        line_height_ratio: 1.,
        text_color: WHITE,
        baseline_ratio: TEST_BASELINE_OFFSET,
        fixed_width_tab_size: None,
    },
    code_text: ParagraphStyles {
        font_family: FamilyId(0),
        font_size: 10.,
        font_weight: Weight::Normal,
        line_height_ratio: 1.,
        text_color: WHITE,
        baseline_ratio: TEST_BASELINE_OFFSET,
        fixed_width_tab_size: Some(4),
    },
    code_background: Fill::None,
    embedding_background: Fill::None,
    embedding_text: ParagraphStyles {
        font_family: FamilyId(0),
        font_size: 10.,
        font_weight: Weight::Normal,
        line_height_ratio: 1.,
        text_color: WHITE,
        baseline_ratio: TEST_BASELINE_OFFSET,
        fixed_width_tab_size: Some(4),
    },
    code_border: Border::new(0.),
    placeholder_color: WHITE,
    selection_fill: Fill::None,
    cursor_fill: Fill::None,
    inline_code_style: InlineCodeStyle {
        font_family: FamilyId(0),
        background: WHITE,
        font_color: WHITE,
    },
    check_box_style: CheckBoxStyle {
        border_width: 2.,
        border_color: WHITE,
        icon_path: "bundled/svg/check-thick.svg",
        background: WHITE,
        hover_background: WHITE,
    },
    horizontal_rule_style: HorizontalRuleStyle {
        rule_height: 2.,
        color: WHITE,
    },
    broken_link_style: BrokenLinkStyle {
        icon_path: "bundled/svg/link-broken-02.svg",
        icon_color: WHITE,
    },
    block_spacings: DEFAULT_BLOCK_SPACINGS,
    show_placeholder_text_on_empty_block: false,
    minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
    cursor_width: 1.,
    highlight_urls: true,
    table_style: TableStyle {
        border_color: WHITE,
        header_background: WHITE,
        cell_background: WHITE,
        alternate_row_background: None,
        text_color: WHITE,
        header_text_color: WHITE,
        scrollbar_nonactive_thumb_color: WHITE,
        scrollbar_active_thumb_color: WHITE,
        font_family: FamilyId(0),
        font_size: 10.,
        cell_padding: 8.0,
        outer_border: true,
        column_dividers: true,
        row_dividers: true,
    },
};

/// Minimal text layout for unit tests that require populated text frames.
///
/// This implementation soft-wraps at `max_width`, but without any segmentation
/// rules.
pub fn layout(text: &str, styles: &RichTextStyles, max_width: impl IntoPixels) -> TextFrame {
    let max_width = max_width.into_pixels();
    // For simplicity, pretend characters are square.
    let char_width = styles.base_text.font_size.into_pixels();

    let mut lines_acc = vec![];
    let mut glyphs_acc = vec![];
    let mut carets_acc = vec![];
    let mut line_width = Pixels::zero();
    for (index, ch) in text.chars().enumerate() {
        assert_ne!(ch, '\n', "Hard breaks not supported");

        if line_width + char_width > max_width {
            lines_acc.push(Line {
                width: line_width.as_f32(),
                trailing_whitespace_width: 0.,
                runs: vec![Run {
                    font_id: warpui_core::fonts::FontId(0),
                    styles: Default::default(),
                    glyphs: mem::take(&mut glyphs_acc),
                    width: line_width.as_f32(),
                }],
                font_size: styles.base_text.font_size,
                line_height_ratio: styles.base_text.line_height_ratio,
                baseline_ratio: TEST_BASELINE_OFFSET,
                ascent: styles.base_text.font_size * TEST_BASELINE_OFFSET,
                descent: styles.base_text.font_size * (1. - TEST_BASELINE_OFFSET),
                clip_config: None,
                caret_positions: mem::take(&mut carets_acc),
                chars_with_missing_glyphs: Vec::new(),
            });
            line_width = Pixels::zero();
        }

        glyphs_acc.push(Glyph {
            id: 0,
            position_along_baseline: vec2f(line_width.as_f32(), 0.),
            index,
            width: char_width.as_f32(),
        });

        carets_acc.push(CaretPosition {
            position_in_line: line_width.as_f32(),
            start_offset: index,
            last_offset: index,
        });

        line_width += char_width;
    }

    // Push any remaining characters (or an empty line, if the text frame was empty).
    if !glyphs_acc.is_empty() || lines_acc.is_empty() {
        lines_acc.push(Line {
            width: line_width.as_f32(),
            trailing_whitespace_width: 0.,
            runs: vec![Run {
                font_id: warpui_core::fonts::FontId(0),
                styles: Default::default(),
                glyphs: glyphs_acc,
                width: line_width.as_f32(),
            }],
            font_size: styles.base_text.font_size,
            line_height_ratio: styles.base_text.line_height_ratio,
            baseline_ratio: TEST_BASELINE_OFFSET,
            ascent: styles.base_text.font_size * TEST_BASELINE_OFFSET,
            descent: styles.base_text.font_size * (1. - TEST_BASELINE_OFFSET),
            clip_config: None,
            caret_positions: carets_acc,
            chars_with_missing_glyphs: Vec::new(),
        });
    }

    let max_width = lines_acc
        .iter()
        .map(|line| OrderedFloat(line.width))
        .max()
        .unwrap_or_default();
    match Vec1::try_from_vec(lines_acc) {
        Ok(lines) => TextFrame::new(lines, max_width.into_inner(), Default::default()),
        Err(_) => TextFrame::empty(
            styles.base_text.font_size,
            styles.base_text.line_height_ratio,
        ),
    }
}

/// Initialize logging for tests. This should be called at the start of any test that needs logging.
pub fn init_logging() {
    // If multiple tests run in the same process, we should still only set up logging once.
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = env_logger::builder()
            .parse_filters("warp_editor=trace")
            .is_test(true)
            .try_init();
    });
}
