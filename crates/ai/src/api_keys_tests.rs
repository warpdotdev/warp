use std::time::{Duration, SystemTime};

use super::*;

fn make_manager(keys: ApiKeys) -> ApiKeyManager {
    make_manager_with_grok(keys, None)
}

fn make_manager_with_grok(keys: ApiKeys, grok_tokens: Option<GrokTokens>) -> ApiKeyManager {
    ApiKeyManager {
        keys,
        grok_tokens,
        #[cfg(not(target_family = "wasm"))]
        grok_refresh_allowed: false,
        #[cfg(not(target_family = "wasm"))]
        grok_refresh_in_flight: false,
        aws_credentials_state: AwsCredentialsState::Missing,
        aws_credentials_refresh_strategy: AwsCredentialsRefreshStrategy::default(),
        geap_credentials_state: GeapCredentialsState::Missing,
        secure_storage_write_version: 0,
        grok_secure_storage_write_version: 0,
    }
}

fn make_manager_with_geap(geap_credentials_state: GeapCredentialsState) -> ApiKeyManager {
    let mut manager = make_manager(ApiKeys::default());
    manager.geap_credentials_state = geap_credentials_state;
    manager
}

fn grok_tokens(access_token: &str, expires_in: Option<u64>) -> GrokTokens {
    GrokTokens {
        access_token: access_token.into(),
        refresh_token: Some("refresh".into()),
        expires_at: expires_in.map(|secs| SystemTime::now() + Duration::from_secs(secs)),
        connected_at: None,
    }
}

fn geap_credentials(access_token: &str, expires_in: Option<u64>) -> GeapCredentials {
    GeapCredentials::new(
        access_token.into(),
        expires_in.map(|secs| SystemTime::now() + Duration::from_secs(secs)),
    )
}

fn geap_binding() -> GeapMintBinding {
    GeapMintBinding {
        user_uid: "user-1".into(),
        audience:
            "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/providers/q"
                .into(),
        federation: GeapFederation::ServiceAccount {
            email: "sa@proj.iam.gserviceaccount.com".into(),
        },
    }
}

// The expected binding the request build site passes in is the same type as
// the stored `minted_for`, so the attach check is a plain `==`.
fn geap_gate() -> GeapMintBinding {
    geap_binding()
}

fn geap_loaded(access_token: &str, expires_in: Option<u64>) -> GeapCredentialsState {
    GeapCredentialsState::Loaded {
        credentials: geap_credentials(access_token, expires_in),
        loaded_at: SystemTime::now(),
        minted_for: geap_binding(),
    }
}

fn endpoint(
    name: &str,
    url: &str,
    api_key: &str,
    models: &[(&str, Option<&str>)],
) -> CustomEndpoint {
    endpoint_with_keys(
        name,
        url,
        api_key,
        &models
            .iter()
            .enumerate()
            .map(|(i, (n, a))| (*n, *a, format!("cfg-{i}")))
            .collect::<Vec<_>>()
            .iter()
            .map(|(n, a, k)| (*n, *a, k.as_str()))
            .collect::<Vec<_>>(),
    )
}

fn endpoint_with_keys(
    name: &str,
    url: &str,
    api_key: &str,
    models: &[(&str, Option<&str>, &str)],
) -> CustomEndpoint {
    CustomEndpoint {
        name: name.into(),
        url: url.into(),
        api_key: api_key.into(),
        models: models
            .iter()
            .map(|(n, a, cfg)| CustomEndpointModel {
                name: (*n).into(),
                alias: a.map(|s| s.into()),
                config_key: (*cfg).into(),
            })
            .collect(),
    }
}

// ── serde round-trip ────────────────────────────────────────────

#[test]
fn serde_round_trip_empty() {
    let keys = ApiKeys::default();
    let json = serde_json::to_string(&keys).unwrap();
    let deser: ApiKeys = serde_json::from_str(&json).unwrap();
    assert_eq!(keys, deser);
}

