use super::SettingsFooterKind;

// Hidden takes precedence over everything when the feature flag is off.

#[test]
fn feature_disabled_hides_footer_regardless_of_error_or_dismissal() {
    assert_eq!(
        SettingsFooterKind::choose(false, false, false),
        SettingsFooterKind::Hidden
    );
    assert_eq!(
        SettingsFooterKind::choose(false, true, false),
        SettingsFooterKind::Hidden
    );
    assert_eq!(
        SettingsFooterKind::choose(false, false, true),
        SettingsFooterKind::Hidden
    );
    assert_eq!(
        SettingsFooterKind::choose(false, true, true),
        SettingsFooterKind::Hidden
    );
}

// ErrorAlert only appears when BOTH an error is present AND the banner is
// dismissed. The workspace banner is still in charge otherwise.

#[test]
fn error_alert_shown_only_when_error_and_banner_dismissed() {
    assert_eq!(
        SettingsFooterKind::choose(true, true, true),
        SettingsFooterKind::ErrorAlert
    );
}

#[test]
fn error_present_but_banner_not_dismissed_shows_open_button() {
    // User is still seeing the workspace banner at the top of the workspace,
    // so the nav rail should just offer the plain button.
    assert_eq!(
        SettingsFooterKind::choose(true, true, false),
        SettingsFooterKind::OpenButton
    );
}

#[test]
fn no_error_but_banner_dismissed_shows_open_button() {
    // `banner_dismissed` is sticky across error/no-error transitions in the
    // workspace today — without an error, we still want the plain button.
    assert_eq!(
        SettingsFooterKind::choose(true, false, true),
        SettingsFooterKind::OpenButton
    );
}

#[test]
fn no_error_and_banner_not_dismissed_shows_open_button() {
    assert_eq!(
        SettingsFooterKind::choose(true, false, false),
        SettingsFooterKind::OpenButton
    );
}
