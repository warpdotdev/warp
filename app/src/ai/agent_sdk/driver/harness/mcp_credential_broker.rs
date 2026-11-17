//! Serves the current credential of a run's managed MCP servers to its third-party harness.
//!
//! A harness reads its MCP config once, so a credential written there expires while the harness
//! is still running. Where a harness can run a command for a server's headers, its config names
//! this executable's `mcp-credential` worker instead. The worker asks the broker, which the
//! driver runs for as long as the harness, over a [`local_socket`](super::local_socket); the
//! broker answers with the credential it holds, minting a new one first unless it minted one a
//! moment ago. Nothing is minted until a harness asks.
//!
//! Like the ACP bridge, this requires the driver and the harness to share one OS instance.
use std::collections::HashMap;
use std::future::Future;
use std::io::Write as _;
use std::net::Shutdown;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use futures::io::AsyncWriteExt as _;
use futures::lock::Mutex;
use futures::stream::FuturesUnordered;
use futures::{FutureExt as _, StreamExt as _, future};
use serde::{Deserialize, Serialize};
use shell_words::quote as shell_quote;
use warp_cli::mcp::McpCredentialFormat;
use warp_core::features::FeatureFlag;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{ModelSpawner, SingletonEntity};

use super::ResolvedMCPServers;
use super::local_socket::{
    Connection, Endpoint, Listener, VERDICT_ACCEPTED, VERDICT_REJECTED, connect_with_token,
    new_token, read_connection_line, read_line, read_token_line,
};
use crate::ai::agent_sdk::driver::AgentDriver;
use crate::ai::agent_sdk::driver::managed_mcp_refresh::{ManagedMcpLease, mint_credential};
use crate::auth::AuthStateProvider;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::managed_mcp::ManagedMcpClient;

const ENDPOINT_FILE_NAME: &str = "endpoint.json";
/// Least time between two mints for one server. A harness asks once per connection and again
/// when a request is rejected, so a helper that asks sooner gets the credential just minted.
const MINT_INTERVAL: Duration = Duration::from_secs(60);
/// Upper bound on one mint. Harnesses give a helper ten seconds, so the broker answers with the
/// credential it holds rather than let a slow mint use them up.
const MINT_TIMEOUT: Duration = Duration::from_secs(6);
/// Upper bound on answering one connection, so a client that stalls cannot hold it open.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the helper waits for its answer before it gives up with an error of its own.
const HELPER_TIMEOUT: Duration = Duration::from_secs(8);
/// How long the broker waits before accepting again after its listener failed.
const ACCEPT_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const MAX_REQUEST_LINE: usize = 4 * 1024;
const MAX_RESPONSE_LINE: usize = 64 * 1024;

/// What the driver hands the helper through the endpoint file.
#[derive(Serialize, Deserialize)]
struct BrokerEndpoint {
    endpoint: Endpoint,
    /// Presented first so the broker can tell the helper from any other local client that
    /// found the endpoint.
    token: String,
}

#[derive(Serialize, Deserialize)]
struct CredentialRequest {
    server: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CredentialResponse {
    Credential {
        header_name: String,
        header_value: String,
    },
    UnknownServer,
}

/// The command a harness runs to learn the current value of a header that its MCP server's
/// config would otherwise carry as a fixed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct McpCredentialHelper {
    pub(crate) header_name: String,
    command: String,
}

impl McpCredentialHelper {
    /// Prints a JSON object mapping the header's name to its current value.
    pub(crate) fn headers_command(&self) -> &str {
        &self.command
    }

    /// Prints the header's current value alone.
    pub(crate) fn value_command(&self) -> String {
        format!("{} --format value", self.command)
    }

    #[cfg(test)]
    pub(crate) fn for_test(header_name: &str, command: &str) -> Self {
        Self {
            header_name: header_name.to_owned(),
            command: command.to_owned(),
        }
    }
}

/// Driver-side listener for the helpers of one harness run.
pub(crate) struct McpCredentialBroker {
    /// Holds the private directory the endpoint file (and socket file, where used) lives in;
    /// removed with the broker.
    _dir: tempfile::TempDir,
    token: String,
    listener: Listener,
    /// The credential of each server, by the name the harness knows it under.
    leases: HashMap<String, ManagedMcpLease>,
    /// When the credentials in `leases` were minted.
    resolved_at: DateTime<Utc>,
}

impl McpCredentialBroker {
    /// Starts a broker for the expiring credentials among `resolved`, and records the helper
    /// command of each in `resolved`. `None` leaves every credential where it is: in the
    /// server's config, as a fixed value.
    pub(crate) fn bind_for(resolved: &mut ResolvedMCPServers) -> Option<Self> {
        let leases = std::mem::take(&mut resolved.leases);
        // Harnesses run the helper through a POSIX shell.
        if !cfg!(unix)
            || leases.is_empty()
            || !FeatureFlag::ManagedMcpHarnessCredentialHelper.is_enabled()
        {
            return None;
        }
        let bound = std::env::current_exe()
            .context("Failed to locate the running executable")
            .and_then(|executable| Self::bind(leases, &executable, Utc::now()));
        match bound {
            Ok((broker, helpers)) => {
                resolved.credential_helpers = helpers;
                Some(broker)
            }
            Err(error) => {
                log::warn!(
                    "Managed MCP credentials will not be refreshed for this harness: {error:#}"
                );
                None
            }
        }
    }

