use serde::Serialize;
use warpui::{
    elements::MouseStateHandle, notification::RequestPermissionsOutcome, AppContext, Element,
};

use crate::{
    appearance::Appearance,
    localization,
    terminal::{
        session_settings::NotificationsMode,
        view::{InlineBannerId, NotificationsTrigger, TerminalAction},
    },
};

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerCloseButton,
    InlineBannerContent, InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonVariant,
};

#[derive(Clone, Copy, Debug, Serialize)]
pub enum NotificationsDiscoveryBannerAction {
    LearnMore,
    Troubleshoot,
    TurnOn(NotificationsTrigger),
    Configure,
    Close,
}

#[derive(Default)]
pub struct NotificationsDiscoveryBannerMouseStates {
    pub learn_more: MouseStateHandle,
    pub troubleshoot: MouseStateHandle,
    pub turn_on: MouseStateHandle,
    pub configure: MouseStateHandle,
    pub close: MouseStateHandle,
}

/// State necessary to render the (singleton) notifications discovery banner.
pub struct NotificationsDiscoveryBannerState {
    pub banner_id: InlineBannerId,
    pub mouse_states: NotificationsDiscoveryBannerMouseStates,
}

pub fn render_inline_notifications_discovery_banner(
    trigger: NotificationsTrigger,
    request_outcome: Option<RequestPermissionsOutcome>,
    state: &NotificationsDiscoveryBannerState,
    notifications_mode: NotificationsMode,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();

    let learn_more_button = InlineBannerTextButton {
        text: localization::text_for_app(app, "auth.learn_more"),
        text_color: active_ui_text_color,
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::NotificationsDiscoveryBanner(
                NotificationsDiscoveryBannerAction::LearnMore,
            ),
            mouse_state_handle: state.mouse_states.learn_more.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Secondary,
    };
    let troubleshoot_button = InlineBannerTextButton {
        text: localization::text_for_app(app, "terminal.inline_banner.troubleshoot"),
        text_color: active_ui_text_color,
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::NotificationsDiscoveryBanner(
                NotificationsDiscoveryBannerAction::Troubleshoot,
            ),
            mouse_state_handle: state.mouse_states.troubleshoot.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Secondary,
    };

    let (title, buttons) = match notifications_mode {
        NotificationsMode::Dismissed => (
            localization::text_for_app(
                app,
                "terminal.inline_banner.notifications_discovery.dismissed",
            ),
            vec![],
        ),
        NotificationsMode::Disabled => (
            localization::text_for_app(
                app,
                "terminal.inline_banner.notifications_discovery.disabled",
            ),
            vec![],
        ),
        NotificationsMode::Unset => (
            localization::text_for_app(app, trigger.discovery_banner_copy_key()),
            vec![
                learn_more_button,
                InlineBannerTextButton {
                    text: localization::text_for_app(
                        app,
                        "terminal.inline_banner.notifications_discovery.enable",
                    ),
                    text_color: active_ui_text_color,
                    button_state: InlineBannerButtonState {
                        on_click_event: TerminalAction::NotificationsDiscoveryBanner(
                            NotificationsDiscoveryBannerAction::TurnOn(trigger),
                        ),
                        mouse_state_handle: state.mouse_states.turn_on.clone(),
                    },
                    font: Default::default(),
                    position_id: None,
                    variant: InlineBannerTextButtonVariant::Primary,
                },
            ],
        ),
        NotificationsMode::Enabled => {
            // Determine the messaging based on what the user's response was to the
            // permissions request (if any)
            let (title, docs_button) = match request_outcome {
                Some(request_outcome) => match request_outcome {
                    RequestPermissionsOutcome::Accepted => (
                        localization::text_for_app(
                            app,
                            "terminal.inline_banner.notifications_discovery.success",
                        ),
                        learn_more_button,
                    ),
                    RequestPermissionsOutcome::PermissionsDenied => (
                        localization::text_for_app(
                            app,
                            "terminal.inline_banner.notifications_discovery.permissions_denied",
                        ),
                        troubleshoot_button,
                    ),
                    RequestPermissionsOutcome::OtherError { .. } => (
                        localization::text_for_app(
                            app,
                            "terminal.inline_banner.notifications_discovery.permissions_error",
                        ),
                        troubleshoot_button,
                    ),
                },
                None => (
                    localization::text_for_app(
                        app,
                        "terminal.inline_banner.notifications_discovery.allow_prompt",
                    ),
                    learn_more_button,
                ),
            };

            (
                title,
                vec![
                    docs_button,
                    InlineBannerTextButton {
                        text: localization::text_for_app(
                            app,
                            "terminal.inline_banner.notifications_discovery.configure",
                        ),
                        text_color: active_ui_text_color,
                        button_state: InlineBannerButtonState {
                            on_click_event: TerminalAction::NotificationsDiscoveryBanner(
                                NotificationsDiscoveryBannerAction::Configure,
                            ),
                            mouse_state_handle: state.mouse_states.configure.clone(),
                        },
                        font: Default::default(),
                        position_id: None,
                        variant: InlineBannerTextButtonVariant::Secondary,
                    },
                ],
            )
        }
    };

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::NotificationsDiscoveryBanner(
            NotificationsDiscoveryBannerAction::Close,
        ),
        mouse_state_handle: state.mouse_states.close.clone(),
    });

    render_inline_block_list_banner(
        InlineBannerStyle::CallToAction,
        appearance,
        InlineBannerContent {
            title,
            buttons,
            close_button: Some(close_button),
            ..Default::default()
        },
    )
}
