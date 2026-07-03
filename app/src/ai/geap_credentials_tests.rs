use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ai::api_keys::ApiKeyManager;
use settings::{PrivatePreferences, PublicPreferences};
use warp_managed_secrets::ManagedSecretManager;
use warpui::{AddSingletonModel, App};
use warpui_extras::user_preferences;

use super::*;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::server_api::ServerApiProvider;
use crate::workspaces::team::Team;
use crate::workspaces::workspace::{HostEnablementSetting, LlmHostSettings, Workspace};

// ── pure helpers ────────────────────────────────────────────────

const TEST_AUDIENCE: &str =
    "//iam.googleapis.com/projects/123456/locations/global/workloadIdentityPools/warp-pool/providers/warp-provider";
const TEST_SA_EMAIL: &str = "warp-geap@test-project.iam.gserviceaccount.com";

#[test]
fn mint_binding_from_parts_trims_and_normalizes() {
    let binding = geap_mint_binding_from_parts(
        "user-1".into(),
        Some(&format!("  {TEST_AUDIENCE} ")),
        Some(&format!(" {TEST_SA_EMAIL}  ")),
    )
    .expect("a configured audience yields a mintable binding");
    assert_eq!(binding.audience, TEST_AUDIENCE);
    assert_eq!(
        binding.federation,
        GeapFederation::ServiceAccount {
            email: TEST_SA_EMAIL.to_string()
        }
    );
}

#[test]
fn mint_binding_from_parts_requires_an_audience() {
    // Missing or blank audience -> not mintable (rests at Unconfigured/Missing).
    assert_eq!(
        geap_mint_binding_from_parts("user-1".into(), None, Some(TEST_SA_EMAIL)),
        None
    );
    assert_eq!(
        geap_mint_binding_from_parts("user-1".into(), Some("   "), Some(TEST_SA_EMAIL)),
        None
    );
}

#[test]
fn mint_binding_from_parts_uses_direct_wif_without_sa() {
    // A whitespace-only or missing SA email means "no impersonation"
    // (DirectWif), not an SA named "".
    let binding = geap_mint_binding_from_parts("user-1".into(), Some(TEST_AUDIENCE), Some("   "))
        .expect("audience present");
    assert_eq!(binding.federation, GeapFederation::DirectWif);

    let binding = geap_mint_binding_from_parts("user-1".into(), Some(TEST_AUDIENCE), None)
        .expect("audience present");
    assert_eq!(binding.federation, GeapFederation::DirectWif);
}

#[test]
fn sts_expires_at_prefers_expires_in_and_falls_back_to_jwt_expiry() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let jwt_expires_at = now + Duration::from_secs(900);

    // STS reported a lifetime: absolute expiry is now + expires_in.
    assert_eq!(
        sts_expires_at(Some(3600), jwt_expires_at, now),
        now + Duration::from_secs(3600)
    );
    // STS omitted it (allowed by RFC 8693): fall back to the JWT's own
    // expiry as a conservative bound.
    assert_eq!(sts_expires_at(None, jwt_expires_at, now), jwt_expires_at);
}

#[test]
fn impersonation_expiry_parses_rfc3339() {
    let parsed = parse_generate_access_token_expiry("2026-06-11T15:01:23Z").unwrap();
    assert!(parsed > SystemTime::UNIX_EPOCH);
}

#[test]
fn impersonation_expiry_rejects_invalid_timestamps() {
    let err = parse_generate_access_token_expiry("not-a-timestamp").unwrap_err();
    assert!(err.contains("invalid expireTime"));
}

