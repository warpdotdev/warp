use super::*;

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
