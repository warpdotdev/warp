use std::collections::HashMap;
use std::time::Duration;

use instant::Instant;
use serde::{Deserialize, Serialize};
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences;
use warp_managed_secrets::client::SecretOwner;
use warp_managed_secrets::{ManagedSecretManager, ManagedSecretValue};
use warpui::{Entity, ModelContext, RequestState, SingletonEntity};

use crate::ai::harness_display;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;
use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};
use crate::report_error;
use crate::server::retry_strategies::{
    is_transient_graphql_or_http_error, OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
};
use crate::server::server_api::ServerApiProvider;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

const CACHE_KEY: &str = "AvailableHarnesses";
const AUTH_SECRET_FETCH_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessModelInfo {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessAvailability {
    pub harness: Harness,
    pub display_name: String,
    pub enabled: bool,
    #[serde(default)]
    pub available_models: Vec<HarnessModelInfo>,
}

/// Default fallback used before the server responds.
/// Oz is enabled by default so the UI is usable pre-fetch; the server
/// list (which respects admin overrides) replaces this once available.
fn default_harnesses() -> Vec<HarnessAvailability> {
    vec![HarnessAvailability {
        harness: Harness::Oz,
        display_name: "Warp".to_string(),
        enabled: true,
        available_models: vec![],
    }]
}

#[derive(Debug, Clone)]
pub enum AuthSecretFetchState {
    NotFetched,
    Loading,
    Loaded(Vec<AuthSecretEntry>),
    Failed(#[allow(dead_code)] String),
}

#[derive(Debug, Clone)]
pub struct AuthSecretEntry {
    pub name: String,
}

pub enum HarnessAvailabilityEvent {
    Changed,
    AuthSecretsLoaded,
    /// Emitted when a lazy auth-secrets fetch fails. Subscribers should
    /// re-render so any "Loading…" placeholders can transition to an
    /// error state — without this signal the picker would otherwise be
    /// stuck on the loading placeholder until the next refetch.
    AuthSecretsFetchFailed,
    AuthSecretCreated {
        harness: Harness,
        name: String,
    },
    AuthSecretCreationFailed {
        error: String,
    },
}

pub struct HarnessAvailabilityModel {
    harnesses: Vec<HarnessAvailability>,
    auth_secrets: HashMap<Harness, AuthSecretFetchState>,
    auth_secret_retry_after: HashMap<Harness, Instant>,
}

impl HarnessAvailabilityModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let harnesses = get_cached(ctx).unwrap_or_else(default_harnesses);

        ctx.subscribe_to_model(&NetworkStatus::handle(ctx), |me, event, ctx| {
            if let NetworkStatusEvent::NetworkStatusChanged {
                new_status: NetworkStatusKind::Online,
            } = event
            {
                me.refresh(ctx);
            }
        });

        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, event, ctx| {
            if let AuthManagerEvent::AuthComplete = event {
                let cached_harnesses: Vec<Harness> = me.auth_secrets.keys().copied().collect();
                for harness in cached_harnesses {
                    me.invalidate_auth_secrets(harness);
                }
                me.refresh(ctx);
            }
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
                me.refresh(ctx);
            }
        });

        let me = Self {
            harnesses,
            auth_secrets: HashMap::new(),
            auth_secret_retry_after: HashMap::new(),
        };
        me.refresh(ctx);
        me
    }

    pub fn available_harnesses(&self) -> &[HarnessAvailability] {
        &self.harnesses
    }

    pub fn display_name_for(&self, harness: Harness) -> &str {
        self.harnesses
            .iter()
            .find(|h| h.harness == harness)
            .map(|h| h.display_name.as_str())
            .unwrap_or_else(|| harness_display::display_name(harness))
    }

    /// Whether the harness selector should be shown (>1 known harness, including disabled).
    pub fn should_show_harness_selector(&self) -> bool {
        FeatureFlag::AgentHarness.is_enabled() && self.harnesses.len() > 1
    }

    /// Whether any harness is available at all (at least one enabled).
    pub fn has_any_enabled_harness(&self) -> bool {
        self.harnesses.iter().any(|h| h.enabled)
    }

    /// Whether a harness is both known and enabled.
    pub fn is_harness_enabled(&self, harness: Harness) -> bool {
        self.harnesses
            .iter()
            .any(|h| h.harness == harness && h.enabled)
    }

    pub fn models_for(&self, harness: Harness) -> Option<&[HarnessModelInfo]> {
        self.harnesses
            .iter()
            .find(|h| h.harness == harness)
            .map(|h| h.available_models.as_slice())
            .filter(|m| !m.is_empty())
    }

    pub fn auth_secrets_for(&self, harness: Harness) -> &AuthSecretFetchState {
        self.auth_secrets
            .get(&harness)
            .unwrap_or(&AuthSecretFetchState::NotFetched)
    }

    pub fn ensure_auth_secrets_fetched(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        match self.auth_secrets_for(harness) {
            AuthSecretFetchState::NotFetched => self.fetch_auth_secrets(harness, ctx),
            AuthSecretFetchState::Failed(_) if self.can_retry_auth_secret_fetch(harness) => {
                self.fetch_auth_secrets(harness, ctx);
            }
            AuthSecretFetchState::Failed(_)
            | AuthSecretFetchState::Loading
            | AuthSecretFetchState::Loaded(_) => {}
        }
    }

    fn fetch_auth_secrets(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        let Some(agent_harness) = harness_to_graphql_harness(harness) else {
            return;
        };

        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }

        self.auth_secrets
            .insert(harness, AuthSecretFetchState::Loading);
        self.auth_secret_retry_after.remove(&harness);

        let api = ServerApiProvider::as_ref(ctx).get_managed_secrets_client();
        ctx.spawn_with_retry_on_error_when(
            move || {
                let api = api.clone();
                let agent_harness = agent_harness.clone();
                async move { api.list_harness_auth_secrets(agent_harness).await }
            },
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
            is_transient_graphql_or_http_error,
            move |me,
                  result: RequestState<Vec<warp_graphql::managed_secrets::ManagedSecret>>,
                  ctx| match result {
                RequestState::RequestSucceeded(secrets) => {
                    let entries = secrets
                        .into_iter()
                        .map(|s| AuthSecretEntry { name: s.name })
                        .collect();
                    me.auth_secrets
                        .insert(harness, AuthSecretFetchState::Loaded(entries));
                    me.auth_secret_retry_after.remove(&harness);
                    ctx.emit(HarnessAvailabilityEvent::AuthSecretsLoaded);
                }
                RequestState::RequestFailedRetryPending(e) => {
                    log::warn!("Failed to fetch harness auth secrets; retrying: {e:#}");
                }
                RequestState::RequestFailed(e) => {
                    let msg = e.to_string();
                    report_error!(e.context("Failed to fetch harness auth secrets"));
                    me.auth_secrets
                        .insert(harness, AuthSecretFetchState::Failed(msg));
                    me.auth_secret_retry_after
                        .insert(harness, Instant::now() + AUTH_SECRET_FETCH_FAILURE_COOLDOWN);
                    // Notify subscribers so they can drop any
                    // "Loading…" placeholder rendered during the
                    // in-flight fetch and surface the error state.
                    ctx.emit(HarnessAvailabilityEvent::AuthSecretsFetchFailed);
                }
            },
        );
    }

    fn can_retry_auth_secret_fetch(&self, harness: Harness) -> bool {
        self.auth_secret_retry_after
            .get(&harness)
            .map(|retry_after| Instant::now() >= *retry_after)
            .unwrap_or(true)
    }

    pub fn invalidate_auth_secrets(&mut self, harness: Harness) {
        self.auth_secrets.remove(&harness);
        self.auth_secret_retry_after.remove(&harness);
    }

    pub fn create_auth_secret(
        &mut self,
        harness: Harness,
        name: String,
        value: ManagedSecretValue,
        owner: SecretOwner,
        ctx: &mut ModelContext<Self>,
    ) {
        let manager = ManagedSecretManager::handle(ctx);
        let create_future = manager.as_ref(ctx).create_secret(owner, name, value, None);
        ctx.spawn(create_future, move |me, result, ctx| match result {
            Ok(secret) => {
                let entry = AuthSecretEntry {
                    name: secret.name.clone(),
                };
                match me.auth_secrets.get_mut(&harness) {
                    Some(AuthSecretFetchState::Loaded(entries)) => {
                        entries.push(entry);
                    }
                    _ => {
                        me.auth_secrets
                            .insert(harness, AuthSecretFetchState::Loaded(vec![entry]));
                    }
                }
                ctx.emit(HarnessAvailabilityEvent::AuthSecretCreated {
                    harness,
                    name: secret.name,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                report_error!(e.context("Failed to create harness auth secret"));
                ctx.emit(HarnessAvailabilityEvent::AuthSecretCreationFailed { error: msg });
            }
        });
    }

    pub fn refresh(&self, ctx: &mut ModelContext<Self>) {
        // The endpoint queries `user`, which requires auth.
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        ctx.spawn(
            async move { ai_client.get_available_harnesses().await },
            |me, result, ctx| match result {
                Ok(new_harnesses) => {
                    if new_harnesses != me.harnesses {
                        me.harnesses = new_harnesses;
                        me.cache(ctx);
                        // Invalidate cached auth secrets so the next menu open refetches.
                        let stale: Vec<Harness> = me.auth_secrets.keys().copied().collect();
                        for harness in stale {
                            me.invalidate_auth_secrets(harness);
                        }
                        ctx.emit(HarnessAvailabilityEvent::Changed);
                    }
                }
                Err(e) => {
                    report_error!(e.context("Failed to fetch available harnesses"));
                }
            },
        );
    }

    fn cache(&self, ctx: &ModelContext<Self>) {
        if let Ok(serialized) = serde_json::to_string(&self.harnesses) {
            if let Err(e) = ctx
                .private_user_preferences()
                .write_value(CACHE_KEY, serialized)
            {
                report_error!(anyhow::anyhow!(e).context("Failed to cache available harnesses"));
            }
        }
    }
}

fn get_cached(ctx: &ModelContext<HarnessAvailabilityModel>) -> Option<Vec<HarnessAvailability>> {
    let raw = ctx
        .private_user_preferences()
        .read_value(CACHE_KEY)
        .ok()??;
    serde_json::from_str::<Vec<HarnessAvailability>>(&raw).ok()
}

fn harness_to_graphql_harness(harness: Harness) -> Option<warp_graphql::ai::AgentHarness> {
    match harness {
        Harness::Oz => Some(warp_graphql::ai::AgentHarness::Oz),
        Harness::Claude => Some(warp_graphql::ai::AgentHarness::ClaudeCode),
        Harness::Gemini => Some(warp_graphql::ai::AgentHarness::Gemini),
        Harness::Codex => Some(warp_graphql::ai::AgentHarness::Codex),
        Harness::OpenCode | Harness::Unknown => None,
    }
}

impl Entity for HarnessAvailabilityModel {
    type Event = HarnessAvailabilityEvent;
}

impl SingletonEntity for HarnessAvailabilityModel {}
