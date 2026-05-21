use pathfinder_color::ColorU;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_core::ui::{
    appearance::Appearance,
    builder::UiBuilder,
    color::{darken, lighten},
    theme::ColorScheme,
};
use warpui::{
    assets::asset_cache::AssetSource,
    elements::{
        Border, CacheOption, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Fill,
        Flex, Image, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
        Shrinkable,
    },
    fonts::Weight,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
    Action, AppContext, Element, SingletonEntity as _,
};

use crate::settings::PrivacySettings;
use crate::themes::theme::ThemeKind;

const PRIVACY_URL: &str = "https://warp.dev/privacy";

pub const AUTH_MODAL_GAP: f32 = 16.;
const MODAL_CORNER_RADIUS: Radius = Radius::Pixels(8.);

pub fn action_button_color_and_variant(appearance: &Appearance) -> (ColorU, ButtonVariant) {
    let (button_color, button_variant) = match appearance.theme().name() {
        Some(name) if ThemeKind::Dark.matches(&name) => {
            (ColorU::new(0, 109, 168, 255), ButtonVariant::Basic)
        }
        Some(_) => (appearance.theme().accent().into(), ButtonVariant::Accent),
        None => (appearance.theme().accent().into(), ButtonVariant::Accent),
    };
    (button_color, button_variant)
}

pub fn render_offline_contents<A>(
    appearance: &Appearance,
    ui_builder: &UiBuilder,
    mouse_state_handle: MouseStateHandle,
    action: A,
    app: &AppContext,
) -> Box<dyn Element>
where
    A: Action + Clone,
{
    let disclaimer_color = appearance
        .theme()
        .sub_text_color(appearance.theme().background())
        .into();

    let disclaimer_styles = UiComponentStyles {
        font_color: Some(disclaimer_color),
        ..Default::default()
    };

    let text = "You are currently offline. An internet connection is required to use Warp for the first time.";

    let (button_color, button_variant) = action_button_color_and_variant(appearance);
    let button_styles = UiComponentStyles {
        font_size: Some(14.),
        font_family_id: Some(appearance.ui_font_family()),
        font_weight: Some(Weight::Bold),
        background: Some(Fill::Solid(button_color)),
        border_width: Some(2.),
        border_color: Some(Fill::Solid(ColorU::transparent_black())),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
        padding: Some(Coords {
            top: 0.,
            bottom: 0.,
            left: 8.,
            right: 8.,
        }),
        height: Some(40.),
        ..Default::default()
    };

    let hover_button_style = UiComponentStyles {
        border_color: Some(Fill::Solid(lighten(button_color))),
        ..button_styles
    };

    let click_button_style = UiComponentStyles {
        background: Some(Fill::Solid(darken(button_color))),
        ..hover_button_style
    };

    let button = ui_builder
        .button_with_custom_styles(
            button_variant,
            mouse_state_handle.clone(),
            button_styles,
            Some(hover_button_style),
            Some(click_button_style),
            None,
        )
        .with_centered_text_label(crate::i18n::tr_static(app, "Learn more").into())
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish();

    Flex::column()
        .with_child(
            Container::new(
                ui_builder
                    .paragraph(crate::i18n::tr_static(app, text))
                    .with_style(disclaimer_styles)
                    .build()
                    .finish(),
            )
            .with_margin_bottom(AUTH_MODAL_GAP)
            .finish(),
        )
        .with_child(button)
        .finish()
}

