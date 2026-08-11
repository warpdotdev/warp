//! Running app-side server for local Warp control requests.
//!
//! This module owns the in-process listener, discovery registration, credential
//! broker socket, and request handoff from Axum into the WarpUI model graph.
//! It complements `crates/local_control/src/discovery.rs`: that shared module
//! defines how clients find and validate candidate instances, while this module
//! creates the app-owned endpoints and publishes their routing metadata through
//! `RegisteredInstance`.
//!
//! A client uses all three transports in order. It reads the filesystem record
//! to find an instance, connects to that instance's Unix socket to obtain
//! temporary authority, and presents that authority to the instance's loopback
//! HTTP endpoint with one typed action. The filesystem and socket are therefore
//! complementary parts of discovery and credential bootstrap, not competing
//! discovery mechanisms.
//!
//! Credential broker security flow:
//!
//! ```text
//! owner-only discovery record
//! (loopback endpoint + broker path; never a token)
//!                 |
//!                 v
//! CLI client -- instance-bound Unix socket --> credential broker
//!                 [0600 socket + kernel-reported peer UID]
//!                                             |
//!                                             v
//!                           feature flag + Settings > Scripting gate
//!                           + protocol + exact action metadata
//!                                             |
//!                                             v
//!                           short-lived, instance-bound, action-scoped
//!                           bearer grant stored only in process memory
//!                                             |
//!                                             v
//! CLI client -- loopback HTTP + bearer --> /v1/control
//!                 [reject browser Origin + require exact Host
//!                  + validate grant existence, expiry, instance, and scope]
//!                                             |
//!                                             v
//!                           typed allowlisted action
//!                                             |
//!                                             v
//!                           main-thread LocalControlBridge
//!                           [re-check current settings before dispatch]
//! ```
//!
//! These boundaries prevent browser-origin clients, other OS users,
//! unauthenticated clients that only obtain or guess the HTTP endpoint, stale
//! or wrong-instance credentials, and accidentally over-scoped credentials from
//! invoking actions. The broker authenticates the OS account, not the calling
//! application: malicious software already running as the same user remains
//! outside this boundary.
//!
//! The Settings > Scripting gates used here are local-only settings backed by
//! Warp's secure storage provider.
//!
//! Discovery records never include raw bearer tokens: discovery only exposes
//! endpoint metadata and credential broker references while Scripting is enabled.
mod bridge;
mod handlers;
mod permissions;
mod resolver;

use std::collections::HashMap;
#[cfg(unix)]
use std::fs::Permissions;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::sync::{Arc, Mutex};

use ::local_control::auth::CredentialGrant;
#[cfg(any(unix, test))]
use ::local_control::auth::{CredentialRequest, ScopedCredential};
use ::local_control::{
    ActionKind, AuthToken, ControlEndpoint, ControlError, ControlResponse, ErrorCode,
    ErrorResponseEnvelope, InstanceId, InstanceRecord, RegisteredInstance, RequestEnvelope,
    ResponseEnvelope,
};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
pub use bridge::LocalControlBridge;
#[cfg(any(unix, test))]
use chrono::Duration;
use permissions::ensure_feature_enabled;
#[cfg(any(unix, test))]
use permissions::{ensure_action_allowed, ensure_protocol_version};
#[cfg(unix)]
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use warp_core::channel::ChannelState;
use warpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

#[cfg(any(unix, test))]
const MAX_ACTIVE_CREDENTIALS: usize = 128;

