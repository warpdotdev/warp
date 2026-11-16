use pathfinder_geometry::vector::vec2f;
use warp_core::ui::external_product_icon::ExternalProductIcon;
use warp_core::ui::icons::Icon;
use warpui::elements::{
    self, Align, Border, CacheOption, ChildAnchor, ConstrainedBox, Container, Element, Image,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Stack, Text,
};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};

use super::red_notification_dot::RedNotificationDot;

pub enum AvatarContent {
    /// Rendered as capital initial of the given display name.
    DisplayName(String),

    /// Renders the icon directly.
    Icon(Icon),

    ExternalProductIcon(ExternalProductIcon),

    /// Renders the image on a colored background.
    Image {
        url: String,
        /// The first initial is rendered prior to loading the image.
        display_name: String,
    },
}

#[derive(Clone)]
pub enum StatusElementTypes {
    Circle,
    Icon(Icon),
}

/// Avatar UI component.
pub struct Avatar {
    content: AvatarContent,
    styles: UiComponentStyles,
    /// If this is set, we will render a status symbol on the upper right corner of the avatar.
    status_element_type: Option<StatusElementTypes>,
    // Styles for the status
    status_styles: Option<UiComponentStyles>,
    /// Optional additional offset for the status indicator (x to the right, y downward).
    status_offset: Option<(f32, f32)>,
}

impl UiComponent for Avatar {
    type ElementType = Container;
    fn build(self) -> Container {
        let styles = self.styles;
        let inner_element = match self.content {
            AvatarContent::Image { url, display_name } => {
                let mut image = Image::new(asset_cache::url_source(url), CacheOption::BySize)
                    .before_load(
                        Align::new(Self::first_initial(&display_name, self.styles)).finish(),
                    );
                if let Some(radius) = styles.border_radius {
                    image = image.with_corner_radius(radius);
                }
                image.finish()
            }
            AvatarContent::Icon(icon) => {
                let icon_size = {
                    let height = styles.height.unwrap_or_default();
                    // One third of the total avatar height/width should be padding.
                    height * 0.66
                };
                ConstrainedBox::new(
                    elements::Icon::new(icon.into(), styles.font_color.unwrap_or_default())
                        .finish(),
                )
                .with_width(icon_size)
                .with_height(icon_size)
                .finish()
            }
            AvatarContent::ExternalProductIcon(external_product_icon) => {
                let icon_size = {
                    let height = styles.height.unwrap_or_default();
                    // One third of the total avatar height/width should be padding.
                    height * 0.66
                };
                ConstrainedBox::new(
                    elements::Icon::new(
                        external_product_icon.get_path(),
                        styles.font_color.unwrap_or_default(),
                    )
                    .finish(),
                )
                .with_width(icon_size)
                .with_height(icon_size)
                .finish()
            }
            AvatarContent::DisplayName(name) => Self::first_initial(&name, self.styles),
        };

        let mut constrained_box = ConstrainedBox::new(Align::new(inner_element).finish());

        if let Some(height) = styles.height {
            constrained_box = constrained_box.with_height(height);
        }
        if let Some(width) = styles.width {
            constrained_box = constrained_box.with_width(width);
        }

        let mut container = Container::new(
            if let Some(status_element_type) = self.status_element_type {
                let offset = self.status_offset.unwrap_or((0., 0.));
                let status_styles = self.status_styles.unwrap_or_default();
                match status_element_type {
                    StatusElementTypes::Circle => RedNotificationDot::render_with_offset(
                        constrained_box.finish(),
                        &status_styles,
                        offset,
                    ),
                    StatusElementTypes::Icon(icon) => Self::render_icon_with_offset(
                        constrained_box.finish(),
                        icon,
                        &status_styles,
                        offset,
                    ),
                }
            } else {
                constrained_box.finish()
            },
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
        Avatar {
            styles: self.styles.merge(styles),
            ..self
        }
    }
}

impl Avatar {
    pub fn new(content: AvatarContent, styles: UiComponentStyles) -> Self {
        Avatar {
            content,
            styles,
            status_element_type: None,
            status_styles: None,
            status_offset: None,
        }
    }

    pub fn with_status_element(
        mut self,
        status_element_type: StatusElementTypes,
        status_styles: UiComponentStyles,
    ) -> Self {
        self.status_element_type = Some(status_element_type);
        self.status_styles = Some(status_styles);
        self.status_offset = None;
        self
    }

    pub fn with_status_element_with_offset(
        mut self,
        status_element_type: StatusElementTypes,
        status_styles: UiComponentStyles,
        x_delta: f32,
        y_delta: f32,
    ) -> Self {
        self.status_element_type = Some(status_element_type);
        self.status_styles = Some(status_styles);
        self.status_offset = Some((x_delta, y_delta));
        self
    }

    /// Returns an element with the first initial of the user, capitalized.
    /// Note: Unicode characters can be more than one byte, and uppercasing a unicode character
    /// can produce more than one character. For example, the uppercase of ß is SS.
    /// In that case we take the first character (just S).
    fn first_initial(display_name: &str, styles: UiComponentStyles) -> Box<dyn Element> {
        Text::new_inline(
            display_name
                .chars()
                .next()
                .unwrap_or_default()
                .to_uppercase()
                .next()
                .unwrap_or_default()
                .to_string(),
            styles.font_family_id.expect("text must have font family"),
            styles.font_size.unwrap_or_default(),
        )
        .with_color(styles.font_color.unwrap_or_default())
        .with_style(styles.font_properties())
        .finish()
    }

    fn render_icon_with_offset(
        element: Box<dyn Element>,
        icon: Icon,
        styles: &UiComponentStyles,
        (x_delta, y_delta): (f32, f32),
    ) -> Box<dyn Element> {
        let icon_size = styles.width.unwrap_or(12.0);
        let x_axis_offset = icon_size / 2.;
        let y_axis_offset = -(icon_size / 2.);

        let icon_element = ConstrainedBox::new(
            elements::Icon::new(icon.into(), styles.font_color.unwrap_or_default()).finish(),
        )
        .with_width(icon_size)
        .with_height(icon_size)
        .finish();

        let mut stack = Stack::new();
        stack.add_child(element);
        stack.add_positioned_child(
            icon_element,
            OffsetPositioning::offset_from_parent(
                vec2f(x_axis_offset + x_delta, y_axis_offset + y_delta),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );
        stack.finish()
    }
}
