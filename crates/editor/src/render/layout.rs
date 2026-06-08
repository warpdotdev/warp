//! Shared text-layout utilities needed throughout the editor implementation.

use std::ops::Range;
use std::sync::Arc;

#[cfg(test)]
use markdown_parser::FormattedTextInline;
use warpui_core::color::ColorU;
use warpui_core::fonts::TextLayoutSystem;
#[cfg(test)]
use warpui_core::fonts::{Style, Weight};
use warpui_core::text_layout::{
    ClipConfig, LayoutCache, Line, StyleAndFont, TextAlignment, TextBorder, TextFrame, TextStyle,
};
use warpui_core::units::{IntoPixels, Pixels};
use warpui_core::{AppContext, LayoutContext};

use super::model::{BlockSpacing, ParagraphStyles, RenderState, RichTextStyles};
use crate::content::text::{BufferBlockStyle, TextStylesWithMetadata};

const HYPERLINK_UNDERLINE_COLOR: u32 = 0x7aa6daff;

#[derive(Clone, Debug, Default)]
pub(crate) struct InlineTextLayoutInput {
    pub text: String,
    pub style_runs: Vec<(Range<usize>, StyleAndFont)>,
}

/// Utility for laying out rich text.
pub struct TextLayout<'a> {
    layout_cache: &'a LayoutCache,
    font_cache: TextLayoutSystem<'a>,
    rich_text_styles: &'a RichTextStyles,
    max_width: f32,
    container_scrolls_horizontally: bool,
}

impl<'a> TextLayout<'a> {
    pub fn new(
        layout_cache: &'a LayoutCache,
        font_cache: TextLayoutSystem<'a>,
        rich_text_styles: &'a RichTextStyles,
        max_width: f32,
    ) -> Self {
        Self {
            layout_cache,
            font_cache,
            rich_text_styles,
            max_width,
            container_scrolls_horizontally: false,
        }
    }

    /// Indicate that the surrounding container (for example, a code editor) already provides
    /// horizontal scrolling over its full content area. Blocks whose rendering would otherwise
    /// introduce a nested horizontal scroll (like wide Markdown tables) should render at their
    /// full intrinsic width instead and rely on the container's scroll.
    pub fn with_container_scrolls_horizontally(mut self, flag: bool) -> Self {
        self.container_scrolls_horizontally = flag;
        self
    }

    /// Whether the surrounding container already provides horizontal scrolling over its full
    /// content area. See [`Self::with_container_scrolls_horizontally`].
    pub fn container_scrolls_horizontally(&self) -> bool {
        self.container_scrolls_horizontally
    }

    /// Builds a [`TextLayout`] from the context passed to `Element::layout`.
    pub fn from_layout_context(
        ctx: &LayoutContext<'a>,
        app: &'a AppContext,
        model: &'a RenderState,
    ) -> Self {
        Self::new(
            ctx.text_layout_cache,
            app.font_cache().text_layout_system(),
            model.styles(),
            model.viewport().width().as_f32(),
        )
        .with_container_scrolls_horizontally(model.container_scrolls_horizontally())
    }

    /// Lay out a single frame of text. The caller is responsible for mapping rich text into
    /// the paragraph's styling and spacing, as well as the per-character styles.
    ///
    /// See [`Self::style_and_font`] for help constructing the `style_runs`.
    pub fn layout_text(
        &self,
        text: &str,
        paragraph_style: &ParagraphStyles,
        spacing: &BlockSpacing,
        style_runs: &[(Range<usize>, StyleAndFont)],
    ) -> Arc<TextFrame> {
        self.layout_text_with_options(
            text,
            paragraph_style,
            style_runs,
            self.content_width(spacing),
            Default::default(),
        )
    }

    pub fn layout_text_with_options(
        &self,
        text: &str,
        paragraph_style: &ParagraphStyles,
        style_runs: &[(Range<usize>, StyleAndFont)],
        max_width: f32,
        alignment: TextAlignment,
    ) -> Arc<TextFrame> {
        if text.is_empty() {
            return Arc::new(TextFrame::empty(
                paragraph_style.font_size,
                paragraph_style.line_height_ratio,
            ));
        }
        self.layout_cache.layout_text(
            text,
            paragraph_style.line_style(),
            style_runs,
            max_width,
            f32::MAX,
            alignment,
            None,
            &self.font_cache,
        )
    }

    /// Lays out placeholder text for empty blocks.
    pub fn layout_placeholder(
        &self,
        text: &str,
        block_type: &BufferBlockStyle,
        spacing: &BlockSpacing,
    ) -> Arc<Line> {
        let paragraph_styles = self.paragraph_styles(block_type);
        let style_and_font = self.style_and_font(
            &paragraph_styles,
            &TextStylesWithMetadata::default().for_placeholder(),
        );
        let style_runs = &[(0..text.chars().count(), style_and_font)];
        self.layout_cache.layout_line(
            text,
            paragraph_styles.line_style(),
            style_runs,
            self.content_width(spacing),
            ClipConfig::end(),
            &self.font_cache,
        )
    }

    /// Returns the maximum width for text content laid out with the given spacing.
    fn content_width(&self, spacing: &BlockSpacing) -> f32 {
        self.max_width - spacing.x_axis_offset().as_f32()
    }