pub fn render_square_logo(appearance: &Appearance) -> Box<dyn Element> {
    let image_path = if appearance.theme().inferred_color_scheme() == ColorScheme::LightOnDark {
        "bundled/svg/warp-logo-light.svg"
    } else {
        "bundled/svg/warp-logo-dark.svg"
    };

    ConstrainedBox::new(
        Container::new(
            Image::new(
                AssetSource::Bundled { path: image_path },
                CacheOption::BySize,
            )
            .finish(),
        )
        .with_background(appearance.theme().surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
        .with_horizontal_padding(11.)
        .finish(),
    )
    .with_width(64.)
    .with_height(64.)
    .finish()
}

pub fn render_offline_info_overlay_body<A>(
    appearance: &Appearance,
    app: &AppContext,
    mouse_state_handle: MouseStateHandle,
    action: A,
) -> Box<dyn Element>
where
    A: Action + Clone,
{
    let header_styles = UiComponentStyles {
        font_family_id: Some(appearance.header_font_family()),
        font_color: Some(appearance.theme().active_ui_text_color().into()),
        font_size: Some(20.),
        font_weight: Some(Weight::Semibold),
        ..Default::default()
    };

    let body_text_color = appearance
        .theme()
        .sub_text_color(appearance.theme().background())
        .into();

    let body_text_styles = UiComponentStyles {
        font_color: Some(body_text_color),
        ..Default::default()
    };

    let paragraph_1 = "All of Warp’s non-cloud features work offline.";
    let paragraph_2 = "However, we require users to be online when using Warp for the first time in order to enable Warp's AI and cloud features.";
    let paragraph_3 = "We offer cloud features to all users, and so we need an internet connection to meter AI usage, prevent abuse, and associate cloud objects with users. If you opt to use Warp logged-out, a unique ID will be attached to an anonymous user account in order to support these features.";

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(render_square_logo(appearance))
                    .with_margin_bottom(AUTH_MODAL_GAP)
                    .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(crate::i18n::tr_static(app, "Using Warp Offline"))
                        .with_style(header_styles)
                        .build()
                        .finish(),
                )
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph(crate::i18n::tr_static(app, paragraph_1))
                        .with_style(body_text_styles)
                        .build()
                        .finish(),
                )
                .with_margin_bottom(4.)
                .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph(crate::i18n::tr_static(app, paragraph_2))
                        .with_style(body_text_styles)
                        .build()
                        .finish(),
                )
                .with_margin_bottom(4.)
                .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph(crate::i18n::tr_static(app, paragraph_3))
                        .with_style(body_text_styles)
                        .build()
                        .finish(),
                )
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
            )
            .with_child(render_close_overlay_button(
                appearance,
                appearance.ui_builder(),
                crate::i18n::tr_static(app, "Dismiss").into(),
                mouse_state_handle,
                action,
            ))
            .finish(),
    )
    .finish()
}

pub fn render_close_overlay_button<A>(
    appearance: &Appearance,
    ui_builder: &UiBuilder,
    label: String,
    mouse_state_handle: MouseStateHandle,
    action: A,
) -> Box<dyn Element>
where
    A: Action + Clone,
{
    let (button_color, button_variant) = action_button_color_and_variant(appearance);
    let button_styles = UiComponentStyles {
        font_size: Some(14.),
        font_family_id: Some(appearance.ui_font_family()),
        font_weight: Some(Weight::Bold),
        background: Some(Fill::Solid(button_color)),
        border_width: Some(2.),
        border_color: Some(Fill::Solid(ColorU::transparent_black())),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
        padding: Some(Coords {
            top: 0.,
            bottom: 0.,
            left: 8.,
            right: 8.,
        }),
        height: Some(40.),
        ..Default::default()
    };

    let hover_button_style = UiComponentStyles {
        border_color: Some(Fill::Solid(lighten(button_color))),
        ..button_styles
    };

    let click_button_style = UiComponentStyles {
        background: Some(Fill::Solid(darken(button_color))),
        ..hover_button_style
    };

    ui_builder
        .button_with_custom_styles(
            button_variant,
            mouse_state_handle.clone(),
            button_styles,
            Some(hover_button_style),
            Some(click_button_style),
            None,
        )
        .with_centered_text_label(label)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
}

pub fn render_overlay(overlay_body: Box<dyn Element>, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(overlay_body)
        .with_background(appearance.theme().surface_1())
        .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
        .with_corner_radius(CornerRadius::with_all(MODAL_CORNER_RADIUS))
        .with_uniform_padding(32.)
        .finish()
}

// ---------------------------------------------------------------------------
// Privacy settings overlay (shared between AuthViewBody and LoginSlideView)
// ---------------------------------------------------------------------------

