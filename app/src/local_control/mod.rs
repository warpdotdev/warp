//! Local HTTP server entry point for Warp control requests.
//!
//! This module owns the in-process listener, discovery registration, credential
//! broker endpoint, and request handoff from Axum into the WarpUI model graph.
//!
//! Authentication is split into two localhost endpoints. Clients first request a
//! short-lived scoped credential from `/v1/control/credentials`; the localhost
//! server running inside Warp checks the feature flag, requested invocation
//! context, action metadata, execution-context proof, and Settings > Scripting
//! permissions before minting a bearer token. Outside-Warp clients use the
//! localhost credential broker directly; verified inside-Warp terminal
//! credentials remain future work until the app-issued proof broker is
//! implemented. The client then presents that bearer token to `/v1/control`,
//! where the server looks up the in-memory grant, verifies it still matches the
//! requested action, and only then hands the request to the main-thread
//! `LocalControlBridge`.
//!
//! The Settings > Scripting gates used here are local-only settings backed by
//! Warp's secure storage provider.
//!
//! Discovery records never include raw bearer tokens: discovery only exposes
//! endpoint metadata and credential broker references when outside-Warp control
//! is enabled.
mod bridge;
mod handlers;
mod permissions;
mod resolver;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use ::local_control::auth::{CredentialGrant, CredentialRequest, ScopedCredential};
use ::local_control::{
    ActionKind, AuthToken, ControlEndpoint, ControlError, ControlResponse, ErrorCode,
    ErrorResponseEnvelope, InstanceId, InstanceRecord, RegisteredInstance, RequestEnvelope,
    ResponseEnvelope,
};
use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Duration;
use warp_core::channel::ChannelState;
use warpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

pub use bridge::LocalControlBridge;
use permissions::{ensure_action_allowed, ensure_feature_enabled, ensure_protocol_version};
const MAX_ACTIVE_CREDENTIALS: usize = 128;

/// Shared state made available to Axum handlers for one localhost server
/// running inside Warp.
#[derive(Clone)]
struct ControlServerState {
    bridge_spawner: ModelSpawner<LocalControlBridge>,
    instance_id: InstanceId,
    expected_host: String,
    credentials: Arc<Mutex<HashMap<String, CredentialGrant>>>,
}
/// Process-local localhost server running inside Warp for control actions.
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
            |server, _, ctx| {
                if let Err(error) = server.refresh_for_settings(ctx) {
                    log::warn!("Failed to refresh local-control server state: {error:#}");
                }
            },
        );
        server
    }

    fn refresh_for_settings(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), ControlError> {
        if !permissions::warp_control_cli_enabled() {
            self.stop();
            return Ok(());
        }
        if !outside_warp_publication_supported() {
            self.stop();
            return Ok(());
        }
        let outside_warp_control_enabled =
            crate::settings::LocalControlSettings::as_ref(ctx).outside_warp_control_enabled();
        if !outside_warp_control_enabled {
            self.stop();
            return Ok(());
        }
        if self._runtime.is_some() {
            return self.refresh_discovery_record(ctx);
        }
        *self = Self::start(ctx)?;
        Ok(())
    }

    fn stop(&mut self) {
        self.registered_instance = None;
        self.control_endpoint = None;
        self._runtime = None;
    }

    fn start(ctx: &mut ModelContext<Self>) -> Result<Self, ControlError> {
        ensure_feature_enabled()?;
        if !outside_warp_publication_supported() {
            return Err(ControlError::new(
                ErrorCode::LocalControlDisabled,
                "outside-Warp local control is disabled until this platform enforces discovery-record ACLs",
            ));
        }
        if !crate::settings::LocalControlSettings::as_ref(ctx).outside_warp_control_enabled() {
            return Ok(Self {
                _runtime: None,
                control_endpoint: None,
                registered_instance: None,
            });
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
        let state = ControlServerState {
            bridge_spawner,
            instance_id,
            expected_host: format!("{}:{}", control_endpoint.host, control_endpoint.port),
            credentials: Arc::default(),
        };
        let router = Router::new()
            .route("/v1/control", post(handle_control_request))
            .route("/v1/control/credentials", post(handle_credential_request))
            .with_state(state);
        runtime.spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                log::warn!("local-control listener stopped: {err:#}");
            }
        });
        Ok(Self {
            _runtime: Some(runtime),
            control_endpoint: Some(control_endpoint),
            registered_instance: Some(registered_instance),
        })
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
        registered_instance.update(record)
    }
}

