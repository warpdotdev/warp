//! MCP server startup for an agent run: resolving `--mcp` specs into installations, spawning
//! them, and bounding how long the first turn waits on file-based servers discovered during
//! setup.
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::mem;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{self, Either};
use handlebars::get_arguments;
use instant::Instant;
use itertools::Itertools as _;
use oneshot::Canceled;
use uuid::Uuid;
use warp_cli::mcp::MCPSpec;
use warp_core::execution_mode::AppExecutionMode;
use warp_core::features::FeatureFlag;
use warp_managed_secrets::ManagedSecretValue;
use warpui::r#async::{FutureExt as _, TimeoutError};
use warpui::{Entity, ModelContext, ModelHandle, ModelSpawner, SingletonEntity};

use super::{AgentDriver, AgentDriverError};
use crate::ai::agent_sdk::retry::{is_transient_graphql_or_http_error, with_bounded_retry_using};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::mcp::file_based_manager::{FileBasedMCPManager, FileBasedMCPManagerEvent};
use crate::ai::mcp::parsing::{ParsedTemplatableMCPServerResult, normalize_mcp_json, resolve_json};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManagerEvent;
use crate::ai::mcp::{
    JSONMCPServer, MCPServerState, TemplatableMCPServerInstallation, TemplatableMCPServerManager,
    VariableType, VariableValue, builtin,
};
use crate::auth::AuthStateProvider;
use crate::auth::credentials::Credentials;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::TaskStatusUpdate;
use crate::server::server_api::managed_mcp::ManagedMcpClient;

pub(super) const MCP_SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
/// Attempt budget for resolving one managed MCP server's client config
/// (`ManagedMcpClient::create_managed_mcp_client_config`) in
/// [`AgentDriver::resolve_mcp_specs_with_local_uuids`].
///
/// A transient warp-server 5xx during this call otherwise fails the whole run (see
/// `AgentDriverError::ManagedMcpResolutionFailed`), so this needs a larger budget than the
/// shared [`crate::server::retry_strategies::MAX_ATTEMPTS`] default (~2s), which is too short
/// to ride out even a brief backend blip. Against the shared exponential backoff schedule
/// (500ms, 1s, 2s, 4s, 8s), 6 attempts sum to ~15.5s nominal (~20s with jitter): more than
/// double the shortest fully-failed window observed in a real incident (a 503 storm from
/// dying Cloud Run instances, ~6s with zero successful responses inside a ~30s degraded-
/// capacity period), while staying well under the ~35s a run had before it was marked
/// FAILED in that incident.
const MANAGED_MCP_RESOLVE_MAX_ATTEMPTS: usize = 6;

/// Fixed namespace for [`ephemeral_mcp_installation_id`]. Arbitrary; never reused.
const EPHEMERAL_MCP_INSTALLATION_NAMESPACE: Uuid = Uuid::from_bytes([
    0xf9, 0x79, 0x1a, 0x88, 0xff, 0xb7, 0x41, 0x88, 0xa6, 0x39, 0xa7, 0x2d, 0xf2, 0x94, 0x22, 0x3f,
]);

/// Installation id for an ephemeral MCP server (well-known sentinel or non-local
/// managed MCP UUID) resolved from a managed MCP client config.
///
/// With `task_id` (ambient/cloud runs), the id is deterministic: hashing run id +
/// spec token + server name means a rebuilt sandbox re-resolves the same server to
/// the same id. Ids can persist in the model's conversation history across a
/// rebuild; a random id would go stale and fail as "MCP server not found".
///
/// Without `task_id` (local sessions; no rebuilds), ids stay random: hashing only
/// the spec token would collide across concurrent conversations in the same
/// process, since `TemplatableMCPServerManager` keys installations by this id
/// process-wide.
fn ephemeral_mcp_installation_id(
    task_id: Option<AmbientAgentTaskId>,
    spec_token: &str,
    server_name: &str,
) -> Uuid {
    match task_id {
        Some(task_id) => Uuid::new_v5(
            &EPHEMERAL_MCP_INSTALLATION_NAMESPACE,
            format!("{task_id}:{spec_token}:{server_name}").as_bytes(),
        ),
        None => Uuid::new_v4(),
    }
}

/// Warn that an MCP server's `{{secret_name}}` references resolved to nothing.
/// Logs secret names only; resolved values must never reach a log line.
fn log_unresolved_secret_refs(
    installation: &TemplatableMCPServerInstallation,
    unresolved_secret_names: &[String],
) {
    if unresolved_secret_names.is_empty() {
        return;
    }
    log::warn!(
        "MCP server '{}' references secret(s) that are not available to this run: {}. \
         Check that each secret exists and is attached to this agent or run.",
        installation.templatable_mcp_server().name,
        unresolved_secret_names.join(", ")
    );
}

#[derive(Debug, Default)]
struct ResolvedMcpSpecs {
    local_uuids: Vec<Uuid>,
    ephemeral_installations: Vec<TemplatableMCPServerInstallation>,
}

/// Why an [`AgentDriver::await_model_event`] wait resolved without a value.
#[derive(Debug)]
enum ModelEventWaitError {
    /// The subscription was torn down before the predicate matched.
    SubscriptionDropped,
    TimedOut,
}

