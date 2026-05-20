use block::{Block, ConcreteBlock};
use core_foundation::array::CFArray;
use core_foundation::attributed_string::CFMutableAttributedStringRef;
use core_foundation::base::CFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::mach_port::CFIndex;
use core_foundation::number::CFNumber;
use core_foundation::{
    attributed_string::CFMutableAttributedString,
    base::CFTypeID,
    base::{CFRange, TCFType},
    declare_TCFType, impl_TCFType,
    string::CFString,
};
use core_graphics::base::CGFloat;
use core_graphics::color::CGColor;
use core_graphics::display::{CGPoint, CGRect, CGSize};
use core_graphics::path::CGPath;
use core_text::framesetter::CTFramesetter;
use core_text::line::CTLineRef;
use core_text::run::{CTRun, CTRunRef};
use core_text::string_attributes::kCTKernAttributeName;
use core_text::{
    font::CTFont,
    line::CTLine,
    string_attributes::{kCTFontAttributeName, kCTParagraphStyleAttributeName},
};
use itertools::Itertools;
use ordered_float::OrderedFloat;
use pathfinder_geometry::vector::vec2f;
use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ops::Range;
use std::rc::Rc;
use std::slice;
use vec1::Vec1;
use warpui_core::fonts::GlyphId;
use warpui_core::platform::LineStyle;
use warpui_core::text_layout::{
    CaretPosition, ClipConfig, Glyph, Line, Run, StyleAndFont, TextAlignment, TextBorder,
    TextFrame, TextStyle,
};

use super::fonts::FontDB;
use super::utils::{cg_color_to_color_u, color_u_to_cg_color};

pub enum __CTParagraphStyle {}
type CTParagraphStyleRef = *const __CTParagraphStyle;

declare_TCFType!(CTParagraphStyle, CTParagraphStyleRef);
impl_TCFType!(
    CTParagraphStyle,
    CTParagraphStyleRef,
    CTParagraphStyleGetTypeID
);

/// From https://developer.apple.com/documentation/coretext/ctparagraphstylespecifier
/// Note: there are many more of these possible style specifiers
/// but we are not using them!
#[derive(Clone, Copy)]
#[repr(u32)]
enum CTParagraphStyleSpecifier {
    FirstLineHeadIndent = 1,
    TabStops = 4,
    DefaultTabInterval = 5,
    LineHeightMultiple = 7,
}

/// See https://developer.apple.com/documentation/coretext/ctparagraphstylesetting
/// for the API specification on paragraph style settings.
/// We tie the lifetime of this struct to ParagraphStyleSetting to ensure
/// that we don't drop the `value` before consuming the setting (in the context
/// of creating an attributed string).
#[repr(C)]
struct CTParagraphStyleSetting<'a> {
    spec: CTParagraphStyleSpecifier,
    value_size: usize,
    value: *const c_void,
    /// PhantomData is used to add a marker that CTParagraphStyleSetting should not
    /// live any longer than ParagraphStyleSetting is alive.
    _phantom: PhantomData<&'a ParagraphStyleSetting>,
}

enum ParagraphStyleValue {
    Float(Box<CGFloat>),
    Array {
        _array: CFArray<CFType>,
        stable_ref: Box<*const c_void>,
    },
}

/// We wrap CTParagraphStyleSetting with a custom struct to correctly hold a raw
/// pointer to a heap-allocated value, which is needed for Core Foundation FFIs.
struct ParagraphStyleSetting {
    /// The specific style we are trying to define for the paragraph.
    spec: CTParagraphStyleSpecifier,
    value: ParagraphStyleValue,
}

impl ParagraphStyleSetting {
    fn new_float_setting(spec: CTParagraphStyleSpecifier, value: CGFloat) -> ParagraphStyleSetting {
        ParagraphStyleSetting {
            spec,
            value: ParagraphStyleValue::Float(Box::new(value)),
        }
    }

    fn new_empty_tab_stops() -> ParagraphStyleSetting {
        // Core Text has a built-in default list of tab stops. Setting DefaultTabInterval alone
        // doesn't override those initial stops, so we set an explicit (empty) TabStops array
        // to force Core Text to use DefaultTabInterval from the first tab.
        let array: CFArray<CFType> = CFArray::from_CFTypes(&[]);
        let array_ref = array.as_concrete_TypeRef() as *const c_void;

        ParagraphStyleSetting {
            spec: CTParagraphStyleSpecifier::TabStops,
            value: ParagraphStyleValue::Array {
                _array: array,
                stable_ref: Box::new(array_ref),
            },
        }
    }

