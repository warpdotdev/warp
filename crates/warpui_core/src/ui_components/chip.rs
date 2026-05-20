use crate::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, Icon, MainAxisAlignment, MainAxisSize,
        ParentElement,
    },
    scene::Border,
    Element,
};

use super::{
    components::{UiComponent, UiComponentStyles},
    text::Span,
};

pub struct Chip {
    label: String,
    styles: UiComponentStyles,
    icon: Option<Icon>,
    close_button: Option<Box<dyn Element>>,
}

impl Chip {
    pub fn new(label: String, styles: UiComponentStyles) -> Self {
        Self {
            label,
            styles,
            icon: Default::default(),
            close_button: Default::default(),
        }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_close_button(mut self, close_button: Box<dyn Element>) -> Self {
        self.close_button = Some(close_button);
        self
    }

    fn styles(&self) -> UiComponentStyles {
        self.styles
    }
}

impl UiComponent for Chip {
    type ElementType = Container;
    fn build(self) -> Container {
        let styles = self.styles();

        let mut label_and_button = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        let has_icon = self.icon.is_some();
        if let Some(icon) = self.icon {
            label_and_button.add_child(
                ConstrainedBox::new(icon.finish())
                    .with_width(styles.font_size.unwrap_or_default())
                    .with_height(styles.font_size.unwrap_or_default())
                    .finish(),
            );
        }

        let mut label_container = Container::new(
            ConstrainedBox::new(Span::new(self.label, styles).build().finish())
                .with_max_width(240.)
                .finish(),
        );
        if has_icon {
            label_container = label_container.with_margin_left(5.);
        }
        label_and_button.add_child(label_container.finish());

        if let Some(close_button) = self.close_button {
            label_and_button.add_child(Container::new(close_button).with_margin_left(10.).finish());
        }

        let mut container = Container::new(label_and_button.finish())
            .with_horizontal_padding(4.)
            .with_vertical_padding(2.)
            .with_border(
                Border::all(styles.border_width.unwrap_or_default())
                    .with_border_fill(styles.border_color.unwrap_or_default()),
            )
            .with_corner_radius(styles.border_radius.unwrap_or_default());

        if let Some(background) = styles.background {
            container = container.with_background(background);
        }

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
            styles: self.styles.merge(styles),
            ..self
        }
    }
}
