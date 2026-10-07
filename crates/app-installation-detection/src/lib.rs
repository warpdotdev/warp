use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, Response};
use axum::routing::get;
use tower::ServiceBuilder;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::Span;

pub fn make_router() -> Router {
    let trace_service = ServiceBuilder::new().layer(
        TraceLayer::new_for_http()
            .make_span_with(|request: &Request<Body>| {
                tracing::info_span!(
                    "http-request",
                    method = request.method().as_str(),
                    uri = request.uri().to_string(),
                )
            })
            .on_request(())
            .on_response(
                |response: &Response<Body>, _latency: Duration, _span: &Span| {
                    tracing::info!(response_status = response.status().as_u16());
                },
            )
            .on_body_chunk(())
            .on_eos(())
            .on_failure(()),
    );

    // We allow requests from localhost, warp.dev and any subdomain of warp.dev.
    let allow_origin_predicate =
        AllowOrigin::predicate(|origin: &HeaderValue, _request_parts: &Parts| {
            origin == "http://localhost:8080"
                || origin == "http://localhost:8082"
                || origin == "https://warp.dev"
                || origin.as_bytes().ends_with(b".warp.dev")
        });

    let cors = CorsLayer::new()
        .allow_methods([Method::GET])
        .allow_origin(allow_origin_predicate);

    Router::new()
        .route_service("/install_detection", get(detect_installation))
        .layer(trace_service)
        .layer(cors)
}

async fn detect_installation() -> &'static str {
    "ok"
}
