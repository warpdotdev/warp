//! Keeps the proxy credentials of a run's managed MCP servers valid: replaces each one shortly
//! before it expires, and on request once a server has rejected it.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use futures::channel::{mpsc, oneshot};
use futures::future::{self, BoxFuture};
use futures::{FutureExt as _, StreamExt as _};
use mcp::refreshable_headers::{RefreshableHeaders, RejectionHandler};
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_graphql::mutations::create_managed_mcp_client_config::{
    CreateManagedMcpClientConfigOutput, ManagedMcpTransportKind,
};
use warp_server_client::HttpStatusError;
use warp_server_client::graphql_helpers::is_auth_rejection;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{ModelSpawner, SingletonEntity};

use super::AgentDriver;
use crate::ai::mcp::parsing::resolve_json;
use crate::ai::mcp::{JSONMCPServer, JSONTransportType, TemplatableMCPServerInstallation};
use crate::auth::AuthStateProvider;
use crate::server::graphql::GraphQLError;
use crate::server::retry_strategies::{is_auth_error, is_transient_graphql_or_http_error};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::managed_mcp::{ManagedMcpClient, ManagedMcpClientConfigRejected};

/// How long before a credential expires its replacement is requested.
const REFRESH_LEAD_TIME: Duration = Duration::from_secs(15 * 60);
/// Upper bound of the random extra lead time, which keeps runs that started together from all
/// refreshing at the same moment.
const REFRESH_LEAD_JITTER: Duration = Duration::from_secs(2 * 60);
/// Least time between two mint attempts for one credential, whatever asked for them.
const MIN_ATTEMPT_INTERVAL: Duration = Duration::from_secs(60);
/// Longest wait before a failed mint is tried again, before jitter.
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Upper bound of the random share added to a retry wait.
const RETRY_JITTER: f32 = 0.2;
/// Longest single sleep. Timers do not advance while the machine is suspended, so the schedule
/// is re-read from the wall clock at least this often.
const MAX_SLEEP: Duration = Duration::from_secs(60);
/// How long a rejected request waits for the refresh it asked for before failing.
const REJECTION_REFRESH_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest wait for the answer to one mint. Shorter than [`REJECTION_REFRESH_TIMEOUT`], so a
/// request that asked for the mint still hears how it ended.
const MINT_TIMEOUT: Duration = Duration::from_secs(10);

/// A credential the managed MCP proxy accepts until it expires.
#[derive(Clone)]
pub(crate) struct ManagedMcpCredential {
    pub(crate) header_value: String,
    pub(crate) expires_at: DateTime<Utc>,
}

impl fmt::Debug for ManagedMcpCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedMcpCredential")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// The replaceable credential of one managed MCP server installation.
#[derive(Clone, Debug)]
pub(crate) struct ManagedMcpLease {
    pub(crate) installation_id: Uuid,
    pub(crate) server_name: String,
    /// The managed server uid or well-known integration id a replacement is minted for.
    pub(crate) warp_id: String,
    pub(crate) header_name: String,
    pub(crate) credential: ManagedMcpCredential,
}

/// The leases of the `installations` resolved from `config`, which was minted for `warp_id`.
/// Empty when the config carries no expiring credential.
pub(super) fn leases_for_installations(
    config: &CreateManagedMcpClientConfigOutput,
    installations: &[TemplatableMCPServerInstallation],
    warp_id: &str,
) -> Vec<ManagedMcpLease> {
    if !FeatureFlag::ManagedMcpCredentialRefresh.is_enabled() {
        return Vec::new();
    }
    let Some((header_name, credential)) = credential_from_client_config(config) else {
        return Vec::new();
    };
    let leases: Vec<_> = installations
        .iter()
        .filter(|installation| sends_header(installation, &header_name, &credential.header_value))
        .map(|installation| ManagedMcpLease {
            installation_id: installation.uuid(),
            server_name: installation.templatable_mcp_server().name.clone(),
            warp_id: warp_id.to_owned(),
            header_name: header_name.clone(),
            credential: credential.clone(),
        })
        .collect();
    if leases.is_empty() && !installations.is_empty() {
        log::warn!(
            "The credential of managed MCP server '{warp_id}' will not be refreshed: no \
             installation resolved from its config sends it"
        );
    }
    leases
}

