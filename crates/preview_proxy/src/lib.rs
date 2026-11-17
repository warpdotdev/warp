use std::collections::HashSet;
use std::convert::Infallible;
use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http::header::{CONNECTION, HOST, ORIGIN, TE, UPGRADE};
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
use uuid::Uuid;

mod upstream;

// Preview routing must not expose Warp's per-channel control servers.
const WARP_RESERVED_PORTS: [u16; 6] = [9277, 9278, 9279, 9280, 9281, 9282];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const PORT_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const PREVIEW_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const MISS_REFRESH_BACKOFF: Duration = Duration::from_secs(1);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_CONNECTIONS: usize = 256;

type Body = BoxBody<Bytes, hyper::Error>;
#[derive(Clone, Debug)]
pub struct PreviewRecord {
    pub id: Uuid,
    pub port: u16,
    pub origin: String,
}

pub trait PortSource: Send + Sync {
    fn previews(&self) -> impl Future<Output = Result<Vec<PreviewRecord>>> + Send;
}

#[derive(Default)]
struct PreviewCache {
    previews: Vec<PreviewRecord>,
    refreshed_at: Option<Instant>,
    last_attempt_at: Option<Instant>,
    refresh_generation: u64,
}

impl PreviewCache {
    fn is_fresh(&self, now: Instant) -> bool {
        self.refreshed_at
            .is_some_and(|refreshed_at| now.duration_since(refreshed_at) < PREVIEW_CACHE_TTL)
    }

    fn miss_refresh_due(&self, now: Instant) -> bool {
        self.last_attempt_at.is_none_or(|last_attempt_at| {
            now.duration_since(last_attempt_at) >= MISS_REFRESH_BACKOFF
        })
    }
}

struct ParsedRoute {
    port: u16,
    id: Uuid,
    origin: Option<String>,
}

struct AuthorizedRoute {
    port: u16,
    translated_origin: Option<String>,
}

struct Routing<S> {
    listener_port: u16,
    source: Arc<S>,
    upstream: UpstreamClient,
    cache: RwLock<PreviewCache>,
    refreshing: Mutex<()>,
}

impl<S: PortSource> Routing<S> {
    async fn authorize(&self, route: &ParsedRoute) -> Option<AuthorizedRoute> {
        let refresh_generation = self.cache.read().await.refresh_generation;
        if let Some(authorized) = self.authorize_from_cache(route).await {
            return Some(authorized);
        }
        self.refresh_after_miss(refresh_generation).await;
        self.authorize_from_cache(route).await
    }

    async fn authorize_from_cache(&self, route: &ParsedRoute) -> Option<AuthorizedRoute> {
        let now = Instant::now();
        let cache = self.cache.read().await;
        if !cache.is_fresh(now) {
            return None;
        }
        let preview = cache
            .previews
            .iter()
            .find(|preview| preview.port == route.port && preview.id == route.id)?;
        let translated_origin = match &route.origin {
            Some(origin) => {
                if preview.origin != *origin {
                    return None;
                }
                Some(format!("http://localhost:{}", preview.port))
            }
            None => None,
        };
        Some(AuthorizedRoute {
            port: preview.port,
            translated_origin,
        })
    }

    async fn refresh_after_miss(&self, observed_generation: u64) {
        let _refresh = self.refreshing.lock().await;
        let now = Instant::now();
        let cache = self.cache.read().await;
        if cache.refresh_generation != observed_generation || !cache.miss_refresh_due(now) {
            return;
        }
        drop(cache);
        self.refresh_locked().await;
    }

    async fn refresh_locked(&self) {
        let result = timeout(PORT_FETCH_TIMEOUT, self.source.previews()).await;
        let previews = match result {
            Ok(Ok(previews)) => match validate_previews(previews, self.listener_port) {
                Ok(previews) => Some(previews),
                Err(error) => {
                    log::warn!("Preview proxy received an invalid preview list: {error:#}");
                    None
                }
            },
            Ok(Err(error)) => {
                log::warn!("Preview proxy preview-list fetch failed: {error:#}");
                None
            }
            Err(error) => {
                log::warn!("Preview proxy preview-list fetch timed out: {error:#}");
                None
            }
        };
        let mut cache = self.cache.write().await;
        cache.last_attempt_at = Some(Instant::now());
        cache.refresh_generation = cache.refresh_generation.wrapping_add(1);
        if let Some(previews) = previews {
            cache.previews = previews;
            cache.refreshed_at = Some(Instant::now());
        }
    }
}

