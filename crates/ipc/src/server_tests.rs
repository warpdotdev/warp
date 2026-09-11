use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use warpui_core::r#async::block_on;
use warpui_core::r#async::executor::Background;

use crate::client::ClientError;
use crate::protocol::ProtocolError;
use crate::{
    Client, ConnectionAddress, Server, ServerBuilder, Service, ServiceImpl, service_caller,
};

/// A service whose response type carries no payload, matching the single-instance URI service.
/// Refusal has to be expressed without one, which is the property these tests pin down.
struct UnitService {}

impl Service for UnitService {
    type Request = u32;
    type Response = ();
}

#[derive(Clone)]
struct UnitServiceImpl {
    refusal: Option<String>,
}

#[async_trait]
impl ServiceImpl for UnitServiceImpl {
    type Service = UnitService;

    async fn handle_request(&self, _request: u32) -> Result<(), String> {
        match &self.refusal {
            Some(refusal) => Err(refusal.clone()),
            None => Ok(()),
        }
    }
}

fn unique_address() -> ConnectionAddress {
    static NEXT_ID: AtomicU32 = AtomicU32::new(0);
    ConnectionAddress::from(format!(
        "warp-ipc-test-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn serve(refusal: Option<String>) -> (Server, ConnectionAddress, Arc<Background>) {
    let executor = Arc::new(Background::new(1, |_| "ipc-test-server".to_owned()));
    let (server, address) = ServerBuilder::default()
        .with_fixed_address(unique_address().to_string())
        .with_service(UnitServiceImpl { refusal })
        .build_and_run(executor.clone())
        .expect("server should start");
    (server, address, executor)
}

#[test]
fn a_handled_request_returns_the_service_response() {
    let (_server, address, executor) = serve(None);

    let response = block_on(async {
        let client = Client::connect(address, executor.clone())
            .await
            .expect("client should connect");
        service_caller::<UnitService>(Arc::new(client))
            .call(7)
            .await
    });

    assert!(matches!(response, Ok(())));
}

/// A refusal must reach the caller as a protocol-level error, decided before the service response
/// is deserialized. That is what lets a service refuse without spending a payload on it, and what
/// keeps a caller built against a different version of the service from misreading one.
#[test]
fn a_refused_request_returns_an_error_without_deserializing_a_response() {
    let (_server, address, executor) = serve(Some("queue is full".to_owned()));

    let response = block_on(async {
        let client = Client::connect(address, executor.clone())
            .await
            .expect("client should connect");
        service_caller::<UnitService>(Arc::new(client))
            .call(7)
            .await
    });

    match response {
        Err(ClientError::InternalProtocol(ProtocolError::Other(reason))) => {
            assert_eq!(reason, "queue is full");
        }
        other => panic!("expected a protocol-level refusal, got {other:?}"),
    }
}
