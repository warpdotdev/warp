use super::*;

fn team(name: &str, member_uids: &[&str]) -> Team {
    Team::from_local_cache(
        ServerId::from_string_lossy(format!("{name:0>22}")),
        name.to_string(),
        None,
        None,
        Some(
            member_uids
                .iter()
                .map(|uid| TeamMember {
                    uid: UserUid::new(uid),
                    email: format!("{uid}@example.com"),
                    role: MembershipRole::User,
                })
                .collect(),
        ),
    )
}

fn workspace(teams: Vec<Team>) -> Workspace {
    Workspace::from_local_cache(
        format!("{:0>22}", "workspace").into(),
        "workspace".to_string(),
        Some(teams),
    )
}

fn team_names(workspace: &Workspace) -> Vec<&str> {
    workspace
        .teams
        .iter()
        .map(|team| team.name.as_str())
        .collect()
}

#[test]
fn order_authenticated_teams_before_non_member_teams() {
    let mut workspace = workspace(vec![
        team("non-member", &["other-user"]),
        team("member", &["current-user"]),
    ]);

    order_authenticated_teams_first(&mut workspace, UserUid::new("current-user"));

    assert_eq!(team_names(&workspace), ["member", "non-member"]);
}

#[test]
fn preserve_relative_order_within_member_groups() {
    let mut workspace = workspace(vec![
        team("non-member-one", &["other-user"]),
        team("member-one", &["current-user"]),
        team("non-member-two", &["another-user"]),
        team("member-two", &["current-user"]),
    ]);

    order_authenticated_teams_first(&mut workspace, UserUid::new("current-user"));

    assert_eq!(
        team_names(&workspace),
        [
            "member-one",
            "member-two",
            "non-member-one",
            "non-member-two"
        ]
    );
}

#[test]
fn preserve_server_order_when_user_has_no_team_membership() {
    let mut workspace = workspace(vec![
        team("first", &["other-user"]),
        team("second", &["another-user"]),
    ]);

    order_authenticated_teams_first(&mut workspace, UserUid::new("current-user"));

    assert_eq!(team_names(&workspace), ["first", "second"]);
}

mod team_settings_conversion {
    use warp_graphql::workspace as gqlws;

    use crate::ai::execution_profiles::{
        ActionPermission, ComputerUsePermission, WriteToPtyPermission,
    };
    use crate::workspaces::workspace::{
        AdminEnablementSetting, TeamSettings, UgcCollectionEnablementSetting,
    };

    fn admin_info(value: gqlws::AdminEnablementSetting) -> gqlws::AdminEnablementSettingInfo {
        gqlws::AdminEnablementSettingInfo {
            value,
            is_enforced_by_workspace: false,
        }
    }

    fn bool_info(value: bool) -> gqlws::BooleanSettingInfo {
        gqlws::BooleanSettingInfo {
            value,
            is_enforced_by_workspace: false,
        }
    }

    fn str_list(values: &[&str]) -> gqlws::StringListSettingInfo {
        gqlws::StringListSettingInfo {
            values: values.iter().map(|s| s.to_string()).collect(),
            workspace_entries: vec![],
            team_entries: vec![],
        }
    }

    fn autonomy_info(value: gqlws::AiAutonomyValue) -> gqlws::AiAutonomySettingInfo {
        gqlws::AiAutonomySettingInfo {
            value,
            is_enforced_by_workspace: false,
        }
    }