#[test]
fn impersonation_response_parses_camel_case() {
    let response: GenerateAccessTokenResponse =
        serde_json::from_str(r#"{"accessToken":"ya29.token","expireTime":"2026-06-11T15:01:23Z"}"#)
            .unwrap();
    assert_eq!(response.access_token, "ya29.token");
    assert_eq!(response.expire_time, "2026-06-11T15:01:23Z");
}

#[test]
fn sts_response_parses_with_and_without_expires_in() {
    let with: StsTokenExchangeResponse =
        serde_json::from_str(r#"{"access_token":"fed-token","expires_in":3599}"#).unwrap();
    assert_eq!(with.access_token, "fed-token");
    assert_eq!(with.expires_in, Some(3599));

    let without: StsTokenExchangeResponse =
        serde_json::from_str(r#"{"access_token":"fed-token"}"#).unwrap();
    assert_eq!(without.expires_in, None);
}

#[test]
fn timer_delay_fires_lead_time_before_expiry() {
    let now = SystemTime::now();
    let delay = geap_refresh_timer_delay(now + Duration::from_secs(3600), now);
    assert_eq!(delay, Duration::from_secs(3600) - GEAP_REFRESH_LEAD_TIME);
}

#[test]
fn timer_delay_clamps_to_floor_when_near_or_past_expiry() {
    let now = SystemTime::now();
    // Within the lead window: never immediate, clamped up to the floor.
    assert_eq!(
        geap_refresh_timer_delay(now + Duration::from_secs(30), now),
        GEAP_MIN_TIMER_DELAY
    );
    // Already expired (e.g. a badly skewed local clock): same floor, so the
    // timer cannot spin a hot mint loop.
    assert_eq!(
        geap_refresh_timer_delay(now - Duration::from_secs(30), now),
        GEAP_MIN_TIMER_DELAY
    );
}

// The structured `LoadGeapCredentialsError` is intentionally not user-facing
// (no `Display` prose, no truncation) — the UI layer owns turning a leg + HTTP
// `status` + raw `detail` into actionable copy. Coverage that the structured
// error round-trips into `Failed { error }` lives in the mint-completion tests
// below.

// ── refresh guard / safety net (app harness) ───────────────────

fn team_for_test() -> Team {
    Team {
        uid: 123.into(),
        name: "test".to_string(),
        invite_code: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        organization_settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn workspace_with_geap_host(enabled: bool) -> Workspace {
    let team = team_for_test();
    let mut workspace = Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    };
    workspace.settings.llm_settings.enabled = true;
    workspace.settings.llm_settings.host_configs.insert(
        crate::ai::llms::LLMModelHost::GeminiEnterprise,
        LlmHostSettings {
            enabled,
            enablement_setting: HostEnablementSetting::Enforce,
            gcp_audience: Some(TEST_AUDIENCE.to_string()),
            gcp_sa_email: Some(TEST_SA_EMAIL.to_string()),
        },
    );
    workspace
}

/// Registers the minimal singleton set the refresh path touches: workspace
/// policy (gate), auth (uid), settings (member toggle), the secret manager
/// (leg 1 mint), and the `ApiKeyManager` under test.
fn initialize_app(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(|_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    app.add_singleton_model(|_| {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    let auth_state_provider = crate::auth::AuthStateProvider::new_for_test();
    let auth_state = auth_state_provider.get().clone();
    app.add_singleton_model(|_| auth_state_provider);
    app.add_singleton_model(crate::settings::AISettings::new_with_defaults);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(|ctx| {
        ManagedSecretManager::new(
            ServerApiProvider::as_ref(ctx).get_managed_secrets_client(),
            auth_state,
        )
    });
    app.update(|ctx| {
        warpui_extras::secure_storage::register_noop("test", ctx);
        ctx.add_singleton_model(ApiKeyManager::new);
    });
}

fn fresh_credentials() -> GeapCredentials {
    GeapCredentials::new(
        "geap-token".into(),
        Some(SystemTime::now() + Duration::from_secs(3600)),
    )
}

fn expired_credentials() -> GeapCredentials {
    GeapCredentials::new(
        "geap-token".into(),
        Some(SystemTime::now() - Duration::from_secs(30)),
    )
}

/// A binding that does not match the harness gate (minted before an account
/// switch).
fn stale_binding() -> GeapMintBinding {
    GeapMintBinding {
        user_uid: "previous-user".into(),
        audience: TEST_AUDIENCE.into(),
        federation: GeapFederation::ServiceAccount {
            email: TEST_SA_EMAIL.into(),
        },
    }
}

/// The mintable binding for the harness gate. The harness enables the GEAP
/// host with a configured audience, so the policy is always `Mintable`.
fn current_binding(ctx: &mut ModelContext<ApiKeyManager>) -> GeapMintBinding {
    match current_geap_policy(ctx) {
        GeapPolicy::Mintable(binding) => binding,
        other => panic!("expected a mintable GEAP policy, got {other:?}"),
    }
}

#[test]
fn refresh_disables_and_drops_tokens_when_gate_is_off() {
    // GEAP host present but disabled by the admin.
    let workspace = workspace_with_geap_host(false);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);

        // Even a previously loaded token is dropped: no token is retained
        // while disabled.
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_geap_credentials_state(
                GeapCredentialsState::Loaded {
                    credentials: fresh_credentials(),
                    loaded_at: SystemTime::now(),
                    minted_for: GeapMintBinding {
                        user_uid: "user".into(),
                        audience: TEST_AUDIENCE.into(),
                        federation: GeapFederation::ServiceAccount {
                            email: TEST_SA_EMAIL.into(),
                        },
                    },
                },
                ctx,
            );
            refresh_geap_credentials(manager, ctx);
            assert_eq!(
                *manager.geap_credentials_state(),
                GeapCredentialsState::Disabled
            );
        });
    })
}

#[test]
fn refresh_rests_at_missing_when_enabled_but_unconfigured() {
    let mut workspace = workspace_with_geap_host(true);
    // Enabled, but the admin has not configured an audience yet.
    workspace
        .settings
        .llm_settings
        .host_configs
        .get_mut(&crate::ai::llms::LLMModelHost::GeminiEnterprise)
        .unwrap()
        .gcp_audience = Some("   ".to_string());
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            refresh_geap_credentials(manager, ctx);
            assert_eq!(
                *manager.geap_credentials_state(),
                GeapCredentialsState::Missing
            );
        });
    })
}

