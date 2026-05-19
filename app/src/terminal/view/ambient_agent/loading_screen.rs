//! Loading screen UI for cloud mode initialization.

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::AnsiColorIdentifier;
use warp_core::ui::Icon;
use warpui::elements::shimmering_text::ShimmeringTextStateHandle;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CrossAxisAlignment, Element, Expanded, Flex,
    FormattedTextElement, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    SelectableArea, SelectionHandle, Text,
};
use warpui::fonts::Properties;
use warpui::fonts::Weight;
use warpui::prelude::{CornerRadius, Radius};
use warpui::text_layout::TextAlignment;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use crate::ai::agent_tips::{AITip, AITipModel};
use crate::ai::loading::shimmering_warp_loading_text;
use crate::terminal::view::ambient_agent::CloudModeTip;
use crate::ui_components::blended_colors;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Icon size for the error icon
const ERROR_ICON_SIZE: f32 = 24.;

/// Renders the cloud mode loading screen with shimmering warp logo and tips.
pub fn render_cloud_mode_loading_screen(
    message: &str,
    appearance: &Appearance,
    shimmer_handle: &ShimmeringTextStateHandle,
    tip_model: &ModelHandle<AITipModel<CloudModeTip>>,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // Larger font size for the main loading text
    let loading_font_size = appearance.monospace_font_size() + 2.;

    // Create the shimmering warp loading text element
    let shimmer_element =
        shimmering_warp_loading_text(message, loading_font_size, shimmer_handle.clone(), app);

    // Get current tip from the model and render with link
    let tip_element = if let Some(tip) = tip_model.as_ref(app).current_tip() {
        let mut fragments = tip.to_formatted_text(app);

        // Add link at the end if it exists
        if let Some(link_target) = tip.link() {
            fragments.push(FormattedTextFragment::plain_text(" "));
            fragments.push(FormattedTextFragment::hyperlink("Learn more", link_target));
        }

        let formatted_text = FormattedText::new(vec![FormattedTextLine::Line(fragments)]);
        let tip_font_size = appearance.monospace_font_size() - 2.;
        FormattedTextElement::new(
            formatted_text,
            tip_font_size,
            appearance.ui_font_family(),
            appearance.monospace_font_family(),
            blended_colors::text_sub(theme, theme.surface_1()),
            Default::default(),
        )
        .with_alignment(TextAlignment::Center)
        .with_hyperlink_font_color(theme.accent().into())
        .set_selectable(true)
        .register_default_click_handlers_with_action_support(|link, _evt, app| {
            use warpui::elements::HyperlinkLens;
            if let HyperlinkLens::Url(url) = link {
                app.open_url(url);
            }
        })
        .finish()
    } else {
        // Fallback if no tip is available
        Text::new(
            "",
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .finish()
    };

    // Get tier info for the concurrency limits footer
    let tier_footer_element = render_tier_limits_footer(appearance, app);

    // Vertical layout with centered main content and footer at bottom
    Flex::column()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Expanded::new(
                1.,
                // Align centers content both horizontally and vertically within the Expanded area
                Align::new(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Container::new(shimmer_element)
                                .with_horizontal_padding(4.)
                                .finish(),
                        )
                        .with_child(
                            Container::new(tip_element)
                                .with_horizontal_padding(4.)
                                .with_margin_top(8.)
                                .finish(),
                        )
                        .finish(),
                )
                .finish(),
            )
            .finish(),
        )
        // Footer anchored at bottom (only if we have tier info to show)
        .with_children(
            tier_footer_element
                .into_iter()
                .map(|element| {
                    Container::new(element)
                        .with_horizontal_padding(16.)
                        .with_vertical_padding(12.)
                        .finish()
                })
                .collect::<Vec<_>>(),
        )
        .finish()
}