/// App-owned authority shared by one instance's broker and HTTP listener.
///
/// Broker-issued bearer tokens map to grants only in this process-local state.
/// Knowing the endpoint from discovery is therefore insufficient to authenticate
/// an HTTP request.
#[derive(Clone)]
struct ControlServerState {
    bridge_spawner: ModelSpawner<LocalControlBridge>,
    instance_id: InstanceId,
    expected_host: String,
    credentials: Arc<Mutex<HashMap<String, CredentialGrant>>>,
}
/// Process-local publisher, credential broker, and HTTP server for one Warp instance.
///
/// Holding the runtime and registration keeps both listeners and the discovery
/// route alive. Dropping them stops request handling and removes the app's
/// published record and broker socket.
pub struct LocalControlServer {
    _runtime: Option<tokio::runtime::Runtime>,
    control_endpoint: Option<ControlEndpoint>,
    registered_instance: Option<RegisteredInstance>,
}

impl Entity for LocalControlServer {
    type Event = ();
}

impl SingletonEntity for LocalControlServer {}

impl LocalControlServer {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut server = Self {
            _runtime: None,
            control_endpoint: None,
            registered_instance: None,
        };
        if let Err(error) = server.refresh_for_settings(ctx) {
            log::warn!("Failed to refresh local-control server state: {error:#}");
        }
        ctx.subscribe_to_model(
            &crate::settings::LocalControlSettings::handle(ctx),
            |server, _, _, ctx| {
                if let Err(error) = server.refresh_for_settings(ctx) {
                    log::warn!("Failed to refresh local-control server state: {error:#}");
                }
            },
        );
        server
    }

    /// Starts, refreshes, or removes local-control publication as settings change.
    fn refresh_for_settings(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), ControlError> {
        if !permissions::warp_control_cli_enabled() {
            self.stop(ctx);
            return Ok(());
        }
        if !local_control_publication_supported() {
            self.stop(ctx);
            return Ok(());
        }
        if !crate::settings::LocalControlSettings::as_ref(ctx).is_enabled() {
            self.stop(ctx);
            return Ok(());
        }
        if self._runtime.is_some() {
            return self.refresh_discovery_record(ctx);
        }
        self.start(ctx)
    }

    /// Stops both listeners and removes the discovery record and broker socket.
    fn stop(&mut self, _ctx: &mut ModelContext<Self>) {
        self.registered_instance = None;
        self.control_endpoint = None;
        self._runtime = None;
    }

    /// Binds both transports and publishes the routing record that connects them.
    ///
    /// Startup first binds an ephemeral loopback HTTP port, publishes that port
    /// plus the instance-derived broker filename, binds the broker socket, and
    /// then serves credential issuance and typed control requests concurrently.
    fn start(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), ControlError> {
        if self._runtime.is_some() {
            return Err(ControlError::new(
                ErrorCode::Internal,
                "local-control server is already running",
            ));
        }
        ensure_feature_enabled()?;
        if !local_control_publication_supported() {
            return Err(ControlError::new(
                ErrorCode::LocalControlDisabled,
                "local control is disabled until this platform enforces discovery-record ACLs",
            ));
        }
        if !crate::settings::LocalControlSettings::as_ref(ctx).is_enabled() {
            return Ok(());
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .build()
            .map_err(|err| {
                ControlError::with_details(
                    ErrorCode::Internal,
                    "failed to create local-control runtime",
                    err.to_string(),
                )
            })?;
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(SocketAddr::from((
                [127, 0, 0, 1],
                0,
            ))))
            .map_err(|err| {
                ControlError::with_details(
                    ErrorCode::Internal,
                    "failed to bind local-control listener",
                    err.to_string(),
                )
            })?;
        let port = listener.local_addr().map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to read local-control listener address",
                err.to_string(),
            )
        })?;
        let control_endpoint = ControlEndpoint::localhost(port.port());
        let record = discovery_record_for_settings(ctx, control_endpoint.clone());
        let instance_id = record.instance_id.clone();
        let bridge_spawner = LocalControlBridge::handle(ctx).update(ctx, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            ctx.spawner()
        });
        let registered_instance = RegisteredInstance::register(record)?;
        #[cfg(unix)]
        let broker_listener = {
            let runtime_guard = runtime.enter();
            let listener = bind_credential_broker(registered_instance.record())?;
            drop(runtime_guard);
            listener
        };
        let state = ControlServerState {
            bridge_spawner,
            instance_id,
            expected_host: format!("{}:{}", control_endpoint.host, control_endpoint.port),
            credentials: Arc::default(),
        };
        let router = Router::new()
            .route("/v1/control", post(handle_control_request))
            .with_state(state.clone());
        runtime.spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                log::warn!("local-control listener stopped: {err:#}");
            }
        });
        #[cfg(unix)]
        runtime.spawn(run_credential_broker(broker_listener, state));
        let endpoint_url = control_endpoint.url();
        self._runtime = Some(runtime);
        self.control_endpoint = Some(control_endpoint);
        self.registered_instance = Some(registered_instance);
        log::info!("local-control server started at {endpoint_url}");
        Ok(())
    }

    fn refresh_discovery_record(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), ControlError> {
        let Some(control_endpoint) = self.control_endpoint.clone() else {
            return Ok(());
        };
        let Some(registered_instance) = &mut self.registered_instance else {
            return Ok(());
        };
        let mut record = discovery_record_for_settings(ctx, control_endpoint);
        record.instance_id = registered_instance.record().instance_id.clone();
        record.credential_broker = registered_instance.record().credential_broker.clone();
        registered_instance.update(record)
    }
}