#[test]
fn refresh_skips_when_token_is_fresh_and_binding_matches() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            let loaded = GeapCredentialsState::Loaded {
                credentials: fresh_credentials(),
                loaded_at: SystemTime::now(),
                minted_for: current_binding(ctx),
            };
            manager.set_geap_credentials_state(loaded.clone(), ctx);
            // Skip-if-valid: a fresh token under the current binding means no
            // re-mint and no state change.
            refresh_geap_credentials(manager, ctx);
            assert_eq!(*manager.geap_credentials_state(), loaded);
        });
    })
}

#[test]
fn refresh_noops_while_a_mint_is_in_flight() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            let in_flight = GeapCredentialsState::Refreshing {
                previous: Some((fresh_credentials(), current_binding(ctx))),
            };
            manager.set_geap_credentials_state(in_flight.clone(), ctx);
            // One mint at a time — force included.
            refresh_geap_credentials(manager, ctx);
            assert_eq!(*manager.geap_credentials_state(), in_flight);
            force_refresh_geap_credentials(manager, ctx);
            assert_eq!(*manager.geap_credentials_state(), in_flight);
        });
    })
}

#[test]
fn refresh_remints_when_token_needs_refresh() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_geap_credentials_state(
                GeapCredentialsState::Loaded {
                    credentials: expired_credentials(),
                    loaded_at: SystemTime::now(),
                    minted_for: current_binding(ctx),
                },
                ctx,
            );
            refresh_geap_credentials(manager, ctx);
            // The expired-but-still-serving token rides along as `previous`
            // while the re-mint is in flight: tokens stay until replaced.
            match manager.geap_credentials_state() {
                GeapCredentialsState::Refreshing { previous: Some(_) } => {}
                other => panic!("expected Refreshing with a previous token, got {other:?}"),
            }
        });
    })
}

#[test]
fn refresh_remints_on_binding_mismatch() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            // Fresh token, but minted for a different user (e.g. before an
            // account switch).
            manager.set_geap_credentials_state(
                GeapCredentialsState::Loaded {
                    credentials: fresh_credentials(),
                    loaded_at: SystemTime::now(),
                    minted_for: stale_binding(),
                },
                ctx,
            );
            refresh_geap_credentials(manager, ctx);
            // The mismatched token must NOT ride along as `previous`: it is
            // unservable, and restoring it on a failed re-mint would mask the
            // failure behind a misleading `Loaded`.
            match manager.geap_credentials_state() {
                GeapCredentialsState::Refreshing { previous: None } => {}
                other => panic!("expected a re-mint with no carried token, got {other:?}"),
            }
        });
    })
}

// ── mint completion (apply_geap_mint_result) ─────────────────────

