use super::{LoginPurpose, LoginSlideSource};

#[test]
fn all_login_slide_sources_require_skip_confirmation() {
    assert!(LoginSlideSource::AccountFirstOnboarding.skip_requires_confirmation());
    assert!(LoginSlideSource::OnboardingFlow.skip_requires_confirmation());
    assert!(LoginSlideSource::LoginExistingUserFromWelcome.skip_requires_confirmation());
    assert!(
        LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme.skip_requires_confirmation()
    );
}

#[test]
fn account_first_copy_matches_product_spec() {
    assert_eq!(
        LoginPurpose::AccountFirst.copy(),
        (
            "Create an account",
            "Access AI, run cloud agents, collaborate with teammates, and sync settings across devices",
        )
    );
    assert_eq!(
        LoginPurpose::AccountFirst.work_email_callout_copy(),
        Some((
            "Use a work email to find teammates",
            "Signing in with a work email helps us find your teammates and may unlock special offers.",
        ))
    );
}