    /// Starts listening and writes the endpoint file, with a fresh token, for credentials that
    /// were minted at `resolved_at`. Returns the command that asks for each server's credential
    /// through `executable`.
    fn bind(
        leases: HashMap<String, ManagedMcpLease>,
        executable: &Path,
        resolved_at: DateTime<Utc>,
    ) -> Result<(Self, HashMap<String, McpCredentialHelper>)> {
        // The token is only as private as this directory, and so is the socket. `tempfile`
        // creates it mode 0700.
        let dir = tempfile::Builder::new()
            .prefix("oz-mcp-")
            .tempdir()
            .context("Failed to create a directory for the MCP credential endpoint")?;
        let (listener, endpoint) = Listener::bind_in(dir.path())?;
        let token = new_token();
        let endpoint_file = dir.path().join(ENDPOINT_FILE_NAME);
        let contents = serde_json::to_vec(&BrokerEndpoint {
            endpoint,
            token: token.clone(),
        })?;
        std::fs::write(&endpoint_file, contents).with_context(|| {
            format!(
                "Failed to write MCP credential endpoint file {}",
                endpoint_file.display()
            )
        })?;
        let helpers = leases
            .iter()
            .map(|(server, lease)| {
                let helper = McpCredentialHelper {
                    header_name: lease.header_name.clone(),
                    command: render_helper_command(executable, &endpoint_file, server),
                };
                (server.clone(), helper)
            })
            .collect();
        let broker = Self {
            _dir: dir,
            token,
            listener,
            leases,
            resolved_at,
        };
        Ok((broker, helpers))
    }

    /// Answers helpers for as long as the returned future is polled; it never resolves. Nothing
    /// is minted while `is_authenticated` reports that this process has no usable credentials
    /// of its own.
    pub(crate) async fn serve<A, AFut>(
        self,
        client: Arc<dyn ManagedMcpClient>,
        now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
        is_authenticated: A,
    ) where
        A: Fn() -> AFut,
        AFut: Future<Output = bool>,
    {
        let Self {
            _dir,
            token,
            listener,
            leases,
            resolved_at,
        } = self;
        let mint_not_before = resolved_at + MINT_INTERVAL;
        let answers = Answers {
            token,
            entries: leases
                .into_iter()
                .map(|(server, lease)| {
                    let entry = Entry {
                        lease,
                        mint_not_before,
                    };
                    (server, Mutex::new(entry))
                })
                .collect(),
            client,
            now,
            is_authenticated,
            reported_unauthenticated: AtomicBool::new(false),
        };

        let mut connections = FuturesUnordered::new();
        loop {
            let accepted = {
                let accept = listener.accept().fuse();
                futures::pin_mut!(accept);
                futures::select! {
                    accepted = accept => accepted,
                    _ = connections.select_next_some() => continue,
                }
            };
            match accepted {
                Ok(connection) => connections.push(answers.answer(connection)),
                Err(error) => {
                    log::warn!("Failed to accept an MCP credential helper connection: {error}");
                    Timer::after(ACCEPT_RETRY_INTERVAL).await;
                }
            }
        }
    }
}

