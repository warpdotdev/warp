//! A reusable warning callout component with optional action button.
use markdown_parser::{FormattedText, FormattedTextInline, FormattedTextLine};
use warp_core::ui::color::blend::Blend;
use warpui::EventContext;
use warpui::color::ColorU;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Expanded, Flex,
    FormattedTextElement, Hoverable, HyperlinkLens, MainAxisSize, MouseStateHandle, ParentElement,
    Radius, Text,
};
use warpui::platform::Cursor;

use crate::appearance::Appearance;
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::icons::Icon;

pub struct WarningBoxButtonConfig {
    pub label: String,
    pub mouse_state: MouseStateHandle,
    pub on_click: Box<dyn Fn(&mut EventContext) + 'static>,
}

impl WarningBoxButtonConfig {
    pub fn new(
        label: impl Into<String>,
        mouse_state: MouseStateHandle,
        on_click: impl Fn(&mut EventContext) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            mouse_state,
            on_click: Box::new(on_click),
        }
    }
}
pub enum WarningBoxTitle {
    Text(String),
    Formatted(FormattedTextInline),
}

pub struct WarningBoxConfig {
    pub icon: Icon,
    pub title: WarningBoxTitle,
    pub description: Option<String>,

    /// Optional max width. If provided, the WarningBox will not exceed this width,
    /// but can shrink on smaller screens.
    pub width: Option<f32>,

    pub margin_top: f32,

    pub button: Option<WarningBoxButtonConfig>,
}

impl WarningBoxConfig {
    pub fn new(title: impl Into<String>) -> Self {
        Self::from_title(WarningBoxTitle::Text(title.into()))
    }

    pub fn formatted_title(title: FormattedTextInline) -> Self {
        Self::from_title(WarningBoxTitle::Formatted(title))
    }

    fn from_title(title: WarningBoxTitle) -> Self {
        Self {
            icon: Icon::AlertTriangle,
            title,
            description: None,
            width: None,
            margin_top: 8.,
            button: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn with_button(mut self, button: WarningBoxButtonConfig) -> Self {
        self.button = Some(button);
        self
    }
}
pub fn render_warning_box(config: WarningBoxConfig, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let icon_size = appearance.ui_font_size() * 1.1;

    let warning_color = theme.ui_warning_color();

    // Use a lighter yellow for readability while still clearly communicating “warning”.
    let text_color: ColorU = ThemeFill::Solid(theme.ui_yellow_color())
        .blend(&theme.foreground().with_opacity(70))
        .into();

    let warning_fill = ThemeFill::Solid(warning_color);
    let icon_fill = ThemeFill::Solid(text_color);

    let background = theme.surface_2().blend(&warning_fill.with_opacity(15));

    let title = match config.title {
        WarningBoxTitle::Text(title) => Text::new(
            title,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(text_color)
        .soft_wrap(true)
        .finish(),
        WarningBoxTitle::Formatted(title) => FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(title)]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            text_color,
            Default::default(),
        )
        .with_hyperlink_font_color(theme.accent().into())
        .register_default_click_handlers_with_action_support(|hyperlink, _event, app| {
            if let HyperlinkLens::Url(url) = hyperlink {
                app.open_url(url);
            }
        })
        .finish(),
    };

    let mut text_col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(2.)
        .with_child(title);

    if let Some(description) = config.description {
        text_col.add_child(
            Text::new(
                description,
                appearance.ui_font_family(),
                appearance.ui_font_size() * 0.9,
            )
            .with_color(text_color)
            .soft_wrap(true)
            .finish(),
        );
    }

    // Treat warning boxes as flexible by default so they wrap and shrink with their container.
    let should_use_flex = true;
    let has_action_button = config.button.is_some();

    let left = if should_use_flex {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.)
            .with_child(
                ConstrainedBox::new(config.icon.to_warpui_icon(icon_fill).finish())
                    .with_width(icon_size)
                    .with_height(icon_size)
                    .finish(),
            )
            .with_child(Expanded::new(1., text_col.finish()).finish())
            .finish()
    } else {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.)
            .with_child(
                ConstrainedBox::new(config.icon.to_warpui_icon(icon_fill).finish())
                    .with_width(icon_size)
                    .with_height(icon_size)
                    .finish(),
            )
            .with_child(text_col.finish())
            .finish()
    };

    let action_button = config.button.map(|button| {
        let WarningBoxButtonConfig {
            label,
            mouse_state,
            on_click,
        } = button;

        Hoverable::new(mouse_state, move |state| {
            let bg = if state.is_mouse_over_element() {
                theme.surface_2()
            } else {
                theme.surface_3()
            };

            Container::new(
                Text::new(
                    label.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.active_ui_text_color().into())
                .finish(),
            )
            .with_horizontal_padding(12.)
            .with_vertical_padding(8.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_background(bg)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            (on_click)(ctx);
        })
        .finish()
    });

    let mut row = Flex::row()
        .with_cross_axis_alignment(if has_action_button {
            CrossAxisAlignment::Center
        } else {
            CrossAxisAlignment::Start
        })
        .with_spacing(12.);

    if should_use_flex {
        row = row.with_main_axis_size(MainAxisSize::Max);
        row.add_child(Expanded::new(1., left).finish());
    } else {
        row.add_child(left);
    }

    if let Some(action_button) = action_button {
        row.add_child(action_button);
    }

    let mut element = ConstrainedBox::new(
        Container::new(row.finish())
            .with_margin_top(config.margin_top)
            .with_uniform_padding(12.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_background(background)
            .finish(),
    );

    if let Some(width) = config.width {
        element = element.with_max_width(width);
    }
    element.finish()
}
