use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

struct Control {
    calls: Arc<AtomicUsize>,
    ports: mpsc::UnboundedSender<u16>,
}

impl ControlPlane for Control {
    async fn start(&self, request: &StartRequest) -> Result<Session, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.ports.send(request.proxy_port).unwrap();
        Ok(Session {
            session_id: "session".into(),
        })
    }
}

#[tokio::test(start_paused = true)]
async fn registers_once_and_keeps_listener_until_driver_shutdown() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (ports, mut receiver) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let task = tokio::spawn(run(
        Control {
            calls: calls.clone(),
            ports,
        },
        "boot".into(),
        crate::tests::TOKEN.into(),
        None,
        cancel.clone(),
    ));
    let port = receiver.recv().await.unwrap();
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    cancel.cancel();
    task.await.unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

struct MovingControl {
    registrations: mpsc::UnboundedSender<(u16, Option<u16>, oneshot::Sender<()>)>,
}

impl ControlPlane for MovingControl {
    async fn start(&self, request: &StartRequest) -> Result<Session, Error> {
        let (acknowledge, acknowledged) = oneshot::channel();
        self.registrations
            .send((request.proxy_port, request.replaces_proxy_port, acknowledge))
            .unwrap();
        acknowledged.await.unwrap();
        Ok(Session {
            session_id: "session".into(),
        })
    }
}

#[tokio::test]
async fn relocation_waits_for_registration_then_releases_only_the_old_listener() {
    let (registrations, mut receiver) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let task = tokio::spawn(run(
        MovingControl { registrations },
        "boot".into(),
        crate::tests::TOKEN.into(),
        None,
        cancel.clone(),
    ));
    let (old_port, predecessor, acknowledge) = receiver.recv().await.unwrap();
    assert_eq!(predecessor, None);
    acknowledge.send(()).unwrap();

    for (token, expected) in [("wrong-token", "403"), (crate::tests::TOKEN, "202")] {
        let mut stream = TcpStream::connect(("127.0.0.1", old_port)).await.unwrap();
        stream.write_all(format!(
            "POST {} HTTP/1.1\r\nHost: ingress.invalid\r\n{}: {token}\r\n{}: relocate\r\nContent-Length: 0\r\n\r\n",
            crate::RELOCATE_PATH, crate::TOKEN_HEADER, crate::ACTION_HEADER,
        ).as_bytes()).await.unwrap();
        assert!(
            crate::tests::headers(&mut stream)
                .await
                .starts_with(&format!("HTTP/1.1 {expected}"))
        );
        if token == "wrong-token" {
            assert!(receiver.try_recv().is_err());
        }
    }
    let (new_port, predecessor, acknowledge) = receiver.recv().await.unwrap();
    assert_ne!(new_port, old_port);
    assert_eq!(predecessor, Some(old_port));
    assert!(TcpStream::connect(("127.0.0.1", old_port)).await.is_ok());
    assert!(TcpStream::connect(("127.0.0.1", new_port)).await.is_ok());
    acknowledge.send(()).unwrap();
    let app = timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", old_port)).await {
                break listener;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let serve_app = tokio::spawn(async move {
        let (mut stream, _) = app.accept().await.unwrap();
        let headers = crate::tests::headers(&mut stream).await;
        assert!(headers.contains(&format!("host: localhost:{old_port}")));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let mut stream = crate::tests::request(new_port, old_port, "Connection: close\r\n").await;
    assert!(
        crate::tests::headers(&mut stream)
            .await
            .starts_with("HTTP/1.1 200")
    );
    serve_app.await.unwrap();
    cancel.cancel();
    task.await.unwrap();
}
