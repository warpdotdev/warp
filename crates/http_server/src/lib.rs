use std::net::SocketAddr;

use tower_http::trace::TraceLayer;
use warpui::{Entity, ModelContext, SingletonEntity};

// Spells "Warp" - should hopefully not conflict with other ports.
// Does not conflict with known ports on https://en.wikipedia.org/wiki/List_of_TCP_and_UDP_port_numbers
const PORT: u16 = 9277;

/// A singleton model for the small HTTP server that is run by the Warp client.
pub struct HttpServer {
    /// The tokio runtime that the HTTP server runs on.
    ///
    /// We use a private runtime only because we don't currently have a shared
    /// tokio runtime.
    ///
    /// TODO(vorporeal): Remove this when we have a shared tokio runtime.
    _runtime: Option<tokio::runtime::Runtime>,
}

impl HttpServer {
    pub fn new(
        routers: impl IntoIterator<Item = axum::Router>,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        let runtime = Self::spawn_server(routers)
            .inspect_err(|err| {
                log::warn!("Failed to start local HTTP server: {err:#}");
            })
            .ok();

        Self { _runtime: runtime }
    }

    fn spawn_server(
        routers: impl IntoIterator<Item = axum::Router>,
    ) -> Result<tokio::runtime::Runtime, std::io::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .build()?;
        let root = root_router(routers);

        runtime.spawn(async move {
            let listener = tokio::net::TcpListener::bind(server_addr()).await?;

            axum::serve(listener, root.layer(TraceLayer::new_for_http())).await
        });

        Ok(runtime)
    }
}
fn root_router(routers: impl IntoIterator<Item = axum::Router>) -> axum::Router {
    let mut root = axum::Router::new();
    for router in routers {
        root = root.merge(router);
    }
    root
}

fn server_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], PORT))
}

impl Entity for HttpServer {
    type Event = ();
}

impl SingletonEntity for HttpServer {}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        routing::get,
    };
    use tower::Service;

    use super::*;

    #[test]
    fn server_addr_uses_localhost_and_warp_port() {
        assert_eq!(server_addr(), SocketAddr::from(([127, 0, 0, 1], 9277)));
    }

    #[test]
    fn root_router_serves_routes_from_all_merged_routers() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime should build");

        runtime.block_on(async {
            let first = axum::Router::new().route("/first", get(|| async { "first" }));
            let second = axum::Router::new().route("/second", get(|| async { "second" }));
            let mut router = root_router([first, second]);

            assert_response_body(&mut router, "/first", "first").await;
            assert_response_body(&mut router, "/second", "second").await;
        });
    }

    #[test]
    fn root_router_without_inputs_returns_not_found() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime should build");

        runtime.block_on(async {
            let mut router = root_router([]);
            let response = router
                .call(
                    Request::builder()
                        .uri("/missing")
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should respond");

            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        });
    }

    async fn assert_response_body(router: &mut axum::Router, uri: &str, expected_body: &str) {
        let response = router
            .call(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        assert_eq!(&body[..], expected_body.as_bytes());
    }
}
