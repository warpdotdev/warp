use serde::Serialize;
use warpui::Element;
use warpui::elements::MouseStateHandle;
use warpui::notification::RequestPermissionsOutcome;

use super::{
    InlineBannerButtonState, InlineBannerCloseButton, InlineBannerContent, InlineBannerStyle,
    InlineBannerTextButton, InlineBannerTextButtonVariant, render_inline_block_list_banner,
};
use crate::appearance::Appearance;
use crate::terminal::session_settings::NotificationsMode;
use crate::terminal::view::{InlineBannerId, NotificationsTrigger, TerminalAction};

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
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();

    let learn_more_button = InlineBannerTextButton {
        text: t!("workspace.learn_more").to_string(),
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
        text: t!("help_ui.troubleshoot").to_string(),
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
            t!("help_ui.we_won_t_show_this_banner_again_but_you"),
            vec![],
        ),
        NotificationsMode::Disabled => (
            t!("help_ui.notifications_were_turned_off_but_you_can_always_go"),
            vec![],
        ),
        NotificationsMode::Unset => (
            trigger.discovery_banner_copy(),
            vec![
                learn_more_button,
                InlineBannerTextButton {
                    text: t!("common.enable").to_string(),
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
                        t!("help_ui.success_you_are_now_ready_to_receive_desktop_notifications"),
                        learn_more_button,
                    ),
                    RequestPermissionsOutcome::PermissionsDenied => (
                        t!("help_ui.warp_was_denied_permissions_to_send_you_notifications"),
                        troubleshoot_button,
                    ),
                    RequestPermissionsOutcome::OtherError { .. } => (
                        t!("help_ui.something_went_wrong_while_requesting_permissions"),
                        troubleshoot_button,
                    ),
                },
                None => (
                    t!("help_ui.don_t_forget_to_allow_the_permissions_request_to"),
                    learn_more_button,
                ),
            };

            (
                title,
                vec![
                    docs_button,
                    InlineBannerTextButton {
                        text: t!("help_ui.configure_notifications").to_string(),
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
            title: title.to_owned(),
            buttons,
            close_button: Some(close_button),
            ..Default::default()
        },
    )
}