/// The header name and expiring credential of a URL-backed config.
fn credential_from_client_config(
    config: &CreateManagedMcpClientConfigOutput,
) -> Option<(String, ManagedMcpCredential)> {
    match config.transport_kind {
        ManagedMcpTransportKind::Url => {}
        ManagedMcpTransportKind::Command => return None,
    }
    Some((
        config.authorization_header_name.clone()?,
        ManagedMcpCredential {
            header_value: config.authorization_header_value.clone()?,
            expires_at: config.expires_at?.utc(),
        },
    ))
}

/// Whether `installation` connects over HTTP sending `value` in the header `name`.
fn sends_header(installation: &TemplatableMCPServerInstallation, name: &str, value: &str) -> bool {
    let Ok(servers) =
        serde_json::from_str::<HashMap<String, JSONMCPServer>>(&resolve_json(installation))
    else {
        return false;
    };
    servers.values().any(|server| match &server.transport_type {
        JSONTransportType::SSEServer { headers, .. } => headers
            .iter()
            .any(|(header, sent)| header.eq_ignore_ascii_case(name) && sent == value),
        JSONTransportType::CLIServer { .. } => false,
    })
}

/// Why a credential could not be minted.
#[derive(Debug)]
pub(crate) enum MintError {
    /// The request may succeed if it is sent again.
    Transient(anyhow::Error),
    /// The server rejected this process's own credentials.
    Unauthenticated(anyhow::Error),
    /// The server declined to issue a credential, or issued one that cannot replace the
    /// current one.
    Rejected(anyhow::Error),
}

impl MintError {
    fn from_request_error(error: anyhow::Error) -> Self {
        if error
            .chain()
            .any(|cause| cause.is::<ManagedMcpClientConfigRejected>())
        {
            return Self::Rejected(error);
        }
        if is_auth_error(&error) || is_auth_rejection(&error) {
            return Self::Unauthenticated(error);
        }
        let is_transport_error = error
            .chain()
            .any(|cause| cause.is::<GraphQLError>() || cause.is::<HttpStatusError>());
        if is_transport_error && !is_transient_graphql_or_http_error(&error) {
            return Self::Rejected(error);
        }
        // Anything else never reached the server or got no verdict from it (a failure before the
        // request was sent, GraphQL errors without data), so it says nothing about the next try.
        Self::Transient(error)
    }
}

/// Mints a credential that can take the place of the one `lease` holds.
pub(crate) async fn mint_credential(
    client: &dyn ManagedMcpClient,
    lease: &ManagedMcpLease,
) -> Result<ManagedMcpCredential, MintError> {
    let config = client
        .create_managed_mcp_client_config(lease.warp_id.clone())
        .await
        .map_err(MintError::from_request_error)?;
    let (header_name, credential) = credential_from_client_config(&config).ok_or_else(|| {
        MintError::Rejected(anyhow!("the server returned no expiring credential"))
    })?;
    if !header_name.eq_ignore_ascii_case(&lease.header_name) {
        return Err(MintError::Rejected(anyhow!(
            "the server now sends the credential in a different header"
        )));
    }
    Ok(credential)
}

/// When to ask for the replacement of a credential that expires at `expires_at`: `lead_time`
/// ahead of the expiry, or at `not_before` when that is later.
fn next_refresh_at(
    expires_at: DateTime<Utc>,
    lead_time: Duration,
    not_before: DateTime<Utc>,
) -> DateTime<Utc> {
    (expires_at - lead_time).max(not_before)
}

/// How long to wait before retrying after `consecutive_failures` failed mints, with `jitter` a
/// fraction in `[0, 1)`.
fn retry_interval(consecutive_failures: u32, jitter: f32) -> Duration {
    let doublings = consecutive_failures.saturating_sub(1).min(16);
    let base = MIN_ATTEMPT_INTERVAL
        .saturating_mul(1 << doublings)
        .min(MAX_RETRY_INTERVAL);
    base + base.mul_f32(RETRY_JITTER * jitter)
}