fn discovery_record_for_settings(
    ctx: &ModelContext<LocalControlServer>,
    control_endpoint: ControlEndpoint,
) -> InstanceRecord {
    let outside_warp_control_enabled =
        crate::settings::LocalControlSettings::as_ref(ctx).outside_warp_control_enabled();
    let endpoint = outside_warp_control_enabled.then_some(control_endpoint);
    InstanceRecord::for_current_process(
        endpoint,
        ChannelState::channel().to_string(),
        ChannelState::app_id().to_string(),
        ChannelState::app_version().map(str::to_owned),
        ActionKind::implemented_metadata(),
    )
}

async fn handle_credential_request(
    State(state): State<ControlServerState>,
    headers: HeaderMap,
    payload: Result<Json<CredentialRequest>, JsonRejection>,
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
    let request = match payload {
        Ok(Json(request)) => request,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponseEnvelope::new(ControlError::with_details(
                    ErrorCode::InvalidRequest,
                    "failed to decode local-control credential request",
                    err.to_string(),
                ))),
            )
                .into_response();
        }
    };
    if let Err(error) = ensure_protocol_version(request.protocol_version) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let metadata = request.action.metadata();
    if !request.action.is_implemented() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::UnsupportedAction,
                format!(
                    "{} is not implemented by this local-control bridge",
                    request.action.as_str()
                ),
            ))),
        )
            .into_response();
    }
    if !metadata
        .allowed_invocation_contexts
        .contains(&request.invocation_context)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                format!(
                    "{} cannot run from the requested invocation context",
                    request.action.as_str()
                ),
            ))),
        )
            .into_response();
    }
    if let Err(error) = request.verify_execution_context_proof() {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let settings_check = state
        .bridge_spawner
        .spawn({
            let action = request.action;
            let invocation_context = request.invocation_context;
            move |_, ctx| ensure_action_allowed(invocation_context, action, ctx)
        })
        .await;
    match settings_check {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponseEnvelope::new(error)),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponseEnvelope::new(ControlError::new(
                    ErrorCode::BridgeUnavailable,
                    "local-control app bridge is unavailable",
                ))),
            )
                .into_response();
        }
    }
    let auth_token = AuthToken::generate();
    let grant = CredentialGrant::new(
        state.instance_id.clone(),
        request.action,
        request.invocation_context,
        Duration::minutes(5),
    );
    let mut credentials = match state.credentials.lock() {
        Ok(credentials) => credentials,
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
    insert_credential(
        &mut credentials,
        auth_token.secret().to_owned(),
        grant.clone(),
    );
    Json(ScopedCredential {
        bearer_token: auth_token.secret().to_owned(),
        grant,
    })
    .into_response()
}

async fn handle_control_request(
    State(state): State<ControlServerState>,
    headers: HeaderMap,
    payload: Result<Json<RequestEnvelope>, JsonRejection>,
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
    let request = match payload {
        Ok(Json(request)) => request,
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
    let grant = match state.credentials.lock() {
        Ok(credentials) => credentials.get(auth_token.secret()).cloned(),
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
    let Some(grant) = grant else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential is invalid",
            ))),
        )
            .into_response();
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

fn outside_warp_publication_supported() -> bool {
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
pub(crate) use permissions::{
    capabilities, ensure_settings_allow_action, outside_warp_control_enabled_for_settings,
};
#[cfg(test)]
pub(crate) use resolver::{
    require_active_window_id, validate_action_params, validate_tab_create_target,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