#[test]
fn serde_round_trip_with_provider_keys() {
    let keys = ApiKeys {
        openai: Some("sk-openai".into()),
        anthropic: Some("sk-ant-abc".into()),
        google: Some("AIzaSy123".into()),
        open_router: Some("sk-or-xxx".into()),
        custom_endpoints: vec![],
    };
    let json = serde_json::to_string(&keys).unwrap();
    let deser: ApiKeys = serde_json::from_str(&json).unwrap();
    assert_eq!(keys, deser);
}

#[test]
fn serde_round_trip_with_custom_endpoints() {
    let keys = ApiKeys {
        openai: None,
        anthropic: None,
        google: None,
        open_router: None,
        custom_endpoints: vec![
            endpoint("ep1", "https://a.io/v1", "key1", &[("gpt-4", Some("fast"))]),
            endpoint(
                "ep2",
                "https://b.io/v1",
                "key2",
                &[("llama-70b", None), ("mixtral", Some("mix"))],
            ),
        ],
    };
    let json = serde_json::to_string(&keys).unwrap();
    let deser: ApiKeys = serde_json::from_str(&json).unwrap();
    assert_eq!(keys, deser);
}

#[test]
fn serde_ignores_unknown_fields() {
    let json = r#"{"openai":"sk-x","unknown_field":"value","custom_endpoints":[]}"#;
    let keys: ApiKeys = serde_json::from_str(json).unwrap();
    assert_eq!(keys.openai, Some("sk-x".into()));
    assert!(keys.custom_endpoints.is_empty());
}

// ── has_any_key ─────────────────────────────────────────────────

#[test]
fn has_any_key_false_when_empty() {
    assert!(!ApiKeys::default().has_any_key());
}

#[test]
fn has_any_key_true_for_openai_only() {
    let keys = ApiKeys {
        openai: Some("sk-x".into()),
        ..Default::default()
    };
    assert!(keys.has_any_key());
}

#[test]
fn has_any_key_true_for_custom_endpoints_only() {
    let keys = ApiKeys {
        custom_endpoints: vec![endpoint("ep", "https://a.io", "key", &[("m", None)])],
        ..Default::default()
    };
    assert!(keys.has_any_key());
}

#[test]
fn has_any_key_false_for_endpoint_with_empty_api_key() {
    let keys = ApiKeys {
        custom_endpoints: vec![endpoint("ep", "https://a.io", "", &[("m", None)])],
        ..Default::default()
    };
    assert!(!keys.has_any_key());
}

// ── provider_key_count ─────────────────────────────────────────

#[test]
fn provider_key_count_zero_when_empty() {
    assert_eq!(ApiKeys::default().provider_key_count(), 0);
}

#[test]
fn provider_key_count_counts_each_provider_key() {
    let keys = ApiKeys {
        openai: Some("sk-o".into()),
        anthropic: Some("sk-a".into()),
        google: Some("AIza".into()),
        open_router: Some("sk-or".into()),
        custom_endpoints: vec![],
    };
    assert_eq!(keys.provider_key_count(), 4);
}

#[test]
fn provider_key_count_ignores_blank_keys_and_endpoints() {
    let keys = ApiKeys {
        openai: Some("sk-o".into()),
        anthropic: Some("   ".into()),
        google: None,
        open_router: None,
        custom_endpoints: vec![endpoint("ep", "https://a.io", "k", &[("m", None)])],
    };
    // Only the non-blank OpenAI key counts; the whitespace Anthropic key and the
    // custom endpoint are excluded.
    assert_eq!(keys.provider_key_count(), 1);
}

// ── custom_model_providers_for_request ──────────────────────────

#[test]
fn custom_model_providers_none_when_empty() {
    let mgr = make_manager(ApiKeys::default());
    assert!(mgr.custom_model_providers_for_request(true).is_none());
}

