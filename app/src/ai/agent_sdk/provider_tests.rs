use std::sync::Arc;

use warp_cli::provider::ProviderType;
use warp_cli::scope::{ObjectScope, TeamSelection};
use warpui::App;

use super::ProviderCommandRunner;
use crate::auth::AuthStateProvider;
use crate::network::NetworkStatus;
use crate::server::ids::ServerId;
use crate::server::server_api::team::MockTeamClient;
use crate::settings::PrivacySettings;
use crate::workspaces::team::Team;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::{
    UserWorkspaces, WorkspacesMetadataResponse, WorkspacesMetadataWithPricing,
};
use crate::workspaces::workspace::{Workspace, WorkspaceUid};

#[test]
fn setup_resolves_team_scope_from_refreshed_workspace_metadata() {
    App::test((), |mut app| async move {
        let workspace_uid = WorkspaceUid::from(ServerId::from(1));
        let team_uid = ServerId::from(2);
        let workspace = Workspace::from_local_cache(
            workspace_uid,
            "Test Workspace".to_string(),
            Some(vec![Team::from_local_cache(
                team_uid,
                "Test Team".to_string(),
                None,
                None,
                None,
                None,
            )]),
            None,
        );
        let mut team_client = MockTeamClient::new();
        team_client
            .expect_workspaces_metadata()
            .times(1)
            .return_once(|| {
                Ok(WorkspacesMetadataWithPricing {
                    metadata: WorkspacesMetadataResponse {
                        workspaces: vec![workspace],
                        joinable_teams: vec![],
                        experiments: None,
                        ai_credit_availability: None,
                        user_purchase_policy: None,
                    },
                    pricing_info: None,
                })
            });

        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(TeamTesterStatus::new);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(PrivacySettings::mock);
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(|ctx| TeamUpdateManager::new(Arc::new(team_client), None, ctx));

        let (opened_url_sender, opened_url_receiver) = async_channel::bounded(1);
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _| {
                opened_url_sender.try_send(url.to_string()).unwrap();
                url.to_string()
            });
        });

        let runner = app.add_model(|_| ProviderCommandRunner);
        runner.update(&mut app, |runner, ctx| {
            runner.setup(
                ProviderType::Slack,
                ObjectScope {
                    team_selection: TeamSelection { team: Some(None) },
                    personal: false,
                },
                ctx,
            );
        });

        assert!(matches!(
            opened_url_receiver.try_recv(),
            Err(async_channel::TryRecvError::Empty)
        ));
        let opened_url = opened_url_receiver.recv().await.unwrap();
        assert!(opened_url.ends_with(&format!(
            "/oauth/connect/slack?principalType=team&principalId={team_uid}"
        )));
        assert!(app.termination_result().is_none());
    });
}
