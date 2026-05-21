use super::blended_colors;
use crate::appearance::Appearance;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Element, Flex,
    MainAxisAlignment, MainAxisSize, ParentElement, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};

const DIALOG_PADDING: f32 = 20.;

/// UiComponent that implements a dialog.
/// As such by default it's only a box with title and a child (whatever's in the middle of the
/// dialog) wrapped in a Dismiss, however, it provides couple methods that allow for adding extra
/// elements to the bottom row (like buttons, or links to documentation), and a close button.
/// UiComponent::build method returns Dismiss so the user can add their own on_dismiss action.
pub struct Dialog {
    bottom_row: Vec<Box<dyn Element>>,
    bottom_row_left: Vec<Box<dyn Element>>,
    title: String,
    body: Option<String>,
    child: Option<Box<dyn Element>>,
    styles: UiComponentStyles,
    close_button: Option<Box<dyn Element>>,
    /// Optional icon rendered above the title. When set, the header row becomes
    /// `[icon] … [close button]` with the title on its own row below.
    header_icon: Option<Box<dyn Element>>,
    show_separator: bool,
}

pub fn dialog_styles(appearance: &Appearance) -> UiComponentStyles {
    let theme = appearance.theme();
    let background = theme.surface_1();
    UiComponentStyles {
        font_family_id: Some(appearance.header_font_family()),
        font_size: Some(16.),
        font_color: Some(blended_colors::text_main(theme, background)),
        font_weight: Some(warpui::fonts::Weight::Bold),
        background: Some(background.into()),
        border_color: Some(theme.surface_3().into()),
        border_radius: Some(CornerRadius::with_all(warpui::elements::Radius::Pixels(8.))),
        border_width: Some(1.),
        ..Default::default()
    }
}

impl Dialog {
    pub fn new(title: String, body: Option<String>, styles: UiComponentStyles) -> Self {
        Self {
            title,
            body,
            child: None,
            styles,
            bottom_row: Default::default(),
            bottom_row_left: Default::default(),
            close_button: None,
            header_icon: None,
            show_separator: false,
        }
    }

    pub fn with_child(mut self, child: Box<dyn Element>) -> Self {
        self.child = Some(child);
        self
    }

    pub fn with_close_button(mut self, close_button: Box<dyn Element>) -> Self {
        self.close_button = Some(close_button);
        self
    }

    /// Sets an icon element rendered in the top row alongside the close button,
    /// with the dialog title displayed below that row.
    pub fn with_header_icon(mut self, icon: Box<dyn Element>) -> Self {
        self.header_icon = Some(icon);
        self
    }

    pub fn with_bottom_row_child(mut self, child: Box<dyn Element>) -> Self {
        self.bottom_row.push(child);
        self
    }

    pub fn with_bottom_row_left_child(mut self, child: Box<dyn Element>) -> Self {
        self.bottom_row_left.push(child);
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.styles.width = Some(width);
        self
    }

    pub fn with_separator(mut self) -> Self {
        self.show_separator = true;
        self
    }
}

impl UiComponent for Dialog {
    type ElementType = Dismiss;

    fn build(self) -> Dismiss {
        let title_element = Shrinkable::new(
            1.,
            Text::new(
                self.title,
                self.styles.font_family_id.expect("FamilyId set"),
                self.styles.font_size.expect("Font size set"),
            )
            .with_style(self.styles.font_properties())
            .with_color(self.styles.font_color.unwrap_or_default())
            .finish(),
        )
        .finish();

        let footer = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children(self.bottom_row_left)
                    .finish(),
            )
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children(self.bottom_row)
                    .finish(),
            )
            .finish();

        let (left_padding, top_padding, right_padding, bottom_padding) =
            if let Some(custom_padding) = self.styles.padding {
                (
                    custom_padding.left,
                    custom_padding.top,
                    custom_padding.right,
                    custom_padding.bottom,
                )
            } else {
                (
                    DIALOG_PADDING,
                    DIALOG_PADDING,
                    DIALOG_PADDING,
                    DIALOG_PADDING,
                )
            };

        let mut main_content =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        if let Some(header_icon) = self.header_icon {
            // Icon + close button in the top row, title on its own row below.
            let mut icon_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(header_icon);
            if let Some(close_button) = self.close_button {
                icon_row.add_child(close_button);
            }
            main_content.add_child(
                Container::new(icon_row.finish())
                    .with_padding_bottom(12.)
                    .finish(),
            );
            main_content.add_child(
                Container::new(title_element)
                    .with_padding_bottom(DIALOG_PADDING)
                    .finish(),
            );
        } else {
            // Original layout: title and close button share the same row.
            let mut header = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(title_element);
            if let Some(close_button) = self.close_button {
                header.add_child(close_button);
            }
            main_content.add_child(
                Container::new(header.finish())
                    .with_padding_bottom(DIALOG_PADDING)
                    .finish(),
            );
        }

        if let Some(body) = self.body {
            main_content.add_child(
                Container::new(
                    Text::new(body, self.styles.font_family_id.expect("FamilyId set"), 14.)
                        .with_style(Properties {
                            style: self.styles.font_properties().style,
                            weight: Weight::Thin,
                        })
                        .with_color(self.styles.font_color.unwrap_or_default())
                        .finish(),
                )
                .with_padding_bottom(DIALOG_PADDING)
                .finish(),
            );
        }

        if let Some(child) = self.child {
            main_content = main_content.with_child(
                Container::new(child)
                    .with_padding_bottom(DIALOG_PADDING)
                    .finish(),
            );
        }

        let padded_main_content = Container::new(main_content.finish())
            .with_padding_left(left_padding)
            .with_padding_top(top_padding)
            .with_padding_right(right_padding)
            .finish();

        let footer_container = if self.show_separator {
            let border_color = self.styles.border_color.unwrap_or_default();

            Container::new(footer)
                .with_padding_left(left_padding)
                .with_padding_right(right_padding)
                .with_padding_top(bottom_padding)
                .with_border(Border::top(1.).with_border_fill(border_color))
                .finish()
        } else {
            Container::new(footer)
                .with_padding_left(left_padding)
                .with_padding_right(right_padding)
                .with_padding_top(0.)
                .finish()
        };

        let flex_column_contents = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(padded_main_content)
            .with_child(footer_container)
            .finish();

        let mut dialog = Container::new(flex_column_contents).with_padding_bottom(bottom_padding);

        if let Some(background) = self.styles.background {
            dialog = dialog.with_background(background);
        }
        if let Some(border_radius) = self.styles.border_radius {
            dialog = dialog.with_corner_radius(border_radius);
        }
        if let Some(border_fill) = self.styles.border_color {
            let border = Border::all(self.styles.border_width.unwrap_or_default())
                .with_border_fill(border_fill);
            dialog = dialog.with_border(border);
        }

        let mut dialog_box = ConstrainedBox::new(dialog.finish());

        if let Some(width) = self.styles.width {
            dialog_box = dialog_box.with_width(width);
        }

        Dismiss::new(dialog_box.finish())
    }

    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self { styles, ..self }
    }
}