impl AgentDriver {
    /// Resolves with the first value `on_event` produces for an event from `handle`, or an
    /// error once `timeout` elapses.
    ///
    /// Owns the whole subscription lifecycle so callers only supply the predicate: any stale
    /// driver subscription to `handle` is cleared first, the subscription is removed as soon as
    /// the predicate matches, and a timed-out wait removes it again before resolving so a late
    /// event can't reach the stale closure and tear down a later wait's subscription.
    /// `unsubscribe_from_model` removes every driver subscription to `handle`, so two waits on
    /// the same model must never overlap.
    ///
    /// The timeout clock starts when the returned future is first polled, not when it is
    /// created, so a caller may subscribe ahead of the work that emits the event and only
    /// start the clock afterwards.
    fn await_model_event<S, T, F>(
        handle: &ModelHandle<S>,
        timeout: Duration,
        ctx: &mut ModelContext<Self>,
        mut on_event: F,
    ) -> impl Future<Output = Result<T, ModelEventWaitError>> + use<S, T, F>
    where
        S: Entity,
        S::Event: 'static,
        T: Send + 'static,
        F: FnMut(&S::Event, &mut ModelContext<Self>) -> Option<T> + 'static,
    {
        let (tx, rx) = oneshot::channel::<T>();
        let mut tx = Some(tx);
        ctx.unsubscribe_from_model(handle);
        ctx.subscribe_to_model(handle, move |_, handle, event, ctx| {
            let Some(value) = on_event(event, ctx) else {
                return;
            };
            if let Some(sender) = tx.take() {
                let _ = sender.send(value);
            }
            ctx.unsubscribe_from_model(&handle);
        });

        let spawner = ctx.spawner();
        let handle = handle.clone();
        async move {
            match rx.with_timeout(timeout).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(Canceled)) => Err(ModelEventWaitError::SubscriptionDropped),
                Err(TimeoutError) => {
                    // Completes before this future resolves, so it cannot race with a
                    // subsequent wait's own subscription.
                    let _ = spawner
                        .spawn(move |_, ctx| ctx.unsubscribe_from_model(&handle))
                        .await;
                    Err(ModelEventWaitError::TimedOut)
                }
            }
        }
    }

    /// Resolve MCP specs into a map of MCP name to `JSONMCPServer` for use in
    /// third-party harnesses. Each spec is fully resolved (secrets applied, templates
    /// rendered) so harnesses can serialize directly into their native config format.
    pub(super) async fn resolve_mcp_specs_to_json(
        specs: &[MCPSpec],
        secrets: Arc<HashMap<String, ManagedSecretValue>>,
        managed_mcp_client: Arc<dyn ManagedMcpClient>,
        foreground: &ModelSpawner<Self>,
    ) -> Result<HashMap<String, JSONMCPServer>, AgentDriverError> {
        let resolved_specs = Self::resolve_mcp_specs(specs, managed_mcp_client, foreground).await?;

        let local_uuids = resolved_specs.local_uuids;
        let mut installations = foreground
            .spawn(move |_, ctx| -> Result<Vec<_>, AgentDriverError> {
                let manager = TemplatableMCPServerManager::as_ref(ctx);
                local_uuids
                    .iter()
                    .map(|uuid| {
                        manager
                            .get_installed_server(uuid)
                            .cloned()
                            .ok_or(AgentDriverError::MCPServerNotFound(*uuid))
                    })
                    .collect()
            })
            .await??;
        installations.extend(resolved_specs.ephemeral_installations);

        Self::mcp_installations_to_json(installations, secrets.as_ref())
    }

    fn mcp_installations_to_json(
        mut installations: Vec<TemplatableMCPServerInstallation>,
        secrets: &HashMap<String, ManagedSecretValue>,
    ) -> Result<HashMap<String, JSONMCPServer>, AgentDriverError> {
        let mut result = HashMap::new();

        for installation in installations.iter_mut() {
            let unresolved_secret_names = installation.apply_secrets(secrets);
            if !unresolved_secret_names.is_empty() {
                return Err(AgentDriverError::MCPUnresolvedSecrets {
                    server_name: installation.templatable_mcp_server().name.clone(),
                    secret_names: unresolved_secret_names,
                });
            }
            let resolved = resolve_json(installation);
            let servers: HashMap<String, JSONMCPServer> = serde_json::from_str(&resolved)
                .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;
            result.extend(servers);
        }

        Ok(result)
    }

    fn apply_secrets_to_ephemeral_mcp_installations(
        installations: Vec<TemplatableMCPServerInstallation>,
        secrets: &HashMap<String, ManagedSecretValue>,
    ) -> (Vec<TemplatableMCPServerInstallation>, Vec<String>) {
        let mut ready = Vec::with_capacity(installations.len());
        let mut failures = Vec::new();

        for mut installation in installations {
            let unresolved_secret_names = installation.apply_secrets(secrets);
            if unresolved_secret_names.is_empty() {
                ready.push(installation);
                continue;
            }

            log_unresolved_secret_refs(&installation, &unresolved_secret_names);
            failures.push(format!(
                "'{}' was not started: unresolved secret reference(s): {}",
                installation.templatable_mcp_server().name,
                unresolved_secret_names.join(", ")
            ));
        }

        (ready, failures)
    }

    /// Resolve MCP specs into local UUIDs and ephemeral installations. UUIDs
    /// are local-first; only non-local UUIDs call managed MCP GraphQL.
    async fn resolve_mcp_specs(
        specs: &[MCPSpec],
        managed_mcp_client: Arc<dyn ManagedMcpClient>,
        foreground: &ModelSpawner<Self>,
    ) -> Result<ResolvedMcpSpecs, AgentDriverError> {
        let (local_installed_uuids, task_id) = foreground
            .spawn(|me, ctx| {
                let local_installed_uuids = TemplatableMCPServerManager::as_ref(ctx)
                    .get_installed_templatable_servers()
                    .keys()
                    .copied()
                    .collect::<HashSet<_>>();
                (local_installed_uuids, me.task_id)
            })
            .await?;

        Self::resolve_mcp_specs_with_local_uuids(
            specs,
            &local_installed_uuids,
            managed_mcp_client,
            task_id,
        )
        .await
    }

    async fn resolve_mcp_specs_with_local_uuids(
        specs: &[MCPSpec],
        local_installed_uuids: &HashSet<Uuid>,
        managed_mcp_client: Arc<dyn ManagedMcpClient>,
        task_id: Option<AmbientAgentTaskId>,
    ) -> Result<ResolvedMcpSpecs, AgentDriverError> {
        let mut resolved = ResolvedMcpSpecs::default();

        for spec in specs {
            match spec {
                MCPSpec::Uuid(uuid) if local_installed_uuids.contains(uuid) => {
                    resolved.local_uuids.push(*uuid);
                }
                MCPSpec::Uuid(uuid) => {
                    let client_config = with_bounded_retry_using(
                        &format!("resolve managed MCP server '{uuid}'"),
                        MANAGED_MCP_RESOLVE_MAX_ATTEMPTS,
                        is_transient_graphql_or_http_error,
                        || managed_mcp_client.create_managed_mcp_client_config(uuid.to_string()),
                    )
                    .await
                    .map_err(|err| {
                        AgentDriverError::ManagedMcpResolutionFailed {
                            uid: *uuid,
                            message: format!("{err:#}"),
                        }
                    })?;
                    let installations = Self::installations_from_managed_client_config_json(
                        &client_config.mcp_config_json,
                        task_id,
                        &uuid.to_string(),
                    )
                    .map_err(|err| {
                        AgentDriverError::ManagedMcpResolutionFailed {
                            uid: *uuid,
                            message: err.to_string(),
                        }
                    })?;
                    resolved.ephemeral_installations.extend(installations);
                }
                MCPSpec::WellKnown(id) => {
                    // Backstop for specs created before the flag was disabled
                    // (e.g. persisted configs): skip rather than resolve.
                    if !FeatureFlag::WellKnownMcpIds.is_enabled() {
                        log::warn!(
                            "Skipping well-known MCP server '{id}': WellKnownMcpIds is disabled"
                        );
                        continue;
                    }
                    // Well-known MCP ids (e.g. "linear") resolve best-effort:
                    // the server owns the set of recognized ids, and the
                    // backing integration may be disconnected or the feature
                    // disabled between dispatch and run setup — so resolution
                    // failures skip the server instead of failing the run. A
                    // transient failure still gets the same retry budget as the
                    // UUID case first, so a brief backend blip doesn't silently
                    // drop the server from an otherwise-healthy run.
                    let client_config = match with_bounded_retry_using(
                        &format!("resolve well-known MCP server '{id}'"),
                        MANAGED_MCP_RESOLVE_MAX_ATTEMPTS,
                        is_transient_graphql_or_http_error,
                        || managed_mcp_client.create_managed_mcp_client_config(id.clone()),
                    )
                    .await
                    {
                        Ok(client_config) => client_config,
                        Err(err) => {
                            log::warn!("Skipping well-known MCP server '{id}': {err:#}");
                            continue;
                        }
                    };
                    match Self::installations_from_managed_client_config_json(
                        &client_config.mcp_config_json,
                        task_id,
                        id,
                    ) {
                        Ok(installations) => {
                            resolved.ephemeral_installations.extend(installations);
                        }
                        Err(err) => {
                            log::warn!("Skipping well-known MCP server '{id}': {err}");
                        }
                    }
                }
                MCPSpec::Json(json_str) => {
                    resolved
                        .ephemeral_installations
                        .extend(Self::installations_from_user_mcp_json(json_str)?);
                }
            }
        }

        Ok(resolved)
    }

    /// Returns the built-in Factory MCP server installation to attach to this
    /// run, or `None` when it should not be attached.
    ///
    /// Interactive clients (GUI/TUI) attach built-in Warp-hosted servers via
    /// [`TemplatableMCPServerManager::sync_builtin_servers`], which skips CLI
    /// agent runs. The driver mirrors the same eligibility rules for its
    /// run-scoped ephemeral startup path: the `FactoryMcp` feature flag, a
    /// usable bearer token, and no configured server already named
    /// `warp-factory` (an explicit configuration wins over the built-in).
    ///
    /// The token is pinned into the transport at spawn time and is not
    /// refreshed mid-run: cloud runs authenticate with API keys, which do not
    /// rotate, so only Firebase-authenticated local runs that outlive their
    /// token would see factory tool calls start failing.
    fn builtin_factory_mcp_for_run(
        credentials: Option<&Credentials>,
        taken_server_names: &HashSet<String>,
    ) -> Option<TemplatableMCPServerInstallation> {
        if !FeatureFlag::FactoryMcp.is_enabled() {
            return None;
        }
        if taken_server_names.contains(builtin::FACTORY_MCP_SERVER_NAME) {
            log::info!(
                "Skipping the built-in Factory MCP server: a server named '{}' is already configured for this run",
                builtin::FACTORY_MCP_SERVER_NAME
            );
            return None;
        }
        let token = builtin::builtin_bearer_token(credentials?)?;
        log::info!("Attaching the built-in Factory MCP server to this agent run");
        Some(builtin::factory_mcp_installation(&token))
    }

    fn installations_from_user_mcp_json(
        json_str: &str,
    ) -> Result<Vec<TemplatableMCPServerInstallation>, AgentDriverError> {
        let normalized_json = normalize_mcp_json(json_str)
            .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;
        let parsed_results = ParsedTemplatableMCPServerResult::from_user_json(&normalized_json)
            .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;

        parsed_results
            .into_iter()
            .map(|result| {
                result
                    .templatable_mcp_server_installation
                    .ok_or(AgentDriverError::MCPMissingVariables)
            })
            .collect()
    }

    fn installations_from_managed_client_config_json(
        json_str: &str,
        task_id: Option<AmbientAgentTaskId>,
        spec_token: &str,
    ) -> Result<Vec<TemplatableMCPServerInstallation>, AgentDriverError> {
        let normalized_json = normalize_mcp_json(json_str)
            .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;
        let parsed_results = ParsedTemplatableMCPServerResult::from_user_json(&normalized_json)
            .map_err(|e| AgentDriverError::MCPJsonParseError(e.to_string()))?;

        parsed_results
            .into_iter()
            .map(|result| {
                let ParsedTemplatableMCPServerResult {
                    mut templatable_mcp_server,
                    mut variable_values,
                    ..
                } = result;

                // Server-rendered literal values (no `{{...}}` ref) must be preserved verbatim.
                // Drop them from the template's variable list so `apply_secrets` never sees them —
                // its implicit key-name matching would otherwise let a colliding local secret
                // (e.g. one named `Authorization`) overwrite a server-issued proxy header.
                // They stay in `variable_values`, so `resolve_json` still renders them into the
                // config.
                templatable_mcp_server
                    .template
                    .variables
                    .retain(|variable| {
                        let is_literal = variable_values
                            .get(&variable.key)
                            .is_some_and(|v| get_arguments(&v.value).is_empty());
                        !is_literal
                    });

                // Remaining variables are explicit `{{...}}` placeholders the client fills from
                // local secrets via `apply_secrets`. Synthesize a placeholder value for any not
                // captured from env/headers (e.g. command-arg refs like `--token={{API_TOKEN}}`).
                for variable in templatable_mcp_server.template.variables.iter() {
                    variable_values
                        .entry(variable.key.clone())
                        .or_insert_with(|| VariableValue {
                            variable_type: VariableType::Text,
                            value: format!("{{{{{}}}}}", variable.key),
                        });
                }

                let installation_id = ephemeral_mcp_installation_id(
                    task_id,
                    spec_token,
                    &templatable_mcp_server.name,
                );
                Ok(TemplatableMCPServerInstallation::new(
                    installation_id,
                    templatable_mcp_server,
                    variable_values,
                ))
            })
            .collect()
    }

    /// Starts the MCP servers requested for the run (`--mcp` specs plus the built-in Factory
    /// server) and waits for them to settle. Both startup phases run even when one degrades,
    /// collecting degradation details so non-strict runs can continue with whichever servers
    /// did start.
    pub(super) async fn start_task_mcp_servers(
        mcp_specs: &[MCPSpec],
        managed_mcp_client: Arc<dyn ManagedMcpClient>,
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let resolved_mcp_specs =
            Self::resolve_mcp_specs(mcp_specs, managed_mcp_client, foreground).await?;
        let existing_uuids = resolved_mcp_specs.local_uuids;
        let mut ephemeral_installations = resolved_mcp_specs.ephemeral_installations;

        // Attach the built-in Factory MCP server. Interactive clients attach built-ins via
        // `TemplatableMCPServerManager::sync_builtin_servers`, which skips CLI agent runs, so
        // the driver injects the same code-owned installation here, scoped to this run.
        let local_uuids = existing_uuids.clone();
        let mut taken_server_names: HashSet<String> = ephemeral_installations
            .iter()
            .map(|installation| installation.templatable_mcp_server().name.clone())
            .collect();
        let credentials = foreground
            .spawn(move |_, ctx| {
                let (local_names, builtin_already_active) = {
                    let manager = TemplatableMCPServerManager::as_ref(ctx);
                    let local_names = local_uuids
                        .iter()
                        .filter_map(|uuid| {
                            manager.get_installed_server(uuid).map(|installation| {
                                installation.templatable_mcp_server().name.clone()
                            })
                        })
                        .collect::<Vec<_>>();
                    let builtin_already_active =
                        manager.is_server_active_or_pending(builtin::FACTORY_MCP_INSTALLATION_UUID);
                    (local_names, builtin_already_active)
                };
                // Interactive clients (GUI/TUI) attach built-ins through `sync_builtin_servers`,
                // under the same stable installation UUID. The driver currently only runs in SDK
                // mode, where that path never spawns, but guard anyway so this injection can
                // never double-spawn the built-in if the driver is ever hosted in an interactive
                // process.
                let builtin_owned_by_manager = builtin_already_active
                    || AppExecutionMode::as_ref(ctx).can_autostart_mcp_servers();
                let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
                let credentials = (!builtin_owned_by_manager
                    && !auth_state.is_anonymous_or_logged_out())
                .then(|| auth_state.credentials())
                .flatten();
                (credentials, local_names)
            })
            .await
            .map(|(credentials, local_names)| {
                taken_server_names.extend(local_names);
                credentials
            })?;
        if let Some(installation) =
            Self::builtin_factory_mcp_for_run(credentials.as_ref(), &taken_server_names)
        {
            ephemeral_installations.push(installation);
        }

        log::info!(
            "Starting {} existing and {} ephemeral MCP servers",
            existing_uuids.len(),
            ephemeral_installations.len()
        );

        let mut degraded = Vec::new();
        if !existing_uuids.is_empty() {
            let result = foreground
                .spawn(move |me, ctx| me.start_mcp_servers(&existing_uuids, ctx))
                .await?
                .await;
            Self::collect_mcp_degradation(result, &mut degraded)?;
        }
        if !ephemeral_installations.is_empty() {
            let result = foreground
                .spawn(move |me, ctx| me.start_ephemeral_mcp_servers(ephemeral_installations, ctx))
                .await?
                .await;
            Self::collect_mcp_degradation(result, &mut degraded)?;
        }
        if degraded.is_empty() {
            Ok(())
        } else {
            Err(AgentDriverError::MCPStartupFailed { details: degraded })
        }
    }

    /// Start MCP servers from profile allowlist for the terminal.
    pub(super) fn start_profile_mcp_servers(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> + use<> {
        let terminal_id = self.terminal_driver.as_ref(ctx).terminal_view().id();
        let permissions = BlocklistAIPermissions::as_ref(ctx);
        let profile_allowlist = permissions.get_mcp_allowlist(ctx, Some(terminal_id));

        if !profile_allowlist.is_empty() {
            log::info!(
                "Starting {} MCP servers allowlisted in profile",
                profile_allowlist.len()
            );
        }
        self.start_mcp_servers(&profile_allowlist, ctx)
    }

    fn get_mcp_servers_to_start(
        &self,
        uuids: &[Uuid],
        ctx: &mut ModelContext<Self>,
    ) -> Result<HashSet<Uuid>, AgentDriverError> {
        let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);

        let mut servers_to_start: HashSet<Uuid> = HashSet::new();

        for uuid in uuids.iter() {
            if templatable_mcp_manager
                .as_ref(ctx)
                .is_server_active_or_pending(*uuid)
            {
                log::debug!("MCP server {uuid} is already active or pending; skipping");
                continue;
            } else if templatable_mcp_manager
                .as_ref(ctx)
                .get_installed_server(uuid)
                .is_some()
            {
                servers_to_start.insert(*uuid);
            } else {
                return Err(AgentDriverError::MCPServerNotFound(*uuid));
            }
        }

        Ok(servers_to_start)
    }

    /// Wait for every server in `servers` (keyed by installation UUID, valued by display name)
    /// to reach a terminal state (`Running` or `FailedToStart`), up to the configured startup
    /// timeout.
    ///
    /// Returns [`AgentDriverError::MCPStartupFailed`] naming the servers that failed to start
    /// or were still starting at the deadline. Callers decide whether that is fatal (see strict
    /// MCP startup handling in `run_internal`).
    ///
    /// Must be called before the servers are spawned so no state changes are missed. See
    /// [`Self::await_model_event`] for why waits on the manager must not overlap.
    fn wait_for_mcp_servers_started(
        &self,
        servers: HashMap<Uuid, String>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> + use<> {
        if servers.is_empty() {
            return Either::Right(future::ready(Ok(())));
        }

        // Stall for user-configured timeout, else 20 seconds (configured in [`AgentDriverOptions`]).
        let timeout = self.mcp_startup_timeout;
        let pending = Arc::new(Mutex::new(servers));
        let failed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let pending_for_subscription = Arc::clone(&pending);
        let failed_for_subscription = Arc::clone(&failed);

        let manager = TemplatableMCPServerManager::handle(ctx);
        let wait = Self::await_model_event(&manager, timeout, ctx, move |event, ctx| {
            let TemplatableMCPServerManagerEvent::StateChanged { uuid, state } = event else {
                return None;
            };
            let mut pending_servers = pending_for_subscription.lock().ok()?;
            // A state change for a server that isn't awaited is ignored.
            let name = pending_servers.get(uuid).cloned()?;
            match state {
                MCPServerState::Running => {
                    pending_servers.remove(uuid);
                }
                MCPServerState::FailedToStart => {
                    pending_servers.remove(uuid);
                    let error = TemplatableMCPServerManager::as_ref(ctx)
                        .get_server_error_message(*uuid)
                        .map(|message| format!(": {message}"))
                        .unwrap_or_default();
                    let detail = format!("'{name}' failed to start{error}");
                    log::warn!("MCP server {detail}");
                    if let Ok(mut failed_servers) = failed_for_subscription.lock() {
                        failed_servers.push(detail);
                    }
                }
                MCPServerState::NotRunning
                | MCPServerState::Starting
                | MCPServerState::Authenticating
                | MCPServerState::ShuttingDown => return None,
            }
            if !pending_servers.is_empty() {
                return None;
            }
            log::info!("All requested MCP servers reached a terminal state");
            Some(())
        });

        Either::Left(async move {
            let mut still_starting: Vec<String> = Vec::new();
            match wait.await {
                Ok(()) => {}
                Err(ModelEventWaitError::SubscriptionDropped) => {
                    log::error!("Subscription dropped before MCP servers started");
                    return Err(AgentDriverError::InvalidRuntimeState);
                }
                Err(ModelEventWaitError::TimedOut) => {
                    still_starting = pending
                        .lock()
                        .map(|pending_servers| pending_servers.values().cloned().collect())
                        .unwrap_or_default();
                    still_starting.sort();
                }
            }

            let mut details = failed
                .lock()
                .map(|failed_servers| failed_servers.clone())
                .unwrap_or_default();
            details.sort();
            details.extend(
                still_starting
                    .iter()
                    .map(|name| format!("'{name}' did not start within {}s", timeout.as_secs())),
            );

            if details.is_empty() {
                Ok(())
            } else {
                Err(AgentDriverError::MCPStartupFailed { details })
            }
        })
    }

    /// Fold an MCP startup result into `degraded`, propagating any error that
    /// is fatal regardless of the strict MCP startup setting.
    fn collect_mcp_degradation(
        result: Result<(), AgentDriverError>,
        degraded: &mut Vec<String>,
    ) -> Result<(), AgentDriverError> {
        match result {
            Ok(()) => Ok(()),
            Err(AgentDriverError::MCPStartupFailed { details }) => {
                degraded.extend(details);
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// Apply strict MCP startup handling to a recorded startup result.
    ///
    /// Degraded startup (`MCPStartupFailed`) is fatal only in strict mode.
    /// Otherwise the run continues without the unavailable servers: the
    /// degradation is logged and reported as a run status message.
    pub(super) async fn handle_mcp_startup_result(
        result: Result<(), AgentDriverError>,
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let Err(error) = result else {
            return Ok(());
        };
        let AgentDriverError::MCPStartupFailed { details } = &error else {
            return Err(error);
        };
        let details = details.join("; ");

        let strict = foreground.spawn(|me, _| me.strict_mcp_startup).await?;
        if strict {
            return Err(error);
        }

        log::warn!(
            "MCP startup degraded ({details}); continuing without the unavailable MCP servers"
        );

        // Surface the degradation on the run itself. The server currently only
        // persists status messages on terminal state transitions, so this is
        // best-effort until message-only updates are supported.
        let (task_id, ai_client) = foreground
            .spawn(|me, ctx| {
                (
                    me.task_id,
                    ServerApiProvider::as_ref(ctx).get_ai_client().clone(),
                )
            })
            .await?;
        if let Some(task_id) = task_id {
            let message = format!(
                "Warning: some MCP servers were unavailable during startup ({details}); continuing without their tools."
            );
            if let Err(err) = ai_client
                .update_agent_task(
                    task_id,
                    None,
                    None,
                    None,
                    Some(TaskStatusUpdate::message(message)),
                    None,
                    None,
                )
                .await
            {
                log::warn!("Failed to report MCP startup warning for task {task_id}: {err:#}");
            }
        }
        Ok(())
    }

    fn spawn_inactive_servers(
        &self,
        servers_to_start: HashSet<Uuid>,
        ctx: &mut ModelContext<Self>,
    ) {
        let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);
        templatable_mcp_manager.update(ctx, |manager, ctx| {
            for uuid in servers_to_start {
                manager.spawn_server(uuid, ctx);
            }
        });
    }

    fn start_mcp_servers(
        &self,
        uuids: &[Uuid],
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> + use<> {
        let servers_to_start = match self.get_mcp_servers_to_start(uuids, ctx) {
            Ok(val) => val,
            Err(e) => {
                return Either::Right(future::ready(Err(e)));
            }
        };

        // If we don't need to start any servers, complete immediately.
        if servers_to_start.is_empty() {
            return Either::Right(future::ready(Ok(())));
        }

        log::info!("Starting {} MCP servers...", servers_to_start.len());

        let named_servers: HashMap<Uuid, String> = {
            let manager = TemplatableMCPServerManager::as_ref(ctx);
            servers_to_start
                .iter()
                .map(|uuid| {
                    let name = manager
                        .get_installed_server(uuid)
                        .map(|installation| installation.templatable_mcp_server().name.clone())
                        .unwrap_or_else(|| uuid.to_string());
                    (*uuid, name)
                })
                .collect()
        };
        let wait = self.wait_for_mcp_servers_started(named_servers, ctx);

        self.spawn_inactive_servers(servers_to_start, ctx);

        Either::Left(wait)
    }

    /// Start ephemeral MCP servers from inline JSON specifications.
    /// These servers are not persisted and exist only for the duration of the agent run.
    fn start_ephemeral_mcp_servers(
        &self,
        installations: Vec<TemplatableMCPServerInstallation>,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> + use<> {
        if installations.is_empty() {
            return Either::Right(future::ready(Ok(())));
        }
        let (installations, mut unresolved_failures) =
            Self::apply_secrets_to_ephemeral_mcp_installations(installations, &self.secrets);

        if !installations.is_empty() {
            log::info!("Starting {} ephemeral MCP servers...", installations.len());
        }

        let named_servers: HashMap<Uuid, String> = installations
            .iter()
            .map(|installation| {
                (
                    installation.uuid(),
                    installation.templatable_mcp_server().name.clone(),
                )
            })
            .collect();
        let wait = self.wait_for_mcp_servers_started(named_servers, ctx);

        // Spawn the ephemeral servers.
        let templatable_mcp_manager = TemplatableMCPServerManager::handle(ctx);
        templatable_mcp_manager.update(ctx, move |manager, ctx| {
            for installation in installations {
                manager.spawn_cli_ephemeral_server(installation, ctx);
            }
        });

        Either::Left(async move {
            let mut failures = match wait.await {
                Ok(()) => Vec::new(),
                Err(AgentDriverError::MCPStartupFailed { details }) => details,
                Err(other) => return Err(other),
            };
            failures.append(&mut unresolved_failures);
            if failures.is_empty() {
                Ok(())
            } else {
                failures.sort();
                Err(AgentDriverError::MCPStartupFailed { details: failures })
            }
        })
    }

    /// Awaits a file-based MCP `scan`, then the readiness of the servers it reports, within
    /// one shared `timeout`: the readiness wait only ever gets what the scan left of it, so
    /// the two phases can't compound into two full timeouts. `scan` is expected to bound
    /// itself by the same `timeout`.
    pub(super) async fn await_file_based_mcp_startup(
        scan_step: SetupStep,
        readiness_step: SetupStep,
        scan: impl Future<Output = Vec<Uuid>>,
        timeout: Duration,
        setup_events: &SetupClientEventReporter,
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        // Monotonic, so a backward wall-clock correction can't stretch the remaining budget.
        let deadline = Instant::now() + timeout;
        let wait_uuids = setup_events.record_value(scan_step, scan).await;
        if wait_uuids.is_empty() {
            return Ok(());
        }

        log::info!(
            "Checking readiness for {} file-based MCP server(s)",
            wait_uuids.len()
        );
        setup_events
            .record_result(readiness_step, async {
                foreground
                    .spawn(move |me, ctx| {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        me.wait_for_file_based_mcps_running(wait_uuids, remaining, ctx)
                    })
                    .await?
                    .await;
                Ok::<(), AgentDriverError>(())
            })
            .await
    }

    /// Awaits [`FileBasedMCPManagerEvent::CloudEnvMcpScanComplete`] for every repo in
    /// `expected_repos`, resolving with the UUIDs of servers auto-start requested while those
    /// repos were scanned. Must be called **before** `prepare_environment` so no events are
    /// missed; the `timeout` clock only starts once the returned future is polled. Non-fatal:
    /// resolves with an empty set on timeout or cancellation.
    pub(super) fn wait_for_cloud_env_file_based_mcp_scan(
        &self,
        expected_repos: Vec<PathBuf>,
        timeout: Duration,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Vec<Uuid>> + use<> {
        if expected_repos.is_empty() {
            return Either::Right(future::ready(vec![]));
        }

        log::info!(
            "Waiting for {} cloud environment repo(s) to report back file-based MCP server UUIDs...",
            expected_repos.len()
        );

        let mut pending_repos: HashSet<PathBuf> = HashSet::from_iter(expected_repos);
        let mut collected_wait_uuids = Vec::<Uuid>::new();
        let manager = FileBasedMCPManager::handle(ctx);
        let wait = Self::await_model_event(&manager, timeout, ctx, move |event, _| {
            let FileBasedMCPManagerEvent::CloudEnvMcpScanComplete {
                repo_path,
                wait_server_uuids,
                ..
            } = event
            else {
                return None;
            };
            if !pending_repos.remove(repo_path) {
                return None;
            }
            collected_wait_uuids.extend(wait_server_uuids.iter().copied());
            if !pending_repos.is_empty() {
                return None;
            }
            log::info!(
                "Collected {} auto-started file-based MCP server(s) from cloud environment repos",
                collected_wait_uuids.len()
            );
            Some(mem::take(&mut collected_wait_uuids))
        });

        Either::Left(async move {
            match wait.await {
                Ok(wait_uuids) => wait_uuids,
                Err(ModelEventWaitError::SubscriptionDropped) => {
                    log::warn!(
                        "File-based MCP discovery subscription dropped early; proceeding without"
                    );
                    vec![]
                }
                Err(ModelEventWaitError::TimedOut) => {
                    log::warn!(
                        "Timed out waiting for file-based MCP servers to be parsed; proceeding without"
                    );
                    vec![]
                }
            }
        })
    }

    /// Whether `uuid` still needs to be awaited by [`Self::wait_for_file_based_mcps_running`]:
    /// it must still be tracked by [`FileBasedMCPManager`] (i.e. its config has not been
    /// removed) and not yet have reached a terminal state (`Running`, `FailedToStart`, or
    /// `NotRunning`). A config removed before or during the wait despawns its installation and
    /// reports `NotRunning`, which settles the wait rather than blocking it: later file changes
    /// must stay dynamic and never delay the first request.
    fn is_file_based_mcp_pending(
        templatable_manager: &TemplatableMCPServerManager,
        file_based_manager: &FileBasedMCPManager,
        uuid: Uuid,
    ) -> bool {
        file_based_manager.get_hash_by_uuid(uuid).is_some()
            && !matches!(
                templatable_manager.get_server_state(uuid),
                Some(MCPServerState::Running)
                    | Some(MCPServerState::FailedToStart)
                    | Some(MCPServerState::NotRunning)
            )
    }

    /// Wait for auto-start-requested file-based MCP servers to reach a terminal state
    /// (`Running`, `FailedToStart`, or a despawn's `NotRunning`), bounded by `timeout`.
    /// Non-fatal: always completes without returning an error.
    ///
    /// `timeout` is the caller's remaining budget rather than a fresh one: a caller that
    /// already spent part of a shared deadline on a preceding phase (e.g. awaiting scan
    /// completion) must pass only what is left of it, so the combined phases stay within one
    /// bounded window instead of compounding into multiple full timeouts.
    ///
    /// See [`Self::await_model_event`] for why this must never run concurrently with
    /// [`Self::start_mcp_servers`] or [`Self::start_ephemeral_mcp_servers`].
    fn wait_for_file_based_mcps_running(
        &self,
        uuids: Vec<Uuid>,
        timeout: Duration,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = ()> + use<> {
        // Filter out UUIDs that have already reached a terminal state, or whose installation
        // is no longer tracked at all (e.g. despawned before this wait ever subscribed).
        let mut pending_uuids: HashSet<Uuid> = {
            let templatable_manager = TemplatableMCPServerManager::as_ref(ctx);
            let file_based_manager = FileBasedMCPManager::as_ref(ctx);
            uuids
                .into_iter()
                .filter(|uuid| {
                    Self::is_file_based_mcp_pending(templatable_manager, file_based_manager, *uuid)
                })
                .collect()
        };

        if pending_uuids.is_empty() {
            log::info!("All file-based MCP servers have reached a terminal state; proceeding");
            return Either::Right(future::ready(()));
        }

        let file_based_mcp_names = {
            let file_based_manager = FileBasedMCPManager::as_ref(ctx);
            pending_uuids
                .iter()
                .map(|uuid| {
                    let server_name = file_based_manager
                        .get_installation_by_uuid(*uuid)
                        .map(|installation| installation.templatable_mcp_server().name.clone())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    (*uuid, server_name)
                })
                .collect::<HashMap<_, _>>()
        };
        let pending_state_details = {
            let templatable_manager = TemplatableMCPServerManager::as_ref(ctx);
            Arc::new(Mutex::new(
                pending_uuids
                    .iter()
                    .map(|uuid| {
                        let server_name = file_based_mcp_names
                            .get(uuid)
                            .map(String::as_str)
                            .unwrap_or("<unknown>");
                        let state = templatable_manager
                            .get_server_state(*uuid)
                            .map(|state| format!("{state:?}"))
                            .unwrap_or_else(|| "no state".to_string());
                        let error = templatable_manager
                            .get_server_error_message(*uuid)
                            .map(|message| format!(", error={message}"))
                            .unwrap_or_default();
                        (*uuid, format!("{server_name} ({uuid}): {state}{error}"))
                    })
                    .collect::<HashMap<_, _>>(),
            ))
        };
        log::info!(
            "Waiting for {} file-based MCP server(s) to reach a terminal state",
            pending_uuids.len()
        );

        let manager = TemplatableMCPServerManager::handle(ctx);
        let pending_state_details_for_subscription = Arc::clone(&pending_state_details);
        let wait = Self::await_model_event(&manager, timeout, ctx, move |event, ctx| {
            let TemplatableMCPServerManagerEvent::StateChanged { uuid, state } = event else {
                return None;
            };
            if !pending_uuids.contains(uuid) {
                return None;
            }
            let server_name = file_based_mcp_names
                .get(uuid)
                .map(String::as_str)
                .unwrap_or("<unknown>");
            let error = TemplatableMCPServerManager::as_ref(ctx)
                .get_server_error_message(*uuid)
                .map(|message| format!(", error={message}"))
                .unwrap_or_default();
            if let Ok(mut details) = pending_state_details_for_subscription.lock() {
                details.insert(*uuid, format!("{server_name} ({uuid}): {state:?}{error}"));
            }
            match state {
                MCPServerState::Running
                | MCPServerState::FailedToStart
                | MCPServerState::NotRunning => {
                    pending_uuids.remove(uuid);
                    if let Ok(mut details) = pending_state_details_for_subscription.lock() {
                        details.remove(uuid);
                    }
                }
                MCPServerState::Starting
                | MCPServerState::Authenticating
                | MCPServerState::ShuttingDown => return None,
            }
            if !pending_uuids.is_empty() {
                return None;
            }
            log::info!("All file-based MCP servers reached a terminal state; proceeding");
            Some(())
        });

        Either::Left(async move {
            match wait.await {
                Ok(()) => {}
                Err(ModelEventWaitError::SubscriptionDropped) => {
                    log::warn!(
                        "File-based MCP server readiness subscription dropped early; proceeding"
                    );
                }
                Err(ModelEventWaitError::TimedOut) => {
                    let pending_details = pending_state_details
                        .lock()
                        .map(|details| details.values().cloned().join("; "))
                        .unwrap_or_else(|_| "<unable to read pending state>".to_string());
                    log::warn!(
                        "Timed out waiting for file-based MCP servers to reach a terminal state; proceeding without. Still pending: {pending_details}"
                    );
                }
            }
        })
    }

    /// Await the one-time initial global home-config MCP scan (e.g. `~/.warp/.mcp.json`,
    /// `~/.claude.json`, `~/.codex/config.toml`), returning the UUIDs of servers that were
    /// actually auto-start requested while it ran.
    ///
    /// Checks [`FileBasedMCPManager::initial_global_scan_result`] before subscribing for the
    /// transient completion event. Non-fatal: resolves with an empty snapshot on timeout or
    /// cancellation.
    pub(super) fn wait_for_initial_global_file_based_mcp_scan(
        &self,
        timeout: Duration,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Vec<Uuid>> + use<> {
        let manager = FileBasedMCPManager::handle(ctx);
        if let Some(wait_uuids) = manager.as_ref(ctx).initial_global_scan_result() {
            return Either::Right(future::ready(wait_uuids));
        }

        log::info!("Waiting for the initial global file-based MCP scan to complete...");
        let wait = Self::await_model_event(&manager, timeout, ctx, |event, _| {
            let FileBasedMCPManagerEvent::InitialGlobalMcpScanComplete { wait_server_uuids } =
                event
            else {
                return None;
            };
            Some(wait_server_uuids.clone())
        });

        Either::Left(async move {
            match wait.await {
                Ok(wait_uuids) => wait_uuids,
                Err(ModelEventWaitError::SubscriptionDropped) => {
                    log::warn!(
                        "Initial global file-based MCP scan subscription dropped early; proceeding without"
                    );
                    vec![]
                }
                Err(ModelEventWaitError::TimedOut) => {
                    log::warn!(
                        "Timed out waiting for the initial global file-based MCP scan to complete; proceeding without"
                    );
                    vec![]
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "mcp_startup_tests.rs"]
mod tests;
