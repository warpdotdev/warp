use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

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
