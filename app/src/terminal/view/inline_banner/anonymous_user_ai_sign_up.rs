use warpui::Element;
use warpui::elements::{
    Container, CornerRadius, CrossAxisAlignment, Flex, Icon, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};

use super::{
    INLINE_BANNER_BUTTON_HOVER_OPACITY, INLINE_BANNER_BUTTON_PADDING,
    INLINE_BANNER_MARGIN_BETWEEN_BUTTONS, INLINE_BANNER_RIGHT_MARGIN,
};
use crate::appearance::Appearance;
use crate::terminal::view::{InlineBannerId, TerminalAction};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon as UiIcon;

const TITLE: &str = "Login for AI";
const CONTENT: &str =
    "AI features are unavailable for logged-out users. Create an account to use AI.";
const SIGN_UP_BUTTON_TEXT: &str = "Sign Up";

// Layout constants for three-column banner
const ICON_SIZE_OFFSET: f32 = 3.0;
const TEXT_COLUMN_LEFT_PADDING: f32 = 8.0;
const ICON_LEFT_PADDING: f32 = 5.0;
const CONTENT_TOP_PADDING: f32 = 4.0;
const BANNER_VERTICAL_PADDING: f32 = 8.0;

#[derive(Clone, Copy, Debug)]
pub enum AnonymousUserLoginBannerAction {
    SignUp,
    Close,
}

pub struct AnonymousUserAISignUpBannerState {
    pub id: InlineBannerId,
    pub sign_up_button_mouse_state: MouseStateHandle,
    pub close_button_mouse_state: MouseStateHandle,
}

impl AnonymousUserAISignUpBannerState {
    pub fn new(id: InlineBannerId) -> Self {
        Self {
            id,
            sign_up_button_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
        }
    }

    pub fn render(&self, appearance: &Appearance) -> Box<dyn Element> {
        render_three_column_inline_banner(
            appearance,
            TITLE,
            CONTENT,
            SIGN_UP_BUTTON_TEXT,
            self.sign_up_button_mouse_state.clone(),
            self.close_button_mouse_state.clone(),
        )
    }
}

/// Renders a three-column inline banner with:
/// - Column 1: Icon
/// - Column 2: Text column (Title row + Content row)
/// - Column 3: Buttons (Sign Up + Close)
fn render_three_column_inline_banner(
    appearance: &Appearance,
    title: &str,
    content: &str,
    button_text: &str,
    button_mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let title_font_size = appearance.ui_font_size() + 2.;
    let content_font_size = appearance.ui_font_size();
    let button_text_size = appearance.ui_font_size();
    let active_text_color = theme.active_ui_text_color().into_solid();
    let content_text_color = theme.nonactive_ui_text_color().into_solid();

    let hover_background_fill = warpui::elements::Fill::from(
        theme
            .active_ui_text_color()
            .with_opacity(INLINE_BANNER_BUTTON_HOVER_OPACITY),
    );

    let default_button_styles = UiComponentStyles {
        font_size: Some(button_text_size),
        font_family_id: Some(appearance.ui_font_family()),
        padding: Some(Coords {
            top: INLINE_BANNER_BUTTON_PADDING,
            bottom: INLINE_BANNER_BUTTON_PADDING,
            left: INLINE_BANNER_BUTTON_PADDING * 2.0,
            right: INLINE_BANNER_BUTTON_PADDING * 2.0,
        }),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(
            INLINE_BANNER_BUTTON_PADDING,
        ))),
        ..Default::default()
    };

    let hovered_and_clicked_styles = UiComponentStyles {
        background: Some(hover_background_fill),
        ..default_button_styles
    };

    // Main horizontal row with SpaceBetween, centered vertically
    let mut main_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max);

    // Left section: Icon + Text column (grouped together)
    let mut left_section = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Start);

    // Column 1: Icon (sized to match title)
    let icon_width = title_font_size + ICON_SIZE_OFFSET;
    let icon_height = title_font_size + ICON_SIZE_OFFSET;
    let icon_color = active_text_color;

    let icon_column = Container::new(
        warpui::elements::ConstrainedBox::new(
            Icon::new("bundled/svg/info.svg", icon_color).finish(),
        )
        .with_width(icon_width)
        .with_height(icon_height)
        .finish(),
    )
    .with_padding_left(ICON_LEFT_PADDING)
    .finish();

    left_section.add_child(icon_column);

    // Column 2: Text column (Title + Content)
    let mut text_column = Flex::column();

    // Row 1: Title
    let title_row = Container::new(
        Text::new(
            title.to_owned(),
            appearance.ui_font_family(),
            title_font_size,
        )
        .with_color(active_text_color)
        .soft_wrap(true)
        .finish(),
    )
    .with_padding_left(TEXT_COLUMN_LEFT_PADDING)
    .finish();
    text_column.add_child(title_row);

    // Row 2: Content
    let content_row = Container::new(
        Text::new(
            content.to_owned(),
            appearance.ui_font_family(),
            content_font_size,
        )
        .with_color(content_text_color)
        .soft_wrap(true)
        .finish(),
    )
    .with_padding_left(TEXT_COLUMN_LEFT_PADDING)
    .with_padding_top(CONTENT_TOP_PADDING)
    .finish();
    text_column.add_child(content_row);

    // Add text column to left section
    left_section.add_child(text_column.finish());

    // Add left section to main row (wrapped in Shrinkable to provide bounded constraint)
    main_row.add_child(Shrinkable::new(1.0, left_section.finish()).finish());
    let mut buttons_column = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Sign Up Button
    let button_styles = UiComponentStyles {
        font_color: Some(active_text_color),
        font_size: Some(button_text_size),
        font_weight: Some(warpui::fonts::Weight::Semibold),
        border_color: Some(warpui::elements::Fill::Solid(content_text_color)),
        border_width: Some(1.0),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(
            INLINE_BANNER_BUTTON_PADDING,
        ))),
        ..Default::default()
    };

    let button_on_click_event =
        TerminalAction::AnonymousUserAISignUpBanner(AnonymousUserLoginBannerAction::SignUp);
    let button = appearance
        .ui_builder()
        .button_with_custom_styles(
            ButtonVariant::Text,
            button_mouse_state,
            default_button_styles,
            Some(hovered_and_clicked_styles),
            Some(hovered_and_clicked_styles),
            Some(hovered_and_clicked_styles),
        )
        .with_text_label(button_text.to_string())
        .with_style(button_styles)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(button_on_click_event.clone());
        })
        .finish();

    buttons_column.add_child(
        Container::new(button)
            .with_margin_left(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
            .finish(),
    );

    // Close button
    let close_button_on_click_event =
        TerminalAction::AnonymousUserAISignUpBanner(AnonymousUserLoginBannerAction::Close);
    let close_button = icon_button(appearance, UiIcon::X, false, close_button_mouse_state)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(close_button_on_click_event.clone());
        })
        .finish();

    buttons_column.add_child(
        Container::new(close_button)
            .with_margin_right(INLINE_BANNER_RIGHT_MARGIN)
            .with_margin_left(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
            .finish(),
    );

    main_row.add_child(buttons_column.finish());

    // Apply background and padding
    Container::new(main_row.finish())
        .with_background(theme.surface_2())
        .with_padding_top(BANNER_VERTICAL_PADDING)
        .with_padding_bottom(BANNER_VERTICAL_PADDING)
        .finish()
}
