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
    use crate::workspaces::gql_convert::team_settings_from_gql;
    use crate::workspaces::team::TeamSettingsCache;
    use crate::workspaces::workspace::{
        AdminEnablementSetting, TeamSettings, UgcCollectionEnablementSetting, WorkspaceSettings,
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

    /// Builds a minimal `GqlWorkspaceSettings` with the given distinctive values.
    fn workspace_settings_with(
        llm_enabled: bool,
        codebase_context: gqlws::AdminEnablementSetting,
        is_invite_link_enabled: bool,
        is_discoverable: bool,
    ) -> gqlws::WorkspaceSettings {
        gqlws::WorkspaceSettings {
            is_discoverable,
            is_invite_link_enabled,
            llm_settings: gqlws::LlmSettings {
                enabled: llm_enabled,
                host_configs: vec![],
            },
            team_byo: None,
            telemetry_settings: gqlws::TelemetrySettings {
                force_enabled: false,
            },
            ugc_collection_settings: gqlws::UgcCollectionSettings {
                setting: gqlws::UgcCollectionEnablementSetting::RespectUserSetting,
            },
            cloud_conversation_storage_settings: gqlws::CloudConversationStorageSettings {
                setting: gqlws::AdminEnablementSetting::RespectUserSetting,
            },
            ai_permissions_settings: gqlws::AiPermissionsSettings {
                allow_ai_in_remote_sessions: false,
                remote_session_regex_list: vec![],
            },
            link_sharing_settings: gqlws::LinkSharingSettings {
                anyone_with_link_sharing_enabled: false,
                direct_link_sharing_enabled: false,
            },
            secret_redaction_settings: gqlws::SecretRedactionSettings {
                enabled: false,
                regexes: vec![],
            },
            ai_autonomy_settings: gqlws::AiAutonomySettings {
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
            usage_based_pricing_settings: gqlws::UsageBasedPricingSettings {
                enabled: false,
                max_monthly_spend_cents: None,
            },
            addon_credits_settings: gqlws::AddonCreditsSettings {
                auto_reload_enabled: false,
                max_monthly_spend_cents: None,
                selected_auto_reload_credit_denomination: None,
            },
            codebase_context_settings: gqlws::CodebaseContextSettings {
                enabled: false,
                setting: codebase_context,
            },
            sandboxed_agent_settings: None,
            ambient_agent_settings: None,
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
    fn from_gql_sources_settings_from_team_and_flags_from_workspace() {
        // Team payload: codebase_context = Enable, llm enabled = true.
        let team_settings = sample_gql_team_settings();
        // Workspace payload deliberately DIFFERENT: codebase_context = Disable and
        // llm disabled, but invite-link + discoverability enabled.
        let workspace_settings =
            workspace_settings_with(false, gqlws::AdminEnablementSetting::Disable, true, true);

        let (settings, is_invite_link_enabled, is_discoverable) =
            team_settings_from_gql(&workspace_settings, team_settings);

        // Effective settings come from the TEAM payload, not the workspace.
        assert!(
            settings.llm_settings.enabled,
            "Team.settings must be sourced from the team payload, not cloned from workspace settings"
        );
        assert_eq!(
            settings.codebase_context.value,
            AdminEnablementSetting::Enable,
            "team codebase_context value must win over the workspace value"
        );
        // The two flags come from the WORKSPACE settings (not on TeamSettings).
        assert!(is_invite_link_enabled);
        assert!(is_discoverable);
    }

    #[test]
    fn migrates_legacy_workspace_settings_cache_row() {
        // A row written by the previous release: a serialized `WorkspaceSettings`
        // with the old fields at the top level (no `settings` key).
        let mut legacy = WorkspaceSettings {
            is_invite_link_enabled: true,
            is_discoverable: true,
            ..Default::default()
        };
        legacy.llm_settings.enabled = true;
        legacy.codebase_context_settings.setting = AdminEnablementSetting::Enable;
        legacy.enable_warp_attribution = AdminEnablementSetting::Disable;
        let legacy_json = serde_json::to_string(&legacy).expect("serialize legacy row");

        let cache = TeamSettingsCache::from_cached_json(&legacy_json)
            .expect("legacy WorkspaceSettings cache row should decode, not fall back to default");

        // Cached LLM / policy values and the two flags survive the migration.
        assert!(
            cache.settings.llm_settings.enabled,
            "cached custom-LLM value must not be silently lost"
        );
        assert!(cache.is_invite_link_enabled);
        assert!(cache.is_discoverable);
        assert_eq!(
            cache.settings.codebase_context.value,
            AdminEnablementSetting::Enable
        );
        assert_eq!(
            cache.settings.enable_warp_attribution,
            AdminEnablementSetting::Disable
        );

        // The current cache shape (which has a nested `settings` key) still decodes.
        let current = TeamSettingsCache {
            is_invite_link_enabled: true,
            ..Default::default()
        };
        let current_json = serde_json::to_string(&current).expect("serialize current row");
        let decoded = TeamSettingsCache::from_cached_json(&current_json)
            .expect("current cache shape should decode");
        assert!(decoded.is_invite_link_enabled);
    }
}