#[test]
fn custom_model_providers_none_when_byo_disabled() {
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![endpoint("ep", "https://a.io", "k", &[("m", None)])],
        ..Default::default()
    });
    assert!(mgr.custom_model_providers_for_request(false).is_none());
}

#[test]
fn custom_model_providers_populates_single_endpoint() {
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![endpoint_with_keys(
            "My EP",
            "https://custom.io/v1",
            "ep-key",
            &[("big-model", Some("alias"), "uuid-1")],
        )],
        ..Default::default()
    });
    let result = mgr.custom_model_providers_for_request(true).unwrap();
    assert_eq!(result.providers.len(), 1);
    let p = &result.providers[0];
    assert_eq!(p.base_url, "https://custom.io/v1");
    assert_eq!(p.api_key, "ep-key");
    assert_eq!(p.models.len(), 1);
    assert_eq!(p.models[0].slug, "big-model");
    assert_eq!(p.models[0].config_key, "uuid-1");
}

#[test]
fn multiple_endpoints_all_serialize() {
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![
            endpoint_with_keys(
                "ep1",
                "https://a.io",
                "k1",
                &[("gpt-4", Some("fast"), "uuid-a")],
            ),
            endpoint_with_keys(
                "ep2",
                "https://b.io",
                "k2",
                &[
                    ("llama-70b", None, "uuid-b"),
                    ("mixtral", Some("mix"), "uuid-c"),
                ],
            ),
        ],
        ..Default::default()
    });
    let result = mgr.custom_model_providers_for_request(true).unwrap();
    assert_eq!(result.providers.len(), 2);
    assert_eq!(result.providers[0].base_url, "https://a.io");
    assert_eq!(result.providers[0].models[0].config_key, "uuid-a");
    assert_eq!(result.providers[1].base_url, "https://b.io");
    assert_eq!(result.providers[1].models.len(), 2);
    assert_eq!(result.providers[1].models[0].slug, "llama-70b");
    assert_eq!(result.providers[1].models[0].config_key, "uuid-b");
    assert_eq!(result.providers[1].models[1].config_key, "uuid-c");
}

#[test]
fn byok_disabled_returns_none_even_with_endpoints() {
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![endpoint("ep", "https://a.io", "k", &[("m", None)])],
        ..Default::default()
    });
    assert!(mgr.custom_model_providers_for_request(false).is_none());
}

#[test]
fn empty_api_key_endpoints_are_skipped() {
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![
            endpoint_with_keys("empty", "https://a.io", "", &[("m", None, "uuid-x")]),
            endpoint_with_keys("ok", "https://b.io", "k", &[("m", None, "uuid-y")]),
        ],
        ..Default::default()
    });
    let result = mgr.custom_model_providers_for_request(true).unwrap();
    assert_eq!(result.providers.len(), 1);
    assert_eq!(result.providers[0].base_url, "https://b.io");
}

#[test]
fn endpoints_with_only_empty_models_are_skipped() {
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![endpoint_with_keys(
            "ep",
            "https://a.io",
            "k",
            &[("", None, "uuid-z")],
        )],
        ..Default::default()
    });
    assert!(mgr.custom_model_providers_for_request(true).is_none());
}

// ── display_label fallback ─────────────────────────────────────

#[test]
fn display_label_uses_alias_when_present() {
    let m = CustomEndpointModel {
        name: "raw-name".into(),
        alias: Some("My Alias".into()),
        config_key: "k".into(),
    };
    assert_eq!(m.display_label(), "My Alias");
}

#[test]
fn display_label_falls_back_to_name_when_alias_missing() {
    let m = CustomEndpointModel {
        name: "raw-name".into(),
        alias: None,
        config_key: "k".into(),
    };
    assert_eq!(m.display_label(), "raw-name");
}

#[test]
fn display_label_falls_back_to_name_when_alias_is_whitespace() {
    let m = CustomEndpointModel {
        name: "raw-name".into(),
        alias: Some("   ".into()),
        config_key: "k".into(),
    };
    assert_eq!(m.display_label(), "raw-name");
}

