use std::sync::Arc;

use settings::Setting;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity as _};

use super::*;
use crate::ai::llms::{LLMModelHost, RoutingHostConfig};
use crate::auth::AuthStateProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::TeamContextForOperation;
use crate::workspaces::workspace::{HostEnablementSetting, LlmHostSettings, Workspace};

#[test]
fn model_sidecars_only_offer_variants_with_usable_hosts() {
    let _geap = FeatureFlag::GeminiEnterprise.override_enabled(true);
    let variants = [
        ("auto", "auto", None, LLMModelHost::AwsBedrock),
        ("auto-genius", "auto", None, LLMModelHost::DirectApi),
        (
            "reason-low",
            "reason",
            Some("low"),
            LLMModelHost::AwsBedrock,
        ),
        (
            "reason-medium",
            "reason",
            Some("medium"),
            LLMModelHost::GeminiEnterprise,
        ),
        (
            "reason-high",
            "reason",
            Some("high"),
            LLMModelHost::DirectApi,
        ),
    ]
    .into_iter()
    .map(|(id, base, level, host)| {
        let mut llm = LLMInfo::new_for_test(id);
        llm.base_model_name = base.to_string();
        llm.reasoning_level = level.map(str::to_string);
        llm.host_configs.insert(
            host.clone(),
            RoutingHostConfig {
                enabled: true,
                model_routing_host: host,
            },
        );
        llm
    })
    .collect::<Vec<_>>();
    let mut team = Team::from_local_cache(123.into(), "respect".into(), None, None, None, None);
    team.settings.llm_settings.enabled = true;
    for host in [LLMModelHost::AwsBedrock, LLMModelHost::GeminiEnterprise] {
        team.settings.llm_settings.host_configs.insert(
            host,
            LlmHostSettings {
                enabled: true,
                enablement_setting: HostEnablementSetting::RespectUserSetting,
                ..Default::default()
            },
        );
    }
    let workspace = Workspace::from_local_cache(
        "workspace_uid123456789".to_string().into(),
        "test".into(),
        Some(vec![team.clone()]),
        None,
    );
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|ctx| {
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });
        let scope = TeamContextForOperation::new_for_test(team.uid);
        let selected_ids = |items: Vec<MenuItem<ProfileModelSelectorAction>>| {
            items
                .iter()
                .filter_map(|item| item.item_on_select_action()?.selected_model_id())
                .collect::<Vec<_>>()
        };
        app.read(|ctx| {
            assert_eq!(
                selected_ids(ProfileModelSelector::auto_sidecar_items(
                    &variants,
                    &"auto".into(),
                    &scope,
                    ctx,
                )),
                ["auto-genius".into()]
            );
            assert_eq!(
                selected_ids(ProfileModelSelector::reasoning_sidecar_items(
                    &variants,
                    "reason",
                    &"reason-low".into(),
                    &scope,
                    ctx,
                )),
                ["reason-high".into()]
            );
        });
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .aws_bedrock_credentials_enabled
                .set_value(true, ctx)
                .unwrap();
            settings
                .gemini_enterprise_credentials_enabled
                .set_value(true, ctx)
                .unwrap();
        });
        app.read(|ctx| {
            assert_eq!(
                selected_ids(ProfileModelSelector::auto_sidecar_items(
                    &variants,
                    &"auto".into(),
                    &scope,
                    ctx,
                )),
                ["auto".into(), "auto-genius".into()]
            );
            assert_eq!(
                selected_ids(ProfileModelSelector::reasoning_sidecar_items(
                    &variants,
                    "reason",
                    &"reason-low".into(),
                    &scope,
                    ctx,
                )),
                [
                    "reason-low".into(),
                    "reason-medium".into(),
                    "reason-high".into()
                ]
            );
        });
    });
}
