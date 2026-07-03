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
use crate::elements::{Element, PartialClickableElement, Point, SelectableElement, ZIndex};
use crate::event::DispatchedEvent;
use crate::fonts::FamilyId;
use crate::platform::WindowStyle;
use crate::text::word_boundaries::WordBoundariesPolicy;
use crate::text::{BlockHeaderSize, SelectionDirection, SelectionType};
use crate::text_layout::TextFrame;
use crate::{App, AppContext, Entity, Event, Presenter, TypedActionView};

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

const TEST_GLYPH_ADVANCE: f32 = 10.0;

/// Builds a single-line, left-aligned [`FormattedTextElement`] at origin (0, 0) whose glyphs each
/// advance by [`TEST_GLYPH_ADVANCE`], so `expand_selection` results can be checked by x position.
fn positioned_text_element(text: &str) -> FormattedTextElement {
    let text_frame = Arc::new(TextFrame::mock_with_positions(text, TEST_GLYPH_ADVANCE));
    let frame_bounds = RectF::new(
        vec2f(0.0, 0.0),
        vec2f(text_frame.max_width(), text_frame.height()),
    );
    let mouse_handlers = Rc::new(RefCell::new(FrameMouseHandlers::default()));
    let formatted_text = FormattedText::new([FormattedTextLine::Line(vec![
        FormattedTextFragment::plain_text(text),
    ])]);

    let mut element = FormattedTextElement::new(
        formatted_text,
        13.0,
        FamilyId(0),
        FamilyId(0),
        ColorU::black(),
        HighlightedHyperlink::default(),
    );
    element.origin = Some(Point::new(0.0, 0.0, ZIndex::new(0)));
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

/// Drives `expand_selection` for a semantic selection whose tail is over the glyph at
/// `glyph_index`, returning the resulting x position. The element is laid out so x == glyph index
/// times [`TEST_GLYPH_ADVANCE`], so the caller can assert which character boundary it snapped to.
fn semantic_target_x(
    element: &FormattedTextElement,
    glyph_index: usize,
    direction: SelectionDirection,
) -> f32 {
    // Aim near the middle of the target glyph so the point snaps to it.
    let point = vec2f(glyph_index as f32 * TEST_GLYPH_ADVANCE + 2.0, 5.0);
    element
        .expand_selection(
            point,
            direction,
            SelectionType::Semantic,
            &WordBoundariesPolicy::Default,
        )
        .expect("expand_selection should return a point")
        .x()
}

#[test]
fn expand_selection_excludes_trailing_whitespace_after_punctuation() {
    // "alpha, bravo": a0 l1 p2 h3 a4 ,5 <space>6 b7 r8 a9 v10 o11
    let element = positioned_text_element("alpha, bravo");

    // Forward drag with the tail on the comma ends just after the comma (x == 6 * advance),
    // NOT including the following space (which would be x == 7 * advance). This is the
    // reporter's case and matches the terminal block list.
    assert_eq!(
        semantic_target_x(&element, 5, SelectionDirection::Forward),
        6.0 * TEST_GLYPH_ADVANCE,
    );
    // Forward drag with the tail on a word char selects the whole word "alpha" (end x == 5 * advance).
    assert_eq!(
        semantic_target_x(&element, 1, SelectionDirection::Forward),
        5.0 * TEST_GLYPH_ADVANCE,
    );
    // Backward drag with the tail on a word char selects from the start of "bravo" (x == 7 * advance).
    assert_eq!(
        semantic_target_x(&element, 8, SelectionDirection::Backward),
        7.0 * TEST_GLYPH_ADVANCE,
    );
}

#[test]
fn expand_selection_stops_at_end_of_punctuation_run() {
    // "foo... bar": f0 o1 o2 .3 .4 .5 <space>6 b7 a8 r9
    let element = positioned_text_element("foo... bar");

    // Tail on the last dot ends at x == 6 * advance ("foo..."), excluding the trailing space
    // (which would be x == 7 * advance) and never reaching "bar".
    assert_eq!(
        semantic_target_x(&element, 5, SelectionDirection::Forward),
        6.0 * TEST_GLYPH_ADVANCE,
    );
}

#[derive(Default)]
struct LinkClickTestView;

impl Entity for LinkClickTestView {
    type Event = ();
}

impl crate::core::View for LinkClickTestView {
    fn ui_name() -> &'static str {
        "formatted_text_link_click_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        FormattedTextElement::from_str("", FamilyId(0), 13.0).finish()
    }
}

impl TypedActionView for LinkClickTestView {
    type Action = ();
}

