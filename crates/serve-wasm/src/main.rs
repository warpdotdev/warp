use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::Response;
use clap::Parser;
use tower::ServiceBuilder;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::Span;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// A small webserver to serve the Warp wasm bundle and assets for local development.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Directory to serve containing the wasm bundle, index.html, and assets.
    directory: PathBuf,

    /// Port to serve on
    #[arg(short, long, default_value_t = 8000)]
    port: u16,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "serve_wasm=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    println!("Serving Warp on http://localhost:{}", args.port);
    serve(make_router(&args.directory), args.port).await
}

async fn serve(app: Router, port: u16) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app.layer(TraceLayer::new_for_http()))
        .await
        .unwrap();
}

fn make_router(build_directory: &Path) -> Router {
    // Create a tracing service so we can show the files we're serving.
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

    Router::new()
        .route_service("/", ServeFile::new(build_directory.join("index.html")))
        .route_service(
            "/session/{session_id}",
            ServeFile::new(build_directory.join("index.html")),
        )
        .route_service(
            "/drive/{object_type}/{object_id}",
            ServeFile::new(build_directory.join("index.html")),
        )
        .nest_service(
            "/assets/client/wasm",
            ServeDir::new(build_directory.join("wasm")),
        )
        // This needs to be kept in sync with warp_util::path::hashed_asset_url.
        .nest_service(
            "/assets/client/static",
            ServeDir::new(build_directory.join("assets")),
        )
        .layer(trace_service)
}
