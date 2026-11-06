use ::ai::api_keys::ApiKeyManager;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, HyperlinkUrl,
    MainAxisAlignment, MainAxisSize, ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity, View, ViewHandle};

use crate::ai::llms::CHATGPT_USAGE_URL;
use crate::settings_view::billing_and_usage_page::BillingAndUsagePageAction;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, SecondaryTheme};

/// The button that opens OpenAI's usage page from [`render_chatgpt_usage_card`].
pub(crate) fn chatgpt_manage_usage_button() -> ActionButton {
    ActionButton::new("Manage usage", SecondaryTheme)
        .with_icon(Icon::LinkExternal)
        .on_click(|ctx| {
            ctx.dispatch_typed_action(BillingAndUsagePageAction::OpenUrl(HyperlinkUrl {
                url: CHATGPT_USAGE_URL.to_owned(),
            }));
        })
}

/// Points a user with a connected ChatGPT subscription at OpenAI's usage page, since requests
/// billed to that subscription don't show up in Warp's own usage reporting. Renders nothing
/// when no subscription is connected.
pub(crate) fn render_chatgpt_usage_card(
    manage_button: &ViewHandle<ActionButton>,
    appearance: &Appearance,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    if !FeatureFlag::ChatGPTSubscription.is_enabled()
        || ApiKeyManager::as_ref(app).chatgpt_connection().is_none()
    {
        return None;
    }

    let theme = appearance.theme();
    let text_color = theme.active_ui_text_color();

    let logo = ConstrainedBox::new(Icon::OpenAILogo.to_warpui_icon(text_color).finish())
        .with_width(16.)
        .with_height(16.)
        .finish();
    let label = Text::new_inline(
        "View and manage your ChatGPT usage",
        appearance.ui_font_family(),
        14.,
    )
    .with_color(text_color.into())
    .with_style(Properties::default().weight(Weight::Semibold))
    .finish();
    let left_side = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(8.)
        .with_child(logo)
        .with_child(label)
        .finish();

    let row = Flex::row()
        .with_child(Shrinkable::new(1., left_side).finish())
        .with_child(manage_button.as_ref(app).render(app))
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .finish();

    Some(
        Container::new(row)
            .with_border(Border::all(1.).with_border_color(theme.outline().into()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_horizontal_padding(16.)
            .with_vertical_padding(12.)
            .with_margin_bottom(16.)
            .finish(),
    )
}