    fn to_ct_setting(&self) -> CTParagraphStyleSetting<'_> {
        match &self.value {
            ParagraphStyleValue::Float(val) => {
                let raw_ptr = val.as_ref() as *const CGFloat as *const c_void;

                CTParagraphStyleSetting {
                    spec: self.spec,
                    value_size: std::mem::size_of::<CGFloat>(),
                    value: raw_ptr,
                    _phantom: PhantomData,
                }
            }
            ParagraphStyleValue::Array { stable_ref, .. } => CTParagraphStyleSetting {
                spec: self.spec,
                value_size: std::mem::size_of::<*const c_void>(),
                value: stable_ref.as_ref() as *const *const c_void as *const c_void,
                _phantom: PhantomData,
            },
        }
    }
}

/// ParagraphStyle is a wrapper struct that helps tie the underlying lifetimes of the settings
/// being used to the CTParagraphStyle.
struct ParagraphStyle<'a> {
    style: CTParagraphStyle,
    _settings: Vec<CTParagraphStyleSetting<'a>>,
}

impl<'a> ParagraphStyle<'a> {
    fn new(ct_style_settings: Vec<CTParagraphStyleSetting<'a>>) -> ParagraphStyle<'a> {
        let paragraph_style = unsafe {
            CTParagraphStyle::wrap_under_create_rule(CTParagraphStyleCreate(
                ct_style_settings.as_slice().as_ptr(),
                ct_style_settings.len(),
            ))
        };

        ParagraphStyle {
            style: paragraph_style,
            _settings: ct_style_settings,
        }
    }
}

extern "C" {
    fn CFAttributedStringBeginEditing(mutable_string: CFMutableAttributedStringRef);

    fn CFAttributedStringEndEditing(mutable_string: CFMutableAttributedStringRef);

    /// Enumerates caret offsets for characters in a line.
    /// See [CTLineEnumerateCaretOffsets](https://developer.apple.com/documentation/coretext/1508685-ctlineenumeratecaretoffsets?language=objc).
    #[allow(improper_ctypes)]
    // Rust doesn't consider &Block a valid FFI type, but it is the correct thing
    // as long as the block crate invariants are upheld.
    fn CTLineEnumerateCaretOffsets(
        line: CTLineRef,
        block: &Block<(f64, CFIndex, bool, *mut bool), ()>,
    );

    fn CTLineGetTrailingWhitespaceWidth(line: CTLineRef) -> f64;

    fn CTParagraphStyleGetTypeID() -> CFTypeID;
    fn CTParagraphStyleCreate(
        settings: *const CTParagraphStyleSetting,
        count: usize,
    ) -> CTParagraphStyleRef;

    fn CTRunGetGlyphCount(run: CTRunRef) -> CFIndex;
    fn CTRunGetAdvancesPtr(run: CTRunRef) -> *const CGSize;
    fn CTRunGetAdvances(run: CTRunRef, range: CFRange, buffer: *mut CGSize);
}

const FOREGROUND_COLOR_KEY: &str = "foreground-color";
const SYNTAX_COLOR_KEY: &str = "syntax-color";
const BACKGROUND_COLOR_KEY: &str = "background-color";
const ERROR_UNDERLINE_COLOR_KEY: &str = "error-underline-color";
const BORDER_COLOR_KEY: &str = "border-color";
const BORDER_WIDTH_KEY: &str = "border-width";
const BORDER_RADIUS_KEY: &str = "border-radius";
const BORDER_LINE_HEIGHT_RATIO_KEY: &str = "border-line-height-ratio";
const SHOW_STRIKETHROUGH_KEY: &str = "show-strikethrough";
const HYPERLINK_UNDERLINE_STYLE_KEY: &str = "hyperlink-underline-style";
const HYPERLINK_ID: &str = "hyperlink-id";

fn text_style_as_cf_type_pairs(style: &TextStyle) -> Vec<(CFString, CFType)> {
    let mut key_value_pairs = vec![];
    if let Some(foreground_color) = style.foreground_color.as_ref() {
        key_value_pairs.push((
            CFString::new(FOREGROUND_COLOR_KEY),
            color_u_to_cg_color(*foreground_color).as_CFType(),
        ))
    }

    if let Some(syntax_color) = style.syntax_color.as_ref() {
        key_value_pairs.push((
            CFString::new(SYNTAX_COLOR_KEY),
            color_u_to_cg_color(*syntax_color).as_CFType(),
        ))
    }

    if let Some(background_color) = style.background_color.as_ref() {
        key_value_pairs.push((
            CFString::new(BACKGROUND_COLOR_KEY),
            color_u_to_cg_color(*background_color).as_CFType(),
        ))
    }

    if let Some(error_underline_color) = style.error_underline_color.as_ref() {
        key_value_pairs.push((
            CFString::new(ERROR_UNDERLINE_COLOR_KEY),
            color_u_to_cg_color(*error_underline_color).as_CFType(),
        ))
    }

    if let Some(border) = style.border.as_ref() {
        key_value_pairs.push((
            CFString::new(BORDER_COLOR_KEY),
            color_u_to_cg_color(border.color).as_CFType(),
        ));

        key_value_pairs.push((
            CFString::new(BORDER_RADIUS_KEY),
            CFNumber::from(border.radius as i32).as_CFType(),
        ));

        key_value_pairs.push((
            CFString::new(BORDER_WIDTH_KEY),
            CFNumber::from(border.width as i32).as_CFType(),
        ));

        if let Some(line_height_override) = border.line_height_ratio_override {
            key_value_pairs.push((
                CFString::new(BORDER_LINE_HEIGHT_RATIO_KEY),
                CFNumber::from(line_height_override as i32).as_CFType(),
            ));
        }
    }

    if let Some(underline_color) = style.underline_color.as_ref() {
        key_value_pairs.push((
            CFString::new(HYPERLINK_UNDERLINE_STYLE_KEY),
            color_u_to_cg_color(*underline_color).as_CFType(),
        ))
    }

    if let Some(hyperlink_id) = style.hyperlink_id {
        key_value_pairs.push((
            CFString::new(HYPERLINK_ID),
            CFNumber::from(hyperlink_id).as_CFType(),
        ))
    }

    key_value_pairs.push((
        CFString::new(SHOW_STRIKETHROUGH_KEY),
        CFBoolean::from(style.show_strikethrough).as_CFType(),
    ));

    key_value_pairs
}

fn attributes_to_text_style(attributes_dictionary: CFDictionary<CFString, CFType>) -> TextStyle {
    let mut text_styles = TextStyle::new();

    if let Some(cg_color) = attributes_dictionary
        .find(CFString::new(FOREGROUND_COLOR_KEY))
        .and_then(|value| value.downcast::<CGColor>())
    {
        text_styles = text_styles.with_foreground_color(cg_color_to_color_u(cg_color));
    }

    if let Some(cg_color) = attributes_dictionary
        .find(CFString::new(SYNTAX_COLOR_KEY))
        .and_then(|value| value.downcast::<CGColor>())
    {
        text_styles = text_styles.with_syntax_color(cg_color_to_color_u(cg_color));
    }

    if let Some(cg_color) = attributes_dictionary
        .find(CFString::new(BACKGROUND_COLOR_KEY))
        .and_then(|value| value.downcast::<CGColor>())
    {
        text_styles = text_styles.with_background_color(cg_color_to_color_u(cg_color));
    }

    let border_color = attributes_dictionary
        .find(CFString::new(BORDER_COLOR_KEY))
        .and_then(|value| value.downcast::<CGColor>());
    let border_width = attributes_dictionary
        .find(CFString::new(BORDER_WIDTH_KEY))
        .and_then(|value| value.downcast::<CFNumber>())
        .and_then(|num| num.to_i32());
    let border_radius = attributes_dictionary
        .find(CFString::new(BORDER_RADIUS_KEY))
        .and_then(|value| value.downcast::<CFNumber>())
        .and_then(|num| num.to_i32());
    let border_line_height_ratio = attributes_dictionary
        .find(CFString::new(BORDER_LINE_HEIGHT_RATIO_KEY))
        .and_then(|value| value.downcast::<CFNumber>())
        .and_then(|num| num.to_i32());

    if let Some(((color, width), radius)) = border_color.zip(border_width).zip(border_radius) {
        text_styles = text_styles.with_border(TextBorder {
            color: cg_color_to_color_u(color),
            radius: radius as u8,
            width: width as u8,
            line_height_ratio_override: border_line_height_ratio.map(|val| val as u8),
        });
    }

    if let Some(cg_color) = attributes_dictionary
        .find(CFString::new(ERROR_UNDERLINE_COLOR_KEY))
        .and_then(|value| value.downcast::<CGColor>())
    {
        text_styles = text_styles.with_error_underline_color(cg_color_to_color_u(cg_color));
    }

    if let Some(show_strikethrough) = attributes_dictionary
        .find(CFString::new(SHOW_STRIKETHROUGH_KEY))
        .and_then(|value| value.downcast::<CFBoolean>())
    {
        text_styles = text_styles.with_show_strikethrough(bool::from(show_strikethrough));
    }

    if let Some(cg_color) = attributes_dictionary
        .find(CFString::new(HYPERLINK_UNDERLINE_STYLE_KEY))
        .and_then(|value| value.downcast::<CGColor>())
    {
        text_styles = text_styles.with_underline_color(cg_color_to_color_u(cg_color));
    }

    if let Some(hyperlink_id) = attributes_dictionary
        .find(CFString::new(HYPERLINK_ID))
        .and_then(|value| value.downcast::<CFNumber>())
        .and_then(|hyperlink| hyperlink.to_i32())
    {
        text_styles = text_styles.with_hyperlink_id(hyperlink_id);
    }

    text_styles
}

/// Lays out a *single* line using Core Text.
pub fn layout_line(
    text: &str,
    line_style: LineStyle,
    style_runs: &[(Range<usize>, StyleAndFont)],
    font_db: &FontDB,
    clip_config: ClipConfig,
) -> Line {
    layout_line_with_offset(text, line_style, style_runs, font_db, 0, clip_config)
}

/// Lays out a line assuming the starting character index of the `text` starts at `char_offset`.
fn layout_line_with_offset(
    text: &str,
    line_style: LineStyle,
    style_runs: &[(Range<usize>, StyleAndFont)],
    font_db: &FontDB,
    char_offset: usize,
    clip_config: ClipConfig,
) -> Line {
    if text.is_empty() {
        Line::empty(
            line_style.font_size,
            line_style.line_height_ratio,
            char_offset,
        )
    } else {
        let attributed_string =
            create_attributed_string(text, style_runs, font_db, line_style, None);
        let line = CTLine::new_with_attributed_string(attributed_string.as_concrete_TypeRef());
        let utf16_offset_to_char_idx = build_utf16_lookup(text);
        line_from_ct_line(
            line,
            line_style,
            font_db,
            char_offset,
            Some(clip_config),
            &utf16_offset_to_char_idx,
        )
    }
}

fn push_paragraph_style_settings(
    paragraph_style_settings: &mut Vec<ParagraphStyleSetting>,
    first_line_head_indent: Option<f32>,
    tab_interval: Option<CGFloat>,
) {
    if let Some(first_line_head_indent_value) = first_line_head_indent {
        paragraph_style_settings.push(ParagraphStyleSetting::new_float_setting(
            CTParagraphStyleSpecifier::FirstLineHeadIndent,
            first_line_head_indent_value as CGFloat,
        ));
    }

    if let Some(interval) = tab_interval {
        // Core Text has a built-in list of tab stops. To ensure DefaultTabInterval applies from
        // the first tab, we must also set an explicit (empty) TabStops array.
        paragraph_style_settings.push(ParagraphStyleSetting::new_empty_tab_stops());
        paragraph_style_settings.push(ParagraphStyleSetting::new_float_setting(
            CTParagraphStyleSpecifier::DefaultTabInterval,
            interval,
        ));
    }
}

/// Applies paragraph style settings to an attributed string if the settings vec is non-empty.
fn apply_paragraph_style_settings(
    attributed_string: &mut CFMutableAttributedString,
    cf_range: CFRange,
    paragraph_style_settings: &[ParagraphStyleSetting],
) {
    if !paragraph_style_settings.is_empty() {
        let ct_style_settings = paragraph_style_settings
            .iter()
            .map(|setting| setting.to_ct_setting())
            .collect();
        let paragraph_style = ParagraphStyle::new(ct_style_settings);
        unsafe {
            attributed_string.set_attribute(
                cf_range,
                kCTParagraphStyleAttributeName,
                &paragraph_style.style,
            );
        }
    }
}

/// Creates a `CFAttributedString` out of `text` with the correct font ranges based on `runs`.
fn create_attributed_string(
    text: &str,
    style_runs: &[(Range<usize>, StyleAndFont)],
    font_db: &FontDB,
    line_style: LineStyle,
    first_line_head_indent: Option<f32>,
) -> CFMutableAttributedString {
    let mut attributed_string = CFMutableAttributedString::new();
    attributed_string.replace_str(&CFString::new(text), CFRange::init(0, 0));

    // Wrap the edits to the `CFMutableAttributedString` with `BeginEditing` and `EndEditing` calls
    // so that `CFMutableAttributedString` doesn't have to maintain consistency in between edits.
    // See https://developer.apple.com/documentation/corefoundation/cfmutableattributedstring?language=objc.
    unsafe {
        CFAttributedStringBeginEditing(attributed_string.as_concrete_TypeRef());
    }

    let mut utf16_lens = text.chars().map(|c| c.len_utf16());
    let mut prev_char_ix: usize = 0;
    let mut prev_utf16_ix: usize = 0;

    let tab_interval = line_style.fixed_width_tab_size.and_then(|tab_size| {
        // In fully fixed-width paragraphs, style runs should all resolve to the same monospace font.
        // Use the first run to pick a font for computing the tab interval.
        let (_, style_and_font) = style_runs.iter().next()?;
        let font_id = font_db.select_font(style_and_font.font_family, style_and_font.properties);
        font_db
            .space_advance_width(font_id, line_style.font_size)
            .map(|w| (w * tab_size as f64) as CGFloat)
    });

    // Apply paragraph-level settings (like tab stops) over the full string so they
    // still apply even if style runs don't cover whitespace.
    {
        let mut paragraph_style_settings = vec![];
        push_paragraph_style_settings(
            &mut paragraph_style_settings,
            first_line_head_indent,
            tab_interval,
        );
        let full_range = CFRange::init(0, attributed_string.char_len());
        apply_paragraph_style_settings(
            &mut attributed_string,
            full_range,
            &paragraph_style_settings,
        );
    }

    for (range, style_and_font) in style_runs {
        let utf16_start: usize = prev_utf16_ix
            + utf16_lens
                .by_ref()
                .take(range.start - prev_char_ix)
                .sum::<usize>();
        let utf16_end: usize = utf16_start
            + utf16_lens
                .by_ref()
                .take(range.end - range.start)
                .sum::<usize>();
        prev_char_ix = range.end;
        prev_utf16_ix = utf16_end;

        let cf_range = CFRange::init(utf16_start as isize, (utf16_end - utf16_start) as isize);
        let font_id = font_db.select_font(style_and_font.font_family, style_and_font.properties);
        let native_font = font_db.native_font(font_id, line_style.font_size);

        let attributes_pairs = text_style_as_cf_type_pairs(&style_and_font.style);
        unsafe {
            attributed_string.set_attribute(cf_range, kCTFontAttributeName, &native_font);

            // The way the system computes line height can be slightly different from the way we compute line height.
            // See https://www.zsiegel.com/2012/10/23/Core-Text-Calculating-line-heights for how the system computes line height.
            // To make the system use the same line height that we do in the app, we use a multiplier.
            // This is necessary to prevent the case where the system doesn't render any text because it thinks there isn't enough vertical space,
            // but there is actually enough space based on the app line height.
            let system_font_line_height =
                native_font.ascent() + native_font.descent() + native_font.leading();
            let app_font_line_height =
                line_style.font_size as CGFloat * line_style.line_height_ratio as CGFloat;
            let line_height_multiple = app_font_line_height / system_font_line_height;
            let line_height_multiple_setting = ParagraphStyleSetting::new_float_setting(
                CTParagraphStyleSpecifier::LineHeightMultiple,
                line_height_multiple,
            );
            let mut paragraph_style_settings = vec![line_height_multiple_setting];

            // Note: we apply tab stops here *and* over the full string. We need both because we
            // set a per-run paragraph style (to normalize line height), and that overrides the
            // full-range paragraph style for this character range.
            push_paragraph_style_settings(
                &mut paragraph_style_settings,
                first_line_head_indent,
                tab_interval,
            );

            apply_paragraph_style_settings(
                &mut attributed_string,
                cf_range,
                &paragraph_style_settings,
            );

            // When we apply inline code block, we need to leave additional spacing before and after the text to make sure
            // we can render the block without overlapping with text outside of the code block. We add these spacing by setting
            // the kerning attribute on the glyph right before the code block range and the last glyph in the block.
            //
            // For example if we want to highlight `bc` in abcd
            // We would apply kerning on `a` and `c`. The text spacing would look like [a  ][b][c  ][d].
            if style_and_font.style.border.is_some() {
                if utf16_start > 0 {
                    let kerning_range = CFRange::init((utf16_start - 1) as isize, 1_isize);
                    attributed_string.set_attribute(
                        kerning_range,
                        kCTKernAttributeName,
                        &CFNumber::from(6.),
                    );
                }

                if utf16_end > 0 {
                    let kerning_range = CFRange::init((utf16_end - 1) as isize, 1_isize);
                    attributed_string.set_attribute(
                        kerning_range,
                        kCTKernAttributeName,
                        &CFNumber::from(6.),
                    );
                }
            }

            for (key, value) in attributes_pairs {
                attributed_string.set_attribute(cf_range, key.as_concrete_TypeRef(), &value);
            }
        }
    }

    unsafe {
        CFAttributedStringEndEditing(attributed_string.as_concrete_TypeRef());
    }
    attributed_string
}

/// Lays out a string of text into a frame (a series of lines) using Core Text.
#[allow(clippy::too_many_arguments)]
pub fn layout_text(
    text: &str,
    line_style: LineStyle,
    style_runs: &[(Range<usize>, StyleAndFont)],
    font_db: &FontDB,
    max_width: f32,
    max_height: f32,
    alignment: TextAlignment,
    mut first_line_head_indent: Option<f32>,
) -> TextFrame {
    if text.is_empty() {
        TextFrame::empty(line_style.font_size, line_style.line_height_ratio)
    } else {
        // Ensure the max height is finite--under certain conditions `CTFrameSetter` won't terminate
        // if the height is unbounded.
        let max_height = max_height.min(f32::MAX);

        let mut insert_extra_initial_line = false;
        // Core Text always tries to put at least 1 character on the first line, which does not work well
        // in the case of a large head indent which is >= the width of the first line. In this case, we
        // handle the "empty" first line manually (and ask Core Text to lay out the rest of the lines).
        if let Some(indent) = first_line_head_indent {
            // We approximate the width of a single character to be half of the font size. This is to encourage
            // wrapping the character as soon as it starts to get significantly clipped (due to the max width).
            if indent >= max_width - (line_style.font_size / 2.) {
                first_line_head_indent = None;
                insert_extra_initial_line = true;
            }
        }
        let attributed_string = create_attributed_string(
            text,
            style_runs,
            font_db,
            line_style,
            first_line_head_indent,
        );
        let framesetter =
            CTFramesetter::new_with_attributed_string(attributed_string.as_concrete_TypeRef());

        // Create frame from framesetter.
        let cf_range = CFRange::init(0, attributed_string.char_len());
        let frame = framesetter.create_frame(
            cf_range,
            CGPath::from_rect(
                CGRect::new(
                    &CGPoint::new(0., 0.),
                    // Even though we use a multiplier for line height to account for differences between
                    // the system computed line height and our app computed line height, there are cases where
                    // the system still thinks more height is required. We add a small buffer to account for this
                    // because we should be able to render text when max_height is exactly the app computed line height.
                    &CGSize::new(max_width as f64, max_height as f64 + 2.),
                ),
                None,
            )
            .as_ref(),
        );
        let mut max_line_width: f32 = 0.;

        let mut frame_lines = vec![];
        if insert_extra_initial_line {
            // If the head indent was >= the width of the first line, we manually add back in the first empty line!
            frame_lines.push(Line::empty(
                line_style.font_size,
                line_style.line_height_ratio,
                0,
            ));
        }

        let lines = frame.get_lines();
        let num_lines = lines.len();

        let utf16_offset_to_char_idx = build_utf16_lookup(text);

        frame_lines.append(
            &mut lines
                .into_iter()
                .enumerate()
                .map(|(index, line)| {
                    let (line_start_utf_16_index, line_length) = {
                        let range = line.get_string_range();
                        // The `CFRange` returned by CoreText returns a location that is the number of
                        // utf-16 bytes from the start of the string.
                        (range.location as usize, range.length as usize)
                    };

                    // If the last line would be clipped, render the rest of the text into it's own line so
                    // that we can overflow the text by fading the text.
                    let is_last_line = index == num_lines - 1;
                    let line = if is_last_line
                        && line_start_utf_16_index + line_length
                            < attributed_string.char_len() as usize
                    {
                        // The string range returned by CoreText is in terms of the number of UTF-16 bytes,
                        // so iterate through the string to find the corresponding char index.
                        let star_char_index =
                            char_index_from_utf_16_byte_index(text, line_start_utf_16_index);

                        // CoreText does not support multiline overflow when using `CTFrame`. To support
                        // multiline text that is clipped, we first use CoreText to render the text into a
                        // frame and then relayout the last line with the rest of string on that line.
                        // See https://groups.google.com/g/cocoa-unbound/c/Qin6gjYj7XU?pli=1 for more
                        // details.
                        let style_runs = style_runs
                            .iter()
                            .filter_map(|(range, font)| {
                                if range.end >= star_char_index {
                                    Some((
                                        Range {
                                            start: range.start.saturating_sub(star_char_index),
                                            end: range.end.saturating_sub(star_char_index),
                                        },
                                        *font,
                                    ))
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>();

                        let chars: Vec<_> = text.chars().collect();
                        layout_line_with_offset(
                            &chars[star_char_index..].iter().collect::<String>(),
                            line_style,
                            &style_runs,
                            font_db,
                            star_char_index,
                            ClipConfig::default(),
                        )
                    } else {
                        line_from_ct_line(
                            line,
                            line_style,
                            font_db,
                            0,
                            // Only apply clipping to the last line in the frame.
                            is_last_line.then_some(ClipConfig::default()),
                            &utf16_offset_to_char_idx,
                        )
                    };

                    max_line_width = max_line_width.max(line.width);
                    line
                })
                .collect_vec(),
        );

        match Vec1::try_from_vec(frame_lines) {
            Ok(frame_lines) => TextFrame::new(frame_lines, max_line_width, alignment),
            Err(_) => TextFrame::empty(line_style.font_size, line_style.line_height_ratio),
        }
    }
}

fn line_from_ct_line(
    line: CTLine,
    line_style: LineStyle,
    font_db: &FontDB,
    char_offset: usize,
    clip_config: Option<ClipConfig>,
    utf16_offset_to_char_idx: &[usize],
) -> Line {
    let mut runs = Vec::with_capacity(line.glyph_runs().len() as usize);
    let typographic_bounds = line.get_typographic_bounds();
    let width = typographic_bounds.width as f32;

    let mut previous_run_font_and_attribute = None;

    for run in line.glyph_runs().into_iter() {
        let attributes = run.attributes().expect("attributes should exist");

        let font_id = font_db.font_id_for_native_font(unsafe {
            attributes
                .get(kCTFontAttributeName)
                .downcast::<CTFont>()
                .unwrap()
        });

        let glyphs = itertools::multizip((
            run.glyphs().iter(),
            run.positions().iter(),
            run.string_indices().iter(),
            advances(&run).iter(),
        ))
        .map(|(glyph_id, position, utf16_offset, advance)| {
            let utf16_offset = usize::try_from(*utf16_offset).expect("Negative character offset");
            let char_index = utf16_offset_to_char_idx
                .get(utf16_offset)
                .expect("mapping covers whole string");

            Glyph {
                id: *glyph_id as GlyphId,
                position_along_baseline: vec2f(position.x as f32, position.y as f32),
                index: char_offset + char_index,
                width: advance.width as f32,
            }
        })
        .collect_vec();

        // Only separate out text runs with different attribute that we will use in the paint stage.
        // For text attributes that don't matter in the paint stage (e.g. kerning), treat them as one
        // text run.
        let text_style = attributes_to_text_style(attributes);
        let width = run.get_typographic_bounds().width as f32;
        match previous_run_font_and_attribute {
            Some((prev_font_id, prev_attribute))
                if prev_font_id == font_id && text_style == prev_attribute && !runs.is_empty() =>
            {
                let last_run: &mut Run =
                    runs.last_mut().expect("Already checked runs are not empty");
                last_run.glyphs.extend(glyphs);
                last_run.width += width;
            }
            _ => {
                runs.push(Run {
                    font_id,
                    glyphs,
                    styles: text_style,
                    width,
                });

                previous_run_font_and_attribute = Some((font_id, text_style));
            }
        }
    }

    let caret_positions = caret_positions_for_line(&line, char_offset, utf16_offset_to_char_idx);

    Line {
        width,
        trailing_whitespace_width: trailing_whitespace_width_for_line(&line).max(0.) as f32,
        runs,
        font_size: line_style.font_size,
        clip_config,
        line_height_ratio: line_style.line_height_ratio,
        baseline_ratio: line_style.baseline_ratio,
        ascent: typographic_bounds.ascent as f32,
        descent: typographic_bounds.descent as f32,
        caret_positions,
        // TODO(CORE-2004): If we want to support external font fallback on
        // Mac, we need to populate this with the missing chars.
        chars_with_missing_glyphs: vec![],
    }
}

/// Returns the char index within `text` given the number of UTF-16 bytes from the start of the
/// string.
fn char_index_from_utf_16_byte_index(text: &str, utf_16_index: usize) -> usize {
    let mut start_index_utf_16 = 0;
    let mut start_index = 0;
    for (index, char) in text.chars().enumerate() {
        if utf_16_index <= start_index_utf_16 {
            start_index = index;
            break;
        }

        start_index_utf_16 += char.len_utf16();
    }
    start_index
}

/// Builds a lookup table for finding a character's index within the input
/// string given its position in UTF-16 code units.
///
/// This is necessary because the glyph order is not guaranteed to match the
/// character order (e.g.: for RTL text), and because a single character may be
/// represented by multiple UTF-16 code units.
///
/// In the output `Vec`, indices correspond to UTF-16 code unit offsets and values
/// correspond to character indices.
fn build_utf16_lookup(text: &str) -> Vec<usize> {
    // Use the UTF-8 length as a starting estimate, since each ASCII character
    // is represented by both 1 UTF-8 code point and 1 UTF-16 code point.
    let mut table = Vec::with_capacity(text.len());

    for (char_index, ch) in text.chars().enumerate() {
        // For each UTF-16 code unit in the character, add a mapping to the
        // character's index. The UTF-16 position is implicitly tracked by the
        // length of the lookup table.
        for _ in 0..ch.len_utf16() {
            table.push(char_index);
        }
    }

    table
}

/// Extract caret positions from a Core Text line.
fn caret_positions_for_line(
    line: &CTLine,
    char_offset: usize,
    utf16_offset_to_char_idx: &[usize],
) -> Vec<CaretPosition> {
    #[derive(Debug)]
    struct CaretEdge {
        /// Index of the UTF-16 code point corresponding to this caret edge.
        utf16_index: usize,
        /// Whether this is a leading edge or a trailing edge.
        /// For LTR text, the leading edge is the leftmost edge of the cluster,
        /// and for RTL text, it is the rightmost edge.
        leading_edge: bool,
        /// The pixel offset of this edge from the start of the line.
        pixel_offset: f64,
    }

    let positions = Rc::new(RefCell::new(vec![]));

    // Core Text produces leading and trailing edges for each caret position in
    // the line. For our purposes, we only need the leading edge for rendering
    // the caret. However, we use both edges to find the character extent of
    // each caret.
    let block = {
        let positions = positions.clone();
        ConcreteBlock::new(move |offset, char_index: isize, leading_edge, _stop| {
            positions.borrow_mut().push(CaretEdge {
                utf16_index: char_index.try_into().expect("Negative UTF-16 offset"),
                leading_edge,
                pixel_offset: offset,
            });
        })
    };

    // We have to use RcBlock to avoid a double-free, but that takes ownership
    // of utf16_offset_to_char_idx
    let block = block.copy();
    unsafe {
        CTLineEnumerateCaretOffsets(line.as_concrete_TypeRef(), &block);
    }
    drop(block);

    let mut positions = Rc::try_unwrap(positions)
        .expect("Block reference should be dropped")
        .into_inner();

    debug_assert!(
        positions.len() % 2 == 0,
        "Missing a leading or trailing caret edge"
    );
    // Core Text sometimes swaps the order of the trailing edge of one caret and
    // the leading edge of the next, causing edge pairs to be interspersed. So
    // that we can pair the leading and trailing edges into carets, sort by
    // character index, assuming that carets don't overlap with each other.
    // positions should already be almost entirely sorted, except for swapped
    // edges and RTL text.
    positions.sort_unstable_by_key(|position| position.utf16_index);
    let mut carets: Vec<_> = positions
        .chunks_exact(2)
        .map(|edges| {
            // Guaranteed by chunks_exact that there are 2 elements.
            let first = &edges[0];
            let second = &edges[1];

            // Core Text enumerates edges in left-to-right visual order, but
            // sets the leading edge based on logical order, so use that to
            // handle RTL text.
            // For any given leading/trailing edge pair, the pixel offset from
            // the start of the line is the one for the leading edge.
            let pixel_offset = if first.leading_edge {
                first.pixel_offset
            } else {
                debug_assert!(
                    second.leading_edge,
                    "No leading edge in {first:?} or {second:?}"
                );
                second.pixel_offset
            };

            let first_index = utf16_offset_to_char_idx
                .get(first.utf16_index)
                .expect("Mapping covers whole string")
                + char_offset;
            let second_index = utf16_offset_to_char_idx
                .get(second.utf16_index)
                .expect("Mapping covers whole string")
                + char_offset;

            let (start, end) = if first_index < second_index {
                (first_index, second_index)
            } else {
                (second_index, first_index)
            };

            CaretPosition {
                position_in_line: pixel_offset as f32,
                start_offset: start,
                last_offset: end,
            }
        })
        .collect();
    // Callers assume that carets are sorted by left-to-right visual display order,
    // so sort back here.
    carets.sort_unstable_by_key(|caret| OrderedFloat(caret.position_in_line));
    carets
}

fn trailing_whitespace_width_for_line(line: &CTLine) -> f64 {
    unsafe { CTLineGetTrailingWhitespaceWidth(line.as_concrete_TypeRef()) }
}

/// Returns the advances for each glyph in the run.
///
/// If the run has a non-null advances pointer, returns a slice of the advances.
/// Otherwise, returns a vector of advances computed by calling
/// [CTRunGetAdvances](https://developer.apple.com/documentation/coretext/1508691-ctrungetadvances?language=objc).
///
/// This follows the same pattern as the glyph IDs and positions functions provided by
/// the `core-text` crate.
fn advances(run: &CTRun) -> Cow<'_, [CGSize]> {
    unsafe {
        let run_ref = run.as_concrete_TypeRef();
        // CTRunGetAdvancesPtr can return null under some not understood circumstances.
        // If it does the Apple documentation tells us to allocate our own buffer and call
        // CTRunGetAdvances
        let count = CTRunGetGlyphCount(run_ref);
        let advances_ptr = CTRunGetAdvancesPtr(run_ref);
        if !advances_ptr.is_null() {
            Cow::from(slice::from_raw_parts(advances_ptr, count as usize))
        } else {
            let mut vec = Vec::with_capacity(count as usize);
            // "If the length of the range is set to 0, then the copy operation will continue
            // from the start index of the range to the end of the run"
            CTRunGetAdvances(run_ref, CFRange::init(0, 0), vec.as_mut_ptr());
            vec.set_len(count as usize);
            Cow::from(vec)
        }
    }
}

#[cfg(test)]
#[path = "text_layout_tests.rs"]
mod tests;
