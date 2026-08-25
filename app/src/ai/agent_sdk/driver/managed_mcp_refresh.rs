//! Proactive re-minting of managed MCP proxy configs during agent runs.
//!
//! Managed (backend-proxied) MCP servers authenticate with a short-lived
//! proxy session token minted once at run startup. External sessions expire
//! after a few hours, after which every tool call fails until a new token is
//! minted. This loop re-mints each managed server's config shortly before it
//! expires and respawns the server with the fresh token, so long runs never
//! hit the expiry cliff. The reactive path (re-mint on a 401 during
//! reconnect) remains the backstop when this loop can't help.
//!
//! Like the git/Bedrock credential loops, [`refresh_loop`] never resolves on
//! its own — it is raced against the harness future via `futures::select!`
//! in `with_credential_refreshes` and dropped when the run finishes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;
use warp_managed_secrets::ManagedSecretValue;
use warpui::{ModelSpawner, SingletonEntity as _};

use super::AgentDriver;
use crate::ai::agent_sdk::retry::{is_transient_graphql_or_http_error, with_bounded_retry_using};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::mcp::parsing::resolve_json;
use crate::server::server_api::managed_mcp::ManagedMcpClient;

/// Re-mint this long before a token's expiry (mirrors warp-server's own
/// refresh lead time for downstream OAuth tokens).
const REFRESH_LEAD: Duration = Duration::from_secs(5 * 60);
/// Never spin faster than this, even for tokens that are about to expire.
const MIN_DELAY: Duration = Duration::from_secs(60);
/// Attempt budget for a proactive re-mint; off the tool-call latency path,
/// so it gets the same patience as run-startup resolution.
const REMINT_MAX_ATTEMPTS: usize = 6;

/// One managed server's re-mint schedule entry.
pub(crate) struct ManagedRefreshEntry {
    pub installation_uuid: Uuid,
    pub managed_uid: String,
    pub expires_at: DateTime<Utc>,
}

/// Everything the proactive re-mint loop needs, captured at run startup.
pub(crate) struct ManagedMcpRefreshParams {
    pub entries: Vec<ManagedRefreshEntry>,
    pub managed_mcp_client: Arc<dyn ManagedMcpClient>,
    pub task_id: Option<AmbientAgentTaskId>,
    pub secrets: Arc<HashMap<String, ManagedSecretValue>>,
}

enum RefreshOutcome {
    /// Token re-minted (and the server respawned if the config changed);
    /// schedule the next refresh at the new expiry.
    Rescheduled(DateTime<Utc>),
    /// Stop scheduling this server; the reason is logged by the caller.
    Done(String),
}

/// Never-resolving loop that re-mints managed proxy configs before expiry.
pub(crate) async fn refresh_loop(
    params: ManagedMcpRefreshParams,
    foreground: &ModelSpawner<AgentDriver>,
) {
    let ManagedMcpRefreshParams {
        mut entries,
        managed_mcp_client,
        task_id,
        secrets,
    } = params;

    loop {
        let Some(next_expiry) = entries.iter().map(|entry| entry.expires_at).min() else {
            // Nothing left to schedule; park until the run ends.
            return futures::future::pending::<()>().await;
        };
        warpui::r#async::Timer::after(refresh_delay(next_expiry, Utc::now())).await;

        let now = Utc::now();
        let mut index = 0;
        while index < entries.len() {
            if !is_due(entries[index].expires_at, now) {
                index += 1;
                continue;
            }
            let entry = &mut entries[index];
            match refresh_entry(entry, &managed_mcp_client, task_id, &secrets, foreground).await {
                RefreshOutcome::Rescheduled(new_expiry) => {
                    log::info!(
                        "Re-minted managed MCP config '{}'; next refresh before {new_expiry}",
                        entry.managed_uid
                    );
                    entry.expires_at = new_expiry;
                    index += 1;
                }
                RefreshOutcome::Done(reason) => {
                    log::info!(
                        "Stopping proactive re-mint of managed MCP config '{}': {reason}",
                        entry.managed_uid
                    );
                    entries.swap_remove(index);
                }
            }
        }
    }
}

/// How long to sleep before the next re-mint pass: wake [`REFRESH_LEAD`]
/// before the earliest expiry, but never spin faster than [`MIN_DELAY`].
fn refresh_delay(next_expiry: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (next_expiry - now)
        .to_std()
        .unwrap_or_default()
        .saturating_sub(REFRESH_LEAD)
        .max(MIN_DELAY)
}

/// Whether a token is inside its refresh window.
fn is_due(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    expires_at - now <= chrono::Duration::from_std(REFRESH_LEAD).unwrap_or_default()
}

async fn refresh_entry(
    entry: &ManagedRefreshEntry,
    managed_mcp_client: &Arc<dyn ManagedMcpClient>,
    task_id: Option<AmbientAgentTaskId>,
    secrets: &Arc<HashMap<String, ManagedSecretValue>>,
    foreground: &ModelSpawner<AgentDriver>,
) -> RefreshOutcome {
    let installation_uuid = entry.installation_uuid;
    let managed_uid = entry.managed_uid.clone();

    // The retained config is the current source of truth for this server
    // (a reactive re-mint may have already replaced the startup config).
    let stale = match foreground
        .spawn(move |_, ctx| {
            TemplatableMCPServerManager::as_ref(ctx).retained_installation(installation_uuid)
        })
        .await
    {
        Ok(Some(stale)) => stale,
        Ok(None) => return RefreshOutcome::Done("server no longer active".to_string()),
        Err(_) => return RefreshOutcome::Done("driver shutting down".to_string()),
    };

    let client_config = match with_bounded_retry_using(
        &format!("proactively re-mint managed MCP config '{managed_uid}'"),
        REMINT_MAX_ATTEMPTS,
        is_transient_graphql_or_http_error,
        || managed_mcp_client.create_managed_mcp_client_config(managed_uid.clone()),
    )
    .await
    {
        Ok(client_config) => client_config,
        Err(err) => {
            // The reactive reconnect path remains as the backstop.
            return RefreshOutcome::Done(format!("re-mint failed: {err:#}"));
        }
    };

    let Some(new_expiry) = client_config.expires_at.map(|time| time.utc()) else {
        return RefreshOutcome::Done("re-minted config has no expiry".to_string());
    };
    if new_expiry <= entry.expires_at {
        // Runtime-kind tokens are anchored to the task's start, so a re-mint
        // cannot extend the deadline — the sandbox ends then anyway.
        return RefreshOutcome::Done(
            "token deadline is fixed (runtime-anchored); nothing to extend".to_string(),
        );
    }

    let fresh = match AgentDriver::rebuild_managed_installation(
        &stale,
        &client_config.mcp_config_json,
        task_id,
        &managed_uid,
        secrets,
    ) {
        Ok(fresh) => fresh,
        Err(err) => return RefreshOutcome::Done(format!("rebuild failed: {err}")),
    };

    // Respawn only when the rendered config actually changed; respawning
    // drops the live MCP session, and an interrupted tool call relies on
    // the reconnect retry to recover.
    if resolve_json(&fresh) != resolve_json(&stale) {
        let respawn = foreground
            .spawn(move |_, ctx| {
                TemplatableMCPServerManager::handle(ctx).update(ctx, move |manager, ctx| {
                    manager.respawn_with_installation(fresh, ctx);
                });
            })
            .await;
        if respawn.is_err() {
            return RefreshOutcome::Done("driver shutting down".to_string());
        }
    }

    RefreshOutcome::Rescheduled(new_expiry)
}

#[cfg(test)]
#[path = "managed_mcp_refresh_tests.rs"]
mod tests;