/// Renders the tier limits footer showing concurrency limits and upgrade suggestions.
/// Returns None if there are no specs to display.
fn render_tier_limits_footer(
    appearance: &Appearance,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let theme = appearance.theme();
    let footer_font_size = appearance.monospace_font_size() - 2.;

    // Get tier info and billing metadata from UserWorkspaces
    let workspace = UserWorkspaces::as_ref(app).current_workspace()?;
    let policy = workspace.billing_metadata.tier.ambient_agents_policy?;

    let shape = policy.instance_shape.as_ref()?;
    let specs = format!("{}CPU, {}GB", shape.vcpus, shape.memory_gb);

    // If there's no way to upgrade, don't render the footer at all
    // (Build Max users can still upgrade to Business plans)
    if !workspace.billing_metadata.can_upgrade_to_build_plan()
        && !workspace.billing_metadata.can_upgrade_to_build_max_plan()
        && !workspace.billing_metadata.is_on_build_max_plan()
    {
        return None;
    }

    let mut fragments = vec![FormattedTextFragment::plain_text(format!(
        "Your agent is currently running on a {} machine. ",
        specs
    ))];

    // Get the upgrade URL for the current team
    let upgrade_url = UserWorkspaces::as_ref(app)
        .current_team()
        .map(|team| UserWorkspaces::upgrade_link_for_team(team.uid))?;

    fragments.push(FormattedTextFragment::hyperlink("Upgrade", upgrade_url));
    fragments.push(FormattedTextFragment::plain_text(
        " for more powerful cloud agents.",
    ));

    let formatted_text = FormattedText::new(vec![FormattedTextLine::Line(fragments)]);

    let text_element = FormattedTextElement::new(
        formatted_text,
        footer_font_size,
        appearance.ui_font_family(),
        appearance.monospace_font_family(),
        blended_colors::text_sub(theme, theme.surface_1()),
        Default::default(),
    )
    .with_alignment(TextAlignment::Center)
    .with_hyperlink_font_color(theme.accent().into())
    .register_default_click_handlers_with_action_support(|link, _evt, app| {
        use warpui::elements::HyperlinkLens;
        if let HyperlinkLens::Url(url) = link {
            app.open_url(url);
        }
    })
    .finish();

    // Create info icon
    let icon_size = footer_font_size;
    let info_icon = ConstrainedBox::new(
        Icon::Info
            .to_warpui_icon(blended_colors::text_sub(theme, theme.surface_1()).into())
            .finish(),
    )
    .with_width(icon_size)
    .with_height(icon_size)
    .finish();

    Some(
        Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(info_icon)
            .with_child(text_element)
            .finish(),
    )
}