// ── api_keys_for_request ────────────────────────────────────────

#[test]
fn api_keys_for_request_none_when_empty() {
    let mgr = make_manager(ApiKeys::default());
    assert!(mgr.api_keys_for_request(true, false, None).is_none());
}

#[test]
fn api_keys_for_request_populates_provider_keys() {
    let mgr = make_manager(ApiKeys {
        openai: Some("sk-o".into()),
        anthropic: Some("sk-a".into()),
        ..Default::default()
    });
    let result = mgr.api_keys_for_request(true, false, None).unwrap();
    assert_eq!(result.openai, "sk-o");
    assert_eq!(result.anthropic, "sk-a");
    assert!(result.google.is_empty());
}

#[test]
fn api_keys_for_request_omits_keys_when_byo_disabled() {
    let mgr = make_manager(ApiKeys {
        openai: Some("sk-o".into()),
        ..Default::default()
    });
    // With BYO disabled and no other credentials, returns None.
    assert!(mgr.api_keys_for_request(false, false, None).is_none());
}

#[test]
fn api_keys_for_request_none_for_custom_endpoints_only() {
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![endpoint("ep", "https://a.io", "k", &[("m", None)])],
        ..Default::default()
    });
    assert!(mgr.api_keys_for_request(true, false, None).is_none());
}

// ── grok oauth token ────────────────────────────────────────────

#[test]
fn grok_access_token_present_without_expiry() {
    let t = GrokTokens {
        access_token: "tok".into(),
        ..Default::default()
    };
    assert_eq!(t.access_token_for_request(), Some("tok"));
}

#[test]
fn grok_access_token_blank_is_none() {
    let t = GrokTokens {
        access_token: "   ".into(),
        ..Default::default()
    };
    assert_eq!(t.access_token_for_request(), None);
}

#[test]
fn grok_access_token_near_expiry_still_sent() {
    // Expired tokens are still sent; the server is the authority on validity.
    let t = grok_tokens("tok", Some(0));
    assert_eq!(t.access_token_for_request(), Some("tok"));
}

#[test]
fn grok_access_token_far_future_is_some() {
    let t = grok_tokens("tok", Some(3600));
    assert_eq!(t.access_token_for_request(), Some("tok"));
}

#[test]
fn grok_needs_refresh_within_lead_time() {
    assert!(grok_tokens("tok", Some(30)).needs_refresh(Duration::from_secs(300)));
    assert!(!grok_tokens("tok", Some(3600)).needs_refresh(Duration::from_secs(300)));
    // Expired tokens still need a refresh.
    assert!(grok_tokens("tok", Some(0)).needs_refresh(Duration::from_secs(300)));
    // Unknown expiry never reports as needing refresh.
    assert!(!grok_tokens("tok", None).needs_refresh(Duration::from_secs(300)));
}

#[test]
fn api_keys_for_request_includes_grok_token() {
    let mgr = make_manager_with_grok(
        ApiKeys::default(),
        Some(grok_tokens("grok-abc", Some(3600))),
    );
    let result = mgr.api_keys_for_request(true, false, None).unwrap();
    assert_eq!(result.grok_oauth_access_token, "grok-abc");
    assert!(result.anthropic.is_empty());
}

#[test]
fn api_keys_for_request_omits_grok_token_when_byo_disabled() {
    // The Grok subscription is user-provided auth, so it follows the BYO
    // policy gate: with BYO disabled and no other credentials, returns None.
    let mgr = make_manager_with_grok(
        ApiKeys::default(),
        Some(grok_tokens("grok-abc", Some(3600))),
    );
    assert!(mgr.api_keys_for_request(false, false, None).is_none());
}

