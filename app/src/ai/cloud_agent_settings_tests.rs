use warpui::{App, SingletonEntity as _};

use super::*;
use crate::server::ids::ServerId;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};
fn team_scope(team_uid: i64) -> TeamContextForOperation {
    TeamContextForOperation::new_for_test(team_uid.into())
}

#[test]
fn persisted_auth_secret_scope_serializes_only_optional_team_uid() {
    let team = AuthSecretPreferenceScope::from_scope(&team_scope(7));
    let teamless = AuthSecretPreferenceScope::from_scope(&TeamlessScopeForTest);

    assert_eq!(
        serde_json::to_value(team).unwrap(),
        serde_json::json!({ "team_uid": ServerId::from(7).uid() })
    );
    assert_eq!(
        serde_json::to_value(teamless).unwrap(),
        serde_json::json!({ "team_uid": null })
    );
}
#[test]
fn scoped_auth_secret_preferences_are_isolated_by_team() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let team_a = team_scope(7);
        let team_b = team_scope(8);

        CloudAgentSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.persist_auth_secret_preference(
                &team_a,
                Harness::Claude,
                Some(AuthSecretPreference::Named("team-a".to_string())),
                ctx,
            );
            settings.persist_auth_secret_preference(
                &team_b,
                Harness::Claude,
                Some(AuthSecretPreference::Inherit),
                ctx,
            );
        });

        app.read(|ctx| {
            let settings = CloudAgentSettings::as_ref(ctx);
            assert_eq!(
                settings.auth_secret_preference(&team_a, Harness::Claude),
                Some(AuthSecretPreference::Named("team-a".to_string()))
            );
            assert_eq!(
                settings.auth_secret_preference(&team_b, Harness::Claude),
                Some(AuthSecretPreference::Inherit)
            );
            assert_eq!(
                settings.auth_secret_preference(&team_a, Harness::Codex),
                None
            );
        });
    });
}

#[test]
fn scoped_preferences_override_legacy_fallback() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let teamless = TeamlessScopeForTest;
        let team = team_scope(7);

        CloudAgentSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .last_selected_auth_secret
                .set_value(
                    HashMap::from([(
                        Harness::Claude.config_name().to_string(),
                        "legacy".to_string(),
                    )]),
                    ctx,
                )
                .unwrap();
        });
        app.read(|ctx| {
            assert_eq!(
                CloudAgentSettings::as_ref(ctx).auth_secret_preference(&teamless, Harness::Claude),
                Some(AuthSecretPreference::Named("legacy".to_string()))
            );
            assert_eq!(
                CloudAgentSettings::as_ref(ctx).auth_secret_preference(&team, Harness::Claude),
                Some(AuthSecretPreference::Named("legacy".to_string()))
            );
        });

        CloudAgentSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.persist_auth_secret_preference(
                &teamless,
                Harness::Claude,
                Some(AuthSecretPreference::Inherit),
                ctx,
            );
        });
        app.read(|ctx| {
            let settings = CloudAgentSettings::as_ref(ctx);
            assert_eq!(
                settings.auth_secret_preference(&teamless, Harness::Claude),
                Some(AuthSecretPreference::Inherit)
            );
            assert_eq!(
                settings.auth_secret_preference(&team, Harness::Claude),
                Some(AuthSecretPreference::Named("legacy".to_string()))
            );
            assert_eq!(
                settings
                    .last_selected_auth_secret
                    .value()
                    .get(Harness::Claude.config_name())
                    .map(String::as_str),
                Some("legacy")
            );
        });
    });
}
