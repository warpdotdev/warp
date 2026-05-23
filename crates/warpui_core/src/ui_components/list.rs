use pathfinder_color::ColorU;

use crate::elements::{Container, Element, Flex, ParentElement, Text};
use crate::ui_components::components::{Coords, UiComponent, UiComponentStyles};

const BULLET: &str = "•";

pub enum ListStyle {
    Numbered,
    Bulleted,
}

impl ListStyle {
    fn render(&self, idx: usize, text: &str) -> String {
        match self {
            ListStyle::Numbered => format!("{} {}", idx + 1, text),
            ListStyle::Bulleted => format!("{BULLET} {text}"),
        }
    }
}

pub struct List {
    list_style: ListStyle,
    styles: UiComponentStyles,
    items: Vec<String>,
}

impl UiComponent for List {
    type ElementType = Flex;
    fn build(self) -> Flex {
        Flex::column().with_children(
            self.items
                .iter()
                .enumerate()
                .map(|(item_idx, item)| self.render_item(item_idx, item)),
        )
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self {
            items: self.items,
            list_style: self.list_style,
            styles: self.styles.merge(styles),
        }
    }
}

impl List {
    pub fn new(
        list_style: ListStyle,
        default_styles: UiComponentStyles,
        items: Vec<String>,
    ) -> Self {
        Self {
            list_style,
            styles: default_styles,
            items,
        }
    }

    fn render_item(&self, item_idx: usize, item: &str) -> Box<dyn Element> {
        let padding = self.styles.padding.unwrap_or_else(|| Coords::uniform(2.));
        Container::new(
            Text::new(
                self.list_style.render(item_idx, item),
                self.styles.font_family_id.unwrap(),
                self.styles.font_size.unwrap_or(14.),
            )
            .with_color(self.styles.font_color.unwrap_or_else(ColorU::white))
            .finish(),
        )
        .with_padding_top(padding.top)
        .with_padding_bottom(padding.bottom)
        .with_padding_left(padding.left)
        .with_padding_right(padding.right)
        .finish()
    }
}