/// Whether a `WWW-Authenticate` challenge is the managed MCP proxy saying that a newly minted
/// credential would be accepted.
fn is_refreshable_rejection(challenge: &str) -> bool {
    challenge.contains("proxy_token_expired") || challenge.contains("proxy_token_stale")
}

/// The loop's sources of time and randomness, injected so tests can drive the schedule.
pub(crate) struct RefreshClock {
    pub(crate) now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    pub(crate) sleep: Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>,
    /// Returns a fraction in `[0, 1)` used to spread refreshes and retries.
    pub(crate) jitter: Arc<dyn Fn() -> f32 + Send + Sync>,
}

impl Default for RefreshClock {
    fn default() -> Self {
        Self {
            now: Arc::new(Utc::now),
            sleep: Arc::new(|duration| {
                async move {
                    Timer::after(duration).await;
                }
                .boxed()
            }),
            jitter: Arc::new(rand::random),
        }
    }
}

/// A server's rejection of the credential of `installation_id`, answered through `done` once
/// the credential has been replaced or will not be.
struct RefreshRequest {
    installation_id: Uuid,
    done: oneshot::Sender<()>,
}

/// The replaceable credentials of the managed MCP servers started for a run.
pub(crate) struct ManagedMcpRefresh {
    leases: Vec<(ManagedMcpLease, RefreshableHeaders)>,
    requests: mpsc::UnboundedReceiver<RefreshRequest>,
    /// Whether the loop has started answering `requests`. Servers connect before it does, and a
    /// request sent then would only be waited on until it timed out.
    running: Arc<AtomicBool>,
}

impl ManagedMcpRefresh {
    /// Makes the credential of each lease replaceable on an open connection. `None` when no
    /// lease can be.
    pub(super) fn new(leases: impl IntoIterator<Item = ManagedMcpLease>) -> Option<Self> {
        let (sender, requests) = mpsc::unbounded();
        let running = Arc::new(AtomicBool::new(false));
        let leases: Vec<_> = leases
            .into_iter()
            .filter_map(|lease| {
                let headers = RefreshableHeaders::new(
                    HashMap::from([(
                        lease.header_name.clone(),
                        lease.credential.header_value.clone(),
                    )]),
                    Some(rejection_handler(
                        lease.installation_id,
                        sender.clone(),
                        running.clone(),
                    )),
                );
                match headers {
                    Ok(headers) => Some((lease, headers)),
                    Err(error) => {
                        log::warn!(
                            "The credential of managed MCP server '{}' will not be refreshed: {error}",
                            lease.server_name
                        );
                        None
                    }
                }
            })
            .collect();
        (!leases.is_empty()).then_some(Self {
            leases,
            requests,
            running,
        })
    }

    /// The headers the connection of each installation sends, by installation id.
    pub(super) fn headers(&self) -> HashMap<Uuid, RefreshableHeaders> {
        self.leases
            .iter()
            .map(|(lease, headers)| (lease.installation_id, headers.clone()))
            .collect()
    }

