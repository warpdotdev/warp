use settings::Setting as _;
use warpui::App;

use super::*;
use crate::ai::cloud_environments::{AmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel};
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions, Owner};
use crate::server::server_api::ai::{
    FactorySelectorOptionsResponse, FactorySelectorPageInfo, MockAIClient,
};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::TeamContextForOperation;
fn environment(id: SyncId, name: &str, owner: Owner) -> CloudAmbientAgentEnvironment {
    let mut permissions = CloudObjectPermissions::mock_personal();
    permissions.owner = owner;
    CloudAmbientAgentEnvironment::new(
        id,
        CloudAmbientAgentEnvironmentModel::new(AmbientAgentEnvironment::new(
            name.to_owned(),
            None,
            Vec::new(),
            "ubuntu:latest".to_owned(),
            Vec::new(),
        )),
        CloudObjectMetadata::mock(),
        permissions,
    )
}

#[test]
fn managed_backing_is_hidden_but_unmanaged_shared_default_remains_visible() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let team = ServerId::from(7);
        let other_team = ServerId::from(8);
        let managed = SyncId::ServerId(ServerId::from(12));
        let shared_default = SyncId::ServerId(ServerId::from(13));
        let foreign = SyncId::ServerId(ServerId::from(14));
        let cloud_model = app.add_singleton_model(CloudModel::mock);
        app.update(|ctx| {
            cloud_model.update(ctx, |model, ctx| {
                for (id, owner) in [
                    (managed, Owner::Team { team_uid: team }),
                    (shared_default, Owner::Team { team_uid: team }),
                    (
                        foreign,
                        Owner::Team {
                            team_uid: other_team,
                        },
                    ),
                ] {
                    model.create_object(id, environment(id, &id.to_string(), owner), ctx);
                }
            });
        });
        app.add_singleton_model(CloudEnvironmentCatalog::new);
        let selector = app.add_singleton_model(|_| FactorySelectorCatalog {
            team_uid: Some(team),
            initialized: true,
            generation: 1,
            state: FactorySelectorState::Ready(FactorySelectorSnapshot {
                factories: vec![FactorySelectorRow {
                    choice: CloudSelectorChoice::Factory {
                        uid: "factory".to_owned(),
                        environment_uid: shared_default,
                        foreman_agent_uid: "foreman".to_owned(),
                    },
                    name: "Build".to_owned(),
                    alias: None,
                }],
                managed_environment_ids: HashSet::from([managed]),
            }),
        });
        let team_scope = TeamContextForOperation::new_for_test(team);
        let other_scope = TeamContextForOperation::new_for_test(other_team);
        selector.read(&app, |catalog, ctx| {
            assert_eq!(
                catalog
                    .visible_environments(&team_scope, ctx)
                    .into_iter()
                    .map(|environment| environment.id)
                    .collect::<Vec<_>>(),
                vec![shared_default]
            );
            assert!(catalog.state_for(&other_scope).is_none());
            assert!(catalog.visible_environments(&other_scope, ctx).is_empty());
            assert_eq!(
                catalog
                    .preferred_choice(&team_scope, ctx)
                    .expect("unmanaged fallback"),
                CloudSelectorChoice::Environment(shared_default)
            );
        });
        selector.update(&mut app, |catalog, _| {
            catalog.state = FactorySelectorState::Loading;
        });
        selector.read(&app, |catalog, ctx| {
            assert!(catalog.visible_environments(&team_scope, ctx).is_empty());
            assert!(catalog.preferred_choice(&team_scope, ctx).is_none());
        });
        selector.update(&mut app, |catalog, _| {
            catalog.state = FactorySelectorState::Failed;
        });
        selector.read(&app, |catalog, ctx| {
            assert!(catalog.visible_environments(&team_scope, ctx).is_empty());
        });
    });
}

