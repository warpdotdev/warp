use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

pub(super) static PROXY_PORT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) const TOKEN: &str = "a-valid-route-token-with-at-least-32-bytes";

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

impl PortSource for Ports {
    async fn ports(&self) -> Result<Vec<u16>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            bail!("port list unavailable");
        }
        if self.pause.load(Ordering::SeqCst) {
            self.started.notify_one();
            self.resume.notified().await;
        }
        Ok(self.values.lock().unwrap().clone())
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
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    stream.write_all(format!(
        "GET /path?q=preserved HTTP/1.1\r\nHost: ingress.invalid\r\n{TOKEN_HEADER}: {TOKEN}\r\nX-Warp-Preview-Port: {app_port}\r\nX-Warp-Preview-Public-Host: preview.example\r\nX-Warp-Preview-Public-Proto: https\r\nX-Warp-Preview-Public-Port: 443\r\n{extra}\r\n"
    ).as_bytes()).await.unwrap();
    stream
}

async fn app() -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| async move {
                    let status = if request.headers()[HOST] == format!("localhost:{port}") {
                        StatusCode::OK
                    } else {
                        StatusCode::FORBIDDEN
                    };
                    let text = format!("{} {:?}\n", request.uri(), request.headers());
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

#[tokio::test]
async fn normalizes_host_without_rewriting_origin_or_application_credentials() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let (app_port, app_task) = app().await;
    let mut direct = TcpStream::connect((Ipv4Addr::LOCALHOST, app_port))
        .await
        .unwrap();
    direct
        .write_all(b"GET / HTTP/1.1\r\nHost: ingress.invalid\r\n\r\n")
        .await
        .unwrap();
    assert!(headers(&mut direct).await.starts_with("HTTP/1.1 403"));

    let mut proxy = Proxy::bind(TOKEN.into(), None, Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    let mut stream = request(port, app_port,
        "Connection: close, x-remove-me\r\nX-Remove-Me: hidden\r\nOrigin: https://preview.example\r\nAuthorization: Bearer application\r\nCookie: __session=secret; app=visible; __Host-warp_preview_ingress=secret\r\nX-Nsc-Other: secret\r\nX-Warp-Preview-Authorization: secret\r\nX-Forwarded-Host: forged\r\n").await;
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    for expected in [
        "/path?q=preserved",
        "https://preview.example",
        "Bearer application",
        "app=visible",
        "\"x-forwarded-host\": \"preview.example\"",
        "\"x-forwarded-proto\": \"https\"",
        "\"x-forwarded-port\": \"443\"",
    ] {
        assert!(response.contains(expected), "{expected}: {response}");
    }
    for absent in [
        TOKEN,
        "secret",
        "x-remove-me",
        "forged",
        "x-nsc-",
        "x-warp-preview-",
    ] {
        assert!(!response.contains(absent), "{absent}: {response}");
    }
    proxy.shutdown().await;
    app_task.abort();
}

#[tokio::test]
async fn rejects_unauthenticated_and_arbitrary_destination_requests() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let app_port = 12345;
    let mut proxy = Proxy::bind(TOKEN.into(), None, Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    for (target, token, expected) in [
        ("GET /", "", "403"),
        ("GET /", "wrong-token", "403"),
        ("CONNECT other.example:443", TOKEN, "400"),
        ("GET http://other.example/", TOKEN, "400"),
    ] {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "{target} HTTP/1.1\r\nHost: ingress.invalid\r\n{TOKEN_HEADER}: {token}\r\nX-Warp-Preview-Port: 12345\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert!(
            headers(&mut stream)
                .await
                .starts_with(&format!("HTTP/1.1 {expected}"))
        );
    }
    proxy.shutdown().await;
}

#[tokio::test]
async fn routes_eight_apps_through_one_listener_using_each_requests_port() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let source = Ports::new(vec![]);
    let mut proxy = Proxy::bind(TOKEN.into(), None, source.clone())
        .await
        .unwrap();
    let port = proxy.port();
    let mut apps = Vec::new();
    let mut requests = JoinSet::new();
    for _ in 0..8 {
        let (app_port, task) = app().await;
        apps.push(task);
        source.values.lock().unwrap().push(app_port);
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
    let _port_guard = PROXY_PORT.lock().await;
    let (first, first_task) = app().await;
    let (second, second_task) = app().await;
    let source = Ports::new(vec![first]);
    let mut proxy = Proxy::bind(TOKEN.into(), None, source.clone())
        .await
        .unwrap();

    let mut stream = request(proxy.port(), first, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);

    *source.values.lock().unwrap() = vec![second];
    source.fail.store(true, Ordering::SeqCst);
    let mut stream = request(proxy.port(), second, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 403"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    let mut stream = request(proxy.port(), first, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);

    source.fail.store(false, Ordering::SeqCst);
    source.pause.store(true, Ordering::SeqCst);
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
    let mut stream = request(proxy.port(), first, "Connection: close\r\n").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 403"));
    assert_eq!(source.calls.load(Ordering::SeqCst), 4);

    proxy.shutdown().await;
    first_task.abort();
    second_task.abort();
}

#[tokio::test]
async fn rejects_missing_invalid_and_control_ports() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let source = Ports::new(vec![1023, 9277, 49152]);
    let mut proxy = Proxy::bind(TOKEN.into(), None, source.clone())
        .await
        .unwrap();
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
            .write_all(format!("GET / HTTP/1.1\r\nHost: ingress.invalid\r\n{TOKEN_HEADER}: {TOKEN}\r\n{port_header}\r\n").as_bytes())
            .await
            .unwrap();
        assert!(
            headers(&mut stream).await.starts_with("HTTP/1.1 403"),
            "{destination}"
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    proxy.shutdown().await;
}

#[tokio::test]
async fn streams_first_chunk_before_completion_and_closes_at_sandbox_deadline() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
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
    let mut proxy = Proxy::bind(
        TOKEN.into(),
        Some(Instant::now() + Duration::from_millis(500)),
        Ports::new(vec![app_port]),
    )
    .await
    .unwrap();
    let port = proxy.port();
    let mut stream = request(port, app_port, "").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    let mut chunk = [0; 11];
    timeout(Duration::from_secs(2), stream.read_exact(&mut chunk))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&chunk, b"6\r\ndata\n\n\r\n");
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
    proxy.shutdown().await;
    task.await.unwrap();
}

#[tokio::test]
async fn copies_upgraded_frames_and_closes_on_driver_shutdown() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(
            headers(&mut stream)
                .await
                .contains("sec-websocket-protocol: vite-hmr")
        );
        stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Protocol: vite-hmr\r\n\r\n").await.unwrap();
        let mut frame = [0; 8];
        stream.read_exact(&mut frame).await.unwrap();
        assert_eq!(frame, [0x81, 0x82, 1, 2, 3, 4, b'h' ^ 1, b'i' ^ 2]);
        stream.write_all(&[0x81, 2, b'h', b'i']).await.unwrap();
        assert_eq!(stream.read(&mut frame).await.unwrap(), 0);
    });
    let mut proxy = Proxy::bind(TOKEN.into(), None, Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    let mut stream = request(
        port,
        app_port,
        "Connection: upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Protocol: vite-hmr\r\n",
    )
    .await;
    let response = headers(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response}");
    assert!(response.contains("sec-websocket-protocol: vite-hmr"));
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
    let _port_guard = PROXY_PORT.lock().await;
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
    let mut proxy = Proxy::bind(TOKEN.into(), None, Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    let mut stream = request(port, app_port, "").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    proxy.shutdown().await;
    task.await.unwrap();
}
