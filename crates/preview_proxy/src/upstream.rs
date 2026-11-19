use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Context as _;
use http::Uri;
use hyper::body::Incoming;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;
use tower::Service;

pub(super) type UpstreamClient = Client<LoopbackConnector, Incoming>;

pub(super) fn new_client(cancel: CancellationToken) -> UpstreamClient {
    Client::builder(ProxyExecutor { cancel })
        .pool_timer(TokioTimer::new())
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(8)
        .build(LoopbackConnector)
}

#[derive(Clone)]
pub(super) struct LoopbackConnector;

impl Service<Uri> for LoopbackConnector {
    type Response = TokioIo<TcpStream>;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = anyhow::Result<Self::Response>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move {
            anyhow::ensure!(
                uri.scheme_str() == Some("http") && uri.host() == Some("localhost"),
                "Preview upstream must be loopback HTTP"
            );
            let port = uri.port_u16().context("Preview upstream port is missing")?;
            super::connect(port).await.map(TokioIo::new)
        })
    }
}

#[derive(Clone)]
struct ProxyExecutor {
    cancel: CancellationToken,
}

impl<F> hyper::rt::Executor<F> for ProxyExecutor
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            let _ = cancel.run_until_cancelled(future).await;
        });
    }
}