    /// The paragraph-level styling to use for blocks of the given kind.
    pub fn paragraph_styles(&self, block_type: &BufferBlockStyle) -> ParagraphStyles {
        self.rich_text_styles.paragraph_styles(block_type)
    }

    /// Given the [paragraph-level](ParagraphStyles) and [text-level](TextStyles) styles applicable
    /// to a range of text, build the [`StyleAndFont`] configuration for laying out that text.
    pub fn style_and_font(
        &self,
        paragraph_styles: &ParagraphStyles,
        text_styles: &TextStylesWithMetadata,
    ) -> StyleAndFont {
        let font_properties = text_styles.apply_properties(paragraph_styles.properties());
        let font_family = if text_styles.is_inline_code() {
            self.rich_text_styles.inline_code_style.font_family
        } else {
            paragraph_styles.font_family
        };

        let mut styling = TextStyle::default();
        if text_styles.is_placeholder() {
            styling = styling.with_foreground_color(self.rich_text_styles.placeholder_color);
        }

        if text_styles.is_strikethrough() {
            styling = styling.with_show_strikethrough(true);
        }

        if text_styles.is_underlined() {
            styling = styling.with_underline_color(self.rich_text_styles.base_text.text_color);
        }

        if text_styles.is_inline_code() {
            styling = styling
                .with_foreground_color(self.rich_text_styles.inline_code_style.font_color)
                .with_background_color(self.rich_text_styles.inline_code_style.background)
                .with_border(TextBorder {
                    color: self.rich_text_styles.inline_code_style.background,
                    radius: 4,
                    width: 1,
                    // Use the set 1.2 line height ratio for inline code backgrounds.
                    line_height_ratio_override: Some(120),
                });
        }

        if let Some(color) = text_styles.color() {
            styling = styling.with_syntax_color(color);
        }

        let style_and_font = StyleAndFont::new(font_family, font_properties, styling);

        if text_styles.is_link() {
            add_link_to_style_and_font(style_and_font)
        } else {
            style_and_font
        }
    }

    pub fn rich_text_styles(&self) -> &'a RichTextStyles {
        self.rich_text_styles
    }

    pub fn max_width(&self) -> Pixels {
        self.max_width.into_pixels()
    }
}

/// The line height for a line of text. In CSS terminology, this is the height of the
/// [line box](https://www.w3.org/TR/css-inline-3/#line-box). In typographic terms,
/// it should correspond to the distance between the [top](https://stackoverflow.com/questions/27631736/meaning-of-top-ascent-baseline-descent-bottom-and-leading-in-androids-font)
/// of one line and the top of the next.
///
/// Unlike [`Line::height`], this height does not depend on the specific text in the line. The
/// `height` field measures the line's actual bounds, so it depends on whether glyphs go below
/// the baseline or above the [cap line](https://www.canva.com/learn/typography-terms/).
pub(crate) fn line_height(line: &Line) -> f32 {
    line.font_size * line.line_height_ratio
}

pub(crate) fn add_link_to_style_and_font(mut style: StyleAndFont) -> StyleAndFont {
    let hyperlink_color = ColorU::from_u32(HYPERLINK_UNDERLINE_COLOR);
    style.style = style
        .style
        .with_underline_color(hyperlink_color)
        .with_foreground_color(hyperlink_color);
    style
}

#[cfg(test)]
pub(crate) fn markdown_inline_to_text_and_style_runs(
    inline: &FormattedTextInline,
    paragraph_style: &ParagraphStyles,
    link_color: Option<ColorU>,
    inline_code_background: Option<ColorU>,
) -> InlineTextLayoutInput {
    let mut text = String::new();
    let mut style_runs = Vec::new();
    let mut start = 0usize;

    for fragment in inline {
        if fragment.text.is_empty() {
            continue;
        }

        text.push_str(&fragment.text);
        let len = fragment.text.chars().count();
        let end = start + len;

        let mut properties = paragraph_style.properties();
        if let Some(custom_weight) = fragment.styles.weight {
            properties = properties.weight(Weight::from_custom_weight(Some(custom_weight)));
        }
        if fragment.styles.italic {
            properties = properties.style(Style::Italic);
        }

        let mut text_style = TextStyle::new();
        if fragment.styles.strikethrough {
            text_style = text_style.with_show_strikethrough(true);
        }
        if fragment.styles.underline {
            text_style = text_style.with_underline_color(paragraph_style.text_color);
        }
        if fragment.styles.inline_code
            && let Some(background) = inline_code_background
        {
            text_style = text_style.with_background_color(background);
        }
        if fragment.styles.hyperlink.is_some()
            && let Some(link_color) = link_color
        {
            text_style = text_style
                .with_foreground_color(link_color)
                .with_underline_color(link_color);
        }

        style_runs.push((
            start..end,
            StyleAndFont::new(paragraph_style.font_family, properties, text_style),
        ));
        start = end;
    }

    if text.is_empty() {
        style_runs.push((
            0..0,
            StyleAndFont::new(
                paragraph_style.font_family,
                paragraph_style.properties(),
                TextStyle::new(),
            ),
        ));
    }

    InlineTextLayoutInput { text, style_runs }
}
