use pathfinder_color::ColorU;
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum NotificationsDiscoveryBannerAction {
    LearnMore,
    Troubleshoot,
    TurnOn(NotificationsTrigger),
    Configure,
    Close,
    /// Opens the Notifications pane of System Settings, deep-linked to Warp's own entry when
    /// possible. Only offered once the user has denied the OS-level permissions request, since
    /// macOS won't show the request again.
    #[cfg(target_os = "macos")]
    OpenSystemSettings,
}

#[derive(Default)]
pub struct NotificationsDiscoveryBannerMouseStates {
    pub learn_more: MouseStateHandle,
    pub troubleshoot: MouseStateHandle,
    pub turn_on: MouseStateHandle,
    pub configure: MouseStateHandle,
    pub close: MouseStateHandle,
    #[cfg(target_os = "macos")]
    pub open_system_settings: MouseStateHandle,
}

/// State necessary to render the (singleton) notifications discovery banner.
pub struct NotificationsDiscoveryBannerState {
    pub banner_id: InlineBannerId,
    pub mouse_states: NotificationsDiscoveryBannerMouseStates,
}

/// Builds the title and (non-close) buttons offered by the banner for the given mode/outcome.
/// Extracted from [`render_inline_notifications_discovery_banner`] so tests can assert on
/// exactly which actions are offered without needing to introspect the rendered `Element` tree.
fn notifications_discovery_banner_title_and_buttons(
    trigger: NotificationsTrigger,
    request_outcome: Option<RequestPermissionsOutcome>,
    state: &NotificationsDiscoveryBannerState,
    notifications_mode: NotificationsMode,
    active_ui_text_color: ColorU,
) -> (&'static str, Vec<InlineBannerTextButton>) {
    let learn_more_button = InlineBannerTextButton {
        text: "Learn more".to_string(),
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
        text: "Troubleshoot".to_string(),
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
            "We won't show this banner again, but you can always go to Settings to enable notifications.",
            vec![],
        ),
        NotificationsMode::Disabled => (
            "Notifications were turned off, but you can always go to Settings to enable notifications.",
            vec![],
        ),
        NotificationsMode::Unset => (
            trigger.discovery_banner_copy(),
            vec![
                learn_more_button,
                InlineBannerTextButton {
                    text: "Enable".to_string(),
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
            let (title, mut leading_buttons) = match request_outcome {
                Some(request_outcome) => match request_outcome {
                    RequestPermissionsOutcome::Accepted => (
                        "Success! You are now ready to receive desktop notifications.",
                        vec![learn_more_button],
                    ),
                    // One push below is macOS-only, so this can't be a single `vec![...]`
                    // literal on all platforms.
                    #[allow(clippy::vec_init_then_push)]
                    RequestPermissionsOutcome::PermissionsDenied => {
                        let mut buttons = vec![];
                        // Once macOS has denied the request, it won't show the OS prompt again,
                        // so offer a direct path to System Settings instead.
                        #[cfg(target_os = "macos")]
                        buttons.push(InlineBannerTextButton {
                            text: "Open System Settings".to_string(),
                            text_color: active_ui_text_color,
                            button_state: InlineBannerButtonState {
                                on_click_event: TerminalAction::NotificationsDiscoveryBanner(
                                    NotificationsDiscoveryBannerAction::OpenSystemSettings,
                                ),
                                mouse_state_handle: state.mouse_states.open_system_settings.clone(),
                            },
                            font: Default::default(),
                            position_id: None,
                            variant: InlineBannerTextButtonVariant::Primary,
                        });
                        buttons.push(troubleshoot_button);
                        (
                            "Warp was denied permissions to send you notifications.",
                            buttons,
                        )
                    }
                    RequestPermissionsOutcome::OtherError { .. } => (
                        "Something went wrong while requesting permissions.",
                        vec![troubleshoot_button],
                    ),
                },
                None => (
                    "Don't forget to 'Allow' the permissions request to finish setting up notifications.",
                    vec![learn_more_button],
                ),
            };

            leading_buttons.push(InlineBannerTextButton {
                text: "Configure notifications".to_string(),
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
            });

            (title, leading_buttons)
        }
    };

    (title, buttons)
}

pub fn render_inline_notifications_discovery_banner(
    trigger: NotificationsTrigger,
    request_outcome: Option<RequestPermissionsOutcome>,
    state: &NotificationsDiscoveryBannerState,
    notifications_mode: NotificationsMode,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();
    let (title, buttons) = notifications_discovery_banner_title_and_buttons(
        trigger,
        request_outcome,
        state,
        notifications_mode,
        active_ui_text_color,
    );

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

#[cfg(test)]
#[path = "notifications_discovery_tests.rs"]
mod tests;