#[test]
fn api_keys_for_request_includes_expired_grok_token() {
    // Expired tokens are still sent in requests; the server rejects truly
    // invalid ones and the background refresh replaces them.
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(grok_tokens("grok-abc", Some(0))));
    let result = mgr.api_keys_for_request(true, false, None).unwrap();
    assert_eq!(result.grok_oauth_access_token, "grok-abc");
}

#[test]
fn has_grok_subscription_false_when_not_connected() {
    let mgr = make_manager(ApiKeys::default());
    assert!(!mgr.has_grok_subscription());
}

#[test]
fn has_grok_subscription_true_when_connected() {
    let mgr = make_manager_with_grok(
        ApiKeys::default(),
        Some(grok_tokens("grok-abc", Some(3600))),
    );
    assert!(mgr.has_grok_subscription());
}

#[test]
fn has_grok_subscription_true_for_expired_token() {
    // A connected subscription still counts even when its token is past expiry:
    // the token is sent anyway and the server is the authority on validity.
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(grok_tokens("grok-abc", Some(0))));
    assert!(mgr.has_grok_subscription());
}

#[test]
fn has_grok_subscription_false_when_token_blank() {
    // A blank token can't be sent, so it does not count as a usable credential.
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(grok_tokens("   ", None)));
    assert!(!mgr.has_grok_subscription());
}

// ── ApiKeyManager::has_any_key ──────────────────

#[test]
fn manager_has_any_key_false_when_no_keys_and_no_grok() {
    let mgr = make_manager(ApiKeys::default());
    assert!(!mgr.has_any_key());
}

#[test]
fn manager_has_any_key_true_for_pasted_key_without_grok() {
    let mgr = make_manager(ApiKeys {
        openai: Some("sk-x".into()),
        ..Default::default()
    });
    assert!(mgr.has_any_key());
}

#[test]
fn manager_has_any_key_true_for_connected_grok_without_pasted_key() {
    // The crux: a connected Grok subscription counts even with no pasted keys,
    // matching how it's sent as a BYO credential on requests.
    let mgr = make_manager_with_grok(
        ApiKeys::default(),
        Some(grok_tokens("grok-abc", Some(3600))),
    );
    assert!(mgr.has_any_key());
}

#[test]
fn manager_has_any_key_false_for_blank_grok_and_no_keys() {
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(grok_tokens("   ", None)));
    assert!(!mgr.has_any_key());
}

// ── geap credentials ────────────────────────────────────────────

#[test]
fn geap_access_token_present_without_expiry() {
    let credentials = GeapCredentials::new("tok".into(), None);
    assert_eq!(credentials.access_token_for_request(), Some("tok"));
}

#[test]
fn geap_access_token_blank_is_none() {
    let credentials = GeapCredentials::new("   ".into(), None);
    assert_eq!(credentials.access_token_for_request(), None);
}

#[test]
fn geap_access_token_near_expiry_still_sent() {
    // Expired tokens are still sent; Google is the authority on validity.
    let credentials = geap_credentials("tok", Some(0));
    assert_eq!(credentials.access_token_for_request(), Some("tok"));
}

#[test]
fn geap_needs_refresh_lead_time_boundaries() {
    // Within the 5-minute lead window.
    assert!(geap_credentials("tok", Some(30)).needs_refresh());
    // Comfortably fresh.
    assert!(!geap_credentials("tok", Some(3600)).needs_refresh());
    // Already expired -> still needs a refresh.
    assert!(geap_credentials("tok", Some(0)).needs_refresh());
    // Unknown expiry never reports as needing a refresh.
    assert!(!geap_credentials("tok", None).needs_refresh());
}

#[test]
fn api_keys_for_request_includes_geap_token_when_gate_and_binding_match() {
    let mgr = make_manager_with_geap(geap_loaded("geap-abc", Some(3600)));
    let result = mgr
        .api_keys_for_request(false, false, Some(geap_gate()))
        .unwrap();
    let credentials = result.google_cloud_credentials.unwrap();
    assert_eq!(credentials.access_token, "geap-abc");
    // The GEAP token is independent of the BYO key gate.
    assert!(result.anthropic.is_empty());
}

