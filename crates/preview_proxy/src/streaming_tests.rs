use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

#[tokio::test]
async fn downstream_disconnect_closes_upstream_stream() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        tests::headers(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n")
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), stream.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
    let mut proxy = Proxy::bind(None, tests::Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    let mut downstream = tests::request(port, app_port, "").await;
    assert!(
        tests::headers(&mut downstream)
            .await
            .starts_with("HTTP/1.1 200")
    );
    drop(downstream);
    task.await.unwrap();
    proxy.shutdown().await;
}

#[tokio::test]
async fn preserves_response_trailers_and_repeated_headers() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        tests::headers(&mut stream).await;
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: x-checksum\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n\r\n4\r\ndata\r\n0\r\nx-checksum: value\r\n\r\n").await.unwrap();
    });
    let mut proxy = Proxy::bind(None, tests::Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let (mut sender, connection) = client::handshake(TokioIo::new(stream)).await.unwrap();
    let connection = tokio::spawn(connection);
    let request = Request::builder()
        .uri("/")
        .header(HOST, "ingress.invalid")
        .header("x-warp-preview-port", app_port.to_string())
        .header("x-warp-preview-public-host", "preview.example")
        .header("x-warp-preview-public-proto", "https")
        .header("x-warp-preview-public-port", "443")
        .header("te", "trailers")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.headers().get_all("set-cookie").iter().count(), 2);
    let collected = response.into_body().collect().await.unwrap();
    assert_eq!(collected.trailers().unwrap()["x-checksum"], "value");
    assert_eq!(collected.to_bytes(), "data");
    drop(sender);
    connection.await.unwrap().unwrap();
    task.await.unwrap();
    proxy.shutdown().await;
}

#[tokio::test]
async fn forwards_early_response_without_waiting_for_upload_completion() {
    let _port_guard = crate::tests::PROXY_PORT.lock().await;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        tests::headers(&mut stream).await;
        stream
            .write_all(
                b"HTTP/1.1 413 Content Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    });
    let mut proxy = Proxy::bind(None, tests::Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    let mut stream = tests::request(port, app_port, "Content-Length: 10485760\r\n").await;
    assert!(
        tests::headers(&mut stream)
            .await
            .starts_with("HTTP/1.1 413")
    );
    task.await.unwrap();
    proxy.shutdown().await;
}
