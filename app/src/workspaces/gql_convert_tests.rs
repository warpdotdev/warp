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
fn drop_teams_the_user_is_not_a_member_of() {
    let mut workspace = workspace(vec![
        team("non-member", &["other-user"]),
        team("member", &["current-user"]),
    ]);

    retain_authenticated_teams(&mut workspace, UserUid::new("current-user"));

    assert_eq!(team_names(&workspace), ["member"]);
}

#[test]
fn preserve_server_order_across_member_teams() {
    let mut workspace = workspace(vec![
        team("non-member-one", &["other-user"]),
        team("member-one", &["current-user"]),
        team("non-member-two", &["another-user"]),
        team("member-two", &["current-user"]),
    ]);

    retain_authenticated_teams(&mut workspace, UserUid::new("current-user"));

    assert_eq!(team_names(&workspace), ["member-one", "member-two"]);
}

#[test]
fn drop_every_team_when_user_has_no_team_membership() {
    // A workspace admin the server grants every team but who joined none of them
    // has nothing to operate as in the client, so the list ends up empty.
    let mut workspace = workspace(vec![
        team("first", &["other-user"]),
        team("second", &["another-user"]),
    ]);

    retain_authenticated_teams(&mut workspace, UserUid::new("current-user"));

    assert!(team_names(&workspace).is_empty());
}

mod team_settings_conversion {
    use warp_graphql::workspace as gqlws;

    use crate::ai::execution_profiles::{
        ActionPermission, ComputerUsePermission, WriteToPtyPermission,
    };
    use crate::workspaces::gql_convert::team_settings_from_gql;
    use crate::workspaces::workspace::{
        AdminEnablementSetting, TeamSettings, UgcCollectionEnablementSetting,
    };

    fn admin_info(
        value: gqlws::AdminEnablementSetting,
        is_enforced_by_workspace: bool,
    ) -> gqlws::AdminEnablementSettingInfo {
        gqlws::AdminEnablementSettingInfo {
            value,
            is_enforced_by_workspace,
        }
    }

    fn bool_info(value: bool, is_enforced_by_workspace: bool) -> gqlws::BooleanSettingInfo {
        gqlws::BooleanSettingInfo {
            value,
            is_enforced_by_workspace,
        }
    }

    fn str_list(
        values: &[&str],
        workspace: &[&str],
        team: &[&str],
    ) -> gqlws::StringListSettingInfo {
        let owned = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        gqlws::StringListSettingInfo {
            values: owned(values),
            workspace_entries: owned(workspace),
            team_entries: owned(team),
        }
    }

    fn autonomy_info(
        value: gqlws::AiAutonomyValue,
        is_enforced_by_workspace: bool,
    ) -> gqlws::AiAutonomySettingInfo {
        gqlws::AiAutonomySettingInfo {
            value,
            is_enforced_by_workspace,
        }
    }