/// Builds routing metadata without embedding any bearer credential or secret.
///
/// The endpoint and derived broker reference are published only while the
/// protected Scripting setting permits clients to use them.
fn discovery_record_for_settings(
    ctx: &ModelContext<LocalControlServer>,
    control_endpoint: ControlEndpoint,
) -> InstanceRecord {
    let endpoint = crate::settings::LocalControlSettings::as_ref(ctx)
        .is_enabled()
        .then_some(control_endpoint);
    InstanceRecord::for_current_process(
        endpoint,
        ChannelState::channel().to_string(),
        ChannelState::app_id().to_string(),
        ChannelState::app_version().map(str::to_owned),
        ActionKind::implemented_metadata(),
    )
}

/// Binds the instance's credential-bootstrap socket and restricts it to the owning user.
///
/// Any stale socket at the instance-specific path is removed before binding, and
/// the new socket is set to owner-only permissions before it accepts clients.
/// The path came from a validated instance-derived discovery reference, so a
/// record cannot redirect credential requests to an arbitrary socket.
#[cfg(unix)]
fn bind_credential_broker(
    record: &InstanceRecord,
) -> Result<tokio::net::UnixListener, ControlError> {
    let socket_path = record.broker_socket_path()?;
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to remove stale local-control credential broker socket",
                err.to_string(),
            )
        })?;
    }
    let listener = tokio::net::UnixListener::bind(&socket_path).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to bind owner-authenticated local-control credential broker",
            err.to_string(),
        )
    })?;
    std::fs::set_permissions(&socket_path, Permissions::from_mode(0o600)).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to protect local-control credential broker socket",
            err.to_string(),
        )
    })?;
    Ok(listener)
}

#[cfg(unix)]
/// Accepts same-user credential requests independently from the HTTP listener.
async fn run_credential_broker(listener: tokio::net::UnixListener, state: ControlServerState) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_credential_broker_connection(stream, state).await {
                log::warn!("local-control credential broker connection failed: {err:#}");
            }
        });
    }
}

