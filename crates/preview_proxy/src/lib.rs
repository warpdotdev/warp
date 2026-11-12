use std::collections::HashSet;
use std::convert::Infallible;
use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http::header::{CONNECTION, HOST, TE, UPGRADE};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
#[cfg(test)]
use hyper::client::conn::http1 as client;
use hyper::server::conn::http1 as server;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;
use upstream::UpstreamClient;

mod upstream;

// Preview routing must not expose Warp's per-channel control servers.
const WARP_RESERVED_PORTS: [u16; 6] = [9277, 9278, 9279, 9280, 9281, 9282];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const PORT_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_CONNECTIONS: usize = 256;

type Body = BoxBody<Bytes, hyper::Error>;

pub trait PortSource: Send + Sync {
    fn ports(&self) -> impl Future<Output = Result<Vec<u16>>> + Send;
}

#[derive(Default)]
struct PortCache {
    allowed: HashSet<u16>,
    generation: u64,
}

struct Routing<S> {
    listener_port: u16,
    source: Arc<S>,
    upstream: UpstreamClient,
    cache: RwLock<PortCache>,
    refreshing: Mutex<()>,
}

impl<S: PortSource> Routing<S> {
    fn resolve(&self, headers: &HeaderMap) -> Option<u16> {
        let port = headers
            .get("x-warp-preview-port")?
            .to_str()
            .ok()?
            .parse::<u16>()
            .ok()?;
        if port < 1024 || WARP_RESERVED_PORTS.contains(&port) || port == self.listener_port {
            return None;
        }
        Some(port)
    }

    async fn allows(&self, port: u16) -> bool {
        let generation = {
            let cache = self.cache.read().await;
            if cache.allowed.contains(&port) {
                return true;
            }
            cache.generation
        };
        let _refresh = self.refreshing.lock().await;
        {
            let cache = self.cache.read().await;
            if cache.generation != generation {
                return cache.allowed.contains(&port);
            }
        }
        let result = timeout(PORT_FETCH_TIMEOUT, self.source.ports()).await;
        let mut cache = self.cache.write().await;
        cache.generation = cache.generation.wrapping_add(1);
        match result {
            Ok(Ok(ports)) => {
                cache.allowed = ports.into_iter().collect();
                cache.allowed.contains(&port)
            }
            Ok(Err(error)) => {
                log::warn!("Preview proxy allowlist fetch failed for app port {port}: {error:#}");
                false
            }
            Err(error) => {
                log::warn!(
                    "Preview proxy allowlist fetch timed out for app port {port}: {error:#}"
                );
                false
            }
        }
    }
}

pub struct Proxy {
    port: u16,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

impl Proxy {
    pub async fn bind(port: u16, source: Arc<impl PortSource + 'static>) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .context("Failed to bind preview listener")?;
        let port = listener.local_addr()?.port();
        let cancel = CancellationToken::new();
        let routing = Arc::new(Routing {
            listener_port: port,
            source,
            upstream: upstream::new_client(cancel.clone()),
            cache: RwLock::new(PortCache::default()),
            refreshing: Mutex::new(()),
        });
        let task = tokio::spawn(serve(
            listener,
            routing,
            cancel.clone(),
            Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        ));
        Ok(Self { port, cancel, task })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn shutdown(&mut self) {
        self.cancel.cancel();
        let _ = (&mut self.task).await;
    }
}

async fn serve<S: PortSource + 'static>(
    listener: TcpListener,
    routing: Arc<Routing<S>>,
    cancel: CancellationToken,
    capacity: Arc<Semaphore>,
) {
    let mut connections = JoinSet::new();
    let mut accept_after = Instant::now();
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            accepted = async {
                tokio::time::sleep_until(accept_after).await;
                listener.accept().await
            } => {
                let (mut stream, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        log::warn!("Preview proxy failed to accept connection: {error}");
                        accept_after = Instant::now() + ACCEPT_RETRY_DELAY;
                        continue;
                    }
                };
                let Ok(permit) = capacity.clone().try_acquire_owned() else {
                    let _ = timeout(Duration::from_millis(100), stream.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                    )).await;
                    continue;
                };
                let routing = routing.clone();
                let cancel = cancel.clone();
                connections.spawn(async move {
                    let permit = Arc::new(permit);
                    let service = service_fn(|request| {
                        forward(request, routing.clone(), cancel.clone(), permit.clone())
                    });
                    let mut builder = server::Builder::new();
                    builder.timer(TokioTimer::new())
                        .header_read_timeout(HEADER_TIMEOUT)
                        .max_buf_size(64 * 1024);
                    let connection = builder.serve_connection(TokioIo::new(stream), service)
                        .with_upgrades();
                    tokio::select! {
                        _ = cancel.cancelled() => {}
                        _ = connection => {}
                    }
                });
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    cancel.cancel();
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

async fn forward<S: PortSource + 'static>(
    mut request: Request<Incoming>,
    routing: Arc<Routing<S>>,
    cancel: CancellationToken,
    permit: Arc<OwnedSemaphorePermit>,
) -> Result<Response<Body>, Infallible> {
    let Some(app_port) = routing.resolve(request.headers()) else {
        return Ok(empty(StatusCode::FORBIDDEN));
    };
    if request.method() == Method::CONNECT
        || request.uri().scheme().is_some()
        || request.uri().authority().is_some()
    {
        return Ok(empty(StatusCode::BAD_REQUEST));
    }
    let upgrade = request.headers().get(UPGRADE).cloned();
    if upgrade
        .as_ref()
        .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"websocket"))
    {
        return Ok(empty(StatusCode::BAD_REQUEST));
    }
    if !routing.allows(app_port).await {
        return Ok(empty(StatusCode::FORBIDDEN));
    }
    let downstream_upgrade = upgrade.as_ref().map(|_| hyper::upgrade::on(&mut request));
    prepare_headers(request.headers_mut(), app_port);
    if let Some(value) = upgrade {
        request.headers_mut().insert(UPGRADE, value);
        request
            .headers_mut()
            .append(CONNECTION, HeaderValue::from_static("upgrade"));
    }

    let result = async {
        let path = request
            .uri()
            .path_and_query()
            .map_or("/", |path| path.as_str());
        // The pooled client keys on authority and sends origin-form on direct connections.
        *request.uri_mut() = Uri::builder()
            .scheme("http")
            .authority(format!("localhost:{app_port}"))
            .path_and_query(path)
            .build()?;
        let mut response = routing.upstream.request(request).await?;
        let upgrade_header = response.headers_mut().remove(UPGRADE);
        strip_hop_headers(response.headers_mut());
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            let Some(downstream) = downstream_upgrade else {
                bail!("unsolicited upstream upgrade");
            };
            let upstream = hyper::upgrade::on(&mut response);
            let Some(upgrade_header) = upgrade_header else {
                bail!("missing upstream upgrade protocol");
            };
            if !upgrade_header.as_bytes().eq_ignore_ascii_case(b"websocket") {
                bail!("unexpected upstream upgrade protocol");
            }
            response.headers_mut().insert(UPGRADE, upgrade_header);
            response
                .headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("upgrade"));
            tokio::spawn(async move {
                let _permit = permit;
                tokio::select! {
                    _ = cancel.cancelled() => {}
                    _ = async {
                        if let (Ok(downstream), Ok(upstream)) = tokio::join!(downstream, upstream) {
                            let _ = tokio::io::copy_bidirectional(
                                &mut TokioIo::new(downstream),
                                &mut TokioIo::new(upstream),
                            ).await;
                        }
                    } => {}
                }
            });
        }
        strip_credentials(response.headers_mut());
        Ok::<_, anyhow::Error>(response.map(BodyExt::boxed))
    }
    .await;
    Ok(result.unwrap_or_else(|error| {
        log::warn!("Preview proxy upstream request failed for app port {app_port}: {error:#}");
        empty(StatusCode::BAD_GATEWAY)
    }))
}

