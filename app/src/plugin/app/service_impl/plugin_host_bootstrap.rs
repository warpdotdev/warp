//! The implementation of `PluginHostBootstrapService` to be served by the app process to the plugin
//! host process.
use async_channel::{Receiver, Sender};
use async_trait::async_trait;

use crate::plugin::service::{
    PluginHostBootstrapRequest, PluginHostBootstrapResponse, PluginHostBootstrapService,
};

#[derive(Clone)]
pub struct PluginHostBootstrapServiceImpl {
    connection_address_tx: Sender<ipc::ConnectionAddress>,
    connection_address_rx: Receiver<ipc::ConnectionAddress>,
}

impl PluginHostBootstrapServiceImpl {
    pub fn new() -> Self {
        let (connection_key_tx, connection_key_rx) = async_channel::bounded(1);
        Self {
            connection_address_tx: connection_key_tx,
            connection_address_rx: connection_key_rx,
        }
    }

    /// Returns a receiver that emits the connection address received in an incoming request.
    pub fn connection_address_rx(&self) -> Receiver<ipc::ConnectionAddress> {
        self.connection_address_rx.clone()
    }
}

#[async_trait]
impl ipc::ServiceImpl for PluginHostBootstrapServiceImpl {
    type Service = PluginHostBootstrapService;

    async fn handle_request(
        &self,
        request: PluginHostBootstrapRequest,
    ) -> Result<PluginHostBootstrapResponse, String> {
        Ok(
            match self
                .connection_address_tx
                .send(request.connection_address)
                .await
            {
                Ok(_) => PluginHostBootstrapResponse { success: true },
                Err(_) => PluginHostBootstrapResponse { success: false },
            },
        )
    }
}
