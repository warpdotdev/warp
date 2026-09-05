use pathfinder_color::ColorU;

use super::*;

fn state() -> NotificationsDiscoveryBannerState {
    NotificationsDiscoveryBannerState {
        banner_id: 0,
        mouse_states: Default::default(),
    }
}

fn has_action(
    buttons: &[InlineBannerTextButton],
    action: NotificationsDiscoveryBannerAction,
) -> bool {
    buttons.iter().any(|button| {
        matches!(
            &button.button_state.on_click_event,
            TerminalAction::NotificationsDiscoveryBanner(a) if *a == action
        )
    })
}

#[cfg(target_os = "macos")]
#[test]
fn enabled_permissions_denied_offers_open_system_settings() {
    let (_, buttons) = notifications_discovery_banner_title_and_buttons(
        NotificationsTrigger::NeedsAttention,
        Some(RequestPermissionsOutcome::PermissionsDenied),
        &state(),
        NotificationsMode::Enabled,
        ColorU::white(),
    );

    assert!(has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::OpenSystemSettings
    ));
    assert!(has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::Troubleshoot
    ));
    assert!(has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::Configure
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn enabled_permissions_denied_offers_no_system_settings_cta_on_non_mac() {
    let (_, buttons) = notifications_discovery_banner_title_and_buttons(
        NotificationsTrigger::NeedsAttention,
        Some(RequestPermissionsOutcome::PermissionsDenied),
        &state(),
        NotificationsMode::Enabled,
        ColorU::white(),
    );

    assert!(has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::Troubleshoot
    ));
    assert!(has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::Configure
    ));
}

#[test]
fn enabled_accepted_does_not_offer_open_system_settings() {
    let (_, buttons) = notifications_discovery_banner_title_and_buttons(
        NotificationsTrigger::NeedsAttention,
        Some(RequestPermissionsOutcome::Accepted),
        &state(),
        NotificationsMode::Enabled,
        ColorU::white(),
    );

    #[cfg(target_os = "macos")]
    assert!(!has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::OpenSystemSettings
    ));
    assert!(has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::Configure
    ));
}

#[test]
fn unset_offers_turn_on() {
    let (_, buttons) = notifications_discovery_banner_title_and_buttons(
        NotificationsTrigger::NeedsAttention,
        None,
        &state(),
        NotificationsMode::Unset,
        ColorU::white(),
    );

    assert!(has_action(
        &buttons,
        NotificationsDiscoveryBannerAction::TurnOn(NotificationsTrigger::NeedsAttention)
    ));
}
