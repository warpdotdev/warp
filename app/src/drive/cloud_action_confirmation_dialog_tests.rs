use super::*;
const MEMBER_EMAIL: &str = "member@example.com";
const WORKSPACE_NAME: &str = "Acme";

#[test]
fn removal_confirmation_copy_matches_membership_scope() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveNativeWorkspaceTeamMember {
            member_email: MEMBER_EMAIL.to_string(),
            workspace_name: WORKSPACE_NAME.to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Remove from team?");
    assert_eq!(
        dialog.body_text(),
        "Are you sure you want to remove member@example.com from this team? member@example.com will still keep their Acme workspace membership and their other team memberships."
    );
    assert_eq!(dialog.confirm_button_text(), "Remove from team");
    assert!(!dialog.body_text().contains("credits"));

    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveWorkspaceMember {
            member_email: MEMBER_EMAIL.to_string(),
            workspace_name: WORKSPACE_NAME.to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Remove from workspace?");
    assert_eq!(
        dialog.body_text(),
        "Are you sure you want to remove member@example.com from the workspace? This will remove member@example.com from all teams in Acme and from the workspace itself."
    );
    assert_eq!(dialog.confirm_button_text(), "Remove from workspace");

    dialog.set_variant(CloudActionConfirmationDialogVariant::RemoveTeamMemberReloadCredits);

    assert_eq!(
        dialog.title_text(),
        "Are you sure you want to remove this member?"
    );
    assert_eq!(
        dialog.body_text(),
        REMOVE_TEAM_MEMBER_RELOAD_CREDITS_BODY_TEXT
    );
    assert_eq!(dialog.confirm_button_text(), "Remove Member");
}
#[test]
fn native_workspace_leave_preserves_other_access() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::LeaveNativeWorkspaceTeam {
            team_name: "Warp".to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Leave Warp?");
    assert_eq!(
        dialog.body_text(),
        "Your workspace access and other team memberships won’t change."
    );
    assert_eq!(dialog.confirm_button_text(), LEAVE_TEAM_CONFIRM_TEXT);
}
