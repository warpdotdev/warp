use std::io::Read;
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn blocked_sentry_request_does_not_block_transport_shutdown_indefinitely() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_received_tx, request_received_rx) = mpsc::channel();
    let (release_server_tx, release_server_rx) = mpsc::channel();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        stream.read(&mut request).unwrap();
        request_received_tx.send(()).unwrap();
        release_server_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    });

    let options = sentry::ClientOptions {
        dsn: Some(format!("http://public@{address}/1").parse().unwrap()),
        ..Default::default()
    };
    let transport = bounded_sentry_http_transport(&options);
    transport.send_envelope(sentry::protocol::Event::default().into());
    request_received_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap();

    let started_at = Instant::now();
    drop(transport);
    assert!(
        started_at.elapsed() < Duration::from_secs(2),
        "Sentry transport shutdown exceeded its HTTP timeout"
    );

    release_server_tx.send(()).unwrap();
    server.join().unwrap();
}
