use std::time::{Duration, SystemTime};

#[cfg(not(target_family = "wasm"))]
use warpui_core::App;

use super::*;

fn make_manager(keys: ApiKeys) -> ApiKeyManager {
    make_manager_with_grok(keys, None)
}

#[test]
fn llm_provider_parses_supported_api_key_provider_names() {
    assert_eq!(
        LLMProvider::from_api_key_slug("anthropic"),
        Ok(LLMProvider::Anthropic)
    );
    assert_eq!(
        LLMProvider::from_api_key_slug("open-ai"),
        Ok(LLMProvider::OpenAI)
    );
    assert_eq!(
        LLMProvider::from_api_key_slug("google"),
        Ok(LLMProvider::Google)
    );
    assert_eq!(LLMProvider::from_api_key_slug("grok"), Ok(LLMProvider::Xai));
}

#[test]
fn persisted_provider_api_key_updates_request_state() {
    warpui_core::App::test((), |mut app| async move {
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
            warp_core::telemetry::testing::MockTelemetryContextProvider::register(ctx);
        });
        let manager = app.add_singleton_model(ApiKeyManager::new);

        manager
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(
                    LLMProvider::Anthropic,
                    Some("sk-ant-test".to_owned()),
                    ctx,
                )
            })
            .expect("no-op secure storage should accept the provider key");

        manager.read(&app, |manager, _| {
            let request_keys = manager
                .api_keys_for_request(true, false, None)
                .expect("persisted provider key should be available to requests");
            assert_eq!(request_keys.anthropic, "sk-ant-test");
        });
    });
}

#[test]
fn persisted_provider_api_key_can_be_cleared() {
    warpui_core::App::test((), |mut app| async move {
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
            warp_core::telemetry::testing::MockTelemetryContextProvider::register(ctx);
        });
        let manager = app.add_singleton_model(ApiKeyManager::new);

        manager
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(
                    LLMProvider::Anthropic,
                    Some("sk-ant-test".to_owned()),
                    ctx,
                )?;
                manager.persist_provider_key(LLMProvider::Anthropic, None, ctx)
            })
            .expect("no-op secure storage should clear the provider key");

        manager.read(&app, |manager, _| {
            assert_eq!(manager.keys().anthropic, None);
        });
    });
}
#[test]
fn llm_provider_rejects_unsupported_api_key_provider() {
    assert_eq!(
        LLMProvider::from_api_key_slug("openrouter"),
        Err("provider must be one of: anthropic, openai, google, grok".to_owned())
    );
}

#[test]
fn custom_model_providers_preserves_configured_schema() {
    let mut endpoint = endpoint_with_keys(
        "Anthropic",
        "https://custom.io",
        "ep-key",
        &[("claude", None, "uuid-1")],
    );
    endpoint.schema = CustomEndpointSchema::AnthropicMessages;
    let mgr = make_manager(ApiKeys {
        custom_endpoints: vec![endpoint],
        ..Default::default()
    });

    let provider = &mgr
        .custom_model_providers_for_request(true)
        .expect("configured endpoint should be sent")
        .providers[0];
    assert_eq!(
        provider.schema,
        CustomEndpointSchema::AnthropicMessages as i32
    );
}

fn make_manager_with_grok(keys: ApiKeys, grok_tokens: Option<GrokTokens>) -> ApiKeyManager {
    ApiKeyManager {
        keys,
        grok_tokens,
        #[cfg(not(target_family = "wasm"))]
        grok_refresh_allowed: false,
        #[cfg(not(target_family = "wasm"))]
        grok_refresh_waiters: None,
        aws_credentials_state: AwsCredentialsState::Missing,
        aws_credentials_refresh_strategy: AwsCredentialsRefreshStrategy::default(),
        geap_team_states: HashMap::new(),
        secure_storage_write_version: 0,
        grok_secure_storage_write_version: 0,
    }
}

