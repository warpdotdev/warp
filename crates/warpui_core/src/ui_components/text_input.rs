use crate::elements::{Border, ChildView, Clipped, ConstrainedBox, Container, Element};
use crate::ui_components::components::{UiComponent, UiComponentStyles};
use crate::{View, ViewHandle};

pub struct TextInput<T: View> {
    editor: ViewHandle<T>,
    styles: UiComponentStyles,
}

impl<T: View> UiComponent for TextInput<T> {
    type ElementType = ConstrainedBox;
    fn build(self) -> ConstrainedBox {
        self.render_text_input()
    }

    fn with_style(self, styles: UiComponentStyles) -> Self {
        TextInput {
            editor: self.editor,
            styles: self.styles.merge(styles),
        }
    }
}

impl<T: View> TextInput<T> {
    pub fn new(editor: ViewHandle<T>, default_styles: UiComponentStyles) -> Self {
        TextInput {
            editor,
            styles: default_styles,
        }
    }

    fn render_text_input(&self) -> ConstrainedBox {
        let styles = self.styles;

        let mut container =
            Container::new(Clipped::new(ChildView::new(&self.editor).finish()).finish());
        // Setting up the border
        if let Some(corner) = styles.border_radius {
            container = container.with_corner_radius(corner);
        }

        let mut border = Border::all(styles.border_width.unwrap_or_default());
        if let Some(border_color) = styles.border_color {
            border = border.with_border_fill(border_color);
        }
        container = container.with_border(border);

        // Position-related settings
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

        match (styles.height, styles.width) {
            (None, None) => ConstrainedBox::new(container.finish()),
            (_, _) => {
                let mut constrained_box = ConstrainedBox::new(container.finish());
                if let Some(height) = styles.height {
                    constrained_box = constrained_box.with_height(height);
                }
                if let Some(width) = styles.width {
                    constrained_box = constrained_box.with_width(width);
                }
                constrained_box
            }
        }
    }
}
