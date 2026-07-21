use super::{LoginPurpose, LoginSlideSource};

#[test]
fn account_first_skip_does_not_require_confirmation() {
    assert!(!LoginSlideSource::AccountFirstOnboarding.skip_requires_confirmation());
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
            Some(
                "Use your work email if you have one. You may already have access to premium features through your organization",
            ),
        )
    );
}
