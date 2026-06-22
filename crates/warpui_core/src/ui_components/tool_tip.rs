use pathfinder_color::ColorU;

use crate::elements::{Border, Container, Element, Flex, ParentElement, Text};
use crate::ui_components::components::{UiComponent, UiComponentStyles};

pub struct Tooltip {
    label: String,
    styles: UiComponentStyles,
}

const FORTY_PERCENT_OPACITY: u8 = (255. * 0.4) as u8;

impl UiComponent for Tooltip {
    type ElementType = Container;
    fn build(self) -> Container {
        let styles = self.styles;
        let mut container = Container::new(
            Text::new(
                self.label,
                styles.font_family_id.unwrap(),
                styles.font_size.unwrap_or_default(),
            )
            .with_color(styles.font_color.unwrap_or_default())
            .finish(),
        );

        if let Some(corner) = styles.border_radius {
            container = container.with_corner_radius(corner);
        }
        let mut border = Border::all(styles.border_width.unwrap_or_default());
        if let Some(border_color) = styles.border_color {
            border = border.with_border_fill(border_color);
        }
        container = container.with_border(border);

        if let Some(padding) = styles.padding {
            container = container
                .with_padding_left(padding.left)
                .with_padding_top(padding.top)
                .with_padding_right(padding.right)
                .with_padding_bottom(padding.bottom);
        }

        if let Some(margin) = styles.margin {
            container = container
                .with_margin_left(margin.left)
                .with_margin_top(margin.top)
                .with_margin_right(margin.right)
                .with_margin_bottom(margin.bottom);
        }

        if let Some(background) = styles.background {
            container = container.with_background(background);
        }

        container
    }

    fn with_style(self, styles: UiComponentStyles) -> Self {
        Tooltip {
            styles: self.styles.merge(styles),
            ..self
        }
    }
}

impl Tooltip {
    pub fn new(label: String, styles: UiComponentStyles) -> Self {
        Tooltip { label, styles }
    }
}

pub struct TooltipWithSublabel {
    label: String,
    sublabel: String,
    styles: UiComponentStyles,
}

impl UiComponent for TooltipWithSublabel {
    type ElementType = Container;
    fn build(self) -> Container {
        let styles = self.styles;

        let label_text = Container::new(
            Text::new_inline(
                self.label,
                styles.font_family_id.unwrap(),
                styles.font_size.unwrap_or_default(),
            )
            .with_color(styles.font_color.unwrap_or_default())
            .finish(),
        )
        .with_margin_right(4.)
        .finish();

        let label_font_color = styles.font_color.unwrap_or_default();
        let sublabel_font_color = ColorU::new(
            label_font_color.r,
            label_font_color.g,
            label_font_color.b,
            FORTY_PERCENT_OPACITY,
        );
        let sublabel_text = Container::new(
            Text::new_inline(
                self.sublabel,
                styles.font_family_id.unwrap(),
                styles.font_size.unwrap_or_default(),
            )
            .with_color(sublabel_font_color)
            .finish(),
        )
        .with_margin_left(4.)
        .finish();

        let mut container = Container::new(
            Flex::row()
                .with_children([label_text, sublabel_text])
                .finish(),
        );

        if let Some(corner) = styles.border_radius {
            container = container.with_corner_radius(corner);
        }
        let mut border = Border::all(styles.border_width.unwrap_or_default());
        if let Some(border_color) = styles.border_color {
            border = border.with_border_fill(border_color);
        }
        container = container.with_border(border);

        if let Some(padding) = styles.padding {
            container = container
                .with_padding_left(padding.left)
                .with_padding_top(padding.top)
                .with_padding_right(padding.right)
                .with_padding_bottom(padding.bottom);
        }

        if let Some(margin) = styles.margin {
            container = container
                .with_margin_left(margin.left)
                .with_margin_top(margin.top)
                .with_margin_right(margin.right)
                .with_margin_bottom(margin.bottom);
        }

        if let Some(background) = styles.background {
            container = container.with_background(background);
        }

        container
    }

    fn with_style(self, styles: UiComponentStyles) -> Self {
        TooltipWithSublabel {
            styles: self.styles.merge(styles),
            ..self
        }
    }
}

impl TooltipWithSublabel {
    pub fn new(label: String, sublabel: String, styles: UiComponentStyles) -> Self {
        TooltipWithSublabel {
            label,
            sublabel,
            styles,
        }
    }
}
