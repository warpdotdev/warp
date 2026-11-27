use std::sync::Arc;

use async_channel::Sender;
use async_trait::async_trait;
use ipc::{Client, ConnectionAddress};
use warp_errors::report_error;
use warpui::r#async::executor::Background;

use super::single_instance_manager::uri_named_pipe_name;
use crate::app_services::StartupForwardRequest;

/// IPC Service to respond to startup requests sent to the active Warp instance.
pub(super) struct UriService {}

impl ipc::Service for UriService {
    type Request = StartupForwardRequest;
    type Response = ();
}

#[derive(Clone)]
pub(super) struct UriServiceImpl {
    tx: Sender<StartupForwardRequest>,
}

impl UriServiceImpl {
    pub(super) fn new(tx: Sender<StartupForwardRequest>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl ipc::ServiceImpl for UriServiceImpl {
    type Service = UriService;

    async fn handle_request(&self, request: StartupForwardRequest) {
        if let Err(send_error) = self.tx.send(request).await {
            report_error!(
                anyhow::Error::new(send_error)
                    .context("Error sending startup request to local stream")
            );
        }
    }
}

/// Forwards the given startup request to the main running instance of Warp.
pub(super) async fn forward_request_to_sole_running_instance(
    request: StartupForwardRequest,
) -> Result<(), ipc::ClientError> {
    // We need to construct a new background executor because this function is
    // run before we have a `AppContext`.  We explicitly create it with
    // a single backing thread, as we don't need an entire pool of threads.
    let background_executor = Arc::new(Background::new(1, |_| "forward-startup".to_owned()));
    let client = Client::connect(
        ConnectionAddress::from(uri_named_pipe_name()),
        background_executor,
    )
    .await?;
    let uri_service_caller = ipc::service_caller::<UriService>(Arc::new(client));
    uri_service_caller.call(request).await?;
    Ok(())
}