/// Handles needed to render the privacy settings overlay.
#[derive(Default)]
pub struct PrivacySettingsHandles {
    pub telemetry_switch: SwitchStateHandle,
    pub crash_reporting_switch: SwitchStateHandle,
    pub cloud_conversation_storage_switch: SwitchStateHandle,
    pub close_button_mouse: MouseStateHandle,
    pub telemetry_docs_mouse: MouseStateHandle,
}

/// Actions dispatched by the privacy settings overlay toggles.
pub struct PrivacySettingsActions<A: Action + Clone> {
    pub toggle_telemetry: A,
    pub toggle_crash_reporting: A,
    pub toggle_cloud_conversation_storage: A,
    pub hide_overlay: A,
}

/// Renders the full privacy settings overlay body (logo + header + toggles + done button).
/// This is the content that goes inside `render_overlay()`.
///
/// `is_ai_enabled` gates whether AI-dependent toggles (e.g. the cloud conversation
/// storage toggle) are shown. Callers should pass the effective AI-enabled state
/// for their context (the in-memory onboarding selection during the login slide,
/// or the stored setting elsewhere).
pub fn render_privacy_settings_overlay_body<A: Action + Clone + 'static>(
    appearance: &Appearance,
    app: &AppContext,
    handles: &PrivacySettingsHandles,
    actions: &PrivacySettingsActions<A>,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let ui_builder = appearance.ui_builder();

    let header_styles = UiComponentStyles {
        font_family_id: Some(appearance.header_font_family()),
        font_color: Some(appearance.theme().active_ui_text_color().into()),
        font_size: Some(20.),
        font_weight: Some(Weight::Semibold),
        ..Default::default()
    };

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(render_square_logo(appearance))
                    .with_margin_bottom(AUTH_MODAL_GAP)
                    .finish(),
            )
            .with_child(
                Container::new(
                    ui_builder
                        .span(crate::i18n::tr_static(app, "Privacy Settings"))
                        .with_style(header_styles)
                        .build()
                        .finish(),
                )
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
            )
            .with_child(render_privacy_settings_toggles(
                appearance,
                app,
                handles,
                actions,
                is_ai_enabled,
            ))
            .with_child(render_close_overlay_button(
                appearance,
                ui_builder,
                crate::i18n::tr_static(app, "Done").into(),
                handles.close_button_mouse.clone(),
                actions.hide_overlay.clone(),
            ))
            .finish(),
    )
    .with_background(appearance.theme().surface_1())
    .finish()
}

fn render_privacy_settings_section_header(
    text: impl Into<String>,
    appearance: &Appearance,
) -> Container {
    let section_header_styles = UiComponentStyles {
        font_family_id: Some(appearance.header_font_family()),
        font_color: Some(appearance.theme().active_ui_text_color().into()),
        font_size: Some(14.),
        font_weight: Some(Weight::Bold),
        ..Default::default()
    };

    Container::new(
        appearance
            .ui_builder()
            .span(text.into())
            .with_style(section_header_styles)
            .build()
            .finish(),
    )
}