    /// Builds a `GqlTeamSettings` with distinctive effective values so the
    /// conversion can be asserted field-by-field.
    fn sample_gql_team_settings() -> gqlws::TeamSettings {
        gqlws::TeamSettings {
            ugc_collection: gqlws::UgcCollectionSettingInfo {
                value: gqlws::UgcCollectionEnablementSetting::Enable,
                is_enforced_by_workspace: false,
            },
            cloud_conversation_storage: admin_info(gqlws::AdminEnablementSetting::Disable),
            codebase_context: admin_info(gqlws::AdminEnablementSetting::Enable),
            ai_permissions: gqlws::AiPermissionsSettingsInfo {
                allow_ai_in_remote_sessions: bool_info(true),
                remote_session_regex_list: str_list(&["foo.*"]),
            },
            secret_redaction: gqlws::SecretRedactionSettingsInfo {
                enabled: bool_info(true),
                regexes: gqlws::SecretRedactionRegexListInfo {
                    values: vec![gqlws::SecretRedactionRegex {
                        name: Some("api-key".to_string()),
                        pattern: "sk-.*".to_string(),
                    }],
                    workspace_entries: vec![],
                    team_entries: vec![],
                },
            },
            ai_autonomy: gqlws::AiAutonomySettingsInfo {
                apply_code_diffs: autonomy_info(gqlws::AiAutonomyValue::AlwaysAllow),
                read_files: autonomy_info(gqlws::AiAutonomyValue::RespectUserSetting),
                create_plans: autonomy_info(gqlws::AiAutonomyValue::RespectUserSetting),
                execute_commands: autonomy_info(gqlws::AiAutonomyValue::AlwaysAsk),
                write_to_pty: gqlws::WriteToPtySettingInfo {
                    value: gqlws::WriteToPtyAutonomyValue::AlwaysAsk,
                    is_enforced_by_workspace: false,
                },
                computer_use: gqlws::ComputerUseSettingInfo {
                    value: gqlws::ComputerUseAutonomyValue::Never,
                    is_enforced_by_workspace: false,
                },
                read_files_allowlist: str_list(&["/allowed"]),
                execute_commands_allowlist: str_list(&["ls"]),
                execute_commands_denylist: str_list(&["rm"]),
            },
            link_sharing: gqlws::LinkSharingSettingsInfo {
                anyone_with_link_sharing_enabled: bool_info(true),
                direct_link_sharing_enabled: bool_info(false),
            },
            sandboxed_agent: gqlws::SandboxedAgentSettingsInfo {
                execute_commands_denylist: str_list(&["danger"]),
            },
            llm_settings: gqlws::LlmSettings {
                enabled: true,
                host_configs: vec![],
            },
            telemetry_settings: gqlws::TelemetrySettings {
                force_enabled: true,
            },
            usage_based_pricing_settings: gqlws::UsageBasedPricingSettings {
                enabled: true,
                max_monthly_spend_cents: Some(500),
            },
            addon_credits_settings: gqlws::AddonCreditsSettings {
                auto_reload_enabled: true,
                max_monthly_spend_cents: Some(100),
                selected_auto_reload_credit_denomination: Some(50),
            },
            ambient_agent_settings: Some(gqlws::AmbientAgentSettings {
                enable_warp_attribution: gqlws::AdminEnablementSetting::Enable,
                default_host_slug: Some("my-host".to_string()),
            }),
            team_byo: None,
        }
    }

    #[test]
    fn reads_effective_values_from_team_payload() {
        let settings = TeamSettings::from(sample_gql_team_settings());

        // Workspace-governable groups read their effective `.value`.
        assert!(matches!(
            settings.ugc_collection_settings.setting,
            UgcCollectionEnablementSetting::Enable
        ));
        assert_eq!(
            settings.cloud_conversation_storage_settings.setting,
            AdminEnablementSetting::Disable
        );
        assert_eq!(
            settings.codebase_context_settings.setting,
            AdminEnablementSetting::Enable
        );

        // AI permissions unwrap the `BooleanSettingInfo` / `StringListSettingInfo` values.
        assert!(settings.ai_permissions_settings.allow_ai_in_remote_sessions);
        assert_eq!(
            settings
                .ai_permissions_settings
                .remote_session_regex_list
                .iter()
                .map(|r| r.as_str().to_string())
                .collect::<Vec<_>>(),
            vec!["foo.*".to_string()]
        );

        // Secret redaction reads the merged `values` list.
        assert!(settings.secret_redaction_settings.enabled);
        assert_eq!(settings.secret_redaction_settings.regexes.len(), 1);
        assert_eq!(
            settings.secret_redaction_settings.regexes[0].pattern,
            "sk-.*"
        );

        // AI autonomy maps effective values to permissions; RespectUserSetting -> None.
        assert_eq!(
            settings.ai_autonomy_settings.apply_code_diffs_setting,
            Some(ActionPermission::AlwaysAllow)
        );
        assert_eq!(settings.ai_autonomy_settings.read_files_setting, None);
        assert_eq!(
            settings.ai_autonomy_settings.execute_commands_setting,
            Some(ActionPermission::AlwaysAsk)
        );
        assert_eq!(
            settings.ai_autonomy_settings.write_to_pty_setting,
            Some(WriteToPtyPermission::AlwaysAsk)
        );
        assert_eq!(
            settings.ai_autonomy_settings.computer_use_setting,
            Some(ComputerUsePermission::Never)
        );
        assert_eq!(
            settings
                .ai_autonomy_settings
                .read_files_allowlist
                .as_ref()
                .map(|paths| paths.len()),
            Some(1)
        );

        // Link sharing unwraps each boolean.
        assert!(
            settings
                .link_sharing_settings
                .anyone_with_link_sharing_enabled
        );
        assert!(!settings.link_sharing_settings.direct_link_sharing_enabled);

        // Passthrough groups map directly.
        assert!(settings.llm_settings.enabled);
        assert!(settings.telemetry_settings.force_enabled);
        assert!(settings.usage_based_pricing_settings.enabled);
        assert_eq!(
            settings
                .usage_based_pricing_settings
                .max_monthly_spend_cents,
            Some(500)
        );
        assert!(settings.addon_credits_settings.auto_reload_enabled);

        // Ambient agent settings surface attribution + default host slug.
        assert_eq!(
            settings.enable_warp_attribution,
            AdminEnablementSetting::Enable
        );
        assert_eq!(settings.default_host_slug.as_deref(), Some("my-host"));

        // Sandboxed agent denylist is populated from the effective list.
        assert_eq!(
            settings
                .sandboxed_agent_settings
                .as_ref()
                .and_then(|s| s.execute_commands_denylist.as_ref())
                .map(|d| d.len()),
            Some(1)
        );
    }
}
