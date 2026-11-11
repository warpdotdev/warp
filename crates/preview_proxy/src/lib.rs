use std::collections::HashSet;
use std::convert::Infallible;
use std::future::Future;
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
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_LISTENER_PORT: u16 = 49152;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const PORT_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONNECTIONS: usize = 256;

type Body = BoxBody<Bytes, hyper::Error>;

pub trait PortSource: Send + Sync {
    fn ports(&self) -> impl Future<Output = Result<Vec<u16>>> + Send;
}

struct Routing<S> {
    listener_port: u16,
    source: Arc<S>,
    allowed: RwLock<HashSet<u16>>,
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
        if port < 1024 || (9277..=9282).contains(&port) || port == self.listener_port {
            return None;
        }
        Some(port)
    }

    async fn allows(&self, port: u16) -> bool {
        if self.allowed.read().await.contains(&port) {
            return true;
        }
        let _refresh = self.refreshing.lock().await;
        if self.allowed.read().await.contains(&port) {
            return true;
        }
        match timeout(PORT_FETCH_TIMEOUT, self.source.ports()).await {
            Ok(Ok(ports)) => {
                let mut allowed = self.allowed.write().await;
                *allowed = ports.into_iter().collect();
                allowed.contains(&port)
            }
            _ => false,
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
    pub async fn bind(
        port: u16,
        deadline: Option<Instant>,
        source: Arc<impl PortSource + 'static>,
    ) -> Result<Self> {
        anyhow::ensure!(port != 0, "Preview listener port must not be zero");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .context("Failed to bind preview listener")?;
        let port = listener.local_addr()?.port();
        let routing = Arc::new(Routing {
            listener_port: port,
            source,
            allowed: RwLock::new(HashSet::new()),
            refreshing: Mutex::new(()),
        });
        let cancel = CancellationToken::new();
        let task = tokio::spawn(serve(
            listener,
            routing,
            deadline,
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
    deadline: Option<Instant>,
    cancel: CancellationToken,
    capacity: Arc<Semaphore>,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => break,
            accepted = listener.accept() => {
                let Ok((mut stream, _)) = accepted else { break };
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
    if !prepare_headers(request.headers_mut(), app_port) {
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
    let Ok(uri) = format!("http://localhost:{app_port}{path}").parse::<Uri>() else {
        return Ok(empty(StatusCode::BAD_REQUEST));
    };
    *request.uri_mut() = uri;

    let result = async {
        let stream = connect(app_port).await?;
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
    strip_hop_headers(headers);
    strip_credentials(headers);
    for name in [
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