/// Renders the stack of privacy toggles shown in the privacy settings overlay.
///
/// `is_ai_enabled` gates AI-dependent toggles (the cloud conversation storage
/// toggle is hidden entirely when AI is disabled, since it has no effect).
pub fn render_privacy_settings_toggles<A: Action + Clone + 'static>(
    appearance: &Appearance,
    app: &AppContext,
    handles: &PrivacySettingsHandles,
    actions: &PrivacySettingsActions<A>,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    fn render_description(appearance: &Appearance, text: String) -> Box<dyn Element> {
        let disclaimer_styles = UiComponentStyles {
            font_color: Some(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into(),
            ),
            ..Default::default()
        };

        appearance
            .ui_builder()
            .paragraph(text)
            .with_style(disclaimer_styles)
            .build()
            .finish()
    }

    let toggle_telemetry = actions.toggle_telemetry.clone();
    let telemetry_toggle = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_child(
            Shrinkable::new(
                1.,
                render_privacy_settings_section_header(
                    crate::i18n::tr_static(app, "Help improve Warp"),
                    appearance,
                )
                .finish(),
            )
            .finish(),
        )
        .with_child(
            appearance
                .ui_builder()
                .switch(handles.telemetry_switch.clone())
                .check(PrivacySettings::as_ref(app).is_telemetry_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(toggle_telemetry.clone());
                })
                .finish(),
        )
        .finish();

    let telemetry_description = render_description(
        appearance,
        crate::i18n::tr_static(
            app,
            "High-level feature usage data helps Warp's product team prioritize the roadmap.",
        )
        .into(),
    );

    let telemetry_link = Flex::row()
        .with_child(
            appearance
                .ui_builder()
                .link(
                    crate::i18n::tr_static(app, "Learn more").into(),
                    Some(PRIVACY_URL.into()),
                    None,
                    handles.telemetry_docs_mouse.clone(),
                )
                .soft_wrap(false)
                .build()
                .finish(),
        )
        .finish();

    let toggle_crash = actions.toggle_crash_reporting.clone();
    let crash_reporting_toggle = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_child(
            Shrinkable::new(
                1.,
                render_privacy_settings_section_header(
                    crate::i18n::tr_static(app, "Send crash reports"),
                    appearance,
                )
                .finish(),
            )
            .finish(),
        )
        .with_child(
            appearance
                .ui_builder()
                .switch(handles.crash_reporting_switch.clone())
                .check(PrivacySettings::as_ref(app).is_crash_reporting_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(toggle_crash.clone());
                })
                .finish(),
        )
        .finish();

    let crash_reporting_description = render_description(
        appearance,
        crate::i18n::tr_static(
            app,
            "Crash reporting helps Warp's engineering team understand stability and improve performance.",
        )
        .into(),
    );

    let toggle_cloud = actions.toggle_cloud_conversation_storage.clone();
    let cloud_conversation_storage_toggle = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_child(
            Shrinkable::new(
                1.,
                render_privacy_settings_section_header(
                    crate::i18n::tr_static(app, "Store AI conversations in the cloud"),
                    appearance,
                )
                .finish(),
            )
            .finish(),
        )
        .with_child(
            appearance
                .ui_builder()
                .switch(handles.cloud_conversation_storage_switch.clone())
                .check(PrivacySettings::as_ref(app).is_cloud_conversation_storage_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(toggle_cloud.clone());
                })
                .finish(),
        )
        .finish();

    let cloud_conversation_storage_description = render_description(
        appearance,
        if PrivacySettings::as_ref(app).is_cloud_conversation_storage_enabled {
            crate::i18n::tr_static(app, "Agent conversations can be shared with others and are retained when you log in on different devices. This data is only stored for product functionality, and Warp will not use it for analytics.")
        } else {
            crate::i18n::tr_static(app, "Agent conversations are only stored locally on your machine, are lost upon logout, and cannot be shared. Note: conversation data for ambient agents are still stored in the cloud.")
        }
        .into(),
    );

    let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Builds without a telemetry/crash reporting config (e.g. OpenWarp) cannot
    // ship the corresponding events, so the toggles would be no-ops. Hide each
    // one independently based on whether its backing config is present.
    if ChannelState::is_telemetry_available() && !FeatureFlag::GlobalAIAnalyticsBanner.is_enabled()
    {
        col.add_children(vec![
            Container::new(telemetry_toggle)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
            Container::new(telemetry_description)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
            Container::new(telemetry_link)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
        ]);
    }

    if ChannelState::is_crash_reporting_available() {
        col.add_children(vec![
            Container::new(crash_reporting_toggle)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
            Container::new(crash_reporting_description)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
        ]);
    }

    // Hide the cloud conversation storage toggle entirely when AI is disabled:
    // the setting has no effect without AI, and showing it is confusing.
    if FeatureFlag::CloudConversations.is_enabled() && is_ai_enabled {
        col.add_children(vec![
            Container::new(cloud_conversation_storage_toggle)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish(),
            Container::new(cloud_conversation_storage_description)
                .with_margin_bottom(20.)
                .finish(),
        ]);
    }

    col.finish()
}
