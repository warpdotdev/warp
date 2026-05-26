//! Local HTTP server entry point for Warp control requests.
//!
//! This module owns the in-process listener, discovery registration, credential
//! broker endpoint, and request handoff from Axum into the WarpUI model graph.
//!
//! Authentication is split into two localhost endpoints. Clients first request a
//! short-lived scoped credential from `/v1/control/credentials`; the localhost
//! server running inside Warp checks the feature flag, requested invocation
//! context, action metadata, execution-context proof, and Settings > Scripting
//! permissions before minting a bearer token. This foundation branch currently
//! supports only outside-Warp credential requests; verified inside-Warp
//! terminal credentials remain future work until the app-issued proof broker is
//! implemented. The client then presents that bearer token to `/v1/control`,
//! where the server looks up the in-memory grant, verifies it still matches the
//! requested action, and only then hands the request to the main-thread
//! `LocalControlBridge`.
//!
//! The Settings > Scripting gates used here are provisional foundation-branch
//! authority. They are private and local-only, but private preferences are not
//! equivalent to tamper-resistant secure storage; before outside-Warp control
//! or broader grants ship, the authoritative enablement bits should move to
//! protected storage where the platform supports it.
//!
//! This foundation branch intentionally keeps raw bearer tokens out of
//! discovery records: discovery only exposes endpoint metadata and credential
//! broker references when outside-Warp control is enabled.
mod bridge;
mod handlers;
mod permissions;
mod resolver;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use ::local_control::auth::{
    CredentialGrant, CredentialRequest, ScopedCredential, TerminalSessionProofRegistry,
};
use ::local_control::{
    ActionKind, AuthToken, ControlEndpoint, ControlError, ControlResponse, ErrorCode,
    ErrorResponseEnvelope, InstanceId, InstanceRecord, RegisteredInstance, RequestEnvelope,
    ResponseEnvelope, PROTOCOL_VERSION,
};
use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Duration;
use warp_core::channel::ChannelState;
use warpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

pub use bridge::LocalControlBridge;
use permissions::{
    authenticated_user_subject_for_action, ensure_action_allowed, ensure_feature_enabled,
};

/// Shared state made available to Axum handlers for one localhost server
/// running inside Warp.
#[derive(Clone)]
struct ControlServerState {
    bridge_spawner: ModelSpawner<LocalControlBridge>,
    instance_id: InstanceId,
    credentials: Arc<Mutex<HashMap<String, CredentialGrant>>>,
    terminal_proofs: Arc<Mutex<TerminalSessionProofRegistry>>,
}
/// Process-local localhost server running inside Warp for control actions.
pub struct LocalControlServer {
    _runtime: Option<tokio::runtime::Runtime>,
    control_endpoint: Option<ControlEndpoint>,
    _registered_instance: Option<RegisteredInstance>,
    state: Option<ControlServerState>,
}

impl Entity for LocalControlServer {
    type Event = ();
}

impl SingletonEntity for LocalControlServer {}

impl LocalControlServer {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        if !permissions::warp_control_cli_enabled() {
            return Self {
                _runtime: None,
                control_endpoint: None,
                _registered_instance: None,
                state: None,
            };
        }
        match Self::start(ctx) {
            Ok(server) => {
                ctx.subscribe_to_model(
                    &crate::settings::LocalControlSettings::handle(ctx),
                    |server, _, ctx| {
                        server.invalidate_all_grants();
                        if let Err(error) = server.refresh_discovery_record(ctx) {
                            log::warn!(
                                "Failed to refresh local-control discovery record: {error:#}"
                            );
                        }
                    },
                );
                server
            }
            Err(error) => {
                log::warn!("Failed to start local-control server: {error:#}");
                Self {
                    _runtime: None,
                    control_endpoint: None,
                    _registered_instance: None,
                    state: None,
                }
            }
        }
    }

    fn start(ctx: &mut ModelContext<Self>) -> Result<Self, ControlError> {
        ensure_feature_enabled()?;
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
            credentials: Arc::default(),
            terminal_proofs: Arc::default(),
        };
        let router_state = state.clone();
        let router = Router::new()
            .route("/v1/control", post(handle_control_request))
            .route("/v1/control/credentials", post(handle_credential_request))
            .with_state(router_state);
        runtime.spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                log::warn!("local-control listener stopped: {err:#}");
            }
        });
        Ok(Self {
            _runtime: Some(runtime),
            control_endpoint: Some(control_endpoint),
            _registered_instance: Some(registered_instance),
            state: Some(state),
        })
    }

    pub(crate) fn invalidate_all_grants(&self) {
        if let Some(state) = &self.state {
            if let Ok(mut credentials) = state.credentials.lock() {
                credentials.clear();
            }
            if let Ok(mut terminal_proofs) = state.terminal_proofs.lock() {
                terminal_proofs.invalidate_all();
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn invalidate_terminal_session_grants(&self, terminal_session_id: &str) {
        if let Some(state) = &self.state {
            if let Ok(mut credentials) = state.credentials.lock() {
                credentials.retain(|_, grant| {
                    grant.invocation_context != ::local_control::InvocationContext::InsideWarp
                });
            }
            if let Ok(mut terminal_proofs) = state.terminal_proofs.lock() {
                terminal_proofs.revoke_session(terminal_session_id);
            }
        }
    }

    fn refresh_discovery_record(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), ControlError> {
        let Some(control_endpoint) = self.control_endpoint.clone() else {
            return Ok(());
        };
        let Some(registered_instance) = &mut self._registered_instance else {
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
    payload: Result<Json<CredentialRequest>, JsonRejection>,
) -> Response {
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
    if request.protocol_version != PROTOCOL_VERSION {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::ProtocolVersionUnsupported,
                format!("unsupported protocol version {}", request.protocol_version),
            ))),
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
    let proof_check = match state.terminal_proofs.lock() {
        Ok(terminal_proofs) => request
            .verify_execution_context_proof_with_registry(&state.instance_id, &terminal_proofs),
        Err(_) => Err(ControlError::new(
            ErrorCode::Internal,
            "local-control terminal proof registry is unavailable",
        )),
    };
    if let Err(error) = proof_check {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let authorization_check = state
        .bridge_spawner
        .spawn({
            let action = request.action;
            let invocation_context = request.invocation_context;
            move |_, ctx| {
                ensure_action_allowed(invocation_context, action, ctx)?;
                authenticated_user_subject_for_action(action, ctx)
            }
        })
        .await;
    let authenticated_subject = match authorization_check {
        Ok(Ok(subject)) => subject,
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
    };
    let auth_token = AuthToken::generate();
    let mut grant = CredentialGrant::new(
        state.instance_id.clone(),
        request.action,
        request.invocation_context,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = authenticated_subject;
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
    credentials.insert(auth_token.secret().to_owned(), grant.clone());
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
    if let Err(error) = ensure_feature_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
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

#[cfg(test)]
pub(crate) use permissions::{
    capabilities, ensure_agent_profile_allows_action, ensure_settings_allow_action,
    outside_warp_action_enabled_for_settings,
};
#[cfg(test)]
pub(crate) use resolver::{
    require_active_window_id, validate_action_params, validate_tab_create_target,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