/// Renders the cloud mode error screen.
pub fn render_cloud_mode_error_screen(
    error_message: &str,
    appearance: &Appearance,
    selection_handle: &SelectionHandle,
    selected_text: &std::rc::Rc<parking_lot::RwLock<Option<String>>>,
    _app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let error_color = AnsiColorIdentifier::Red.to_ansi_color(&theme.terminal_colors().normal);

    // Error icon with fixed size constraints - using AlertTriangle icon
    let error_icon = ConstrainedBox::new(
        Icon::AlertTriangle
            .to_warpui_icon(error_color.into())
            .finish(),
    )
    .with_width(ERROR_ICON_SIZE)
    .with_height(ERROR_ICON_SIZE)
    .finish();

    // Error title text
    let title_text = Text::new(
        "Failed to start environment",
        appearance.ui_font_family(),
        appearance.monospace_font_size() + 2.,
    )
    .with_style(Properties::default().weight(Weight::Bold))
    .with_color(error_color.into())
    .finish();

    // Error message wrapped in SelectableArea to make it selectable for easy copying
    let error_text = Text::new(
        error_message.to_string(),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(error_color.into())
    .with_selectable(true)
    .soft_wrap(true)
    .finish();

    // Wrap error text in SelectableArea to enable text selection
    let selected_text = selected_text.clone();
    let selectable_error_text = SelectableArea::new(
        selection_handle.clone(),
        move |selection_args, _, _| {
            *selected_text.write() = selection_args.selection.filter(|s| !s.is_empty());
        },
        error_text,
    )
    .finish();

    // Content column with icon, title, and message stacked vertically
    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(12.)
        .with_child(error_icon)
        .with_child(title_text)
        .with_child(selectable_error_text)
        .finish();

    // Red bordered container with 10% opacity background
    let error_background = warp_core::ui::color::coloru_with_opacity(error_color.into(), 10);

    let error_container = Container::new(content)
        .with_background(error_background)
        .with_border(Border::all(1.).with_border_color(error_color.into()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_horizontal_padding(24.)
        .with_vertical_padding(16.)
        .finish();

    // Constrain error container to max 400px width
    let constrained_error = ConstrainedBox::new(error_container)
        .with_max_width(400.)
        .finish();

    // Center the error container in the view
    Flex::column()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(constrained_error)
        .finish()
}

/// Renders the cloud mode GitHub authentication required screen.
pub fn render_cloud_mode_github_auth_required_screen(
    auth_url: &str,
    appearance: &Appearance,
    auth_button_mouse_state: &MouseStateHandle,
    _app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    // Use main text color for the icon and title
    let title_color = blended_colors::text_main(theme, theme.surface_1());
    // Use sub text color for the message
    let message_color = blended_colors::text_sub(theme, theme.surface_1());
    // Use accent color for the button
    let accent_color = blended_colors::accent(theme);
    // Use neutral_4 for the border
    let border_color = blended_colors::neutral_4(theme);

    // Info icon with fixed size constraints
    let auth_icon = ConstrainedBox::new(Icon::Info.to_warpui_icon(accent_color).finish())
        .with_width(ERROR_ICON_SIZE)
        .with_height(ERROR_ICON_SIZE)
        .finish();

    // Title text - "GitHub Authentication Required"
    let title_text = Text::new(
        "GitHub Authentication Required",
        appearance.ui_font_family(),
        appearance.monospace_font_size() + 2.,
    )
    .with_style(Properties::default().weight(Weight::Bold))
    .with_color(title_color)
    .finish();

    // Message text - "Please authenticate with GitHub to continue"
    let message_text = Text::new(
        "Please authenticate with GitHub to continue",
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(message_color)
    .finish();

    // Create the authenticate button
    let auth_url_clone = auth_url.to_string();
    let auth_button = appearance
        .ui_builder()
        .button(ButtonVariant::Accent, auth_button_mouse_state.clone())
        .with_centered_text_label("Authenticate with GitHub".to_string())
        .build()
        .on_click(move |_, app, _| {
            app.open_url(&auth_url_clone);
        })
        .finish();

    // Content column with icon, title, message, and auth button stacked vertically
    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(12.)
        .with_child(auth_icon)
        .with_child(title_text)
        .with_child(message_text)
        .with_child(Container::new(auth_button).with_margin_top(8.).finish())
        .finish();

    // Dark background (surface_2) with subtle border
    let auth_background: warpui::elements::Fill = theme.surface_2().into();

    let auth_container = Container::new(content)
        .with_background(auth_background)
        .with_border(Border::all(1.).with_border_color(border_color))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_horizontal_padding(24.)
        .with_vertical_padding(16.)
        .finish();

    // Constrain container to max 400px width
    let constrained_auth = ConstrainedBox::new(auth_container)
        .with_max_width(400.)
        .finish();

    // Center the container in the view
    Flex::column()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(constrained_auth)
        .finish()
}

/// Renders the cloud mode cancelled screen.
pub fn render_cloud_mode_cancelled_screen(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();

    // Use main text color for the icon and title
    let title_color = blended_colors::text_main(theme, theme.surface_1());
    // Use sub text color for the subtitle
    let subtitle_color = blended_colors::text_sub(theme, theme.surface_1());
    // Use neutral_4 for the border
    let border_color = blended_colors::neutral_4(theme);

    // SlashCircle icon with fixed size constraints
    let cancelled_icon = ConstrainedBox::new(
        Icon::SlashCircle
            .to_warpui_icon(title_color.into())
            .finish(),
    )
    .with_width(ERROR_ICON_SIZE)
    .with_height(ERROR_ICON_SIZE)
    .finish();

    // Title text - "Cloud Agent Run Cancelled"
    let title_text = Text::new(
        "Cloud Agent Run Cancelled",
        appearance.ui_font_family(),
        appearance.monospace_font_size() + 2.,
    )
    .with_style(Properties::default().weight(Weight::Bold))
    .with_color(title_color)
    .finish();

    // Subtitle text - "No cloud environment was started"
    let subtitle_text = Text::new(
        "No cloud environment was started",
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(subtitle_color)
    .finish();

    // Content column with icon, title, and subtitle stacked vertically
    let content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(12.)
        .with_child(cancelled_icon)
        .with_child(title_text)
        .with_child(subtitle_text)
        .finish();

    // Dark background (surface_2) with subtle border
    let cancelled_background: warpui::elements::Fill = theme.surface_2().into();

    let cancelled_container = Container::new(content)
        .with_background(cancelled_background)
        .with_border(Border::all(1.).with_border_color(border_color))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_horizontal_padding(24.)
        .with_vertical_padding(16.)
        .finish();

    // Constrain container to max 400px width
    let constrained_cancelled = ConstrainedBox::new(cancelled_container)
        .with_max_width(400.)
        .finish();

    // Center the container in the view
    Flex::column()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(constrained_cancelled)
        .finish()
}
