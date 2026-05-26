use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use string_offset::ByteOffset;

use super::{
    apply_secret_replacements, FormattedTextElement, FrameMouseHandlers,
    HeadingFontSizeMultipliers, HighlightedHyperlink, HyperlinkSupport, LaidOutTextFrame,
    SecretRange,
};
use crate::elements::{Point, SelectableElement, ZIndex};
use crate::fonts::FamilyId;
use crate::text::BlockHeaderSize;
use crate::text_layout::TextFrame;

#[test]
fn test_default_heading_font_size_multipliers() {
    let multipliers = HeadingFontSizeMultipliers::default();

    // Test that default values match the BlockHeaderSize ratios
    assert_eq!(
        multipliers.h1,
        BlockHeaderSize::Header1.font_size_multiplication_ratio()
    );
    assert_eq!(
        multipliers.h2,
        BlockHeaderSize::Header2.font_size_multiplication_ratio()
    );
    assert_eq!(
        multipliers.h3,
        BlockHeaderSize::Header3.font_size_multiplication_ratio()
    );
    assert_eq!(
        multipliers.h4,
        BlockHeaderSize::Header4.font_size_multiplication_ratio()
    );
    assert_eq!(
        multipliers.h5,
        BlockHeaderSize::Header5.font_size_multiplication_ratio()
    );
    assert_eq!(
        multipliers.h6,
        BlockHeaderSize::Header6.font_size_multiplication_ratio()
    );
}

#[test]
fn test_get_multiplier_method() {
    let multipliers = HeadingFontSizeMultipliers::default();

    // Test valid heading levels
    assert_eq!(multipliers.get_multiplier(1), multipliers.h1);
    assert_eq!(multipliers.get_multiplier(2), multipliers.h2);
    assert_eq!(multipliers.get_multiplier(3), multipliers.h3);
    assert_eq!(multipliers.get_multiplier(4), multipliers.h4);
    assert_eq!(multipliers.get_multiplier(5), multipliers.h5);
    assert_eq!(multipliers.get_multiplier(6), multipliers.h6);

    // Test invalid heading levels return 1.0
    assert_eq!(multipliers.get_multiplier(0), 1.0);
    assert_eq!(multipliers.get_multiplier(7), 1.0);
    assert_eq!(multipliers.get_multiplier(999), 1.0);
}

#[test]
fn test_custom_heading_font_size_multipliers() {
    let custom_multipliers = HeadingFontSizeMultipliers {
        h1: 2.0,
        h2: 1.8,
        h3: 1.5,
        ..Default::default()
    };

    // Test custom values
    assert_eq!(custom_multipliers.h1, 2.0);
    assert_eq!(custom_multipliers.h2, 1.8);
    assert_eq!(custom_multipliers.h3, 1.5);

    // Test that other values still use defaults
    assert_eq!(
        custom_multipliers.h4,
        BlockHeaderSize::Header4.font_size_multiplication_ratio()
    );
    assert_eq!(
        custom_multipliers.h5,
        BlockHeaderSize::Header5.font_size_multiplication_ratio()
    );
    assert_eq!(
        custom_multipliers.h6,
        BlockHeaderSize::Header6.font_size_multiplication_ratio()
    );
}

fn sr(char_start: usize, char_end: usize, byte_start: usize, byte_end: usize) -> SecretRange {
    SecretRange {
        char_range: char_start..char_end,
        byte_range: byte_start..byte_end,
    }
}

#[test]
fn applies_replacements_with_multibyte_and_prefix() {
    let original = "令狐冲abcXYZ"; // Multibyte + ASCII
    let mut text = format!("{}{}", "•  ", original);
    let glyph_offset = 3; // prefix length in chars

    // Secret over chars [2..5): "冲ab"
    let start_byte = original
        .chars()
        .take(2)
        .map(|c| c.len_utf8())
        .sum::<usize>();
    let secret_bytes_len = original
        .chars()
        .skip(2)
        .take(3)
        .map(|c| c.len_utf8())
        .sum::<usize>();
    let secret = sr(2, 5, start_byte, start_byte + secret_bytes_len);

    let replacements = vec![(secret, Cow::Owned("***".to_string()))];
    apply_secret_replacements(&mut text, glyph_offset, &replacements);

    assert_eq!(text, format!("{}{}", "•  ", "令狐***cXYZ"));
}