    /// Keeps the credentials valid for as long as the returned future is polled; it never
    /// resolves. A credential that already outlives `must_outlive` is only replaced when a
    /// server rejects it. Nothing is minted while `is_authenticated` reports that this process
    /// has no usable credentials of its own.
    pub(crate) async fn run<A, AFut>(
        self,
        client: Arc<dyn ManagedMcpClient>,
        clock: RefreshClock,
        must_outlive: Option<DateTime<Utc>>,
        is_authenticated: A,
    ) where
        A: Fn() -> AFut,
        AFut: Future<Output = bool>,
    {
        let Self {
            leases,
            mut requests,
            running,
        } = self;
        running.store(true, Ordering::Relaxed);
        let mut refresher = Refresher {
            client,
            clock,
            must_outlive,
            is_authenticated,
            reported_unauthenticated: false,
        };
        let mut states: Vec<_> = leases
            .into_iter()
            .map(|(lease, headers)| refresher.state_for(lease, headers))
            .collect();

        loop {
            let now = (refresher.clock.now)();
            for state in &mut states {
                if state.refresh_at().is_some_and(|at| at <= now) {
                    refresher.refresh(state).await;
                }
            }

            let now = (refresher.clock.now)();
            let next_refresh = states.iter().filter_map(LeaseState::refresh_at).min();
            let mut sleep = match next_refresh {
                Some(at) => {
                    let sleep_for = (at - now).to_std().unwrap_or(Duration::ZERO);
                    (refresher.clock.sleep)(sleep_for.min(MAX_SLEEP))
                }
                None => future::pending().boxed(),
            }
            .fuse();
            // Every lease's headers hold a sender, so the requests do not end while this runs.
            let RefreshRequest {
                installation_id,
                done,
            } = futures::select! {
                _ = sleep => continue,
                request = requests.select_next_some() => request,
            };
            if let Some(state) = states
                .iter_mut()
                .find(|state| state.lease.installation_id == installation_id)
                && (refresher.clock.now)() >= state.not_before
            {
                refresher.refresh(state).await;
            }
            let _ = done.send(());
        }
    }
}

/// Keeps the credentials of the managed MCP servers `foreground`'s driver started valid for as
/// long as the returned future is polled; it never resolves.
pub(super) async fn keep_run_credentials_valid(foreground: &ModelSpawner<AgentDriver>) {
    let refresh = foreground
        .spawn(|me, ctx| {
            let refresh = me.managed_mcp_refresh.take()?;
            let client = ServerApiProvider::as_ref(ctx).get_managed_mcp_client();
            let must_outlive = me.sandbox_deadline.map(DateTime::<Utc>::from);
            Some((refresh, client, must_outlive))
        })
        .await;
    let Ok(Some((refresh, client, must_outlive))) = refresh else {
        return future::pending().await;
    };
    let is_authenticated = || async {
        foreground
            .spawn(|_, ctx| {
                let auth_state = AuthStateProvider::as_ref(ctx).get();
                auth_state.is_logged_in() && !auth_state.needs_reauth()
            })
            .await
            .unwrap_or(false)
    };
    refresh
        .run(
            client,
            RefreshClock::default(),
            must_outlive,
            is_authenticated,
        )
        .await
}

/// Asks the refresh loop for a new credential when the proxy rejects the current one, and waits
/// for the answer so the rejected request can be sent again. Asks for nothing until the loop is
/// `running`.
fn rejection_handler(
    installation_id: Uuid,
    requests: mpsc::UnboundedSender<RefreshRequest>,
    running: Arc<AtomicBool>,
) -> RejectionHandler {
    Arc::new(move |challenge| {
        let requests = requests.clone();
        let running = running.load(Ordering::Relaxed);
        async move {
            if !running || !is_refreshable_rejection(&challenge) {
                return;
            }
            let (done, refreshed) = oneshot::channel();
            let request = RefreshRequest {
                installation_id,
                done,
            };
            if requests.unbounded_send(request).is_ok() {
                let _ = refreshed.with_timeout(REJECTION_REFRESH_TIMEOUT).await;
            }
        }
        .boxed()
    })
}

struct LeaseState {
    lease: ManagedMcpLease,
    headers: RefreshableHeaders,
    /// Whether a replacement is worth requesting ahead of the expiry. It is not once the
    /// credential outlives the run, or once the server has answered with one that was due for
    /// replacement as soon as it arrived.
    extendable: bool,
    lead_time: Duration,
    /// Earliest time of the next mint attempt.
    not_before: DateTime<Utc>,
    consecutive_failures: u32,
}

impl LeaseState {
    fn refresh_at(&self) -> Option<DateTime<Utc>> {
        self.extendable.then(|| {
            next_refresh_at(
                self.lease.credential.expires_at,
                self.lead_time,
                self.not_before,
            )
        })
    }
}

