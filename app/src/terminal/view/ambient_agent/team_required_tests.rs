use super::{BODY, PrimaryCta, TITLE, should_render};

#[test]
fn blocker_copy_matches_approved_content() {
    assert_eq!(TITLE, "Cloud agents need a team");
    assert_eq!(
        BODY,
        "You’re in this workspace but not on a team, so you can’t start cloud runs. Join or create a team, then try again."
    );
}

#[test]
fn blocker_renders_over_setup_and_composition_only_when_team_is_required() {
    assert!(should_render(true, true, false));
    assert!(should_render(true, false, true));
    assert!(!should_render(false, true, false));
    assert!(!should_render(false, false, true));
    assert!(!should_render(true, false, false));
}

#[test]
fn member_primary_cta_opens_team_settings() {
    let cta = PrimaryCta::for_workspace_admin(false);

    assert_eq!(cta, PrimaryCta::OpenTeamsSettings);
    assert_eq!(cta.label(), "Open Teams settings");
}

#[test]
fn workspace_admin_primary_cta_opens_admin_panel() {
    let cta = PrimaryCta::for_workspace_admin(true);

    assert_eq!(cta, PrimaryCta::OpenAdminPanel);
    assert_eq!(cta.label(), "Open admin panel");
}
