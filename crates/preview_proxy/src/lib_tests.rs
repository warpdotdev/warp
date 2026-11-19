use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use super::*;

pub(super) struct Ports {
    values: std::sync::Mutex<Vec<u16>>,
    calls: AtomicUsize,
    fail: AtomicBool,
    pause: AtomicBool,
    started: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

impl Ports {
    pub(super) fn new(values: Vec<u16>) -> Arc<Self> {
        Arc::new(Self {
            values: std::sync::Mutex::new(values),
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            pause: AtomicBool::new(false),
            started: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        })
    }
}
pub(super) fn preview(port: u16) -> PreviewRecord {
    PreviewRecord {
        id: Uuid::from_u128(u128::from(port) + 1),
        port,
        origin: format!("https://preview-{port}.example.test"),
    }
}

impl PortSource for Ports {
    async fn previews(&self) -> Result<Vec<PreviewRecord>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.pause.load(Ordering::SeqCst) {
            self.started.notify_one();
            self.resume.notified().await;
        }
        if self.fail.load(Ordering::SeqCst) {
            bail!("preview list unavailable");
        }
        Ok(self
            .values
            .lock()
            .unwrap()
            .iter()
            .map(|port| preview(*port))
            .collect())
    }
}

pub(super) async fn headers(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(3), async {
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(stream.read_u8().await.unwrap());
        }
    })
    .await
    .unwrap();
    String::from_utf8(bytes).unwrap()
}

pub(super) async fn request(port: u16, app_port: u16, extra: &str) -> TcpStream {
    request_with_origin(port, app_port, None, extra).await
}

pub(super) async fn request_with_origin(
    proxy_port: u16,
    app_port: u16,
    origin: Option<&str>,
    extra: &str,
) -> TcpStream {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, proxy_port))
        .await
        .unwrap();
    let origin = origin.map_or_else(String::new, |origin| format!("Origin: {origin}\r\n"));
    stream
        .write_all(
            format!(
                "GET /path?q=preserved HTTP/1.1\r\nHost: ingress.invalid\r\n\
                 X-Warp-Preview-ID: {}\r\n\
                 X-Warp-Preview-Port: {app_port}\r\n{origin}{extra}\r\n",
                preview(app_port).id
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream
}

async fn request_with_identity(
    proxy_port: u16,
    route_port: u16,
    id: Uuid,
    extra: &str,
) -> TcpStream {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, proxy_port))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "GET /path?q=preserved HTTP/1.1\r\nHost: ingress.invalid\r\n\
                 X-Warp-Preview-ID: {id}\r\nX-Warp-Preview-Port: {route_port}\r\n{extra}\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream
}

async fn app() -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| async move {
                    let (parts, body) = request.into_parts();
                    let status = if parts.headers[HOST] == format!("localhost:{port}") {
                        StatusCode::OK
                    } else {
                        StatusCode::FORBIDDEN
                    };
                    let body = body.collect().await.unwrap().to_bytes();
                    let text = format!(
                        "{} {} {:?} {}\n",
                        parts.method,
                        parts.uri,
                        parts.headers,
                        String::from_utf8_lossy(&body)
                    );
                    let mut response = Response::new(Full::new(Bytes::from(text)));
                    *response.status_mut() = status;
                    Ok::<_, Infallible>(response)
                });
                let _ = server::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (port, task)
}

