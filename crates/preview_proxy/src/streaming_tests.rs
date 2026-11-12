use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

#[tokio::test]
async fn downstream_disconnect_closes_upstream_stream() {
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
    let mut proxy = Proxy::bind(0, tests::Ports::new(vec![app_port]))
        .await
        .unwrap();
    let mut downstream = tests::request(proxy.port(), app_port, "").await;
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
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = tests::headers(&mut stream).await;
        assert!(request.contains("te: trailers\r\n"), "{request}");
        assert!(request.contains("connection: te\r\n"), "{request}");
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: x-checksum\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n\r\n4\r\ndata\r\n0\r\nx-checksum: value\r\n\r\n").await.unwrap();
    });
    let mut proxy = Proxy::bind(0, tests::Ports::new(vec![app_port]))
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
async fn forwards_early_response_and_serves_the_next_request() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let app_port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        tests::headers(&mut stream).await;
        let mut partial_body = [0];
        timeout(Duration::from_secs(3), stream.read_exact(&mut partial_body))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&partial_body, b"x");
        stream
            .write_all(
                b"HTTP/1.1 413 Content Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        drop(stream);
        let (mut next, _) = listener.accept().await.unwrap();
        tests::headers(&mut next).await;
        next.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let mut proxy = Proxy::bind(0, tests::Ports::new(vec![app_port]))
        .await
        .unwrap();
    let port = proxy.port();
    let mut stream = tests::request(port, app_port, "Content-Length: 10485760\r\n").await;
    stream.write_all(b"x").await.unwrap();
    assert!(
        tests::headers(&mut stream)
            .await
            .starts_with("HTTP/1.1 413")
    );
    drop(stream);
    let mut next = tests::request(port, app_port, "Connection: close\r\n").await;
    assert!(tests::headers(&mut next).await.starts_with("HTTP/1.1 200"));
    timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    proxy.shutdown().await;
}
