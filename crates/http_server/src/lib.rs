use std::net::SocketAddr;

use tower_http::trace::TraceLayer;
use warp_core::channel::{Channel, ChannelState};
use warpui_core::{Entity, ModelContext, SingletonEntity};

// Spells "Warp" - should hopefully not conflict with other ports.
// Does not conflict with known ports on https://en.wikipedia.org/wiki/List_of_TCP_and_UDP_port_numbers
const PORT_BASE: u16 = 9277;

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

        let mut root = axum::Router::new();
        for router in routers {
            root = root.merge(router);
        }

        runtime.spawn(async move {
            let addr = SocketAddr::from(([127, 0, 0, 1], Self::port()));
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => listener,
                Err(err) => {
                    log::error!("Failed to bind local HTTP server to {addr}: {err}");
                    return;
                }
            };

            if let Err(err) = axum::serve(listener, root.layer(TraceLayer::new_for_http())).await {
                log::error!("Local HTTP server exited with error: {err}");
            }
        });

        Ok(runtime)
    }

    pub fn port() -> u16 {
        match ChannelState::channel() {
            Channel::Stable => PORT_BASE,
            Channel::Preview => PORT_BASE + 1,
            Channel::Dev => PORT_BASE + 2,
            Channel::Local => PORT_BASE + 3,
            Channel::Integration => PORT_BASE + 4,
            Channel::Oss => PORT_BASE + 5,
        }
    }
}

impl Entity for HttpServer {
    type Event = ();
}

impl SingletonEntity for HttpServer {}
