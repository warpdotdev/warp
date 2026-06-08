//! This module implements IPC transport on top of the `interprocess` crate, which uses Unix Domain
//! Sockets on Unix platforms and named pipes on Windows under the hood.
use async_compat::CompatExt as _;
use futures::{AsyncRead, AsyncWrite};

use crate::ConnectionAddress;

pub(crate) mod client {
    use interprocess::local_socket::tokio::LocalSocketStream;

    use super::*;
    use crate::client::{ClientError, InitializationError, Result};

    /// Returns a tuple containing structs for reading and writing to a local socket, which is the
    /// underlying IPC transport for native (non-wasm) platforms.
    pub async fn connect_client(
        connection_address: ConnectionAddress,
    ) -> Result<(impl AsyncRead + Unpin, impl AsyncWrite + Unpin)> {
        let stream = LocalSocketStream::connect(connection_address.0.as_str())
            .compat()
            .await
            .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))?;
        Ok(stream.into_split())
    }
}

pub(crate) mod server {
    use interprocess::local_socket::tokio::{LocalSocketListener, LocalSocketStream};

    use super::*;
    use crate::server::{InitializationError, Result, ServerError};

    pub struct ConnectionImpl {
        stream: LocalSocketStream,
    }

    impl ConnectionImpl {
        pub fn into_split(self) -> (impl AsyncRead + Unpin, impl AsyncWrite + Unpin) {
            self.stream.into_split()
        }
    }

    pub struct ConnectionListenerImpl {
        listener: LocalSocketListener,
    }

    impl ConnectionListenerImpl {
        pub fn new(connection_address: ConnectionAddress) -> Result<Self> {
            let listener = warpui_core::r#async::block_on(
                async move { LocalSocketListener::bind(connection_address.to_string()) }.compat(),
            )
            .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;
            Ok(Self { listener })
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            self.listener
                .accept()
                .compat()
                .await
                .map(|stream| ConnectionImpl { stream })
                .map_err(ServerError::AcceptConnection)
        }
    }
}