struct Refresher<A> {
    client: Arc<dyn ManagedMcpClient>,
    clock: RefreshClock,
    must_outlive: Option<DateTime<Utc>>,
    is_authenticated: A,
    /// Limits the report of missing or rejected credentials to one per outage.
    reported_unauthenticated: bool,
}

impl<A, AFut> Refresher<A>
where
    A: Fn() -> AFut,
    AFut: Future<Output = bool>,
{
    fn state_for(&self, lease: ManagedMcpLease, headers: RefreshableHeaders) -> LeaseState {
        LeaseState {
            extendable: self.expires_during_run(&lease.credential),
            lead_time: REFRESH_LEAD_TIME + REFRESH_LEAD_JITTER.mul_f32((self.clock.jitter)()),
            not_before: DateTime::<Utc>::MIN_UTC,
            consecutive_failures: 0,
            lease,
            headers,
        }
    }

    fn expires_during_run(&self, credential: &ManagedMcpCredential) -> bool {
        self.must_outlive
            .is_none_or(|deadline| credential.expires_at < deadline)
    }

    async fn refresh(&mut self, state: &mut LeaseState) {
        let server_name = state.lease.server_name.clone();
        if !(self.is_authenticated)().await {
            self.report_unauthenticated(&server_name, None);
            state.not_before = (self.clock.now)() + MAX_RETRY_INTERVAL;
            return;
        }
        let minted = {
            let mint = mint_credential(self.client.as_ref(), &state.lease).fuse();
            let mut timeout = (self.clock.sleep)(MINT_TIMEOUT).fuse();
            futures::pin_mut!(mint);
            futures::select_biased! {
                minted = mint => minted,
                _ = timeout => Err(MintError::Transient(anyhow!(
                    "the server did not answer within {MINT_TIMEOUT:?}"
                ))),
            }
        };
        let now = (self.clock.now)();
        let credential = match minted {
            Ok(credential) => credential,
            Err(MintError::Transient(error)) => {
                state.consecutive_failures += 1;
                let retry_in = retry_interval(state.consecutive_failures, (self.clock.jitter)());
                log::warn!(
                    "Failed to refresh the credential of managed MCP server '{server_name}', \
                     retrying in {retry_in:?}: {error:#}"
                );
                state.not_before = now + retry_in;
                return;
            }
            Err(MintError::Unauthenticated(error)) => {
                self.report_unauthenticated(&server_name, Some(&error));
                state.not_before = now + MAX_RETRY_INTERVAL;
                return;
            }
            Err(MintError::Rejected(error)) => {
                log::warn!(
                    "The server did not refresh the credential of managed MCP server \
                     '{server_name}': {error:#}"
                );
                state.not_before = now + MAX_RETRY_INTERVAL;
                return;
            }
        };

        let replaced = state.headers.replace(HashMap::from([(
            state.lease.header_name.clone(),
            credential.header_value.clone(),
        )]));
        if let Err(error) = replaced {
            log::warn!(
                "Could not use the refreshed credential of managed MCP server '{server_name}': \
                 {error}"
            );
            state.not_before = now + MAX_RETRY_INTERVAL;
            return;
        }
        log::info!(
            "Refreshed the credential of managed MCP server '{server_name}'; it expires at {}",
            credential.expires_at
        );
        state.not_before = now + MIN_ATTEMPT_INTERVAL;
        state.extendable = credential.expires_at - state.lead_time > state.not_before
            && self.expires_during_run(&credential);
        state.lease.credential = credential;
        state.consecutive_failures = 0;
        self.reported_unauthenticated = false;
    }

    fn report_unauthenticated(&mut self, server_name: &str, error: Option<&anyhow::Error>) {
        if self.reported_unauthenticated {
            return;
        }
        self.reported_unauthenticated = true;
        let reason = error.map_or_else(
            || "this process is not logged in".to_owned(),
            |error| format!("{error:#}"),
        );
        log::warn!(
            "Cannot refresh the credential of managed MCP server '{server_name}' until this \
             process is authenticated again: {reason}"
        );
    }
}

#[cfg(test)]
#[path = "managed_mcp_refresh_tests.rs"]
mod tests;
