//! This module provides a fake, "placeholder" implementation of IPC transport for wasm targets.
//!
//! Eventually, this module will implement transport on top of the WebWorkers MessagePort API.
use futures::{AsyncRead, AsyncWrite};

use crate::ConnectionAddress;

pub(crate) mod client {
    use super::*;
    use crate::client::{ClientError, InitializationError, Result};

    pub async fn connect_client(
        _connection_address: ConnectionAddress,
    ) -> Result<(futures::io::Empty, futures::io::Sink)> {
        Err(ClientError::Initialization(
            InitializationError::UnsupportedPlatform,
        ))
    }
}

pub(crate) mod server {
    use super::*;
    use crate::server::{InitializationError, Result, ServerError};

    /// "Fake" implementation. Note that because a `ConnectionListenerImpl` can't be instantiated,
    /// a `ConnectionImpl` cannot actually be instantiated either.
    pub struct ConnectionImpl {
        /// A dummy placeholder field to prevent instantiation of a Connection because this crate
        /// currently doesn't support wasm.
        _marker: bool,
    }

    impl ConnectionImpl {
        pub fn into_split(self) -> (impl AsyncRead + Unpin, impl AsyncWrite + Unpin) {
            (futures::io::empty(), futures::io::sink())
        }
    }

    /// "Fake" implementation that cannot actually be initialized.
    pub struct ConnectionListenerImpl {
        /// A dummy placeholder field to prevent instantiation of a ConnectionListener because this crate
        /// currently doesn't support wasm.
        _marker: bool,
    }

    impl ConnectionListenerImpl {
        /// Returns an unsupported platform error, since this crate currently doesn't support wasm.
        pub fn new(_connection_address: ConnectionAddress) -> Result<Self> {
            Err(ServerError::Initialization(
                InitializationError::UnsupportedPlatform,
            ))
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            // This can never be called because its impossible to instantiate a ConnectionListener (on
            // wasm).
            unreachable!("ConnectionListener cannot be instantiated when targeting wasm.")
        }
    }
}
