use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http::header::{CONNECTION, HOST, UPGRADE};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::client::conn::http1 as client;
use hyper::server::conn::http1 as server;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_util::sync::CancellationToken;

pub mod protocol;

pub const TOKEN_HEADER: &str = "x-warp-preview-proxy-token";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 256;

type Body = BoxBody<Bytes, hyper::Error>;

#[derive(Clone, PartialEq, Eq)]
pub struct Route {
    pub id: String,
    pub generation: i64,
    pub app_port: u16,
    pub token: String,
}

struct Listener {
    route: Route,
    port: u16,
    deadline: watch::Sender<Instant>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

pub struct Proxy {
    listeners: HashMap<String, Listener>,
    capacity: Arc<Semaphore>,
}

impl Default for Proxy {
    fn default() -> Self {
        Self {
            listeners: HashMap::new(),
            capacity: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        }
    }
}

impl Proxy {
    pub async fn register(&mut self, route: Route, deadline: Instant) -> Result<u16> {
        if deadline <= Instant::now() {
            bail!("preview route lease expired");
        }
        if route.id.is_empty()
            || route.generation <= 0
            || route.app_port < 1024
            || (9277..=9282).contains(&route.app_port)
            || route.token.len() < 32
            || route.token.len() > 128
            || HeaderValue::from_str(&route.token).is_err()
        {
            bail!("invalid preview route");
        }
        if self
            .listeners
            .values()
            .any(|listener| listener.port == route.app_port)
        {
            bail!("preview destination is a proxy listener");
        }
        if let Some(listener) = self.listeners.get(&route.id) {
            if listener.route != route {
                bail!("preview route generation changed without removal");
            }
            if listener.task.is_finished() || listener.cancel.is_cancelled() {
                bail!("preview route listener expired");
            }
            listener.deadline.send_replace(deadline);
            return Ok(listener.port);
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .context("Failed to bind preview listener")?;
        let port = listener.local_addr()?.port();
        if port == route.app_port
            || self
                .listeners
                .values()
                .any(|listener| listener.route.app_port == port)
        {
            bail!("preview listener conflicts with its destination");
        }
        let (deadline_tx, deadline_rx) = watch::channel(deadline);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(serve(
            listener,
            route.clone(),
            deadline_rx,
            cancel.clone(),
            self.capacity.clone(),
        ));
        self.listeners.insert(
            route.id.clone(),
            Listener {
                route,
                port,
                deadline: deadline_tx,
                cancel,
                task,
            },
        );
        Ok(port)
    }

    pub fn retain(&mut self, ids: &[String]) {
        self.listeners.retain(|id, _| ids.contains(id));
    }

    pub async fn shutdown(&mut self) {
        for (_, mut listener) in self.listeners.drain() {
            listener.cancel.cancel();
            let _ = (&mut listener.task).await;
        }
    }
}

async fn serve(
    listener: TcpListener,
    route: Route,
    mut deadline: watch::Receiver<Instant>,
    cancel: CancellationToken,
    capacity: Arc<Semaphore>,
) {
    let mut connections = JoinSet::new();
    loop {
        let expires_at = *deadline.borrow();
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = sleep_until(expires_at) => break,
            changed = deadline.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let Ok((mut stream, _)) = accepted else { break };
                let Ok(permit) = capacity.clone().try_acquire_owned() else {
                    let _ = timeout(Duration::from_millis(100), stream.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                    )).await;
                    continue;
                };
                let route = route.clone();
                let cancel = cancel.clone();
                connections.spawn(async move {
                    let permit = Arc::new(permit);
                    let service = service_fn(|request| {
                        forward(request, route.clone(), cancel.clone(), permit.clone())
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

async fn forward(
    mut request: Request<Incoming>,
    route: Route,
    cancel: CancellationToken,
    permit: Arc<OwnedSemaphorePermit>,
) -> Result<Response<Body>, Infallible> {
    let token = request.headers().get(TOKEN_HEADER);
    if !token.is_some_and(|value| bool::from(value.as_bytes().ct_eq(route.token.as_bytes()))) {
        return Ok(empty(StatusCode::FORBIDDEN));
    }
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
    let downstream_upgrade = upgrade.as_ref().map(|_| hyper::upgrade::on(&mut request));
    if !prepare_headers(request.headers_mut(), route.app_port) {
        return Ok(empty(StatusCode::BAD_REQUEST));
    }
    if let Some(value) = upgrade {
        request.headers_mut().insert(UPGRADE, value);
        request
            .headers_mut()
            .insert(CONNECTION, HeaderValue::from_static("upgrade"));
    }
    let path = request
        .uri()
        .path_and_query()
        .map_or("/", |path| path.as_str());
    let Ok(uri) = format!("http://localhost:{}{path}", route.app_port).parse::<Uri>() else {
        return Ok(empty(StatusCode::BAD_REQUEST));
    };
    *request.uri_mut() = uri;

    let result = async {
        let stream = connect(route.app_port).await?;
        let (mut sender, connection) = client::handshake(TokioIo::new(stream)).await?;
        let connection_cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = connection_cancel.cancelled() => {}
                _ = connection.with_upgrades() => {}
            }
        });
        let mut response = sender.send_request(request).await?;
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            let Some(downstream) = downstream_upgrade else {
                bail!("unsolicited upstream upgrade");
            };
            let upstream = hyper::upgrade::on(&mut response);
            let upgrade_header = response.headers().get(UPGRADE).cloned();
            strip_hop_headers(response.headers_mut());
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
        } else {
            strip_hop_headers(response.headers_mut());
        }
        strip_credentials(response.headers_mut());
        Ok::<_, anyhow::Error>(response.map(BodyExt::boxed))
    }
    .await;
    Ok(result.unwrap_or_else(|_| empty(StatusCode::BAD_GATEWAY)))
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

fn prepare_headers(headers: &mut HeaderMap, port: u16) -> bool {
    let mut public = Vec::new();
    for (source, destination) in [
        ("x-warp-preview-public-host", "x-forwarded-host"),
        ("x-warp-preview-public-proto", "x-forwarded-proto"),
        ("x-warp-preview-public-port", "x-forwarded-port"),
    ] {
        let Some(value) = headers.remove(source) else {
            return false;
        };
        public.push((HeaderName::from_static(destination), value));
    }
    if !public[1].1.as_bytes().eq_ignore_ascii_case(b"https")
        && !public[1].1.as_bytes().eq_ignore_ascii_case(b"http")
    {
        return false;
    }
    strip_hop_headers(headers);
    strip_credentials(headers);
    for name in [
        TOKEN_HEADER,
        "x-nsc-ingress-auth",
        "x-warp-cloud-agent-id",
        "x-warp-ambient-task-token",
        "forwarded",
        "x-original-host",
        "x-original-url",
        "x-forwarded-prefix",
    ] {
        headers.remove(name);
    }
    for (name, value) in public {
        headers.insert(name, value);
    }
    let Ok(host) = HeaderValue::from_str(&format!("localhost:{port}")) else {
        return false;
    };
    headers.insert(HOST, host);
    true
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
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter(|cookie| {
            let name = cookie.split('=').next().unwrap_or_default().trim();
            name != "__session"
                && name != "__session_mode"
                && !name.starts_with("__Host-warp_preview_")
        })
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("; ");
    headers.remove(http::header::COOKIE);
    if !cookies.is_empty()
        && let Ok(value) = HeaderValue::from_str(&cookies)
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