/// Seeds `geap_binding()`'s team with the given state, matching how a single-team account
/// exercises the manager in most of the tests below. Tests that need to prove cross-team
/// isolation seed a second team's entry explicitly instead of using this helper.
fn make_manager_with_geap(geap_credentials_state: GeapCredentialsState) -> ApiKeyManager {
    let mut manager = make_manager(ApiKeys::default());
    manager
        .geap_team_states
        .entry(geap_binding().team_uid)
        .or_default()
        .credentials = geap_credentials_state;
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
        team_uid: "team-a".into(),
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
        schema: CustomEndpointSchema::default(),
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
#[test]
fn serde_legacy_endpoint_defaults_to_chat_completions() {
    let endpoint: CustomEndpoint = serde_json::from_str(
        r#"{"name":"legacy","url":"https://example.com","api_key":"key","models":[]}"#,
    )
    .unwrap();
    assert_eq!(endpoint.schema, CustomEndpointSchema::OpenaiChatCompletions);
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
    assert_eq!(p.schema, CustomEndpointSchema::OpenaiChatCompletions as i32);
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
    assert!(
        mgr.api_keys_for_request(false, false, Some(geap_gate()))
            .is_none()
    );
}

#[test]
fn api_keys_for_request_omits_geap_token_for_non_loaded_states() {
    for state in [
        GeapCredentialsState::Missing,
        GeapCredentialsState::Disabled,
        GeapCredentialsState::Unconfigured,
        GeapCredentialsState::Failed {
            error: LoadGeapCredentialsError::ExchangeToken {
                status: None,
                detail: "boom".into(),
            },
        },
    ] {
        let mgr = make_manager_with_geap(state);
        assert!(
            mgr.api_keys_for_request(false, false, Some(geap_gate()))
                .is_none()
        );
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

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_expired_refresh_eligibility_requires_expired_matching_binding() {
    let binding = geap_gate();
    let expired = make_manager_with_geap(geap_loaded("expired", Some(0)));
    assert!(expired.geap_expired_refresh_eligibility(&binding));

    let valid = make_manager_with_geap(geap_loaded("valid", Some(3600)));
    assert!(!valid.geap_expired_refresh_eligibility(&binding));

    let refreshing = make_manager_with_geap(GeapCredentialsState::Refreshing {
        previous: Some((geap_credentials("expired", Some(0)), binding.clone())),
    });
    assert!(refreshing.geap_expired_refresh_eligibility(&binding));

    let first_mint = make_manager_with_geap(GeapCredentialsState::Refreshing { previous: None });
    assert!(!first_mint.geap_expired_refresh_eligibility(&binding));

    let mut mismatched = binding.clone();
    mismatched.user_uid = "different-user".into();
    assert!(!expired.geap_expired_refresh_eligibility(&mismatched));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn begin_expired_geap_refresh_is_single_flight() {
    App::test((), |mut app| async move {
        let manager = app.add_model(|_| make_manager_with_geap(geap_loaded("expired", Some(0))));
        manager.update(&mut app, |manager, ctx| {
            let binding = geap_gate();
            let mut kickoff_count = 0;
            // The kickoff stands in for the app-layer mint: committing to mint
            // is what installs the waiter and opens the single-flight window.
            let first = manager.begin_expired_geap_refresh(&binding, ctx, |manager, waiter, _| {
                kickoff_count += 1;
                manager.install_geap_refresh_waiter(&binding.team_uid, Some(waiter));
            });
            let second = manager.begin_expired_geap_refresh(&binding, ctx, |manager, waiter, _| {
                kickoff_count += 1;
                manager.install_geap_refresh_waiter(&binding.team_uid, Some(waiter));
            });

            assert!(first.is_some());
            assert!(second.is_some());
            // The second request attached to the in-flight mint instead of
            // starting its own.
            assert_eq!(kickoff_count, 1);
            assert_eq!(
                manager.take_geap_refresh_waiters(&binding.team_uid).len(),
                2
            );
            // Taking the waiters closes the window.
            assert!(
                manager
                    .take_geap_refresh_waiters(&binding.team_uid)
                    .is_empty()
            );
        });
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn begin_expired_geap_refresh_is_independent_across_teams() {
    // Companion to `begin_expired_geap_refresh_is_single_flight`: that test proves
    // same-team coalescing; this proves the opposite must NOT happen across teams. If the
    // team-keyed waiter map ever regressed to one process-wide list, team B would coalesce
    // behind team A's in-flight mint and resume with team A's credential — exactly the
    // cross-team leak this PR exists to prevent — while every same-team test still passes.
    App::test((), |mut app| async move {
        let binding_a = geap_gate();
        let mut binding_b = geap_gate();
        binding_b.team_uid = "team-b".into();

        let manager = app.add_model(|_| {
            let mut manager = make_manager(ApiKeys::default());
            manager
                .geap_team_states
                .entry(binding_a.team_uid.clone())
                .or_default()
                .credentials = geap_loaded("expired-a", Some(0));
            manager
                .geap_team_states
                .entry(binding_b.team_uid.clone())
                .or_default()
                .credentials = GeapCredentialsState::Loaded {
                credentials: geap_credentials("expired-b", Some(0)),
                loaded_at: SystemTime::now(),
                minted_for: binding_b.clone(),
            };
            manager
        });
        manager.update(&mut app, |manager, ctx| {
            let mut kickoff_count = 0;
            let rx_a = manager.begin_expired_geap_refresh(&binding_a, ctx, |manager, waiter, _| {
                kickoff_count += 1;
                manager.install_geap_refresh_waiter(&binding_a.team_uid, Some(waiter));
            });
            let rx_b = manager.begin_expired_geap_refresh(&binding_b, ctx, |manager, waiter, _| {
                kickoff_count += 1;
                manager.install_geap_refresh_waiter(&binding_b.team_uid, Some(waiter));
            });

            assert!(rx_a.is_some());
            assert!(rx_b.is_some());
            // Each team started its own mint instead of team B attaching to team A's.
            assert_eq!(kickoff_count, 2);
            assert_eq!(
                manager.take_geap_refresh_waiters(&binding_a.team_uid).len(),
                1
            );
            assert_eq!(
                manager.take_geap_refresh_waiters(&binding_b.team_uid).len(),
                1
            );
        });
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn declined_geap_kickoff_leaves_no_in_flight_window() {
    App::test((), |mut app| async move {
        let manager = app.add_model(|_| make_manager_with_geap(geap_loaded("expired", Some(0))));
        manager.update(&mut app, |manager, ctx| {
            let binding = geap_gate();
            // A kickoff that hits one of its own guards returns without
            // minting, dropping the sender rather than installing it.
            let receiver = manager.begin_expired_geap_refresh(&binding, ctx, |_, _waiter, _| {});
            assert!(receiver.is_some());
            // No window was opened, so a later request starts a fresh kickoff
            // instead of attaching to a mint that is not running. This is what
            // makes "waiters present" mean "mint in flight".
            assert!(
                manager
                    .take_geap_refresh_waiters(&binding.team_uid)
                    .is_empty()
            );
        });
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_mint_failure_cooldown_suppresses_the_blocking_wait() {
    let binding = geap_gate();
    let mut manager = make_manager_with_geap(geap_loaded("expired", Some(0)));
    assert!(manager.geap_expired_refresh_eligibility(&binding));

    // A failed mint restores the expired credential, so without the cooldown
    // every following request would block on a mint that is failing.
    manager.record_geap_mint_failure(&binding.team_uid);
    assert!(!manager.geap_expired_refresh_eligibility(&binding));

    // A later success reopens the blocking path.
    manager.clear_geap_mint_failure(&binding.team_uid);
    assert!(manager.geap_expired_refresh_eligibility(&binding));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn geap_mint_failure_cooldown_is_scoped_to_its_own_team() {
    // Team B's cooldown must not suppress team A's blocking wait: the failure map is
    // keyed by team, not shared process-wide state.
    let binding_a = geap_gate();
    let mut binding_b = geap_gate();
    binding_b.team_uid = "team-b".into();

    let mut manager = make_manager(ApiKeys::default());
    manager
        .geap_team_states
        .entry(binding_a.team_uid.clone())
        .or_default()
        .credentials = geap_loaded("expired-a", Some(0));
    manager
        .geap_team_states
        .entry(binding_b.team_uid.clone())
        .or_default()
        .credentials = GeapCredentialsState::Loaded {
        credentials: geap_credentials("expired-b", Some(0)),
        loaded_at: SystemTime::now(),
        minted_for: binding_b.clone(),
    };

    manager.record_geap_mint_failure(&binding_a.team_uid);
    assert!(!manager.geap_expired_refresh_eligibility(&binding_a));
    assert!(manager.geap_expired_refresh_eligibility(&binding_b));
}

#[test]
fn geap_credentials_for_team_a_never_leaks_to_team_b() {
    // The crux of PR 3B: two teams' credentials must not collide in the shared
    // `ApiKeyManager` singleton.
    let binding_a = geap_gate();
    let mut binding_b = geap_gate();
    binding_b.team_uid = "team-b".into();

    let mut manager = make_manager(ApiKeys::default());
    manager
        .geap_team_states
        .entry(binding_a.team_uid.clone())
        .or_default()
        .credentials = geap_loaded("token-a", Some(3600));

    // Team B has never minted, so it must see nothing — not team A's token.
    assert!(
        manager
            .api_keys_for_request(false, false, Some(binding_b.clone()))
            .is_none()
    );
    assert!(matches!(
        manager.geap_credentials_state_for_team(&binding_b.team_uid),
        GeapCredentialsState::Missing
    ));

    // Team A's own request still sees its own token.
    let result = manager
        .api_keys_for_request(false, false, Some(binding_a))
        .unwrap();
    assert_eq!(
        result.google_cloud_credentials.unwrap().access_token,
        "token-a"
    );
}

#[test]
fn geap_credentials_state_zero_arg_reports_indeterminate_across_two_teams() {
    let binding_a = geap_gate();
    let mut binding_b = geap_gate();
    binding_b.team_uid = "team-b".into();

    let mut manager = make_manager(ApiKeys::default());
    assert!(matches!(
        manager.geap_credentials_state(),
        GeapCredentialsState::Missing
    ));

    manager
        .geap_team_states
        .entry(binding_a.team_uid.clone())
        .or_default()
        .credentials = geap_loaded("token-a", Some(3600));
    // Exactly one team has minted: the zero-arg read can answer unambiguously.
    assert!(matches!(
        manager.geap_credentials_state(),
        GeapCredentialsState::Loaded { .. }
    ));

    manager
        .geap_team_states
        .entry(binding_b.team_uid.clone())
        .or_default()
        .credentials = geap_loaded("token-b", Some(3600));
    // A second team minted: the zero-arg read must not guess between them, and must
    // never report `Missing` (both teams plainly have real credentials).
    assert!(matches!(
        manager.geap_credentials_state(),
        GeapCredentialsState::Indeterminate
    ));
}

// ── grok expiry + blocking-refresh eligibility ──────────────────

#[cfg(not(target_family = "wasm"))]
fn expired_grok_tokens() -> GrokTokens {
    // Already past hard expiry, with a refresh token available.
    GrokTokens {
        access_token: "stale-access".into(),
        refresh_token: Some("refresh".into()),
        expires_at: Some(SystemTime::now() - Duration::from_secs(60)),
        connected_at: None,
    }
}

#[test]
fn grok_is_expired_semantics() {
    // Past hard expiry.
    assert!(
        GrokTokens {
            expires_at: Some(SystemTime::now() - Duration::from_secs(1)),
            ..Default::default()
        }
        .is_expired()
    );
    // Still valid, even if near expiry (within the proactive lead window).
    assert!(!grok_tokens("tok", Some(60)).is_expired());
    // Unknown expiry is never considered expired.
    assert!(!grok_tokens("tok", None).is_expired());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn grok_expired_refresh_token_returns_token_when_expired() {
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(expired_grok_tokens()));
    assert_eq!(
        mgr.grok_expired_refresh_token(true),
        Some("refresh".to_string())
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn grok_expired_refresh_token_none_when_byo_disabled() {
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(expired_grok_tokens()));
    assert_eq!(mgr.grok_expired_refresh_token(false), None);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn grok_expired_refresh_token_none_when_near_expiry_but_valid() {
    // Within the proactive lead window but not yet expired: the background timer
    // handles this, so the blocking path stays out of it.
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(grok_tokens("near", Some(60))));
    assert_eq!(mgr.grok_expired_refresh_token(true), None);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn grok_expired_refresh_token_none_when_no_tokens() {
    let mgr = make_manager_with_grok(ApiKeys::default(), None);
    assert_eq!(mgr.grok_expired_refresh_token(true), None);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn grok_expired_refresh_token_none_when_no_refresh_token() {
    let mut tokens = expired_grok_tokens();
    tokens.refresh_token = None;
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(tokens));
    assert_eq!(mgr.grok_expired_refresh_token(true), None);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn grok_expired_refresh_token_none_when_no_expiry() {
    // A token with no known expiry is never considered expired.
    let mgr = make_manager_with_grok(ApiKeys::default(), Some(grok_tokens("no-expiry", None)));
    assert_eq!(mgr.grok_expired_refresh_token(true), None);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn grok_expired_refresh_token_ignores_in_flight_refresh() {
    // Eligibility is independent of whether a refresh is already running: a
    // request must still be able to attach to the in-flight refresh (that
    // coordination happens in `begin_expired_grok_refresh`), rather than being
    // told no refresh is needed and sending the expired token.
    let mut mgr = make_manager_with_grok(ApiKeys::default(), Some(expired_grok_tokens()));
    mgr.grok_refresh_waiters = Some(Vec::new());
    assert_eq!(
        mgr.grok_expired_refresh_token(true),
        Some("refresh".to_string())
    );
}
