use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{self, Either};
use oneshot::Canceled;
use uuid::Uuid;
use warpui::r#async::{FutureExt, TimeoutError};
use warpui::{ModelContext, SingletonEntity as _};

use super::{AgentDriver, AgentDriverError};
use crate::ai::mcp::file_based_manager::FileBasedMCPManager;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManagerEvent;
use crate::ai::mcp::{MCPServerState, TemplatableMCPServerManager};

/// How a server's current and subsequent states settle a wait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum McpWaitKind {
    /// Drive and ephemeral spawns: `Running` and `FailedToStart` are terminal.
    /// `NotRunning` stays pending because spawn may not have happened yet.
    Spawned,
    /// File-based auto-starts may already be running, failed, or despawned
    /// (`NotRunning`, or gone from [`FileBasedMCPManager`]) before the wait begins.
    FileBased,
}

/// Per-server result of waiting for a terminal MCP state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum McpServerWaitOutcome {
    Ready,
    FailedToStart { detail: String },
    TimedOut { detail: String },
}

impl McpServerWaitOutcome {
    fn into_startup_detail(self, timeout: Duration) -> Option<String> {
        match self {
            Self::Ready => None,
            Self::FailedToStart { detail } => Some(detail),
            Self::TimedOut { detail } => Some(format!(
                "{detail} did not start within {}s",
                timeout.as_secs()
            )),
        }
    }
}

pub(super) fn startup_result_from_outcomes(
    outcomes: impl IntoIterator<Item = McpServerWaitOutcome>,
    timeout: Duration,
) -> Result<(), AgentDriverError> {
    let mut details: Vec<String> = outcomes
        .into_iter()
        .filter_map(|outcome| outcome.into_startup_detail(timeout))
        .collect();
    details.sort();
    if details.is_empty() {
        Ok(())
    } else {
        Err(AgentDriverError::MCPStartupFailed { details })
    }
}

/// Subscribe, inspect current state, then wait until every server reaches a
/// terminal state or `timeout` elapses.
///
/// Returns typed per-server outcomes. Strict vs degraded handling belongs to the
/// caller. Must not run concurrently with another MCP wait: the driver keeps at
/// most one subscription to [`TemplatableMCPServerManager`].
pub(super) fn wait_for_mcp_servers_terminal(
    servers: HashMap<Uuid, String>,
    timeout: Duration,
    kind: McpWaitKind,
    ctx: &mut ModelContext<AgentDriver>,
) -> impl Future<Output = Result<Vec<McpServerWaitOutcome>, AgentDriverError>> + use<> {
    if servers.is_empty() {
        return Either::Right(future::ready(Ok(Vec::new())));
    }

    let (tx, rx) = oneshot::channel::<()>();
    let mut tx = Some(tx);

    let pending = Arc::new(Mutex::new(servers));
    let failed: Arc<Mutex<Vec<McpServerWaitOutcome>>> = Arc::new(Mutex::new(Vec::new()));
    let pending_details: Arc<Mutex<HashMap<Uuid, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let pending_for_subscription = Arc::clone(&pending);
    let failed_for_subscription = Arc::clone(&failed);
    let pending_details_for_subscription = Arc::clone(&pending_details);

    let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);

    ctx.unsubscribe_from_model(&templatable_mcp_manager);
    ctx.subscribe_to_model(&templatable_mcp_manager, move |_me, manager, event, ctx| {
        let TemplatableMCPServerManagerEvent::StateChanged { uuid, state } = event else {
            return;
        };
        let Ok(mut pending_servers) = pending_for_subscription.lock() else {
            return;
        };
        let Some(name) = pending_servers.get(uuid).cloned() else {
            return;
        };
        record_pending_detail(
            &pending_details_for_subscription,
            *uuid,
            &name,
            Some(*state),
            TemplatableMCPServerManager::as_ref(ctx).get_server_error_message(*uuid),
            kind,
        );
        match classify_state(
            *uuid,
            &name,
            Some(*state),
            TemplatableMCPServerManager::as_ref(ctx).get_server_error_message(*uuid),
            kind,
            file_based_manager(kind, ctx),
        ) {
            None => return,
            Some(McpServerWaitOutcome::Ready) => {
                pending_servers.remove(uuid);
                if let Ok(mut details) = pending_details_for_subscription.lock() {
                    details.remove(uuid);
                }
            }
            Some(outcome) => {
                pending_servers.remove(uuid);
                if let Ok(mut details) = pending_details_for_subscription.lock() {
                    details.remove(uuid);
                }
                if let Ok(mut failed_servers) = failed_for_subscription.lock() {
                    failed_servers.push(outcome);
                }
            }
        }
        if pending_servers.is_empty() {
            log::info!("All requested MCP servers reached a terminal state");
            if let Some(sender) = tx.take() {
                let _ = sender.send(());
            }
            ctx.unsubscribe_from_model(&manager);
        }
    });

    inspect_current_servers(
        Arc::clone(&pending),
        Arc::clone(&failed),
        Arc::clone(&pending_details),
        kind,
        ctx,
    );

    if pending
        .lock()
        .map(|pending_servers| pending_servers.is_empty())
        .unwrap_or(true)
    {
        ctx.unsubscribe_from_model(&templatable_mcp_manager);
        return Either::Right(future::ready(Ok(failed
            .lock()
            .map(|failed_servers| failed_servers.clone())
            .unwrap_or_default())));
    }

    let log_label = match kind {
        McpWaitKind::Spawned => "MCP",
        McpWaitKind::FileBased => "file-based MCP",
    };
    log::info!(
        "Waiting for {} {log_label} server(s) to reach a terminal state",
        pending
            .lock()
            .map(|pending_servers| pending_servers.len())
            .unwrap_or_default()
    );

    let spawner = ctx.spawner();
    Either::Left(async move {
        let mut timed_out = Vec::new();
        match rx.with_timeout(timeout).await {
            Ok(Ok(())) => {}
            Ok(Err(Canceled)) => {
                log::error!("MCP server readiness subscription dropped early");
                return Err(AgentDriverError::InvalidRuntimeState);
            }
            Err(TimeoutError) => {
                timed_out = timeout_details(&pending, &pending_details, kind);
                let pending_log = timed_out.join("; ");
                log::warn!(
                    "Timed out waiting for {log_label} servers to reach a terminal state. Still pending: {pending_log}"
                );
                let _ = spawner
                    .spawn(|_, ctx| {
                        let manager = TemplatableMCPServerManager::handle(ctx);
                        ctx.unsubscribe_from_model(&manager);
                    })
                    .await;
            }
        }

        let mut outcomes = failed
            .lock()
            .map(|failed_servers| failed_servers.clone())
            .unwrap_or_default();
        outcomes.extend(
            timed_out
                .into_iter()
                .map(|detail| McpServerWaitOutcome::TimedOut { detail }),
        );
        Ok(outcomes)
    })
}

