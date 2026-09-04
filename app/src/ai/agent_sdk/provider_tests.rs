use std::sync::{Arc, Mutex};

use warp_cli::provider::ProviderType;
use warp_cli::scope::{ObjectScope, TeamSelection};
use warpui::App;
use warpui::r#async::Timer;

use super::ProviderCommandRunner;
use crate::server::ids::ServerId;
use crate::settings::PrivacySettings;
use crate::workspaces::user_workspaces::UserWorkspaces;

#[test]
fn setup_resolves_team_scope_after_workspace_metadata_refresh() {
    App::test((), |mut app| async move {
        app.add_singleton_model(PrivacySettings::mock);
        let user_workspaces = app.add_singleton_model(UserWorkspaces::default_mock);
        let opened_urls = Arc::new(Mutex::new(Vec::new()));
        let captured_urls = opened_urls.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _| {
                captured_urls.lock().unwrap().push(url.to_string());
                url.to_string()
            });
        });

        let (refresh_sender, refresh_receiver) = async_channel::bounded(1);
        let runner = app.add_model(|_| ProviderCommandRunner);
        runner.update(&mut app, |runner, ctx| {
            runner.setup_after_workspace_metadata_refresh(
                async move {
                    refresh_receiver.recv().await.unwrap();
                    Ok(())
                },
                ProviderType::Slack,
                ObjectScope {
                    team_selection: TeamSelection { team: Some(None) },
                    personal: false,
                },
                ctx,
            );
        });

        for _ in 0..3 {
            futures_lite::future::yield_now().await;
        }
        assert!(opened_urls.lock().unwrap().is_empty());

        user_workspaces.update(&mut app, |workspaces, ctx| {
            workspaces.setup_test_workspace(ctx);
        });
        refresh_sender.send(()).await.unwrap();

        for _ in 0..20 {
            if !opened_urls.lock().unwrap().is_empty() {
                break;
            }
            Timer::after(std::time::Duration::from_millis(10)).await;
        }

        let opened_urls = opened_urls.lock().unwrap();
        assert_eq!(opened_urls.len(), 1);
        let expected_url_suffix = format!(
            "/oauth/connect/slack?principalType=team&principalId={}",
            ServerId::from(2)
        );
        assert!(
            opened_urls[0].ends_with(&expected_url_suffix),
            "opened URL: {}",
            opened_urls[0]
        );
        drop(opened_urls);
        assert!(app.termination_result().is_none());
    });
}
