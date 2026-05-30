use crate::elements::{ConstrainedBox, Container, Element, Empty, Flex, ParentElement};
use crate::ui_components::components::{UiComponent, UiComponentStyles};

pub struct ProgressBar {
    progress: f32,
    styles: UiComponentStyles,
}

impl UiComponent for ProgressBar {
    type ElementType = Flex;
    fn build(self) -> Flex {
        let styles = self.styles;
        let progress_width = self.progress * styles.width.unwrap();
        Flex::row()
            .with_child(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background(styles.foreground.unwrap())
                        .finish(),
                )
                .with_width(progress_width)
                .with_height(styles.height.unwrap())
                .finish(),
            )
            .with_child(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background(styles.background.unwrap())
                        .finish(),
                )
                .with_width(styles.width.unwrap() - progress_width)
                .with_height(styles.height.unwrap())
                .finish(),
            )
    }

    fn with_style(self, styles: UiComponentStyles) -> Self {
        ProgressBar {
            styles: styles.merge(styles),
            ..self
        }
    }
}

impl ProgressBar {
    pub fn new(progress: f32, default_styles: UiComponentStyles) -> Self {
        ProgressBar {
            progress,
            styles: default_styles,
        }
    }
}
