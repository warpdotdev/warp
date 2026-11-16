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

#[derive(Clone, Copy, Debug, Serialize)]
pub enum NotificationsErrorBannerAction {
    SetPermissions,
    Troubleshoot,
    Close,
}

#[derive(Default)]
pub struct NotificationsErrorBannerMouseStates {
    pub troubleshoot: MouseStateHandle,
    pub close: MouseStateHandle,
    pub set_permissions: MouseStateHandle,
}

/// State necessary to render the (singleton) notifications error banner.
pub struct NotificationsErrorBannerState {
    pub banner_id: InlineBannerId,
    pub mouse_states: NotificationsErrorBannerMouseStates,
}

pub fn render_inline_notifications_error_banner(
    title: &str,
    state: &NotificationsErrorBannerState,
    error: &Option<NotificationSendError>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let active_ui_text_color = appearance.theme().active_ui_text_color().into_solid();

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