    /// Builds a `GqlTeamSettings` with distinctive effective values, enforcement
    /// bits, and workspace/team list splits so the conversion can be asserted
    /// field-by-field (including the metadata that must be preserved).
    fn sample_gql_team_settings() -> gqlws::TeamSettings {
        gqlws::TeamSettings {
            ugc_collection: gqlws::UgcCollectionSettingInfo {
                value: gqlws::UgcCollectionEnablementSetting::Enable,
                is_enforced_by_workspace: true,
            },
            cloud_conversation_storage: admin_info(gqlws::AdminEnablementSetting::Disable, false),
            codebase_context: admin_info(gqlws::AdminEnablementSetting::Enable, false),
            ai_permissions: gqlws::AiPermissionsSettingsInfo {
                allow_ai_in_remote_sessions: bool_info(true, true),
                remote_session_regex_list: str_list(&["foo.*"], &["ws.*"], &["team.*"]),
            },
            secret_redaction: gqlws::SecretRedactionSettingsInfo {
                enabled: bool_info(true, false),
                regexes: gqlws::SecretRedactionRegexListInfo {
                    values: vec![gqlws::SecretRedactionRegex {
                        name: Some("api-key".to_string()),
                        pattern: "sk-.*".to_string(),
                    }],
                    workspace_entries: vec![gqlws::SecretRedactionRegex {
                        name: None,
                        pattern: "ws-secret".to_string(),
                    }],
                    team_entries: vec![],
                },
            },
            ai_autonomy: gqlws::AiAutonomySettingsInfo {
                apply_code_diffs: autonomy_info(gqlws::AiAutonomyValue::AlwaysAllow, true),
                read_files: autonomy_info(gqlws::AiAutonomyValue::RespectUserSetting, false),
                create_plans: autonomy_info(gqlws::AiAutonomyValue::RespectUserSetting, false),
                execute_commands: autonomy_info(gqlws::AiAutonomyValue::AlwaysAsk, false),
                write_to_pty: gqlws::WriteToPtySettingInfo {
                    value: gqlws::WriteToPtyAutonomyValue::AlwaysAsk,
                    is_enforced_by_workspace: false,
                },
                computer_use: gqlws::ComputerUseSettingInfo {
                    value: gqlws::ComputerUseAutonomyValue::Never,
                    is_enforced_by_workspace: false,
                },
                read_files_allowlist: str_list(&["/allowed"], &[], &[]),
                execute_commands_allowlist: str_list(&["ls"], &[], &[]),
                execute_commands_denylist: str_list(&["rm"], &[], &[]),
            },
            link_sharing: gqlws::LinkSharingSettingsInfo {
                anyone_with_link_sharing_enabled: bool_info(true, false),
                direct_link_sharing_enabled: bool_info(false, false),
            },
            sandboxed_agent: gqlws::SandboxedAgentSettingsInfo {
                execute_commands_denylist: str_list(&["danger"], &[], &[]),
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
    fn reads_effective_values_and_preserves_metadata() {
        let settings = TeamSettings::from(sample_gql_team_settings());

        // Workspace-governable groups keep both the effective `.value` and the
        // `is_enforced_by_workspace` bit.
        assert!(matches!(
            settings.ugc_collection.value,
            UgcCollectionEnablementSetting::Enable
        ));
        assert!(settings.ugc_collection.is_enforced_by_workspace);
        assert_eq!(
            settings.cloud_conversation_storage.value,
            AdminEnablementSetting::Disable
        );
        assert_eq!(
            settings.codebase_context.value,
            AdminEnablementSetting::Enable
        );

        // AI permissions preserve the enforcement bit and the list split entries.
        assert!(settings.ai_permissions.allow_ai_in_remote_sessions.value);
        assert!(
            settings
                .ai_permissions
                .allow_ai_in_remote_sessions
                .is_enforced_by_workspace
        );
        assert_eq!(
            settings.ai_permissions.remote_session_regex_list.values,
            vec!["foo.*".to_string()]
        );
        assert_eq!(
            settings
                .ai_permissions
                .remote_session_regex_list
                .workspace_entries,
            vec!["ws.*".to_string()]
        );
        assert_eq!(
            settings
                .ai_permissions
                .remote_session_regex_list
                .team_entries,
            vec!["team.*".to_string()]
        );

        // Secret redaction keeps the merged values and the workspace split entries.
        assert!(settings.secret_redaction.enabled.value);
        assert_eq!(settings.secret_redaction.regexes.values.len(), 1);
        assert_eq!(settings.secret_redaction.regexes.values[0].pattern, "sk-.*");
        assert_eq!(
            settings.secret_redaction.regexes.workspace_entries[0].pattern,
            "ws-secret"
        );

        // AI autonomy maps effective values to permissions (RespectUserSetting ->
        // None) while preserving the enforcement bit.
        assert_eq!(
            settings.ai_autonomy.apply_code_diffs.value,
            Some(ActionPermission::AlwaysAllow)
        );
        assert!(
            settings
                .ai_autonomy
                .apply_code_diffs
                .is_enforced_by_workspace
        );
        assert_eq!(settings.ai_autonomy.read_files.value, None);
        assert_eq!(
            settings.ai_autonomy.execute_commands.value,
            Some(ActionPermission::AlwaysAsk)
        );
        assert_eq!(
            settings.ai_autonomy.write_to_pty.value,
            Some(WriteToPtyPermission::AlwaysAsk)
        );
        assert_eq!(
            settings.ai_autonomy.computer_use.value,
            Some(ComputerUsePermission::Never)
        );
        assert_eq!(
            settings.ai_autonomy.read_files_allowlist.values,
            vec!["/allowed".to_string()]
        );

        // Link sharing keeps each boolean value.
        assert!(settings.link_sharing.anyone_with_link_sharing_enabled.value);
        assert!(!settings.link_sharing.direct_link_sharing_enabled.value);

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
            settings.sandboxed_agent.execute_commands_denylist.values,
            vec!["danger".to_string()]
        );
    }

    #[test]
    fn team_settings_from_gql_uses_team_payload() {
        // The team payload carries distinctive values (llm enabled, codebase
        // context Enable, ugc enforced). `team_settings_from_gql` derives
        // `Team.settings` from this payload only — it takes just the team settings,
        // so it structurally cannot clone workspace settings. This is the parse
        // boundary replacing the old `Team::organization_settings` clone.
        let settings = team_settings_from_gql(sample_gql_team_settings());

        assert!(
            settings.llm_settings.enabled,
            "Team.settings must be sourced from the team payload"
        );
        assert_eq!(
            settings.codebase_context.value,
            AdminEnablementSetting::Enable,
            "team codebase_context value must flow through from the team payload"
        );
        assert!(
            settings.ugc_collection.is_enforced_by_workspace,
            "enforcement metadata from the team payload must be preserved"
        );
    }
}

mod team_from_gql {
    use warp_graphql::billing::{
        BillingMetadata as GqlBillingMetadata, BonusGrantsInfo as GqlBonusGrantsInfo,
        CustomerType as GqlCustomerType, DelinquencyStatus as GqlDelinquencyStatus,
        Tier as GqlTier,
    };
    use warp_graphql::workspace::{
        AddonCreditsSettings as GqlAddonCreditsSettings,
        AdminEnablementSetting as GqlAdminEnablementSetting,
        AdminEnablementSettingInfo as GqlAdminEnablementSettingInfo,
        AiAutonomySettingInfo as GqlAiAutonomySettingInfo,
        AiAutonomySettings as GqlAiAutonomySettings,
        AiAutonomySettingsInfo as GqlAiAutonomySettingsInfo, AiAutonomyValue as GqlAiAutonomyValue,
        AiPermissionsSettings as GqlAiPermissionsSettings,
        AiPermissionsSettingsInfo as GqlAiPermissionsSettingsInfo,
        AvailableLlms as GqlAvailableLlms, BooleanSettingInfo as GqlBooleanSettingInfo,
        CloudConversationStorageSettings as GqlCloudConversationStorageSettings,
        CodebaseContextSettings as GqlCodebaseContextSettings,
        ComputerUseAutonomyValue as GqlComputerUseAutonomyValue,
        ComputerUseSettingInfo as GqlComputerUseSettingInfo,
        FeatureModelChoice as GqlFeatureModelChoice, LinkSharingSettings as GqlLinkSharingSettings,
        LinkSharingSettingsInfo as GqlLinkSharingSettingsInfo, LlmSettings as GqlLlmSettings,
        MembershipRole as GqlMembershipRole,
        SandboxedAgentSettingsInfo as GqlSandboxedAgentSettingsInfo,
        SecretRedactionRegexListInfo as GqlSecretRedactionRegexListInfo,
        SecretRedactionSettings as GqlSecretRedactionSettings,
        SecretRedactionSettingsInfo as GqlSecretRedactionSettingsInfo,
        StringListSettingInfo as GqlStringListSettingInfo, Team as GqlTeam,
        TeamMember as GqlTeamMember, TeamSettings as GqlTeamSettings,
        TelemetrySettings as GqlTelemetrySettings,
        UgcCollectionEnablementSetting as GqlUgcCollectionEnablementSetting,
        UgcCollectionSettingInfo as GqlUgcCollectionSettingInfo,
        UgcCollectionSettings as GqlUgcCollectionSettings,
        UsageBasedPricingSettings as GqlUsageBasedPricingSettings, Workspace as GqlWorkspace,
        WorkspaceSettings as GqlWorkspaceSettings,
        WriteToPtyAutonomyValue as GqlWriteToPtyAutonomyValue,
        WriteToPtySettingInfo as GqlWriteToPtySettingInfo,
    };

    use crate::workspaces::team::Team;

    /// A neutral `Tier` with every policy absent, so a fixture only has to
    /// override the field a test cares about.
    fn gql_tier() -> GqlTier {
        GqlTier {
            name: "Free".to_string(),
            description: "Free tier".to_string(),
            warp_ai_policy: None,
            team_size_policy: None,
            shared_notebooks_policy: None,
            shared_workflows_policy: None,
            session_sharing_policy: None,
            ai_autonomy_policy: None,
            telemetry_data_collection_policy: None,
            ugc_data_collection_policy: None,
            usage_based_pricing_policy: None,
            codebase_context_policy: None,
            byo_api_key_policy: None,
            byo_endpoint_policy: None,
            managed_byok_byoe_policy: None,
            purchase_add_on_credits_policy: None,
            enterprise_pay_as_you_go_policy: None,
            enterprise_credits_auto_reload_policy: None,
            multi_admin_policy: None,
            native_workspaces_policy: None,
            ambient_agents_policy: None,
            usage_visibility_policy: None,
        }
    }

    /// A neutral workspace payload with no teams; each test attaches its own.
    fn gql_workspace() -> GqlWorkspace {
        let empty_llms = GqlAvailableLlms {
            default_id: String::new(),
            choices: vec![],
            preferred_codex_model_id: None,
        };
        GqlWorkspace {
            uid: "workspace_uid123456789".into(),
            name: "workspace".to_string(),
            stripe_customer_id: None,
            members: vec![],
            teams: vec![],
            billing_metadata: GqlBillingMetadata {
                customer_type: GqlCustomerType::Free,
                delinquency_status: GqlDelinquencyStatus::NoDelinquency,
                tier: gql_tier(),
                service_agreements: vec![],
                ai_overages: None,
            },
            bonus_grants_info: GqlBonusGrantsInfo {
                grants: vec![],
                spending_info: None,
            },
            billing_cycle_usage_history: None,
            settings: GqlWorkspaceSettings {
                is_discoverable: false,
                is_invite_link_enabled: false,
                llm_settings: GqlLlmSettings {
                    enabled: false,
                    host_configs: vec![],
                },
                team_byo: None,
                telemetry_settings: GqlTelemetrySettings {
                    force_enabled: false,
                },
                ugc_collection_settings: GqlUgcCollectionSettings {
                    setting: GqlUgcCollectionEnablementSetting::RespectUserSetting,
                },
                cloud_conversation_storage_settings: GqlCloudConversationStorageSettings {
                    setting: GqlAdminEnablementSetting::RespectUserSetting,
                },
                ai_permissions_settings: GqlAiPermissionsSettings {
                    allow_ai_in_remote_sessions: true,
                    remote_session_regex_list: vec![],
                },
                link_sharing_settings: GqlLinkSharingSettings {
                    anyone_with_link_sharing_enabled: true,
                    direct_link_sharing_enabled: true,
                },
                secret_redaction_settings: GqlSecretRedactionSettings {
                    enabled: false,
                    regexes: vec![],
                },
                ai_autonomy_settings: GqlAiAutonomySettings {
                    apply_code_diffs_setting: None,
                    read_files_setting: None,
                    read_files_allowlist: None,
                    create_plans_setting: None,
                    execute_commands_setting: None,
                    execute_commands_allowlist: None,
                    execute_commands_denylist: None,
                    write_to_pty_setting: None,
                    computer_use_setting: None,
                },
                usage_based_pricing_settings: GqlUsageBasedPricingSettings {
                    enabled: false,
                    max_monthly_spend_cents: None,
                },
                addon_credits_settings: GqlAddonCreditsSettings {
                    auto_reload_enabled: false,
                    max_monthly_spend_cents: None,
                    selected_auto_reload_credit_denomination: None,
                },
                codebase_context_settings: GqlCodebaseContextSettings {
                    enabled: true,
                    setting: GqlAdminEnablementSetting::RespectUserSetting,
                },
                sandboxed_agent_settings: None,
                ambient_agent_settings: None,
            },
            has_billing_history: false,
            pending_email_invites: vec![],
            invite_link_domain_restrictions: vec![],
            is_eligible_for_discovery: false,
            feature_model_choice: GqlFeatureModelChoice {
                agent_mode: empty_llms.clone(),
                planning: empty_llms.clone(),
                coding: empty_llms.clone(),
                cli_agent: empty_llms.clone(),
                computer_use_agent: empty_llms,
            },
            total_requests_used_since_last_refresh: 0,
        }
    }

    /// Team settings with every group at its neutral value.
    fn gql_team_settings() -> GqlTeamSettings {
        fn admin_info() -> GqlAdminEnablementSettingInfo {
            GqlAdminEnablementSettingInfo {
                value: GqlAdminEnablementSetting::RespectUserSetting,
                is_enforced_by_workspace: false,
            }
        }

        fn bool_info(value: bool) -> GqlBooleanSettingInfo {
            GqlBooleanSettingInfo {
                value,
                is_enforced_by_workspace: false,
            }
        }

        fn autonomy_info() -> GqlAiAutonomySettingInfo {
            GqlAiAutonomySettingInfo {
                value: GqlAiAutonomyValue::RespectUserSetting,
                is_enforced_by_workspace: false,
            }
        }

        fn str_list() -> GqlStringListSettingInfo {
            GqlStringListSettingInfo {
                values: vec![],
                workspace_entries: vec![],
                team_entries: vec![],
            }
        }

        GqlTeamSettings {
            ugc_collection: GqlUgcCollectionSettingInfo {
                value: GqlUgcCollectionEnablementSetting::RespectUserSetting,
                is_enforced_by_workspace: false,
            },
            cloud_conversation_storage: admin_info(),
            codebase_context: admin_info(),
            ai_permissions: GqlAiPermissionsSettingsInfo {
                allow_ai_in_remote_sessions: bool_info(true),
                remote_session_regex_list: str_list(),
            },
            secret_redaction: GqlSecretRedactionSettingsInfo {
                enabled: bool_info(false),
                regexes: GqlSecretRedactionRegexListInfo {
                    values: vec![],
                    workspace_entries: vec![],
                    team_entries: vec![],
                },
            },
            ai_autonomy: GqlAiAutonomySettingsInfo {
                apply_code_diffs: autonomy_info(),
                read_files: autonomy_info(),
                create_plans: autonomy_info(),
                execute_commands: autonomy_info(),
                write_to_pty: GqlWriteToPtySettingInfo {
                    value: GqlWriteToPtyAutonomyValue::RespectUserSetting,
                    is_enforced_by_workspace: false,
                },
                computer_use: GqlComputerUseSettingInfo {
                    value: GqlComputerUseAutonomyValue::RespectUserSetting,
                    is_enforced_by_workspace: false,
                },
                read_files_allowlist: str_list(),
                execute_commands_allowlist: str_list(),
                execute_commands_denylist: str_list(),
            },
            link_sharing: GqlLinkSharingSettingsInfo {
                anyone_with_link_sharing_enabled: bool_info(true),
                direct_link_sharing_enabled: bool_info(true),
            },
            sandboxed_agent: GqlSandboxedAgentSettingsInfo {
                execute_commands_denylist: str_list(),
            },
            llm_settings: GqlLlmSettings {
                enabled: false,
                host_configs: vec![],
            },
            telemetry_settings: GqlTelemetrySettings {
                force_enabled: false,
            },
            usage_based_pricing_settings: GqlUsageBasedPricingSettings {
                enabled: false,
                max_monthly_spend_cents: None,
            },
            addon_credits_settings: GqlAddonCreditsSettings {
                auto_reload_enabled: false,
                max_monthly_spend_cents: None,
                selected_auto_reload_credit_denomination: None,
            },
            ambient_agent_settings: None,
            team_byo: None,
        }
    }

    fn gql_team(uid: &str, invite_link: Option<&str>) -> GqlTeam {
        GqlTeam {
            // `ServerId` rejects anything but a 22-character id.
            uid: format!("{uid:0>22}").into(),
            name: uid.to_string(),
            color: None,
            members: vec![GqlTeamMember {
                uid: "test-user".into(),
                email: "test-user@example.com".to_string(),
                role: GqlMembershipRole::User,
            }],
            settings: gql_team_settings(),
            invite_link: invite_link.map(str::to_string),
        }
    }

    #[test]
    fn preserves_each_teams_own_invite_link_rather_than_cloning_a_shared_value() {
        // Regression: `Team::from_gql` used to clone `Workspace.inviteCode` onto
        // every team, so every team in a workspace showed the same link. It must
        // now read each team's own `inviteLink` field instead.
        let workspace = gql_workspace();
        let team_a = gql_team("team-a", Some("https://app.warp.dev/team/aaa"));
        let team_b = gql_team("team-b", Some("https://app.warp.dev/team/bbb"));

        let converted_a = Team::from_gql(workspace.clone(), team_a);
        let converted_b = Team::from_gql(workspace, team_b);

        assert_eq!(
            converted_a.invite_link.as_deref(),
            Some("https://app.warp.dev/team/aaa")
        );
        assert_eq!(
            converted_b.invite_link.as_deref(),
            Some("https://app.warp.dev/team/bbb")
        );
        assert_ne!(
            converted_a.invite_link, converted_b.invite_link,
            "each team must keep its own server-provided invite link"
        );
    }

    #[test]
    fn preserves_none_invite_link_when_the_team_has_no_link() {
        let workspace = gql_workspace();
        let team = gql_team("team-a", None);

        let converted = Team::from_gql(workspace, team);

        assert_eq!(converted.invite_link, None);
    }
}
