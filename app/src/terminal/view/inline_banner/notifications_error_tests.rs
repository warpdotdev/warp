use pathfinder_color::ColorU;

use super::*;

fn state() -> NotificationsErrorBannerState {
    NotificationsErrorBannerState {
        banner_id: 0,
        mouse_states: Default::default(),
    }
}

fn has_action(buttons: &[InlineBannerTextButton], action: NotificationsErrorBannerAction) -> bool {
    buttons.iter().any(|button| {
        matches!(
            &button.button_state.on_click_event,
            TerminalAction::NotificationsErrorBanner(a) if *a == action
        )
    })
}

#[test]
fn permissions_not_yet_granted_offers_set_permissions_but_not_open_system_settings() {
    let buttons = notifications_error_banner_buttons(
        &Some(NotificationSendError::PermissionsNotYetGranted),
        &state(),
        ColorU::white(),
    );

    assert!(has_action(
        &buttons,
        NotificationsErrorBannerAction::SetPermissions
    ));
    #[cfg(target_os = "macos")]
    assert!(!has_action(
        &buttons,
        NotificationsErrorBannerAction::OpenSystemSettings
    ));
    assert!(has_action(
        &buttons,
        NotificationsErrorBannerAction::Troubleshoot
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn permissions_denied_offers_open_system_settings_but_not_set_permissions() {
    let buttons = notifications_error_banner_buttons(
        &Some(NotificationSendError::PermissionsDenied),
        &state(),
        ColorU::white(),
    );

    assert!(has_action(
        &buttons,
        NotificationsErrorBannerAction::OpenSystemSettings
    ));
    assert!(!has_action(
        &buttons,
        NotificationsErrorBannerAction::SetPermissions
    ));
    assert!(has_action(
        &buttons,
        NotificationsErrorBannerAction::Troubleshoot
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn permissions_denied_offers_no_actionable_button_on_non_mac() {
    let buttons = notifications_error_banner_buttons(
        &Some(NotificationSendError::PermissionsDenied),
        &state(),
        ColorU::white(),
    );

    assert!(!has_action(
        &buttons,
        NotificationsErrorBannerAction::SetPermissions
    ));
    assert!(has_action(
        &buttons,
        NotificationsErrorBannerAction::Troubleshoot
    ));
}

#[test]
fn no_error_offers_only_troubleshoot() {
    let buttons = notifications_error_banner_buttons(&None, &state(), ColorU::white());

    assert!(!has_action(
        &buttons,
        NotificationsErrorBannerAction::SetPermissions
    ));
    #[cfg(target_os = "macos")]
    assert!(!has_action(
        &buttons,
        NotificationsErrorBannerAction::OpenSystemSettings
    ));
    assert!(has_action(
        &buttons,
        NotificationsErrorBannerAction::Troubleshoot
    ));
}