#[test]
fn tagged_preferences_preserve_team_scope_and_never_fall_back_to_factory_backing() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let team = ServerId::from(7);
        let other_team = ServerId::from(8);
        let managed = SyncId::ServerId(ServerId::from(12));
        let ordinary = SyncId::ServerId(ServerId::from(13));
        let cloud_model = app.add_singleton_model(CloudModel::mock);
        app.update(|ctx| {
            cloud_model.update(ctx, |model, ctx| {
                for id in [managed, ordinary] {
                    model.create_object(
                        id,
                        environment(id, &id.to_string(), Owner::Team { team_uid: team }),
                        ctx,
                    );
                }
            });
        });
        app.add_singleton_model(CloudEnvironmentCatalog::new);
        let selector = app.add_singleton_model(|_| FactorySelectorCatalog {
            team_uid: Some(team),
            initialized: true,
            generation: 1,
            state: FactorySelectorState::Ready(FactorySelectorSnapshot {
                factories: vec![],
                managed_environment_ids: HashSet::from([managed]),
            }),
        });
        let scope = TeamContextForOperation::new_for_test(team);
        let foreign_scope = TeamContextForOperation::new_for_test(other_team);
        CloudAgentSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .last_selected_environment_id
                .set_value(Some(ordinary), ctx)
                .expect("legacy selection");
        });
        selector.read(&app, |catalog, ctx| {
            assert_eq!(
                catalog.preferred_choice(&scope, ctx),
                Some(CloudSelectorChoice::Environment(ordinary))
            );
        });
        CloudAgentSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.persist_cloud_selector_preference(
                &scope,
                CloudSelectorPreference::Factory("deleted-factory".to_owned()),
                ctx,
            );
        });
        selector.read(&app, |catalog, ctx| {
            assert_eq!(
                catalog.preferred_choice(&scope, ctx),
                Some(CloudSelectorChoice::Environment(ordinary))
            );
            assert!(catalog.state_for(&foreign_scope).is_none());
            assert_eq!(
                CloudAgentSettings::as_ref(ctx).cloud_selector_preference(&foreign_scope),
                None
            );
            assert_eq!(
                CloudAgentSettings::as_ref(ctx).cloud_selector_preference(&scope),
                Some(CloudSelectorPreference::Factory(
                    "deleted-factory".to_owned()
                ))
            );
        });
    });
}

#[test]
fn tagged_choices_do_not_conflate_factory_and_environment_ids() {
    let id = SyncId::ServerId(ServerId::from(12));
    assert_ne!(
        CloudSelectorChoice::Environment(id),
        CloudSelectorChoice::Factory {
            uid: "environment-uid".into(),
            environment_uid: id,
            foreman_agent_uid: "foreman".into(),
        }
    );
}

#[test]
fn pagination_publishes_factories_and_managed_uids_only_after_a_complete_snapshot() {
    let team_uid = ServerId::from(7);
    let environment_uid = ServerId::from(12);
    let mut mock = MockAIClient::new();
    mock.expect_get_factory_selector_options()
        .times(2)
        .returning(move |scope, cursor| {
            assert_eq!(scope.team_uid(), Some(team_uid));
            let first_page = cursor.is_none();
            if !first_page {
                assert_eq!(cursor.as_deref(), Some("next-page"));
            }
            Ok(FactorySelectorOptionsResponse {
                factories: if first_page {
                    vec![]
                } else {
                    vec![FactorySelectorOption {
                        uid: "factory-12".to_owned(),
                        team_uid: team_uid.uid(),
                        name: "Build".to_owned(),
                        alias: None,
                        default_environment_uid: environment_uid.uid(),
                        foreman_agent_uid: "foreman-12".to_owned(),
                    }]
                },
                managed_environment_uids: vec![environment_uid.uid()],
                page_info: FactorySelectorPageInfo {
                    has_next_page: first_page,
                    next_cursor: first_page.then(|| "next-page".to_owned()),
                },
            })
        });
    let scope = RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(team_uid));
    let snapshot = futures::executor::block_on(fetch_snapshot(Arc::new(mock), scope, team_uid))
        .expect("complete pages");

    assert!(snapshot.is_managed(SyncId::ServerId(environment_uid)));
    assert!(matches!(
        snapshot.factory("factory-12").map(|row| &row.choice),
        Some(CloudSelectorChoice::Factory { foreman_agent_uid, .. }) if foreman_agent_uid == "foreman-12"
    ));
}

#[test]
fn failed_later_page_does_not_publish_partial_factory_choices() {
    let team_uid = ServerId::from(7);
    let mut mock = MockAIClient::new();
    mock.expect_get_factory_selector_options()
        .times(2)
        .returning(|_, cursor| {
            if cursor.is_some() {
                return Err(anyhow!("later page unavailable"));
            }
            Ok(FactorySelectorOptionsResponse {
                factories: vec![FactorySelectorOption {
                    uid: "factory-12".to_owned(),
                    team_uid: ServerId::from(7).uid(),
                    name: "Build".to_owned(),
                    alias: None,
                    default_environment_uid: ServerId::from(12).uid(),
                    foreman_agent_uid: "foreman-12".to_owned(),
                }],
                managed_environment_uids: vec![],
                page_info: FactorySelectorPageInfo {
                    has_next_page: true,
                    next_cursor: Some("next-page".to_owned()),
                },
            })
        });
    let scope = RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(team_uid));
    assert!(futures::executor::block_on(fetch_snapshot(Arc::new(mock), scope, team_uid)).is_err());
}