fn validate_previews(
    previews: Vec<PreviewRecord>,
    listener_port: u16,
) -> Result<Vec<PreviewRecord>> {
    let mut ids = HashSet::new();
    let mut ports = HashSet::new();
    let mut origins = HashSet::new();
    for preview in &previews {
        anyhow::ensure!(
            preview.port >= 1024
                && !WARP_RESERVED_PORTS.contains(&preview.port)
                && preview.port != listener_port,
            "invalid preview app port {}",
            preview.port
        );
        anyhow::ensure!(ports.insert(preview.port), "duplicate preview app port");
        anyhow::ensure!(!preview.id.is_nil(), "preview UUID must not be nil");
        anyhow::ensure!(valid_origin(&preview.origin), "invalid preview origin");
        anyhow::ensure!(ids.insert(preview.id), "duplicate preview UUID");
        anyhow::ensure!(origins.insert(&preview.origin), "duplicate preview origin");
    }
    Ok(previews)
}

fn valid_origin(origin: &str) -> bool {
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    let Some(scheme) = uri.scheme_str() else {
        return false;
    };
    let Some(authority) = uri.authority() else {
        return false;
    };
    if !matches!(scheme, "http" | "https") || authority.as_str().contains('@') {
        return false;
    }
    let path = uri.path_and_query().map_or("", |path| path.as_str());
    (path.is_empty() || path == "/") && origin == format!("{scheme}://{authority}")
}

fn unique_header<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> std::result::Result<Option<&'a HeaderValue>, ()> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(());
    }
    Ok(value)
}

fn parse_route(headers: &HeaderMap, listener_port: u16) -> Option<ParsedRoute> {
    let port_header = unique_header(headers, "x-warp-preview-port").ok()??;
    let port_text = port_header.to_str().ok()?;
    if !port_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port = port_text.parse::<u16>().ok()?;
    if port < 1024 || WARP_RESERVED_PORTS.contains(&port) || port == listener_port {
        return None;
    }

    let id_value = unique_header(headers, "x-warp-preview-id").ok()??;
    let id_text = id_value.to_str().ok()?;
    let id = Uuid::parse_str(id_text).ok()?;
    if id.is_nil() || id.to_string() != id_text {
        return None;
    }
    let origin = match unique_header(headers, "origin").ok()? {
        Some(value) => Some(value.to_str().ok()?.to_owned()),
        None => None,
    };
    Some(ParsedRoute { port, id, origin })
}

fn connection_nominates_identity(headers: &HeaderMap) -> bool {
    const IDENTITY_HEADERS: [&str; 11] = [
        "host",
        "origin",
        "forwarded",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-port",
        "x-original-host",
        "x-original-url",
        "x-original-uri",
        "x-warp-preview-id",
        "x-warp-preview-port",
    ];
    for value in headers.get_all(CONNECTION).iter() {
        let Ok(value) = value.to_str() else {
            return true;
        };
        for token in value.split(',') {
            let token = token.trim();
            let Ok(name) = HeaderName::from_bytes(token.as_bytes()) else {
                return true;
            };
            if IDENTITY_HEADERS.contains(&name.as_str()) {
                return true;
            }
        }
    }
    false
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
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
            .await
            .context("Failed to bind preview listener")?;
        let port = listener.local_addr()?.port();
        let cancel = CancellationToken::new();
        let routing = Arc::new(Routing {
            listener_port: port,
            source,
            upstream: upstream::new_client(cancel.clone()),
            cache: RwLock::new(PreviewCache::default()),
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
    if connection_nominates_identity(request.headers()) {
        return Ok(empty(StatusCode::BAD_REQUEST));
    }
    let Some(route) = parse_route(request.headers(), routing.listener_port) else {
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
    let Some(authorized) = routing.authorize(&route).await else {
        return Ok(empty(StatusCode::FORBIDDEN));
    };
    let app_port = authorized.port;
    let downstream_upgrade = upgrade.as_ref().map(|_| hyper::upgrade::on(&mut request));
    prepare_headers(
        request.headers_mut(),
        app_port,
        authorized.translated_origin,
    );
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

fn prepare_headers(headers: &mut HeaderMap, port: u16, translated_origin: Option<String>) {
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
    for name in ["x-warp-cloud-agent-id", "x-warp-ambient-task-token"] {
        headers.remove(name);
    }
    for name in [
        "forwarded",
        "x-original-host",
        "x-original-url",
        "x-original-uri",
        "x-forwarded-prefix",
    ] {
        headers.remove(name);
    }
    let host = HeaderValue::from_str(&format!("localhost:{port}"))
        .expect("loopback host is a valid header");
    headers.insert(HOST, host.clone());
    if let Some(origin) = translated_origin {
        headers.insert(
            ORIGIN,
            HeaderValue::from_str(&origin).expect("translated loopback origin is valid"),
        );
    }
    headers.insert("x-forwarded-host", host);
    headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
    headers.insert(
        "x-forwarded-port",
        HeaderValue::from_str(&port.to_string()).expect("port is a valid header value"),
    );
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