#[cfg(unix)]
/// Authenticates the socket peer before decoding and evaluating its request.
///
/// This ordering makes the kernel-reported OS user, rather than any field in
/// caller-controlled JSON, the credential broker's client-identity boundary.
async fn handle_credential_broker_connection(
    mut stream: tokio::net::UnixStream,
    state: ControlServerState,
) -> Result<(), ControlError> {
    let response = match ensure_same_user_peer(&stream) {
        Ok(()) => {
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).await.map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidRequest,
                    "failed to read local-control credential request",
                    err.to_string(),
                )
            })?;
            match serde_json::from_slice::<CredentialRequest>(&bytes) {
                Ok(request) => issue_credential(&state, request)
                    .await
                    .and_then(|credential| serialize_credential_broker_response(&credential)),
                Err(err) => Err(ControlError::with_details(
                    ErrorCode::InvalidRequest,
                    "failed to decode local-control credential request",
                    err.to_string(),
                )),
            }
        }
        Err(error) => Err(error),
    };
    let bytes = match response {
        Ok(bytes) => bytes,
        Err(error) => serialize_credential_broker_response(&ErrorResponseEnvelope::new(error))?,
    };
    stream.write_all(&bytes).await.map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to write local-control credential response",
            err.to_string(),
        )
    })
}

#[cfg(unix)]
/// Requires the kernel-reported peer UID to match Warp's effective UID.
///
/// This excludes other OS users but does not distinguish trusted Warp code from
/// arbitrary processes already running as the same user.
fn ensure_same_user_peer(stream: &tokio::net::UnixStream) -> Result<(), ControlError> {
    ensure_peer_uid(stream, unsafe { libc::geteuid() })
}

#[cfg(unix)]
/// Verifies a socket peer against an expected UID obtained outside request data.
fn ensure_peer_uid(stream: &tokio::net::UnixStream, expected_uid: u32) -> Result<(), ControlError> {
    let peer = stream.peer_cred().map_err(|err| {
        ControlError::with_details(
            ErrorCode::UnauthorizedLocalClient,
            "failed to identify local-control credential broker peer",
            err.to_string(),
        )
    })?;
    if peer.uid() != expected_uid {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "local-control credential broker peer belongs to a different OS user",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn serialize_credential_broker_response(
    response: &impl serde::Serialize,
) -> Result<Vec<u8>, ControlError> {
    serde_json::to_vec(response).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control credential response",
            err.to_string(),
        )
    })
}

/// Evaluates current action policy and mints one short-lived exact-action grant.
///
/// The bearer secret and its grant are retained only in the running instance's
/// process-local map; neither is written back into the discovery registry.
#[cfg(any(unix, test))]
async fn issue_credential(
    state: &ControlServerState,
    request: CredentialRequest,
) -> Result<ScopedCredential, ControlError> {
    ensure_feature_enabled()?;
    ensure_protocol_version(request.protocol_version)?;
    if !request.action.is_implemented() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} is not implemented by this local-control bridge",
                request.action.as_str()
            ),
        ));
    }
    state
        .bridge_spawner
        .spawn({
            let action = request.action;
            move |_, ctx| ensure_action_allowed(action, ctx)
        })
        .await
        .map_err(|_| {
            ControlError::new(
                ErrorCode::BridgeUnavailable,
                "local-control app bridge is unavailable",
            )
        })??;
    let auth_token = AuthToken::generate();
    let grant = CredentialGrant::new(
        state.instance_id.clone(),
        request.action,
        Duration::minutes(5),
    );
    let mut credentials = state.credentials.lock().map_err(|_| {
        ControlError::new(
            ErrorCode::Internal,
            "local-control credential broker is unavailable",
        )
    })?;
    insert_credential(
        &mut credentials,
        auth_token.secret().to_owned(),
        grant.clone(),
    );
    Ok(ScopedCredential {
        bearer_token: auth_token.secret().to_owned(),
        grant,
    })
}