#[test]
fn mint_completion_discards_stale_binding_result_and_remints() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_geap_credentials_state(
                GeapCredentialsState::Refreshing { previous: None },
                ctx,
            );
            // A mint stamped for a stale binding completes successfully: the
            // token must be discarded — never stored — with a re-mint under
            // the current binding immediately in flight (no one-request
            // token-less window).
            apply_geap_mint_result(
                manager,
                Ok(fresh_credentials()),
                stale_binding(),
                false,
                ctx,
            );
            match manager.geap_credentials_state() {
                GeapCredentialsState::Refreshing { previous: None } => {}
                other => panic!("expected an immediate re-mint, got {other:?}"),
            }
        });
    })
}

#[test]
fn mint_completion_failure_restores_servable_previous() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            let current = current_binding(ctx);
            let carried = fresh_credentials();
            manager.set_geap_credentials_state(
                GeapCredentialsState::Refreshing {
                    previous: Some((carried.clone(), current.clone())),
                },
                ctx,
            );
            // A failed background re-mint restores the still-servable
            // previous token and parks the chain.
            apply_geap_mint_result(
                manager,
                Err(LoadGeapCredentialsError::ExchangeToken {
                    status: None,
                    detail: "boom".into(),
                }),
                current.clone(),
                false,
                ctx,
            );
            match manager.geap_credentials_state() {
                GeapCredentialsState::Loaded {
                    credentials,
                    minted_for,
                    ..
                } if *credentials == carried && *minted_for == current => {}
                other => panic!("expected the previous token restored, got {other:?}"),
            }
        });
    })
}

#[test]
fn mint_completion_failure_with_unservable_previous_fails() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            let current = current_binding(ctx);
            // The carried token was minted for someone else: restoring it
            // would mask the failure behind a misleading `Loaded`, so the
            // failure must surface instead.
            manager.set_geap_credentials_state(
                GeapCredentialsState::Refreshing {
                    previous: Some((fresh_credentials(), stale_binding())),
                },
                ctx,
            );
            apply_geap_mint_result(
                manager,
                Err(LoadGeapCredentialsError::ExchangeToken {
                    status: None,
                    detail: "boom".into(),
                }),
                current,
                false,
                ctx,
            );
            match manager.geap_credentials_state() {
                GeapCredentialsState::Failed {
                    error: LoadGeapCredentialsError::ExchangeToken { .. },
                } => {}
                other => {
                    panic!("expected Failed carrying the structured leg-2 error, got {other:?}")
                }
            }
        });
    })
}

#[test]
fn safety_net_noops_on_fresh_token_and_rearms_parked_chain() {
    let workspace = workspace_with_geap_host(true);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            let fresh = GeapCredentialsState::Loaded {
                credentials: fresh_credentials(),
                loaded_at: SystemTime::now(),
                minted_for: current_binding(ctx),
            };
            // Fresh token: the safety net must not touch anything.
            manager.set_geap_credentials_state(fresh.clone(), ctx);
            refresh_geap_credentials_if_needed(manager, ctx);
            assert_eq!(*manager.geap_credentials_state(), fresh);

            // Parked chain (an earlier mint failed with nothing to keep):
            // the next request re-arms it.
            manager.set_geap_credentials_state(
                GeapCredentialsState::Failed {
                    error: LoadGeapCredentialsError::ExchangeToken {
                        status: None,
                        detail: "boom".into(),
                    },
                },
                ctx,
            );
            refresh_geap_credentials_if_needed(manager, ctx);
            match manager.geap_credentials_state() {
                GeapCredentialsState::Refreshing { .. } => {}
                other => panic!("expected the safety net to arm a refresh, got {other:?}"),
            }
        });
    })
}

#[test]
fn safety_net_is_a_pure_noop_when_gate_is_off() {
    let workspace = workspace_with_geap_host(false);
    App::test((), |mut app| async move {
        let _geap_flag = FeatureFlag::GeminiEnterprise.override_enabled(true);
        initialize_app(&mut app, vec![workspace]);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            // The request path must not mutate state when the gate is off;
            // state transitions belong to the event-driven triggers.
            refresh_geap_credentials_if_needed(manager, ctx);
            assert_eq!(
                *manager.geap_credentials_state(),
                GeapCredentialsState::Missing
            );
        });
    })
}