/// Builds a positioned single-line element whose chars `0..link_len` are registered as one
/// clickable range (mimicking a detected hyperlink). Each click that lands on that range
/// increments the returned counter.
fn element_with_clickable_link(
    text: &str,
    link_len: usize,
) -> (FormattedTextElement, Rc<RefCell<usize>>) {
    let clicks = Rc::new(RefCell::new(0usize));
    let clicks_for_handler = clicks.clone();
    let handlers = FrameMouseHandlers::default().with_clickable_char_range(
        0..link_len,
        move |_modifiers, _ctx, _app| {
            *clicks_for_handler.borrow_mut() += 1;
        },
    );

    let text_frame = Arc::new(TextFrame::mock_with_positions(text, TEST_GLYPH_ADVANCE));
    let frame_bounds = RectF::new(
        vec2f(0.0, 0.0),
        vec2f(text_frame.max_width(), text_frame.height()),
    );
    let mouse_handlers = Rc::new(RefCell::new(handlers));
    let formatted_text = FormattedText::new([FormattedTextLine::Line(vec![
        FormattedTextFragment::plain_text(text),
    ])]);

    let mut element = FormattedTextElement::new(
        formatted_text,
        13.0,
        FamilyId(0),
        FamilyId(0),
        ColorU::black(),
        HighlightedHyperlink::default(),
    );
    element.origin = Some(Point::new(0.0, 0.0, ZIndex::new(0)));
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
    (element, clicks)
}

fn single_left_click_at(x: f32) -> Event {
    Event::LeftMouseDown {
        position: vec2f(x, 5.0),
        modifiers: Default::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

fn left_mouse_up_at(x: f32) -> Event {
    Event::LeftMouseUp {
        position: vec2f(x, 5.0),
        modifiers: Default::default(),
    }
}

/// A single click on a hyperlink must be reported as handled by `dispatch_event` so an enclosing
/// `SelectableArea` does not also treat the press as the start of a text selection. When it
/// didn't (the regression), the selection path fired its selection handler on the same press and
/// cleared the link tooltip/click that `handle_mouse_down` had just triggered, making clicks on
/// markdown links in AI output appear to do nothing. A click that misses every clickable range
/// must still report unhandled so normal text selection keeps working.
#[test]
fn single_click_on_link_is_handled_so_selection_is_not_started() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| LinkClickTestView);
        let mut presenter = Presenter::new(window_id);

        app.update(move |ctx| {
            // "linktext": chars 0..4 ("link") are clickable; the rest are not.
            let (mut element, clicks) = element_with_clickable_link("linktext", 4);
            let mut event_ctx = presenter.mock_event_context(ctx.font_cache());

            // Click squarely on the first glyph of the link.
            let handled_on_link = element.dispatch_event(
                &DispatchedEvent::from(single_left_click_at(TEST_GLYPH_ADVANCE / 2.0)),
                &mut event_ctx,
                ctx,
            );
            assert!(
                handled_on_link,
                "a single click on a link should be reported as handled"
            );
            assert_eq!(
                *clicks.borrow(),
                1,
                "the link click handler should fire once"
            );

            // Click on a non-link glyph (index 6) within the same element.
            let handled_off_link = element.dispatch_event(
                &DispatchedEvent::from(single_left_click_at(
                    6.0 * TEST_GLYPH_ADVANCE + TEST_GLYPH_ADVANCE / 2.0,
                )),
                &mut event_ctx,
                ctx,
            );
            assert!(
                !handled_off_link,
                "a click that misses every clickable range should not be reported as handled"
            );
            assert_eq!(
                *clicks.borrow(),
                1,
                "clicking off the link should not fire the link handler"
            );

            // The matching mouse-up over the link must also be reported as handled so the
            // enclosing SelectableArea does not run its mouse-up selection handler (which would
            // dismiss the link tooltip the press opened). The release must not re-fire the click.
            let up_on_link = element.dispatch_event(
                &DispatchedEvent::from(left_mouse_up_at(TEST_GLYPH_ADVANCE / 2.0)),
                &mut event_ctx,
                ctx,
            );
            assert!(
                up_on_link,
                "a mouse-up over a link should be reported as handled"
            );
            assert_eq!(
                *clicks.borrow(),
                1,
                "the mouse-up must not re-fire the link click handler"
            );

            // A mouse-up that misses every clickable range stays unhandled so selection finalizes.
            let up_off_link = element.dispatch_event(
                &DispatchedEvent::from(left_mouse_up_at(
                    6.0 * TEST_GLYPH_ADVANCE + TEST_GLYPH_ADVANCE / 2.0,
                )),
                &mut event_ctx,
                ctx,
            );
            assert!(
                !up_off_link,
                "a mouse-up off any link should not be reported as handled"
            );
        });
    });
}