/// Authenticates and hands one typed HTTP request to the app bridge.
///
/// Header hardening rejects browser-origin and wrong-endpoint requests. The
/// process-local credential lookup authenticates the transport, after which the
/// bridge revalidates current settings and exact-action authority before
/// resolving targets or dispatching a handler.
async fn handle_control_request(
    State(state): State<ControlServerState>,
    headers: HeaderMap,
    payload: Bytes,
) -> Response {
    if let Err(error) = validate_loopback_headers(&headers, &state.expected_host) {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    if let Err(error) = ensure_feature_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let auth_header = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let auth_token = match AuthToken::from_authorization_header(auth_header) {
        Ok(token) => token,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponseEnvelope::new(error)),
            )
                .into_response();
        }
    };
    let grant = match state.credentials.lock() {
        Ok(mut credentials) => lookup_credential(&mut credentials, &auth_token, &state.instance_id),
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponseEnvelope::new(ControlError::new(
                    ErrorCode::Internal,
                    "local-control credential broker is unavailable",
                ))),
            )
                .into_response();
        }
    };
    let grant = match grant {
        Ok(grant) => grant,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponseEnvelope::new(error)),
            )
                .into_response();
        }
    };
    let request = match serde_json::from_slice::<RequestEnvelope>(&payload) {
        Ok(request) => request,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponseEnvelope::new(ControlError::with_details(
                    ErrorCode::InvalidRequest,
                    "failed to decode local-control request",
                    err.to_string(),
                ))),
            )
                .into_response();
        }
    };
    let request_id = request.request_id;
    let response = match state
        .bridge_spawner
        .spawn(move |bridge, ctx| bridge.handle_request(request, grant, ctx))
        .await
    {
        Ok(response) => response,
        Err(_) => ResponseEnvelope::error(
            request_id,
            ControlError::new(
                ErrorCode::BridgeUnavailable,
                "local-control app bridge is unavailable",
            ),
        ),
    };
    let status = match &response.response {
        ControlResponse::Ok { .. } => StatusCode::OK,
        ControlResponse::Error { .. } => StatusCode::BAD_REQUEST,
    };
    (status, Json(response)).into_response()
}

#[cfg(any(unix, test))]
fn insert_credential(
    credentials: &mut HashMap<String, CredentialGrant>,
    secret: String,
    grant: CredentialGrant,
) {
    credentials.retain(|_, grant| !grant.is_expired());
    if credentials.len() >= MAX_ACTIVE_CREDENTIALS {
        let oldest_secret = credentials
            .iter()
            .min_by_key(|(_, grant)| grant.issued_at)
            .map(|(secret, _)| secret.clone());
        if let Some(oldest_secret) = oldest_secret {
            credentials.remove(&oldest_secret);
        }
    }
    credentials.insert(secret, grant);
}

/// Resolves an unexpired bearer token issued by this exact running instance.
fn lookup_credential(
    credentials: &mut HashMap<String, CredentialGrant>,
    auth_token: &AuthToken,
    instance_id: &InstanceId,
) -> Result<CredentialGrant, ControlError> {
    if credentials
        .get(auth_token.secret())
        .is_some_and(CredentialGrant::is_expired)
    {
        credentials.remove(auth_token.secret());
    }
    let grant = credentials
        .get(auth_token.secret())
        .cloned()
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential is invalid",
            )
        })?;
    grant.verify_for_action(instance_id, grant.action)?;
    Ok(grant)
}
fn local_control_publication_supported() -> bool {
    cfg!(not(target_os = "windows"))
}

/// Performs browser-origin hardening for local-control endpoints.
///
/// These checks intentionally reject browser-style `Origin` requests and stale
/// endpoint selections, but they are not an authorization boundary. Scoped
/// bearer credentials and grant validation remain the authority for control
/// requests.
pub(crate) fn validate_loopback_headers(
    headers: &HeaderMap,
    expected_host: &str,
) -> Result<(), ControlError> {
    if headers.contains_key(ORIGIN) {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "browser-origin local-control requests are not allowed",
        ));
    }
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Host header is required for local-control requests",
            )
        })?;
    if host != expected_host {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "Host header does not match the selected local-control endpoint",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use bridge::validate_request_authority;
#[cfg(test)]
pub(crate) use permissions::{capabilities, ensure_settings_allow_action};
#[cfg(test)]
pub(crate) use resolver::{
    require_active_window_id, resolve_index_from_ids, resolve_title_from_matches,
    validate_action_params, validate_tab_create_target,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