/// Serves the credentials of the managed MCP servers `foreground`'s driver resolved for its
/// harness for as long as the returned future is polled; it never resolves.
pub(crate) async fn serve_run_credentials(foreground: &ModelSpawner<AgentDriver>) {
    let broker = foreground
        .spawn(|me, ctx| {
            let broker = me.mcp_credential_broker.take()?;
            let client = ServerApiProvider::as_ref(ctx).get_managed_mcp_client();
            Some((broker, client))
        })
        .await;
    let Ok(Some((broker, client))) = broker else {
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
    broker
        .serve(client, Arc::new(Utc::now), is_authenticated)
        .await
}

struct Entry {
    lease: ManagedMcpLease,
    /// Earliest time of the next mint.
    mint_not_before: DateTime<Utc>,
}

struct Answers<A> {
    token: String,
    /// Each server's credential, behind a lock that makes concurrent helpers share one mint.
    entries: HashMap<String, Mutex<Entry>>,
    client: Arc<dyn ManagedMcpClient>,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    is_authenticated: A,
    /// Limits the report of missing credentials to one per outage.
    reported_unauthenticated: AtomicBool,
}

impl<A, AFut> Answers<A>
where
    A: Fn() -> AFut,
    AFut: Future<Output = bool>,
{
    async fn answer(&self, connection: Connection) {
        match self
            .answer_request(&connection)
            .with_timeout(CONNECTION_TIMEOUT)
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Rejected an MCP credential helper: {error:#}"),
            Err(_) => log::warn!("An MCP credential helper did not finish its request in time"),
        }
        let _ = connection.shutdown(Shutdown::Both);
    }

    async fn answer_request(&self, connection: &Connection) -> Result<()> {
        let mut writer = connection;
        let presented = read_token_line(connection).await?;
        if presented != self.token {
            // Best effort: the impostor may already be gone, and either way it is dropped next.
            let _ = writer
                .write_all(format!("{VERDICT_REJECTED}\n").as_bytes())
                .await;
            bail!("wrong token");
        }
        writer
            .write_all(format!("{VERDICT_ACCEPTED}\n").as_bytes())
            .await?;
        let request: CredentialRequest =
            serde_json::from_str(&read_connection_line(connection, MAX_REQUEST_LINE).await?)
                .context("malformed request")?;
        let response = self.credential_for(&request.server).await;
        writer
            .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
            .await?;
        Ok(())
    }

    async fn credential_for(&self, server: &str) -> CredentialResponse {
        let Some(entry) = self.entries.get(server) else {
            return CredentialResponse::UnknownServer;
        };
        let mut entry = entry.lock().await;
        let now = (self.now)();
        if now >= entry.mint_not_before {
            entry.mint_not_before = now + MINT_INTERVAL;
            self.mint(server, &mut entry).await;
        }
        CredentialResponse::Credential {
            header_name: entry.lease.header_name.clone(),
            header_value: entry.lease.credential.header_value.clone(),
        }
    }

    /// Replaces the credential `entry` holds with a new one, keeping the current one when that
    /// is not possible.
    async fn mint(&self, server: &str, entry: &mut Entry) {
        if !(self.is_authenticated)().await {
            if !self.reported_unauthenticated.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Cannot refresh the credential of managed MCP server '{server}' until this \
                     process is authenticated again"
                );
            }
            return;
        }
        match mint_credential(self.client.as_ref(), &entry.lease)
            .with_timeout(MINT_TIMEOUT)
            .await
        {
            Ok(Ok(credential)) => {
                log::info!(
                    "Refreshed the credential of managed MCP server '{server}'; it expires at {}",
                    credential.expires_at
                );
                entry.lease.credential = credential;
                self.reported_unauthenticated
                    .store(false, Ordering::Relaxed);
            }
            Ok(Err(error)) => log::warn!(
                "Failed to refresh the credential of managed MCP server '{server}': {error}"
            ),
            Err(_) => {
                log::warn!("Timed out refreshing the credential of managed MCP server '{server}'")
            }
        }
    }
}

fn render_helper_command(executable: &Path, endpoint_file: &Path, server: &str) -> String {
    format!(
        "{} mcp-credential --endpoint-file {} --server {}",
        shell_quote(&executable.display().to_string()),
        shell_quote(&endpoint_file.display().to_string()),
        shell_quote(server),
    )
}

/// Worker-side entry point: prints the credential of `server` in `format`.
pub(crate) fn run_helper(
    endpoint_file: &Path,
    server: &str,
    format: McpCredentialFormat,
) -> Result<()> {
    // A harness kills a helper that takes too long without saying why, so give up first.
    std::thread::spawn(|| {
        std::thread::sleep(HELPER_TIMEOUT);
        eprintln!("oz: the agent driver did not answer within {HELPER_TIMEOUT:?}");
        std::process::exit(1);
    });
    let response = request_credential(endpoint_file, server)?;
    println!("{}", render_credential(response, server, format)?);
    Ok(())
}

fn request_credential(endpoint_file: &Path, server: &str) -> Result<CredentialResponse> {
    let contents = std::fs::read_to_string(endpoint_file).with_context(|| {
        format!(
            "Failed to read MCP credential endpoint file {}",
            endpoint_file.display()
        )
    })?;
    let BrokerEndpoint { endpoint, token } =
        serde_json::from_str(&contents).with_context(|| {
            format!(
                "Malformed MCP credential endpoint file {}",
                endpoint_file.display()
            )
        })?;
    let mut stream = connect_with_token(&endpoint, &token)?;
    let request = CredentialRequest {
        server: server.to_owned(),
    };
    stream
        .write_all(format!("{}\n", serde_json::to_string(&request)?).as_bytes())
        .context("Failed to send the request to the agent driver")?;
    let response = read_line(&mut stream, MAX_RESPONSE_LINE)
        .context("The agent driver closed the connection before answering")?;
    serde_json::from_str(&response).context("Malformed answer from the agent driver")
}

fn render_credential(
    response: CredentialResponse,
    server: &str,
    format: McpCredentialFormat,
) -> Result<String> {
    let CredentialResponse::Credential {
        header_name,
        header_value,
    } = response
    else {
        return Err(anyhow!(
            "The agent driver holds no credential for MCP server '{server}'"
        ));
    };
    match format {
        McpCredentialFormat::Headers => Ok(serde_json::to_string(&HashMap::from([(
            header_name,
            header_value,
        )]))?),
        McpCredentialFormat::Value => Ok(header_value),
    }
}

#[cfg(test)]
#[path = "mcp_credential_broker_tests.rs"]
mod tests;
