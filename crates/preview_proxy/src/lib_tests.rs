use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) const TOKEN: &str = "a-valid-route-token-with-at-least-32-bytes";

pub(super) fn route(id: usize, app_port: u16) -> Route {
    Route {
        id: id.to_string(),
        generation: 1,
        app_port,
        token: format!("{TOKEN}-{id}"),
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

pub(super) async fn request(port: u16, id: usize, extra: &str) -> TcpStream {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    stream.write_all(format!(
        "GET /path?q=preserved HTTP/1.1\r\nHost: ingress.invalid\r\n{TOKEN_HEADER}: {TOKEN}-{id}\r\nX-Warp-Preview-Public-Host: preview.example\r\nX-Warp-Preview-Public-Proto: https\r\nX-Warp-Preview-Public-Port: 443\r\n{extra}\r\n"
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
    let (app_port, app_task) = app().await;
    let mut direct = TcpStream::connect((Ipv4Addr::LOCALHOST, app_port))
        .await
        .unwrap();
    direct
        .write_all(b"GET / HTTP/1.1\r\nHost: ingress.invalid\r\n\r\n")
        .await
        .unwrap();
    assert!(headers(&mut direct).await.starts_with("HTTP/1.1 403"));

    let mut proxy = Proxy::default();
    let port = proxy
        .register(route(1, app_port), Instant::now() + Duration::from_secs(30))
        .await
        .unwrap();
    let mut stream = request(port, 1,
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
    let mut proxy = Proxy::default();
    let port = proxy
        .register(route(1, 12345), Instant::now() + Duration::from_secs(30))
        .await
        .unwrap();
    for (target, token, expected) in [
        ("GET /", "", "403"),
        ("GET /", "wrong-token", "403"),
        ("CONNECT other.example:443", &format!("{TOKEN}-1"), "400"),
        ("GET http://other.example/", &format!("{TOKEN}-1"), "400"),
    ] {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "{target} HTTP/1.1\r\nHost: ingress.invalid\r\n{TOKEN_HEADER}: {token}\r\n\r\n"
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
async fn retains_eight_independent_ports_and_revokes_only_removed_routes() {
    let mut proxy = Proxy::default();
    let mut ports = Vec::new();
    let mut apps = Vec::new();
    for id in 0..8 {
        let (app_port, task) = app().await;
        let port = proxy
            .register(
                route(id, app_port),
                Instant::now() + Duration::from_secs(30),
            )
            .await
            .unwrap();
        assert!(!ports.contains(&port));
        ports.push(port);
        apps.push((app_port, task));
    }
    let mut requests = JoinSet::new();
    for (id, port) in ports.iter().copied().enumerate() {
        requests.spawn(async move {
            let mut stream = request(port, id, "Connection: close\r\n").await;
            assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
        });
    }
    while let Some(result) = requests.join_next().await {
        result.unwrap();
    }
    let mut wrong_route = request(ports[1], 0, "").await;
    assert!(headers(&mut wrong_route).await.starts_with("HTTP/1.1 403"));
    proxy.retain(&(1..8).map(|id| id.to_string()).collect::<Vec<_>>());
    for id in 1..8 {
        assert_eq!(
            proxy
                .register(
                    route(id, apps[id].0),
                    Instant::now() + Duration::from_secs(30)
                )
                .await
                .unwrap(),
            ports[id]
        );
        let mut stream = request(ports[id], id, "").await;
        assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    }
    proxy.shutdown().await;
    assert!(
        TcpStream::connect((Ipv4Addr::LOCALHOST, ports[0]))
            .await
            .is_err()
    );
    for (_, task) in apps {
        task.abort();
    }
}

#[tokio::test]
async fn streams_first_chunk_before_completion_and_closes_on_lease_expiry() {
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
    let mut proxy = Proxy::default();
    let port = proxy
        .register(
            route(1, app_port),
            Instant::now() + Duration::from_millis(500),
        )
        .await
        .unwrap();
    let mut stream = request(port, 1, "").await;
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
    assert!(
        proxy
            .register(route(1, app_port), Instant::now() + Duration::from_secs(30))
            .await
            .is_err()
    );
    proxy.shutdown().await;
    task.await.unwrap();
}

#[tokio::test]
async fn copies_upgraded_frames_and_holds_capacity_until_revocation() {
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
    let mut proxy = Proxy {
        capacity: Arc::new(Semaphore::new(1)),
        ..Proxy::default()
    };
    let port = proxy
        .register(route(1, app_port), Instant::now() + Duration::from_secs(30))
        .await
        .unwrap();
    let mut stream = request(
        port,
        1,
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
    assert_eq!(proxy.capacity.available_permits(), 0);
    let mut rejected = request(port, 1, "").await;
    assert!(headers(&mut rejected).await.starts_with("HTTP/1.1 503"));
    proxy.retain(&[]);
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
    let mut proxy = Proxy::default();
    let port = proxy
        .register(route(1, app_port), Instant::now() + Duration::from_secs(30))
        .await
        .unwrap();
    let mut stream = request(port, 1, "").await;
    assert!(headers(&mut stream).await.starts_with("HTTP/1.1 200"));
    proxy.shutdown().await;
    task.await.unwrap();
}