fn inspect_current_servers(
    pending: Arc<Mutex<HashMap<Uuid, String>>>,
    failed: Arc<Mutex<Vec<McpServerWaitOutcome>>>,
    pending_details: Arc<Mutex<HashMap<Uuid, String>>>,
    kind: McpWaitKind,
    ctx: &mut ModelContext<AgentDriver>,
) {
    let templatable = TemplatableMCPServerManager::as_ref(ctx);
    let file_based = file_based_manager(kind, ctx);
    let Ok(mut pending_servers) = pending.lock() else {
        return;
    };
    let mut settled = Vec::new();
    for (uuid, name) in pending_servers.iter() {
        let state = templatable.get_server_state(*uuid);
        record_pending_detail(
            &pending_details,
            *uuid,
            name,
            state,
            templatable.get_server_error_message(*uuid),
            kind,
        );
        if let Some(outcome) = classify_state(
            *uuid,
            name,
            state,
            templatable.get_server_error_message(*uuid),
            kind,
            file_based,
        ) {
            settled.push((*uuid, outcome));
        }
    }
    for (uuid, outcome) in settled {
        pending_servers.remove(&uuid);
        if let Ok(mut details) = pending_details.lock() {
            details.remove(&uuid);
        }
        if !matches!(outcome, McpServerWaitOutcome::Ready)
            && let Ok(mut failed_servers) = failed.lock()
        {
            failed_servers.push(outcome);
        }
    }
}

fn file_based_manager<'ctx>(
    kind: McpWaitKind,
    ctx: &'ctx ModelContext<'_, AgentDriver>,
) -> Option<&'ctx FileBasedMCPManager> {
    (kind == McpWaitKind::FileBased).then(|| FileBasedMCPManager::as_ref(ctx))
}

fn classify_state(
    uuid: Uuid,
    name: &str,
    state: Option<MCPServerState>,
    error: Option<&str>,
    kind: McpWaitKind,
    file_based: Option<&FileBasedMCPManager>,
) -> Option<McpServerWaitOutcome> {
    if kind == McpWaitKind::FileBased
        && file_based.is_some_and(|manager| manager.get_hash_by_uuid(uuid).is_none())
    {
        return Some(McpServerWaitOutcome::Ready);
    }
    match state {
        Some(MCPServerState::Running) => Some(McpServerWaitOutcome::Ready),
        Some(MCPServerState::FailedToStart) => {
            let error = error
                .map(|message| format!(": {message}"))
                .unwrap_or_default();
            let detail = format!("'{name}' failed to start{error}");
            log::warn!("MCP server {detail}");
            Some(McpServerWaitOutcome::FailedToStart { detail })
        }
        Some(MCPServerState::NotRunning) if kind == McpWaitKind::FileBased => {
            Some(McpServerWaitOutcome::Ready)
        }
        Some(MCPServerState::NotRunning)
        | Some(MCPServerState::Starting)
        | Some(MCPServerState::Authenticating)
        | Some(MCPServerState::ShuttingDown)
        | None => None,
    }
}

fn record_pending_detail(
    pending_details: &Arc<Mutex<HashMap<Uuid, String>>>,
    uuid: Uuid,
    name: &str,
    state: Option<MCPServerState>,
    error: Option<&str>,
    kind: McpWaitKind,
) {
    if kind != McpWaitKind::FileBased {
        return;
    }
    let state = state
        .map(|state| format!("{state:?}"))
        .unwrap_or_else(|| "no state".to_string());
    let error = error
        .map(|message| format!(", error={message}"))
        .unwrap_or_default();
    if let Ok(mut details) = pending_details.lock() {
        details.insert(uuid, format!("{name} ({uuid}): {state}{error}"));
    }
}

fn timeout_details(
    pending: &Arc<Mutex<HashMap<Uuid, String>>>,
    pending_details: &Arc<Mutex<HashMap<Uuid, String>>>,
    kind: McpWaitKind,
) -> Vec<String> {
    let mut details: Vec<String> = match kind {
        McpWaitKind::FileBased => pending_details
            .lock()
            .map(|details| details.values().cloned().collect())
            .unwrap_or_default(),
        McpWaitKind::Spawned => pending
            .lock()
            .map(|pending_servers| {
                pending_servers
                    .values()
                    .map(|name| format!("'{name}'"))
                    .collect()
            })
            .unwrap_or_default(),
    };
    details.sort();
    details
}

#[cfg(test)]
#[path = "mcp_wait_tests.rs"]
mod tests;
