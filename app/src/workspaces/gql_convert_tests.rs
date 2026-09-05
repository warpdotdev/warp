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
                    is_disabled: false,
                })
                .collect(),
        ),
        None,
    )
}

fn workspace(teams: Vec<Team>) -> Workspace {
    Workspace::from_local_cache(
        format!("{:0>22}", "workspace").into(),
        "workspace".to_string(),
        Some(teams),
        None,
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

#[test]
fn team_member_conversion_preserves_is_disabled() {
    let enabled_member = GqlTeamMember {
        uid: "user-1".into(),
        email: "user1@example.com".to_string(),
        role: GqlMembershipRole::User,
        is_disabled: false,
    };
    let disabled_member = GqlTeamMember {
        uid: "user-2".into(),
        email: "user2@example.com".to_string(),
        role: GqlMembershipRole::User,
        is_disabled: true,
    };

    assert!(!TeamMember::from(enabled_member).is_disabled);
    assert!(TeamMember::from(disabled_member).is_disabled);
}

#[test]
fn workspace_member_conversion_preserves_is_disabled() {
    let usage_info = || GqlWorkspaceMemberUsageInfo {
        is_unlimited: false,
        request_limit: 0,
        requests_used_since_last_refresh: 0,
        is_request_limit_prorated: false,
    };
    let enabled_member = GqlWorkspaceMember {
        uid: "user-1".into(),
        email: "user1@example.com".to_string(),
        role: GqlMembershipRole::User,
        is_disabled: false,
        usage_info: usage_info(),
    };
    let disabled_member = GqlWorkspaceMember {
        uid: "user-2".into(),
        email: "user2@example.com".to_string(),
        role: GqlMembershipRole::User,
        is_disabled: true,
        usage_info: usage_info(),
    };

    assert!(!WorkspaceMember::from(enabled_member).is_disabled);
    assert!(WorkspaceMember::from(disabled_member).is_disabled);
}

mod pending_email_invites_conversion {
    use warp_graphql::workspace::EmailInvite as GqlEmailInvite;

    use crate::workspaces::gql_convert::team_pending_email_invites_from_gql;

    fn gql_invite(email: &str, team_uid: Option<&str>) -> GqlEmailInvite {
        GqlEmailInvite {
            email: email.to_string(),
            expired: false,
            team_uid: team_uid.map(cynic::Id::new),
        }
    }

    #[test]
    fn keeps_only_invites_sent_for_the_given_team() {
        let team_a_uid = format!("{:0>22}", "team-a");
        let team_b_uid = format!("{:0>22}", "team-b");
        let team_a = cynic::Id::new(team_a_uid.clone());
        let team_b = cynic::Id::new(team_b_uid.clone());
        let workspace_invites = vec![
            gql_invite("alice@example.com", Some(team_a_uid.as_str())),
            gql_invite("bob@example.com", Some(team_b_uid.as_str())),
            gql_invite("carol@example.com", Some(team_a_uid.as_str())),
        ];

        let team_a_invites = team_pending_email_invites_from_gql(&workspace_invites, &team_a);
        assert_eq!(
            team_a_invites
                .iter()
                .map(|invite| invite.invitee_email.as_str())
                .collect::<Vec<_>>(),
            vec!["alice@example.com", "carol@example.com"],
            "team A's page must not show team B's pending invite"
        );

        let team_b_invites = team_pending_email_invites_from_gql(&workspace_invites, &team_b);
        assert_eq!(
            team_b_invites
                .iter()
                .map(|invite| invite.invitee_email.as_str())
                .collect::<Vec<_>>(),
            vec!["bob@example.com"],
            "team B's page must not show team A's pending invites"
        );
    }

    #[test]
    fn drops_invites_with_no_team_uid() {
        let team_a = cynic::Id::new(format!("{:0>22}", "team-a"));
        let workspace_invites = vec![gql_invite("dangling@example.com", None)];

        let team_a_invites = team_pending_email_invites_from_gql(&workspace_invites, &team_a);
        assert!(team_a_invites.is_empty());
    }
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

        // AI permissions preserve the enforcement bit and compile the merged patterns.
        assert!(settings.ai_permissions.allow_ai_in_remote_sessions.value);
        assert!(
            settings
                .ai_permissions
                .allow_ai_in_remote_sessions
                .is_enforced_by_workspace
        );
        assert_eq!(
            settings
                .ai_permissions
                .remote_session_regex_list
                .iter()
                .map(|regex| regex.as_str())
                .collect::<Vec<_>>(),
            vec!["foo.*"],
            "only the merged `values` compile into the effective list; the workspace/team \
             split entries have no Rust-client reader to preserve them for"
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
    fn drops_an_uncompilable_remote_session_pattern_without_failing_the_rest() {
        // Compilation now happens at convert time (mirroring the workspace-level path), so an
        // org's one bad pattern must not take down the rest of its list.
        let mut gql = sample_gql_team_settings();
        gql.ai_permissions.remote_session_regex_list = str_list(&["foo.*", "("], &[], &[]);

        let settings = team_settings_from_gql(gql);

        assert_eq!(
            settings
                .ai_permissions
                .remote_session_regex_list
                .iter()
                .map(|regex| regex.as_str())
                .collect::<Vec<_>>(),
            vec!["foo.*"]
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

mod team_visibility_conversion {
    use warp_graphql::workspace::TeamVisibility as GqlTeamVisibility;

    use crate::workspaces::team::TeamVisibility;

    #[test]
    fn maps_known_values() {
        assert_eq!(
            TeamVisibility::from(GqlTeamVisibility::Open),
            TeamVisibility::Open
        );
        assert_eq!(
            TeamVisibility::from(GqlTeamVisibility::Private),
            TeamVisibility::Private
        );
        assert_eq!(
            TeamVisibility::from(GqlTeamVisibility::Hidden),
            TeamVisibility::Hidden
        );
    }

    #[test]
    fn fails_closed_on_unrecognized_value() {
        // An unrecognized value must never be treated as Open, since that
        // would surface the invite-by-link control the server doesn't
        // actually support for it.
        let visibility = TeamVisibility::from(GqlTeamVisibility::Other("future-value".to_string()));
        assert_eq!(visibility, TeamVisibility::Private);
        assert!(!visibility.supports_invite_link());
    }

    #[test]
    fn only_open_supports_invite_link() {
        assert!(TeamVisibility::Open.supports_invite_link());
        assert!(!TeamVisibility::Private.supports_invite_link());
        assert!(!TeamVisibility::Hidden.supports_invite_link());
    }
}
