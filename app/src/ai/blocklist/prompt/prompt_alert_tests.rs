use std::sync::Arc;

use markdown_parser::{FormattedTextFragment, Hyperlink};
use warpui::App;

use super::*;
use crate::auth::user::{TEST_USER_EMAIL, TEST_USER_UID};
use crate::auth::{AuthStateProvider, UserUid};
use crate::server::ids::ServerId;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings_view::SettingsSection;
use crate::workspace::WorkspaceAction;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    BillingMetadata, ByoApiKeyPolicy, CustomerType, PurchaseAddOnCreditsPolicy, Workspace,
    WorkspaceUid,
};

fn test_workspace(
    customer_type: CustomerType,
    role: MembershipRole,
    configure_billing_metadata: impl FnOnce(&mut BillingMetadata),
) -> Workspace {
    let workspace_uid = WorkspaceUid::from(ServerId::from(1));
    let team_uid = ServerId::from(1);
    let mut billing_metadata = BillingMetadata {
        customer_type,
        ..Default::default()
    };
    configure_billing_metadata(&mut billing_metadata);

    let member = TeamMember {
        uid: UserUid::new(TEST_USER_UID),
        email: TEST_USER_EMAIL.to_owned(),
        role,
    };
    let team = Team::from_local_cache(
        team_uid,
        "Test Team".to_owned(),
        None,
        Some(billing_metadata),
        Some(vec![member]),
    );

    Workspace::from_local_cache(workspace_uid, "Test Workspace".to_owned(), Some(vec![team]))
}

fn initialize_app(app: &mut App, workspace: Workspace) {
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![workspace],
            ctx,
        )
    });
}

fn request_limit_fragments(app: &App) -> Vec<FormattedTextFragment> {
    alert_fragments(app, &PromptAlertState::RequestLimitReached)
}

fn alert_fragments(app: &App, state: &PromptAlertState) -> Vec<FormattedTextFragment> {
    app.read(|ctx| {
        PromptAlertView {
            state: state.clone(),
            action_hyperlink: Default::default(),
        }
        .alert_text_fragments(state, ctx)
    })
}

fn fragment_text(fragments: &[FormattedTextFragment]) -> String {
    fragments
        .iter()
        .map(|fragment| fragment.text.as_str())
        .collect()
}

fn workspace_action_for_label<'a>(
    fragments: &'a [FormattedTextFragment],
    label: &str,
) -> Option<&'a WorkspaceAction> {
    fragments.iter().find_map(|fragment| {
        if fragment.text != label {
            return None;
        }
        match &fragment.styles.hyperlink {
            Some(Hyperlink::Action(action)) => action.as_any().downcast_ref::<WorkspaceAction>(),
            Some(Hyperlink::Url(_)) | None => None,
        }
    })
}

#[test]
fn enterprise_admin_out_of_credits_contacts_account_executive() {
    App::test((), |mut app| async move {
        let workspace = test_workspace(CustomerType::Enterprise, MembershipRole::Owner, |_| {});
        initialize_app(&mut app, workspace);

        let fragments = request_limit_fragments(&app);

        assert_eq!(
            fragment_text(&fragments),
            "  Out of credits. Contact account executive"
        );
    });
}

#[test]
fn enterprise_non_admin_out_of_credits_links_to_team_settings() {
    App::test((), |mut app| async move {
        let workspace = test_workspace(CustomerType::Enterprise, MembershipRole::User, |_| {});
        initialize_app(&mut app, workspace);

        let fragments = request_limit_fragments(&app);
        let action = workspace_action_for_label(&fragments, CONTACT_TEAM_ADMIN_TEXT);

        assert_eq!(
            fragment_text(&fragments),
            "  Out of credits. Contact team admin"
        );
        assert!(matches!(
            action,
            Some(WorkspaceAction::ShowSettingsPage(SettingsSection::Teams))
        ));
    });
}

#[test]
fn non_enterprise_non_admin_with_add_on_credits_can_add_credits() {
    App::test((), |mut app| async move {
        let workspace = test_workspace(
            CustomerType::Build,
            MembershipRole::User,
            |billing_metadata| {
                billing_metadata.tier.purchase_add_on_credits_policy =
                    Some(PurchaseAddOnCreditsPolicy { enabled: true });
            },
        );
        initialize_app(&mut app, workspace);

        let fragments = request_limit_fragments(&app);
        let action = workspace_action_for_label(&fragments, "Add credits");

        assert_eq!(fragment_text(&fragments), "  Out of credits  Add credits");
        assert!(matches!(
            action,
            Some(WorkspaceAction::ShowSettingsPage(
                SettingsSection::BillingAndUsage
            ))
        ));
    });
}

#[test]
fn non_enterprise_out_of_credits_keeps_byo_api_key_fallback() {
    App::test((), |mut app| async move {
        let workspace = test_workspace(
            CustomerType::Build,
            MembershipRole::Owner,
            |billing_metadata| {
                billing_metadata.tier.byo_api_key_policy = Some(ByoApiKeyPolicy { enabled: true });
            },
        );
        initialize_app(&mut app, workspace);

        let fragments = request_limit_fragments(&app);
        let action = workspace_action_for_label(&fragments, "use your own API keys");

        assert_eq!(
            fragment_text(&fragments),
            "  Out of credits  Contact support or use your own API keys"
        );
        assert!(matches!(
            action,
            Some(WorkspaceAction::ShowSettingsPageWithSearch {
                section: Some(SettingsSection::WarpAgent),
                ..
            })
        ));
    });
}
