use super::*;
use crate::server::ids::ServerId;

// `ServerId::from_string_lossy` requires exactly 22 characters.
const TEST_WORKSPACE_UID: &str = "workspace_uid123456789";

fn make_workspace(policy: Option<UsageVisibilityPolicy>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        ServerId::from_string_lossy(TEST_WORKSPACE_UID).into(),
        "Test Workspace".to_string(),
        None,
    );
    workspace.billing_metadata.tier.usage_visibility_policy = policy;
    workspace
}

fn policy(
    granularity: UsageVisibilityGranularity,
    max_prior_cycles: MaxPriorCycles,
) -> UsageVisibilityPolicy {
    UsageVisibilityPolicy {
        admin_granularity: granularity,
        max_prior_cycles,
    }
}

#[test]
fn missing_policy_returns_defaults_for_admin_and_non_admin() {
    let workspace = make_workspace(None);

    let as_admin = workspace.resolve_usage_visibility(true);
    assert_eq!(as_admin.granularity, UsageVisibilityGranularity::OwnOnly);
    assert_eq!(as_admin.max_prior_cycles, MaxPriorCycles::None);

    let as_non_admin = workspace.resolve_usage_visibility(false);
    assert_eq!(
        as_non_admin.granularity,
        UsageVisibilityGranularity::OwnOnly
    );
    assert_eq!(as_non_admin.max_prior_cycles, MaxPriorCycles::None);
}

#[test]
fn non_admin_collapses_granularity_but_keeps_max_prior_cycles() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::FullBreakdown,
        MaxPriorCycles::Limited(11),
    )));

    let resolved = workspace.resolve_usage_visibility(false);

    assert_eq!(resolved.granularity, UsageVisibilityGranularity::OwnOnly);
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Limited(11));
}

#[test]
fn admin_inherits_tier_team_aggregate_granularity() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::TeamAggregate,
        MaxPriorCycles::Limited(11),
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::TeamAggregate
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Limited(11));
}

#[test]
fn admin_inherits_tier_per_user_totals_unlimited() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::PerUserTotals,
        MaxPriorCycles::Unlimited,
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::PerUserTotals
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Unlimited);
}

#[test]
fn admin_inherits_tier_full_breakdown_unlimited() {
    let workspace = make_workspace(Some(policy(
        UsageVisibilityGranularity::FullBreakdown,
        MaxPriorCycles::Unlimited,
    )));

    let resolved = workspace.resolve_usage_visibility(true);

    assert_eq!(
        resolved.granularity,
        UsageVisibilityGranularity::FullBreakdown
    );
    assert_eq!(resolved.max_prior_cycles, MaxPriorCycles::Unlimited);
}

// ── teamByo projection (secret-less mirror) ──

#[test]
fn team_byo_graphql_payload_parses_into_secretless_model() {
    use warp_graphql::workspace::TeamByoSettings as GqlTeamByoSettings;

    // A teamByo projection exactly as the server ships it (camelCase GraphQL
    // JSON): one configured first-party key and one enabled endpoint with one
    // enabled model. `firstPartyKeys` mirrors the server list shape; the
    // admin-only credentialUid handles are not selected by the client query.
    let payload = r#"{
        "firstPartyEnabled": true,
        "endpointsEnabled": true,
        "allowUserKeys": false,
        "allowUserEndpoints": true,
        "firstPartyKeys": [
            { "provider": "OPENAI" }
        ],
        "endpoints": [
            {
                "uid": "endpoint-1",
                "name": "Team GPU",
                "enabled": true,
                "models": [
                    {
                        "configKey": "cfg-key-123",
                        "slug": "llama-3.1-70b",
                        "alias": "Fast",
                        "displayName": "Fast (Llama 3.1 70B)",
                        "enabled": true
                    }
                ]
            }
        ]
    }"#;

    // Parse through the cynic fragment, then convert via the same
    // `Option::map(Into::into)` path `gql_convert` uses for `teamByo`.
    let gql: Option<GqlTeamByoSettings> =
        Some(serde_json::from_str(payload).expect("teamByo payload should parse"));
    let parsed: Option<TeamByoSettings> = gql.map(Into::into);

    let settings = parsed.expect("teamByo present should map to Some");
    assert!(settings.first_party_enabled);
    assert!(settings.endpoints_enabled);
    assert!(!settings.allow_user_keys);
    assert!(settings.allow_user_endpoints);
    assert_eq!(settings.first_party_providers, vec![LLMProvider::OpenAI]);
    assert!(settings.has_first_party_key(&LLMProvider::OpenAI));
    assert!(!settings.has_first_party_key(&LLMProvider::Anthropic));
    assert!(!settings.has_first_party_key(&LLMProvider::Google));

    assert_eq!(settings.endpoints.len(), 1);
    let endpoint = &settings.endpoints[0];
    assert_eq!(endpoint.uid, "endpoint-1");
    assert_eq!(endpoint.name, "Team GPU");
    assert!(endpoint.enabled);

    assert_eq!(endpoint.models.len(), 1);
    let model = &endpoint.models[0];
    assert_eq!(model.config_key, "cfg-key-123");
    assert_eq!(model.slug, "llama-3.1-70b");
    assert_eq!(model.alias.as_deref(), Some("Fast"));
    assert_eq!(model.display_name, "Fast (Llama 3.1 70B)");
    assert!(model.enabled);
}

