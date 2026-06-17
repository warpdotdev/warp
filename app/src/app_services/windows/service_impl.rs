use std::sync::Arc;

use async_channel::Sender;
use async_trait::async_trait;
use ipc::{Client, ConnectionAddress};
use url::Url;
use warpui::r#async::executor::Background;

use super::single_instance_manager::uri_named_pipe_name;

/// IPC Service to respond to URIs sent to the active Warp instance.
pub(super) struct UriService {}

impl ipc::Service for UriService {
    type Request = Vec<Url>;
    type Response = ();
}

#[derive(Clone)]
pub(super) struct UriServiceImpl {
    tx: Sender<Vec<Url>>,
}

impl UriServiceImpl {
    pub(super) fn new(tx: Sender<Vec<Url>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl ipc::ServiceImpl for UriServiceImpl {
    type Service = UriService;

    async fn handle_request(&self, request: Vec<Url>) -> () {
        if let Err(send_error) = self.tx.send(request).await {
            log::error!("Error sending urls to local stream: {send_error:#}");
        }
    }
}

/// Forwards the given URLs to the main running instance of Warp.
pub(super) async fn forward_uri_to_sole_running_instance(
    urls: Vec<Url>,
) -> Result<(), ipc::ClientError> {
    // We need to construct a new background executor because this function is
    // run before we have a `AppContext`.  We explicitly create it with
    // a single backing thread, as we don't need an entire pool of threads.
    let background_executor = Arc::new(Background::new(1, |_| "forward-uris".to_owned()));
    let client = Client::connect(
        ConnectionAddress::from(uri_named_pipe_name()),
        background_executor,
    )
    .await?;
    let uri_service_caller = ipc::service_caller::<UriService>(Arc::new(client));
    let _ = uri_service_caller.call(urls).await?;
    Ok(())
}