#[test]
fn applies_multiple_replacements_in_descending_order() {
    let original = "abcdefg";
    let mut text = format!("{}{}", "•  ", original);
    let glyph_offset = 3;

    // [1..3) => "bc", [4..6) => "ef"
    let s1 = sr(1, 3, 1, 3);
    let s2 = sr(4, 6, 4, 6);
    let replacements = vec![(s1, Cow::Borrowed("**")), (s2, Cow::Borrowed("##"))];

    apply_secret_replacements(&mut text, glyph_offset, &replacements);
    assert_eq!(text, format!("{}{}", "•  ", "a**d##g"));
}

#[test]
fn order_matters_when_replacement_changes_length() {
    // This test demonstrates why we apply replacements in descending order.
    // Here, the first replacement shortens the text; if applied before the second,
    // the second range would be misaligned relative to the original char indices.
    let original = "abcdefghi";
    let mut text = original.to_string();
    let glyph_offset = 0;

    // Two secrets in original char coordinates: [1..5) => "bcde" then [5..8) => "fgh"
    // Replace first with a shorter string, second with equal length.
    let s1 = sr(1, 5, 1, 5);
    let s2 = sr(5, 8, 5, 8);
    let replacements = vec![
        (s1, Cow::Borrowed("*")),   // length 1 instead of 4
        (s2, Cow::Borrowed("###")), // same length as original 3
    ];

    apply_secret_replacements(&mut text, glyph_offset, &replacements);
    // With descending-order application, expected result is:
    // apply s2 first:  abcde###i
    // then s1:         a*###i
    assert_eq!(text, "a*###i");
}

fn select_first_character(text: &str, _click_offset: ByteOffset) -> Option<Range<ByteOffset>> {
    (!text.is_empty()).then_some(ByteOffset::zero()..ByteOffset::from(1))
}

fn test_formatted_text_element(text: &str, origin_x: f32, origin_y: f32) -> FormattedTextElement {
    let formatted_text = FormattedText::new([FormattedTextLine::Line(vec![
        FormattedTextFragment::plain_text(text),
    ])]);
    let text_frame = Arc::new(TextFrame::mock(text));
    let frame_bounds = RectF::new(
        vec2f(origin_x, origin_y),
        vec2f(text_frame.max_width(), text_frame.height()),
    );
    let mouse_handlers = Rc::new(RefCell::new(FrameMouseHandlers::default()));

    let mut element = FormattedTextElement::new(
        formatted_text,
        13.0,
        FamilyId(0),
        FamilyId(0),
        ColorU::black(),
        HighlightedHyperlink::default(),
    );
    element.origin = Some(Point::new(origin_x, origin_y, ZIndex::new(0)));
    element.size = Some(frame_bounds.size());
    element.laid_out_text = vec![LaidOutTextFrame::Text {
        text_frame,
        frame_bounds,
        bottom_padding: 0.0,
        raw_text: text.to_string(),
        mouse_handlers: mouse_handlers.clone(),
    }];
    element.text_frame_mouse_handlers = vec![mouse_handlers];
    element.hyperlink_support = HyperlinkSupport {
        highlighted_hyperlink: Arc::new(Mutex::new(None)),
        hyperlink_font_color: ColorU::black(),
    };
    element
}

#[test]
fn smart_select_returns_none_when_point_is_outside_horizontal_bounds() {
    let element = test_formatted_text_element("hello", 10.0, 20.0);
    let point = vec2f(10.0 + 100.0, 25.0);

    assert!(element
        .smart_select(point, select_first_character)
        .is_none());
}