#[test]
fn team_byo_null_projection_maps_to_none() {
    // `teamByo: null` (non-enterprise / unconfigured) must map to `None`,
    // mirroring `gql_workspace_settings.team_byo.map(Into::into)`.
    let gql: Option<warp_graphql::workspace::TeamByoSettings> =
        serde_json::from_str("null").expect("null should parse");
    let parsed: Option<TeamByoSettings> = gql.map(Into::into);
    assert!(parsed.is_none());
}

#[test]
fn team_byo_endpoint_name_for_model_matches_config_key() {
    let settings = TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: false,
        allow_user_endpoints: false,
        first_party_providers: vec![],
        endpoints: vec![
            TeamByoEndpoint {
                uid: "ep-1".into(),
                name: "Team GPU".into(),
                enabled: true,
                models: vec![TeamByoEndpointModel {
                    config_key: "cfg-1".into(),
                    slug: "m1".into(),
                    alias: None,
                    display_name: "M1".into(),
                    enabled: true,
                }],
            },
            TeamByoEndpoint {
                uid: "ep-2".into(),
                name: "Team CPU".into(),
                enabled: true,
                models: vec![TeamByoEndpointModel {
                    config_key: "cfg-2".into(),
                    slug: "m2".into(),
                    alias: None,
                    display_name: "M2".into(),
                    enabled: true,
                }],
            },
        ],
    };

    assert_eq!(settings.endpoint_name_for_model("cfg-1"), Some("Team GPU"));
    assert_eq!(settings.endpoint_name_for_model("cfg-2"), Some("Team CPU"));
    assert_eq!(settings.endpoint_name_for_model("unknown"), None);
}

#[test]
fn team_byo_serialized_form_carries_no_secret_keys() {
    // Compile-time guarantee: `TeamByoSettings` has no api key / base url /
    // ciphertext field (it would not compile otherwise). Runtime guard: the
    // serialized projection never carries a secret-bearing key, so a synced or
    // cached `teamByo` can never leak one.
    let settings = TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: true,
        allow_user_keys: true,
        allow_user_endpoints: true,
        first_party_providers: vec![
            LLMProvider::OpenAI,
            LLMProvider::Anthropic,
            LLMProvider::Google,
        ],
        endpoints: vec![TeamByoEndpoint {
            uid: "ep".into(),
            name: "Team".into(),
            enabled: true,
            models: vec![TeamByoEndpointModel {
                config_key: "cfg".into(),
                slug: "m".into(),
                alias: None,
                display_name: "M".into(),
                enabled: true,
            }],
        }],
    };
    let json = serde_json::to_string(&settings)
        .expect("serialize")
        .to_lowercase();
    for forbidden in [
        "api_key",
        "apikey",
        "base_url",
        "baseurl",
        "ciphertext",
        "value_encrypted",
        "secret",
    ] {
        assert!(
            !json.contains(forbidden),
            "serialized teamByo must not contain `{forbidden}`: {json}"
        );
    }
}
