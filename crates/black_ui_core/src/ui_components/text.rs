use std::borrow::Cow;

use itertools::Itertools;

use crate::elements::{
    Container, Element, Highlight, HighlightedRange, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use crate::fonts::Properties;
use crate::ui_components::components::{UiComponent, UiComponentStyles};

#[derive(Debug, Clone, Default)]
pub struct WrappableText {
    text: Cow<'static, str>,
    styles: UiComponentStyles,
    wrap: bool,
    line_height_ratio: f32,
    highlights: Vec<HighlightedRange>,
    /// Whether the text is selectable when rendered as a descendant of a [`SelectableArea`].
    is_selectable: bool,
}

impl WrappableText {
    pub fn new(text: Cow<'static, str>, soft_wrap: bool, styles: UiComponentStyles) -> Self {
        WrappableText {
            text,
            styles,
            wrap: soft_wrap,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            highlights: vec![],
            is_selectable: true,
        }
    }

    pub fn with_highlights(mut self, highlight_indices: Vec<usize>, highlight: Highlight) -> Self {
        if highlight_indices.is_empty() {
            return self;
        }
        self.highlights = vec![HighlightedRange {
            highlight,
            highlight_indices,
        }];
        self
    }

    pub fn with_line_height_ratio(mut self, line_height_ratio: f32) -> Self {
        self.line_height_ratio = line_height_ratio;
        self
    }

    pub fn with_selectable(mut self, is_selectable: bool) -> Self {
        self.is_selectable = is_selectable;
        self
    }
}

impl UiComponent for WrappableText {
    type ElementType = Container;
    fn build(self) -> Container {
        let styles = self.styles;
        let mut text = Text::new(
            self.text,
            styles.font_family_id.unwrap(),
            styles.font_size.unwrap_or_default(),
        )
        .soft_wrap(self.wrap)
        .with_line_height_ratio(self.line_height_ratio)
        .with_selectable(self.is_selectable);
        if let Some(color) = styles.font_color {
            text = text.with_color(color);
        }
        if let Some(weight) = styles.font_weight {
            text = text.with_style(Properties::default().weight(weight))
        }

        // The text element assumes that highlights are sorted by character index.
        text = text.with_highlights(
            self.highlights
                .iter()
                .sorted_by_key(|highlighted_range| highlighted_range.highlight_indices.first())
                .cloned(),
        );

        let mut container = Container::new(text.finish());
        if let Some(margin) = styles.margin {
            container = container
                .with_margin_left(margin.left)
                .with_margin_top(margin.top)
                .with_margin_right(margin.right)
                .with_margin_bottom(margin.bottom);
        }
        container
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self {
            text: self.text,
            styles: self.styles.merge(styles),
            ..self
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Span {
    text: WrappableText,
}

impl Span {
    pub fn new(text: impl Into<Cow<'static, str>>, styles: UiComponentStyles) -> Self {
        Span {
            text: WrappableText::new(text.into(), false, styles),
        }
    }

    pub fn with_highlights(mut self, highlight_indices: Vec<usize>, highlight: Highlight) -> Self {
        self.text = self.text.with_highlights(highlight_indices, highlight);
        self
    }

    pub fn with_soft_wrap(mut self) -> Self {
        self.text.wrap = true;
        self
    }

    pub fn with_line_height_ratio(mut self, line_height_ratio: f32) -> Self {
        self.text.line_height_ratio = line_height_ratio;
        self
    }

    pub fn with_selectable(mut self, is_selectable: bool) -> Self {
        self.text.is_selectable = is_selectable;
        self
    }
}

impl UiComponent for Span {
    type ElementType = Container;
    fn build(self) -> Container {
        self.text.build()
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self {
            text: self.text.with_style(styles),
        }
    }
}

// Main difference between Span vs Paragraph is that Paragraph wraps the text
// and it's intention is to be used for longer text blocks whereas Span is good
// for short labels etc.
#[derive(Debug, Clone, Default)]
pub struct Paragraph {
    text: WrappableText,
}

impl Paragraph {
    pub fn new(text: impl Into<Cow<'static, str>>, styles: UiComponentStyles) -> Self {
        Paragraph {
            text: WrappableText::new(text.into(), true, styles),
        }
    }

    pub fn with_highlights(mut self, highlight_indices: Vec<usize>, highlight: Highlight) -> Self {
        self.text = self.text.with_highlights(highlight_indices, highlight);
        self
    }

    // TODO(alokedesai): Make it clear throughout the text rendering code that highlights are
    // indexed by _character_, not byte.
    pub fn add_highlight(&mut self, highlight_indices: Vec<usize>, highlight: Highlight) {
        if highlight_indices.is_empty() {
            return;
        }
        self.text.highlights.push(HighlightedRange {
            highlight,
            highlight_indices,
        });
    }
}

impl UiComponent for Paragraph {
    type ElementType = Container;
    fn build(self) -> Container {
        self.text.build()
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self {
            text: self.text.with_style(styles),
        }
    }
}
