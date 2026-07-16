#[cfg(not(target_family = "wasm"))]
use ai::api_keys::ApiKeyManager;
#[cfg(not(target_family = "wasm"))]
use ai::codex_subscription::oauth::TokenResponse;
#[cfg(not(target_family = "wasm"))]
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
#[cfg(not(target_family = "wasm"))]
use base64::Engine as _;
use warp_core::features::FeatureFlag;
#[cfg(not(target_family = "wasm"))]
use warpui::App;

#[cfg(not(target_family = "wasm"))]
use super::{codex_oauth_attempt_is_current, take_codex_tokens_for_disconnect};
use super::{
    derive_agent_attribution_toggle_state, should_render_codex_subscription,
    subscription_controls_enabled, AgentAttributionToggleState, API_KEYS_SEARCH_TERMS,
};
use crate::workspaces::workspace::AdminEnablementSetting;

#[test]
fn respect_user_setting_returns_user_pref_unlocked() {
    let state = derive_agent_attribution_toggle_state(
        &AdminEnablementSetting::RespectUserSetting,
        true,
        true,
    );
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: false,
            is_disabled: false,
        }
    );
}

#[test]
fn respect_user_setting_with_user_off_returns_unchecked_unlocked() {
    let state = derive_agent_attribution_toggle_state(
        &AdminEnablementSetting::RespectUserSetting,
        false,
        true,
    );
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: false,
            is_forced_by_org: false,
            is_disabled: false,
        }
    );
}

#[test]
fn team_enable_locks_toggle_on_regardless_of_user_pref() {
    let state = derive_agent_attribution_toggle_state(&AdminEnablementSetting::Enable, false, true);
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: true,
            is_disabled: true,
        }
    );
}

#[test]
fn team_disable_locks_toggle_off_regardless_of_user_pref() {
    let state = derive_agent_attribution_toggle_state(&AdminEnablementSetting::Disable, true, true);
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: false,
            is_forced_by_org: true,
            is_disabled: true,
        }
    );
}

#[test]
fn ai_globally_disabled_marks_toggle_disabled_but_not_forced() {
    let state = derive_agent_attribution_toggle_state(
        &AdminEnablementSetting::RespectUserSetting,
        true,
        false,
    );
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: false,
            is_disabled: true,
        }
    );
}

#[test]
fn team_force_takes_precedence_over_global_ai_disabled() {
    let state =
        derive_agent_attribution_toggle_state(&AdminEnablementSetting::Enable, false, false);
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: true,
            is_disabled: true,
        }
    );
}

#[test]
fn codex_subscription_visibility_requires_feature_and_provider_keys() {
    let feature = FeatureFlag::CodexSubscription.override_enabled(false);
    assert!(!should_render_codex_subscription(true));
    drop(feature);

    let feature = FeatureFlag::CodexSubscription.override_enabled(true);
    assert!(!should_render_codex_subscription(false));
    assert!(should_render_codex_subscription(true));
    drop(feature);
}

#[test]
fn api_key_search_terms_include_codex_oauth_vocabulary() {
    let terms = API_KEYS_SEARCH_TERMS
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    for term in ["codex", "chatgpt", "openai", "subscription", "oauth"] {
        assert!(terms.contains(&term), "missing search term {term}");
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn codex_connect_stores_and_disconnect_clears_tokens() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
        });
        let manager = app.add_singleton_model(ApiKeyManager::new);
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD
            .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"account-123"}}"#);
        let id_token = format!("{header}.{payload}.signature");

        manager.update(&mut app, |manager, ctx| {
            manager
                .store_codex_tokens(
                    TokenResponse {
                        id_token: Some(id_token.clone()),
                        access_token: "access-token".into(),
                        refresh_token: Some("refresh-token".into()),
                        expires_in: Some(3600),
                    },
                    ctx,
                )
                .unwrap();
        });
        manager.read(&app, |manager, _| {
            let tokens = manager.codex_tokens().expect("Codex tokens stored");
            assert_eq!(tokens.access_token, "access-token");
            assert_eq!(tokens.chatgpt_account_id, "account-123");
            assert!(tokens.connected_at.is_some());
        });

        let revocation = manager
            .update(&mut app, take_codex_tokens_for_disconnect)
            .expect("tokens available for best-effort revocation");
        assert_eq!(revocation.access_token.as_deref(), Some("access-token"));
        assert_eq!(revocation.refresh_token.as_deref(), Some("refresh-token"));
        manager.read(&app, |manager, _| {
            assert!(manager.codex_tokens().is_none());
        });
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn stale_codex_oauth_attempts_cannot_complete() {
    assert!(codex_oauth_attempt_is_current(true, 7, 7));
    assert!(!codex_oauth_attempt_is_current(false, 7, 7));
    assert!(!codex_oauth_attempt_is_current(true, 8, 7));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn subscription_controls_follow_effective_global_and_byo_state() {
    assert!(subscription_controls_enabled(true, true, true));
    assert!(!subscription_controls_enabled(false, true, true));
    assert!(!subscription_controls_enabled(true, false, true));
    assert!(!subscription_controls_enabled(true, true, false));
}
