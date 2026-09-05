use pathfinder_color::ColorU;
use serde::Serialize;
use warpui::Element;
use warpui::elements::MouseStateHandle;
use warpui::notification::NotificationSendError;

use super::{
    InlineBannerButtonState, InlineBannerCloseButton, InlineBannerContent, InlineBannerStyle,
    InlineBannerTextButton, InlineBannerTextButtonVariant, render_inline_block_list_banner,
};
use crate::appearance::Appearance;
use crate::terminal::view::{InlineBannerId, TerminalAction};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum NotificationsErrorBannerAction {
    SetPermissions,
    /// Opens the Notifications pane of System Settings, deep-linked to Warp's own entry when
    /// possible. Only offered once the user has already denied the OS-level permissions request,
    /// since macOS won't show the request again and `SetPermissions` would be a no-op.
    #[cfg(target_os = "macos")]
    OpenSystemSettings,
    Troubleshoot,
    Close,
}

#[derive(Default)]
pub struct NotificationsErrorBannerMouseStates {
    pub troubleshoot: MouseStateHandle,
    pub close: MouseStateHandle,
    pub set_permissions: MouseStateHandle,
    #[cfg(target_os = "macos")]
    pub open_system_settings: MouseStateHandle,
}

/// State necessary to render the (singleton) notifications error banner.
pub struct NotificationsErrorBannerState {
    pub banner_id: InlineBannerId,
    pub mouse_states: NotificationsErrorBannerMouseStates,
}

/// Builds the (non-close) buttons offered by the banner for the given error state. Extracted
/// from [`render_inline_notifications_error_banner`] so tests can assert on exactly which
/// actions are offered without needing to introspect the rendered `Element` tree.
fn notifications_error_banner_buttons(
    error: &Option<NotificationSendError>,
    state: &NotificationsErrorBannerState,
    active_ui_text_color: ColorU,
) -> Vec<InlineBannerTextButton> {
    let mut buttons: Vec<InlineBannerTextButton> = vec![];

    // If permissions haven't been granted or denied, add a button to set the permissions.
    if matches!(error, Some(NotificationSendError::PermissionsNotYetGranted)) {
        buttons.push(InlineBannerTextButton {
            text: "Set permissions".to_string(),
            text_color: active_ui_text_color,
            button_state: InlineBannerButtonState {
                on_click_event: TerminalAction::NotificationsErrorBanner(
                    NotificationsErrorBannerAction::SetPermissions,
                ),
                mouse_state_handle: state.mouse_states.set_permissions.clone(),
            },
            font: Default::default(),
            position_id: None,
            variant: InlineBannerTextButtonVariant::Primary,
        });
    }

    // If the user has already denied permissions, re-requesting them is a no-op on macOS (the
    // system won't show the prompt again), so offer a direct path to System Settings instead.
    #[cfg(target_os = "macos")]
    if matches!(error, Some(NotificationSendError::PermissionsDenied)) {
        buttons.push(InlineBannerTextButton {
            text: "Open System Settings".to_string(),
            text_color: active_ui_text_color,
            button_state: InlineBannerButtonState {
                on_click_event: TerminalAction::NotificationsErrorBanner(
                    NotificationsErrorBannerAction::OpenSystemSettings,
                ),
                mouse_state_handle: state.mouse_states.open_system_settings.clone(),
            },
            font: Default::default(),
            position_id: None,
            variant: InlineBannerTextButtonVariant::Primary,
        });
    }

    buttons.push(InlineBannerTextButton {
        text: "Troubleshoot".to_string(),
        text_color: active_ui_text_color,
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::NotificationsErrorBanner(
                NotificationsErrorBannerAction::Troubleshoot,
            ),
            mouse_state_handle: state.mouse_states.troubleshoot.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Secondary,
    });

    buttons
}

pub fn render_inline_notifications_error_banner(
    title: &str,
    state: &NotificationsErrorBannerState,
    error: &Option<NotificationSendError>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();
    let buttons = notifications_error_banner_buttons(error, state, active_ui_text_color);

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::NotificationsErrorBanner(
            NotificationsErrorBannerAction::Close,
        ),
        mouse_state_handle: state.mouse_states.close.clone(),
    });

    render_inline_block_list_banner(
        InlineBannerStyle::LowPriority,
        appearance,
        InlineBannerContent {
            title: title.into(),
            buttons,
            close_button: Some(close_button),
            ..Default::default()
        },
    )
}

#[cfg(test)]
#[path = "notifications_error_tests.rs"]
mod tests;