#[test]
fn api_keys_for_request_includes_expired_geap_token() {
    // Expired tokens are still attached — never silently dropped. Google
    // rejects truly invalid ones, which surfaces a recoverable error instead
    // of a silent fallback to another route.
    let mgr = make_manager_with_geap(geap_loaded("geap-abc", Some(0)));
    let result = mgr
        .api_keys_for_request(false, false, Some(geap_gate()))
        .unwrap();
    assert_eq!(
        result.google_cloud_credentials.unwrap().access_token,
        "geap-abc"
    );
}

#[test]
fn api_keys_for_request_omits_geap_token_without_gate() {
    // No gate (policy off at the call site) ⇒ no GEAP credentials, even when
    // a token is loaded.
    let mgr = make_manager_with_geap(geap_loaded("geap-abc", Some(3600)));
    assert!(mgr.api_keys_for_request(false, false, None).is_none());
}

#[test]
fn api_keys_for_request_omits_geap_token_on_binding_mismatch() {
    let mgr = make_manager_with_geap(geap_loaded("geap-abc", Some(3600)));

    // A different user (sign-out/account switch).
    let mut gate = geap_gate();
    gate.user_uid = "someone-else".into();
    assert!(mgr.api_keys_for_request(false, false, Some(gate)).is_none());

    // A different audience (admin changed the pool/provider).
    let mut gate = geap_gate();
    gate.audience = "//iam.googleapis.com/projects/2/locations/global/workloadIdentityPools/other/providers/other".into();
    assert!(mgr.api_keys_for_request(false, false, Some(gate)).is_none());

    // A different service account (admin changed impersonation target).
    let mut gate = geap_gate();
    gate.federation = GeapFederation::ServiceAccount {
        email: "other@proj.iam.gserviceaccount.com".into(),
    };
    assert!(mgr.api_keys_for_request(false, false, Some(gate)).is_none());
}

#[test]
fn api_keys_for_request_serves_previous_geap_token_while_refreshing() {
    // A re-mint in flight keeps serving the previous token — tokens stay
    // until replaced.
    let mgr = make_manager_with_geap(GeapCredentialsState::Refreshing {
        previous: Some((geap_credentials("geap-old", Some(10)), geap_binding())),
    });
    let result = mgr
        .api_keys_for_request(false, false, Some(geap_gate()))
        .unwrap();
    assert_eq!(
        result.google_cloud_credentials.unwrap().access_token,
        "geap-old"
    );
}

#[test]
fn api_keys_for_request_omits_geap_token_during_first_mint() {
    // The very first mint has nothing to serve yet.
    let mgr = make_manager_with_geap(GeapCredentialsState::Refreshing { previous: None });
    assert!(mgr
        .api_keys_for_request(false, false, Some(geap_gate()))
        .is_none());
}

#[test]
fn api_keys_for_request_omits_geap_token_for_non_loaded_states() {
    for state in [
        GeapCredentialsState::Missing,
        GeapCredentialsState::Disabled,
        GeapCredentialsState::Failed {
            error: LoadGeapCredentialsError::ExchangeToken {
                status: None,
                detail: "boom".into(),
            },
        },
    ] {
        let mgr = make_manager_with_geap(state);
        assert!(mgr
            .api_keys_for_request(false, false, Some(geap_gate()))
            .is_none());
    }
}

#[test]
fn api_keys_for_request_omits_geap_token_when_previous_binding_mismatches() {
    let mgr = make_manager_with_geap(GeapCredentialsState::Refreshing {
        previous: Some((geap_credentials("geap-old", Some(10)), geap_binding())),
    });
    let mut gate = geap_gate();
    gate.user_uid = "someone-else".into();
    assert!(mgr.api_keys_for_request(false, false, Some(gate)).is_none());
}
