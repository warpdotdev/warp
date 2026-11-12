use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::Service;

use super::*;

#[tokio::test]
async fn reuses_connections_per_app_and_closes_idle_connections_on_shutdown() {
    let mut ports = Vec::new();
    let mut apps = JoinSet::new();
    for _ in 0..2 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let app_port = listener.local_addr().unwrap().port();
        ports.push(app_port);
        apps.spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            for _ in 0..2 {
                let request = tests::headers(&mut stream).await;
                assert_eq!(
                    request.lines().next(),
                    Some("GET /path?q=preserved HTTP/1.1")
                );
                assert!(request.contains(&format!("host: localhost:{app_port}\r\n")));
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .await
                    .unwrap();
            }
            assert_eq!(
                timeout(Duration::from_secs(3), stream.read(&mut [0]))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        });
    }
    let mut proxy = Proxy::bind(0, tests::Ports::new(ports.clone()))
        .await
        .unwrap();
    for app_port in ports.iter().chain(&ports) {
        let mut downstream = tests::request(proxy.port(), *app_port, "Connection: close\r\n").await;
        let mut response = String::new();
        timeout(
            Duration::from_secs(3),
            downstream.read_to_string(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("ok"), "{response}");
    }
    proxy.shutdown().await;
    while let Some(result) = timeout(Duration::from_secs(3), apps.join_next())
        .await
        .unwrap()
    {
        result.unwrap();
    }
}

#[tokio::test]
async fn rejects_non_loopback_upstream_destinations() {
    let mut connector = upstream::LoopbackConnector;
    for uri in [
        "http://example.com:3000/",
        "https://localhost:3000/",
        "http://localhost/",
    ] {
        assert!(connector.call(uri.parse().unwrap()).await.is_err(), "{uri}");
    }
}