async fn strict_same_origin_post_app() -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let service = service_fn(|request: Request<Incoming>| async move {
                    let (parts, body) = request.into_parts();
                    let body = body.collect().await.unwrap().to_bytes();
                    let host = parts.headers[HOST].to_str().unwrap();
                    let expected_origin = format!("http://{host}");
                    let valid = parts.method == Method::POST
                        && parts.headers[ORIGIN].to_str().unwrap() == expected_origin
                        && parts.headers["x-connections-hmr"] == "1"
                        && body.as_ref() == b"preserve this body";
                    let mut response = Response::new(Full::new(body));
                    *response.status_mut() = if valid {
                        StatusCode::OK
                    } else {
                        StatusCode::BAD_REQUEST
                    };
                    Ok::<_, Infallible>(response)
                });
                let _ = server::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (port, task)
}
#[tokio::test]
async fn rebinds_configured_port_after_shutdown() {
    let source = Ports::new(vec![]);
    let mut proxy = Proxy::bind(0, source.clone()).await.unwrap();
    let port = proxy.port();
    assert_ne!(port, 0);
    assert!(Proxy::bind(port, source.clone()).await.is_err());

    let mut stream = request(port, 12345, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 403"));
    let (app_port, app_task) = app().await;
    *source.values.lock().unwrap() = vec![app_port];
    tokio::time::sleep(MISS_REFRESH_BACKOFF).await;
    let mut stream = request(port, app_port, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    proxy.shutdown().await;

    let mut restarted = Proxy::bind(port, source.clone()).await.unwrap();
    assert_eq!(restarted.port(), port);
    let mut stream = request(port, app_port, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    restarted.shutdown().await;
    app_task.abort();
}

#[tokio::test]
async fn normalizes_host_and_forwarded_headers_without_rewriting_application_credentials() {
    let (app_port, app_task) = app().await;
    let mut direct = TcpStream::connect((Ipv4Addr::LOCALHOST, app_port))
        .await
        .unwrap();
    direct
        .write_all(b"GET / HTTP/1.1\r\nHost: ingress.invalid\r\n\r\n")
        .await
        .unwrap();
    assert!(headers(&mut direct).await.starts_with("HTTP/1.1 403"));

    let mut proxy = Proxy::bind(0, Ports::new(vec![app_port])).await.unwrap();
    let port = proxy.port();
    let mut stream = request(port, app_port,
        "Connection: close, x-remove-me\r\nX-Remove-Me: hidden\r\nAuthorization: Bearer application\r\nCookie: __session=secret; app=visible; __Host-warp_preview_ingress=secret\r\nX-Nsc-Other: secret\r\nX-Warp-Preview-Authorization: secret\r\nX-Forwarded-Host: preview.example\r\nX-Forwarded-Proto: https\r\nX-Forwarded-Port: 443\r\n").await;
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    for expected in [
        "/path?q=preserved",
        "Bearer application",
        "app=visible",
        "\"x-forwarded-proto\": \"http\"",
    ] {
        assert!(response.contains(expected), "{expected}: {response}");
    }
    assert!(!response.contains("\"origin\""), "{response}");
    assert!(response.contains(&format!("\"x-forwarded-host\": \"localhost:{app_port}\"")));
    assert!(response.contains(&format!("\"x-forwarded-port\": \"{app_port}\"")));
    for absent in [
        "secret",
        "x-remove-me",
        "x-nsc-",
        "x-warp-preview-",
        "preview.example",
    ] {
        assert!(!response.contains(absent), "{absent}: {response}");
    }
    proxy.shutdown().await;
    app_task.abort();
}

#[tokio::test]
async fn translates_same_origin_uuid_post_and_rejects_foreign_origin() {
    let (app_port, app_task) = strict_same_origin_post_app().await;
    let mut proxy = Proxy::bind(0, Ports::new(vec![app_port])).await.unwrap();
    let id = preview(app_port).id;
    let public_origin = preview(app_port).origin;
    let body = b"preserve this body";
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, proxy.port()))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "POST /submit HTTP/1.1\r\nHost: ingress.invalid\r\n\
                 X-Warp-Preview-ID: {id}\r\nX-Warp-Preview-Port: {app_port}\r\n\
                 Origin: {public_origin}\r\nX-Connections-HMR: 1\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.write_all(body).await.unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("preserve this body"), "{response}");

    let mut foreign = request_with_origin(
        proxy.port(),
        app_port,
        Some("https://foreign.example.test"),
        "Connection: close\r\n",
    )
    .await;
    assert!(headers(&mut foreign).await.starts_with("HTTP/1.1 403"));

    proxy.shutdown().await;
    app_task.abort();
}

#[tokio::test]
async fn translates_destination_origin_and_normalizes_forwarded_headers() {
    let (target_port, target_task) = app().await;
    let mut proxy = Proxy::bind(0, Ports::new(vec![target_port])).await.unwrap();
    let port = proxy.port();
    let target_origin = preview(target_port).origin;
    let mut stream = request_with_origin(
        port,
        target_port,
        Some(&target_origin),
        "Connection: close\r\nAuthorization: Bearer application\r\n\
         Cookie: app=visible\r\nForwarded: for=spoofed\r\n\
         X-Forwarded-For: spoofed\r\nX-Original-URL: https://spoofed.example/\r\n",
    )
    .await;
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(
        response.contains(&format!("http://localhost:{target_port}")),
        "{response}"
    );
    for expected in [
        &format!("localhost:{target_port}"),
        "\"x-forwarded-proto\": \"http\"",
        &format!("\"x-forwarded-port\": \"{target_port}\""),
        "Bearer application",
        "app=visible",
    ] {
        assert!(response.contains(expected), "{expected}: {response}");
    }
    for absent in ["\"forwarded\"", "x-original-url", "x-warp-preview-"] {
        assert!(!response.contains(absent), "{absent}: {response}");
    }
    assert!(response.contains("\"x-forwarded-for\": \"spoofed\""));

    let mut stream = request(port, target_port, "Connection: close\r\n").await;
    let mut response_without_origin = String::new();
    stream
        .read_to_string(&mut response_without_origin)
        .await
        .unwrap();
    let (_, app_request) = response_without_origin.split_once("\r\n\r\n").unwrap();
    assert!(!app_request.contains("\"origin\""), "{app_request}");
    assert!(response_without_origin.starts_with("HTTP/1.1 200"));
    assert!(!response_without_origin.contains("x-warp-preview-"));

    proxy.shutdown().await;
    target_task.abort();
}

#[tokio::test]
async fn rejects_sibling_preview_origin_for_http_and_websocket() {
    let target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let target_port = target.local_addr().unwrap().port();
    let sibling = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let sibling_port = sibling.local_addr().unwrap().port();
    let mut proxy = Proxy::bind(0, Ports::new(vec![target_port, sibling_port]))
        .await
        .unwrap();
    let sibling_origin = preview(sibling_port).origin;

    for extra in [
        "Connection: close\r\n",
        "Connection: upgrade\r\nUpgrade: websocket\r\n",
    ] {
        let mut stream =
            request_with_origin(proxy.port(), target_port, Some(&sibling_origin), extra).await;
        assert!(headers(&mut stream).await.starts_with("HTTP/1.1 403"));
    }
    assert_eq!(
        target.into_std().unwrap().accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    proxy.shutdown().await;
}

#[tokio::test]
async fn rejects_missing_ambiguous_or_unknown_preview_identity() {
    let (app_port, app_task) = app().await;
    let (sibling_port, sibling_task) = app().await;
    let source = Ports::new(vec![app_port, sibling_port]);
    let mut proxy = Proxy::bind(0, source).await.unwrap();
    let port = proxy.port();
    let id = preview(app_port).id;
    let preview_origin = preview(app_port).origin;
    let cases = [
        format!("X-Warp-Preview-Port: {app_port}\r\n"),
        format!("X-Warp-Preview-ID: invalid\r\nX-Warp-Preview-Port: {app_port}\r\n"),
        format!(
            "X-Warp-Preview-ID: {}\r\n\
             X-Warp-Preview-Port: {app_port}\r\nOrigin: {preview_origin}\r\n",
            Uuid::nil()
        ),
        format!(
            "X-Warp-Preview-ID: {id}\r\n\
             X-Warp-Preview-ID: {id}\r\nX-Warp-Preview-Port: {app_port}\r\n"
        ),
        format!(
            "X-Warp-Preview-ID: {id}\r\n\
             X-Warp-Preview-Port: {app_port}\r\nOrigin: null\r\n"
        ),
        format!(
            "X-Warp-Preview-ID: {id}\r\n\
             X-Warp-Preview-Port: {app_port}\r\nOrigin: https://unknown.example\r\n"
        ),
        format!(
            "X-Warp-Preview-ID: {id}\r\n\
             X-Warp-Preview-Port: {app_port}\r\nOrigin: {preview_origin}\r\n\
             Origin: {preview_origin}\r\n"
        ),
    ];
    for request_headers in cases {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream
            .write_all(
                format!("GET / HTTP/1.1\r\nHost: ingress.invalid\r\n{request_headers}\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        assert!(headers(&mut stream).await.starts_with("HTTP/1.1 403"));
    }
    let mut wrong_id = request_with_identity(
        port,
        app_port,
        preview(sibling_port).id,
        "Connection: close\r\n",
    )
    .await;
    assert!(headers(&mut wrong_id).await.starts_with("HTTP/1.1 403"));
    let mut wrong_port =
        request_with_identity(port, sibling_port, id, "Connection: close\r\n").await;
    assert!(headers(&mut wrong_port).await.starts_with("HTTP/1.1 403"));

    let mut stream = request_with_origin(
        port,
        app_port,
        Some(&preview_origin),
        "Connection: origin, close\r\n",
    )
    .await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 400"));
    proxy.shutdown().await;
    app_task.abort();
    sibling_task.abort();
}

#[test]
fn validates_preview_identity_and_enforces_cache_freshness() {
    let duplicate = preview(3000);
    let mut duplicate_id = preview(3001);
    duplicate_id.id = duplicate.id;
    assert!(validate_previews(vec![duplicate, duplicate_id], 0).is_err());

    let mut nil_id = preview(3004);
    nil_id.id = Uuid::nil();
    assert!(validate_previews(vec![nil_id], 0).is_err());

    let refreshed_at = Instant::now();
    let cache = PreviewCache {
        refreshed_at: Some(refreshed_at),
        ..PreviewCache::default()
    };
    assert!(cache.is_fresh(refreshed_at + PREVIEW_CACHE_TTL - Duration::from_millis(1)));
    assert!(!cache.is_fresh(refreshed_at + PREVIEW_CACHE_TTL));
}

#[tokio::test]
async fn rejects_arbitrary_destination_requests() {
    let app_port = 12345;
    let mut proxy = Proxy::bind(0, Ports::new(vec![app_port])).await.unwrap();
    let port = proxy.port();
    let id = preview(app_port).id;
    for target in ["CONNECT other.example:443", "GET http://other.example/"] {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "{target} HTTP/1.1\r\nHost: ingress.invalid\r\nX-Warp-Preview-ID: {id}\r\nX-Warp-Preview-Port: 12345\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert!(headers(&mut stream).await.starts_with("HTTP/1.1 400"));
    }
    proxy.shutdown().await;
}

#[tokio::test]
async fn routes_eight_apps_through_one_listener_using_each_requests_port() {
    let source = Ports::new(vec![]);
    let mut proxy = Proxy::bind(0, source.clone()).await.unwrap();
    let port = proxy.port();
    let mut apps = Vec::new();
    let mut app_ports = Vec::new();
    let mut requests = JoinSet::new();
    for _ in 0..8 {
        let (app_port, task) = app().await;
        apps.push(task);
        app_ports.push(app_port);
    }
    *source.values.lock().unwrap() = app_ports.clone();
    for app_port in app_ports {
        requests.spawn(async move {
            let mut stream = request(port, app_port, "Connection: close\r\n").await;
            assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
            let mut body = String::new();
            stream.read_to_string(&mut body).await.unwrap();
            assert!(body.contains(&format!("localhost:{app_port}")), "{body}");
        });
    }
    while let Some(result) = requests.join_next().await {
        result.unwrap();
    }
    proxy.shutdown().await;
    assert!(
        TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .is_err()
    );
    for task in apps {
        task.abort();
    }
}

#[tokio::test]
async fn replaces_allowed_ports_only_after_a_successful_cache_miss_fetch() {
    let (first, first_task) = app().await;
    let (second, second_task) = app().await;
    let source = Ports::new(vec![first]);
    let mut proxy = Proxy::bind(0, source.clone()).await.unwrap();

    let mut stream = request(proxy.port(), first, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);

    *source.values.lock().unwrap() = vec![second];
    source.fail.store(true, Ordering::SeqCst);
    tokio::time::sleep(MISS_REFRESH_BACKOFF).await;
    let mut stream = request(proxy.port(), second, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 403"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    let mut stream = request(proxy.port(), first, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);

    source.fail.store(false, Ordering::SeqCst);
    source.pause.store(true, Ordering::SeqCst);
    tokio::time::sleep(MISS_REFRESH_BACKOFF).await;
    let mut refreshing = request(proxy.port(), second, "Connection: close\r\n").await;
    timeout(Duration::from_secs(3), source.started.notified())
        .await
        .unwrap();
    let mut stream = request(proxy.port(), first, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    source.pause.store(false, Ordering::SeqCst);
    source.resume.notify_one();
    assert!(headers(&mut refreshing).await.starts_with("HTTP/1.1 200"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 3);
    tokio::time::sleep(MISS_REFRESH_BACKOFF).await;
    let mut stream = request(proxy.port(), first, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 403"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 4);

    proxy.shutdown().await;
    first_task.abort();
    second_task.abort();
}

#[tokio::test]
async fn rejects_missing_invalid_and_control_ports() {
    let source = Ports::new(vec![1023, 9277]);
    let mut proxy = Proxy::bind(0, source.clone()).await.unwrap();
    let port = proxy.port();
    for destination in ["", "not-a-port", "65536", "1023", "9277", &port.to_string()] {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        let port_header = if destination.is_empty() {
            String::new()
        } else {
            format!("X-Warp-Preview-Port: {destination}\r\n")
        };
        stream
            .write_all(
                format!("GET / HTTP/1.1\r\nHost: ingress.invalid\r\n{port_header}\r\n").as_bytes(),
            )
            .await
            .unwrap();
        assert!(
            headers(&mut stream).await.starts_with("HTTP/1.1 403"),
            "{destination}"
        );
    }
    proxy.shutdown().await;
}

#[tokio::test]
async fn streams_first_chunk_before_completion_and_closes_on_shutdown() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        headers(&mut stream).await;
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n6\r\ndata\n\n\r\n").await.unwrap();
        let mut byte = [0];
        let _ = closed_tx.send(stream.read(&mut byte).await.unwrap());
    });
    let mut proxy = Proxy::bind(0, Ports::new(vec![app_port])).await.unwrap();
    let port = proxy.port();
    let mut stream = request(port, app_port, "").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    let mut chunk = [0; 11];
    timeout(Duration::from_secs(2), stream.read_exact(&mut chunk))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&chunk, b"6\r\ndata\n\n\r\n");
    proxy.shutdown().await;
    assert_eq!(
        timeout(Duration::from_secs(2), stream.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(
        timeout(Duration::from_secs(2), closed_rx)
            .await
            .unwrap()
            .unwrap(),
        0
    );
    task.await.unwrap();
}

#[tokio::test]
async fn copies_upgraded_frames_after_http_reuse_and_closes_on_driver_shutdown() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        headers(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        let upgrade_request = headers(&mut stream).await;
        assert!(upgrade_request.contains("sec-websocket-protocol: vite-hmr"));
        assert!(upgrade_request.contains("te: trailers\r\n"));
        assert!(upgrade_request.contains(&format!("origin: http://localhost:{app_port}\r\n")));
        assert!(upgrade_request.contains(&format!("host: localhost:{app_port}\r\n")));
        let connection_tokens = upgrade_request
            .lines()
            .filter_map(|line| line.strip_prefix("connection: "))
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .collect::<Vec<_>>();
        assert!(connection_tokens.contains(&"upgrade"));
        assert!(connection_tokens.contains(&"te"));
        stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Protocol: vite-hmr\r\n\r\n").await.unwrap();
        let mut frame = [0; 8];
        stream.read_exact(&mut frame).await.unwrap();
        assert_eq!(frame, [0x81, 0x82, 1, 2, 3, 4, b'h' ^ 1, b'i' ^ 2]);
        stream.write_all(&[0x81, 2, b'h', b'i']).await.unwrap();
        assert_eq!(stream.read(&mut frame).await.unwrap(), 0);
    });
    let mut proxy = Proxy::bind(0, Ports::new(vec![app_port])).await.unwrap();
    let port = proxy.port();
    let mut warmup = request(port, app_port, "Connection: close\r\n").await;
    let mut response = String::new();
    timeout(Duration::from_secs(3), warmup.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let mut stream = request_with_origin(
        port,
        app_port,
        Some(&preview(app_port).origin),
        "Connection: upgrade, te\r\nTE: trailers\r\nUpgrade: websocket\r\nSec-WebSocket-Protocol: vite-hmr\r\n",
    )
    .await;
    let response = headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response}");
    assert!(response.contains("sec-websocket-protocol: vite-hmr"));
    assert!(!response.contains("x-warp-preview-"));
    stream
        .write_all(&[0x81, 0x82, 1, 2, 3, 4, b'h' ^ 1, b'i' ^ 2])
        .await
        .unwrap();
    let mut frame = [0; 4];
    stream.read_exact(&mut frame).await.unwrap();
    assert_eq!(frame, [0x81, 2, b'h', b'i']);
    proxy.shutdown().await;
    assert_eq!(
        timeout(Duration::from_secs(2), stream.read(&mut frame))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    task.await.unwrap();
}

#[tokio::test]
async fn connects_to_ipv6_only_apps() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        headers(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let mut proxy = Proxy::bind(0, Ports::new(vec![app_port])).await.unwrap();
    let port = proxy.port();
    let mut stream = request(port, app_port, "").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    proxy.shutdown().await;
    task.await.unwrap();
}

#[tokio::test]
async fn concurrent_cache_misses_share_successful_and_failed_refreshes() {
    for fail in [false, true] {
        let source = Ports::new(vec![12346]);
        source.pause.store(true, Ordering::SeqCst);
        source.fail.store(fail, Ordering::SeqCst);
        let cancel = CancellationToken::new();
        let routing = Routing {
            listener_port: 0,
            source: source.clone(),
            upstream: upstream::new_client(cancel.clone()),
            cache: RwLock::new(PreviewCache::default()),
            refreshing: Mutex::new(()),
        };
        let routing_ref = &routing;
        let mut requests = (0..20)
            .map(|index| {
                let port = 12345 + index % 2;
                let routing = routing_ref;
                (
                    port,
                    Box::pin(async move {
                        routing
                            .authorize(&ParsedRoute {
                                port,
                                id: preview(port).id,
                                origin: None,
                            })
                            .await
                    }),
                )
            })
            .collect::<Vec<_>>();
        let mut ctx = std::task::Context::from_waker(std::task::Waker::noop());
        for (_, request) in &mut requests {
            assert!(request.as_mut().poll(&mut ctx).is_pending());
        }
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        source.pause.store(false, Ordering::SeqCst);
        source.resume.notify_one();
        for (port, request) in requests {
            let allowed = timeout(Duration::from_secs(3), request)
                .await
                .unwrap()
                .is_some();
            assert_eq!(allowed, !fail && port == 12346);
        }
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        assert!(routing.cache.read().await.last_attempt_at.is_some());
        cancel.cancel();
    }
}

#[test]
fn preserves_non_ascii_cookie_bytes_while_removing_reserved_cookies() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::COOKIE,
        HeaderValue::from_bytes(
            b"app_session=abc123; display_name=Jos\xc3\xa9; __session=secret; __Host-warp_preview_ingress=secret",
        )
        .unwrap(),
    );
    headers.append(
        http::header::COOKIE,
        HeaderValue::from_static("__session_mode=secret; theme=dark"),
    );
    strip_credentials(&mut headers);
    assert_eq!(
        headers[http::header::COOKIE].as_bytes(),
        b"app_session=abc123; display_name=Jos\xc3\xa9; theme=dark"
    );
}

#[test]
fn forwards_only_trailers_te_negotiation_and_removes_other_hop_headers() {
    for (value, trailers) in [("gzip, TrAiLeRs", true), ("gzip", false)] {
        let mut headers = HeaderMap::new();
        headers.insert(TE, HeaderValue::from_str(value).unwrap());
        headers.insert(CONNECTION, HeaderValue::from_static("te, x-remove"));
        headers.insert("x-remove", HeaderValue::from_static("hidden"));
        prepare_headers(&mut headers, 3000, None);
        assert!(!headers.contains_key("x-remove"));
        if trailers {
            assert_eq!(headers[TE], "trailers");
            assert_eq!(headers[CONNECTION], "te");
        } else {
            assert!(!headers.contains_key(TE));
            assert!(!headers.contains_key(CONNECTION));
        }
    }
}