async fn connect(port: u16) -> Result<TcpStream> {
    let ipv4 = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let ipv6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
    timeout(CONNECT_TIMEOUT, async {
        match TcpStream::connect(ipv4).await {
            Ok(stream) => Ok(stream),
            Err(_) => TcpStream::connect(ipv6).await,
        }
    })
    .await?
    .context("Failed to connect to preview app")
}

fn prepare_headers(headers: &mut HeaderMap, port: u16) {
    let trailers = headers
        .get_all(TE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| value.trim().eq_ignore_ascii_case("trailers"));
    strip_hop_headers(headers);
    if trailers {
        headers.insert(TE, HeaderValue::from_static("trailers"));
        headers.insert(CONNECTION, HeaderValue::from_static("te"));
    }
    strip_credentials(headers);
    for name in [
        "x-warp-cloud-agent-id",
        "x-warp-ambient-task-token",
        "forwarded",
        "x-original-host",
        "x-original-url",
        "x-forwarded-prefix",
    ] {
        headers.remove(name);
    }
    let host = HeaderValue::from_str(&format!("localhost:{port}"))
        .expect("loopback host is a valid header");
    headers.insert(HOST, host);
}

fn strip_credentials(headers: &mut HeaderMap) {
    let names = headers
        .keys()
        .filter(|name| {
            let name = name.as_str();
            name.starts_with("x-nsc-")
                || name.starts_with("x-namespace-")
                || name.starts_with("x-warp-preview-")
        })
        .cloned()
        .collect::<Vec<_>>();
    for name in names {
        headers.remove(name);
    }
    let cookies = headers
        .get_all(http::header::COOKIE)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b';'))
        .filter(|cookie| {
            let name = cookie
                .split(|byte| *byte == b'=')
                .next()
                .unwrap_or_default()
                .trim_ascii();
            name != b"__session"
                && name != b"__session_mode"
                && !name.starts_with(b"__Host-warp_preview_")
        })
        .map(|cookie| cookie.trim_ascii())
        .collect::<Vec<_>>()
        .join(b"; ".as_slice());
    headers.remove(http::header::COOKIE);
    if !cookies.is_empty()
        && let Ok(value) = HeaderValue::from_bytes(&cookies)
    {
        headers.insert(http::header::COOKIE, value);
    }
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let nominated = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect::<Vec<_>>();
    for name in nominated {
        headers.remove(name);
    }
    for name in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

fn empty(status: StatusCode) -> Response<Body> {
    let body = Full::new(Bytes::new())
        .map_err(|error| match error {})
        .boxed();
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "streaming_tests.rs"]
mod streaming_tests;

#[cfg(test)]
#[path = "pooling_tests.rs"]
mod pooling_tests;
