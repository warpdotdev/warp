//! This module implements IPC transport for native (non-wasm) platforms.
//!
//! On Windows, the transport is built directly on `tokio`'s native named-pipe support
//! (`tokio::net::windows::named_pipe`), since fixing REV-1546 requires setting a security
//! descriptor at pipe *creation* time, which the `interprocess` crate does not expose (see
//! `crate::windows_pipe_security` for the full rationale). On all other platforms, this uses the
//! `interprocess` crate, which uses Unix Domain Sockets under the hood.
use crate::ConnectionAddress;

/// Builds the full Windows named-pipe path (`\\.\pipe\<name>`) for the given local socket name.
/// Kept unconditional (rather than `#[cfg(windows)]`-gated) so its construction logic can be unit
/// tested on every platform; it is only ever used on Windows.
#[allow(dead_code)]
pub(crate) fn windows_named_pipe_path(name: &str) -> String {
    format!(r"\\.\pipe\{name}")
}

#[cfg(not(windows))]
pub(crate) mod client {
    use async_compat::CompatExt as _;
    use futures::{AsyncRead, AsyncWrite};
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

#[cfg(windows)]
pub(crate) mod client {
    use async_compat::{Compat, CompatExt as _};
    use futures::{AsyncRead, AsyncWrite};
    use tokio::io::split;
    use tokio::net::windows::named_pipe::ClientOptions;

    use super::*;
    use crate::client::{ClientError, InitializationError, Result};

    /// Returns a tuple containing structs for reading and writing to a local socket, which is the
    /// underlying IPC transport for native (non-wasm) platforms.
    pub async fn connect_client(
        connection_address: ConnectionAddress,
    ) -> Result<(impl AsyncRead + Unpin, impl AsyncWrite + Unpin)> {
        let pipe_path = windows_named_pipe_path(&connection_address.to_string());
        // `ClientOptions::open` requires an active Tokio runtime (it panics/errors otherwise);
        // `.compat()` gives it one, matching the pattern used for the server side below.
        let client = async move { ClientOptions::new().open(&pipe_path) }
            .compat()
            .await
            .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))?;
        let (reader, writer) = split(client);
        Ok((Compat::new(reader), Compat::new(writer)))
    }
}

#[cfg(not(windows))]
pub(crate) mod server {
    use async_compat::CompatExt as _;
    use futures::{AsyncRead, AsyncWrite};
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

#[cfg(windows)]
pub(crate) mod server {
    use async_compat::{Compat, CompatExt as _};
    use futures::{AsyncRead, AsyncWrite};
    use tokio::io::split;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    use super::*;
    use crate::next_instance::NextInstance;
    use crate::server::{InitializationError, Result, ServerError};
    use crate::windows_pipe_security::PipeSecurityAttributes;

    pub struct ConnectionImpl {
        server: NamedPipeServer,
    }

    impl ConnectionImpl {
        pub fn into_split(self) -> (impl AsyncRead + Unpin, impl AsyncWrite + Unpin) {
            let (reader, writer) = split(self.server);
            (Compat::new(reader), Compat::new(writer))
        }
    }

    pub struct ConnectionListenerImpl {
        pipe_path: String,
        // Kept alive for the listener's lifetime and reused for every instance created (the
        // first and all subsequent ones), per the documented pattern for keeping a named pipe's
        // DACL consistent across instances -- see `crate::windows_pipe_security`.
        security_attributes: PipeSecurityAttributes,
        // The next server instance to hand out on `accept_connection`, following tokio's
        // documented named-pipe server pattern: a fresh instance must exist before the previous
        // one is dropped, or a client connecting in between can transiently see `NotFound`.
        //
        // `accept_connection` is only ever awaited serially by the single task spawned in
        // `Server::listen_for_new_connections`, so this is never accessed concurrently; it still
        // recovers gracefully (rather than panicking) if a previous call left it empty after a
        // recoverable error -- see `NextInstance`.
        next: NextInstance<NamedPipeServer>,
    }

    impl ConnectionListenerImpl {
        pub fn new(connection_address: ConnectionAddress) -> Result<Self> {
            let pipe_path = windows_named_pipe_path(&connection_address.to_string());

            // This is the fix for REV-1546: create the pipe with an explicit security descriptor
            // scoped to the current user (across elevation levels), SYSTEM, and Administrators,
            // instead of Windows' default DACL. See `crate::windows_pipe_security` for the full
            // rationale, including why this must happen at creation time rather than afterwards.
            let security_attributes = PipeSecurityAttributes::for_current_user().map_err(|e| {
                ServerError::Initialization(InitializationError::Io(std::io::Error::other(
                    format!("Failed to build named pipe security attributes: {e:?}"),
                )))
            })?;

            // `create_instance` constructs a `NamedPipeServer`, which (like the rest of tokio's
            // I/O types) requires an active Tokio runtime; `.compat()` provides one, matching the
            // pattern used by the non-Windows `interprocess`-based implementation above.
            let first = warpui_core::r#async::block_on(
                async { create_instance(&pipe_path, &security_attributes) }.compat(),
            )
            .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;

            Ok(Self {
                pipe_path,
                security_attributes,
                next: NextInstance::new(first),
            })
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            async {
                // Recreates an instance here if a previous call failed to prepare a replacement
                // below and left the slot empty, rather than assuming one is always present.
                let server = self
                    .next
                    .take_or_create(|| create_instance(&self.pipe_path, &self.security_attributes))
                    .map_err(ServerError::AcceptConnection)?;

                let connect_result = server.connect().await;

                // Prepare the next instance before returning, regardless of whether accepting
                // this one succeeded, so a transient failure here doesn't leave the listener
                // unable to accept again; see the `next` field's doc comment. If this fails too,
                // leave the slot empty -- `take_or_create` will retry on the next call.
                match create_instance(&self.pipe_path, &self.security_attributes) {
                    Ok(next) => self.next.restore(Some(next)),
                    Err(e) => {
                        log::warn!(
                            "Failed to pre-create the next named pipe instance; will retry on \
                             the next accept: {e:?}"
                        );
                        self.next.restore(None);
                    }
                }

                connect_result.map_err(ServerError::AcceptConnection)?;

                Ok(ConnectionImpl { server })
            }
            .compat()
            .await
        }
    }

    fn create_instance(
        pipe_path: &str,
        security_attributes: &PipeSecurityAttributes,
    ) -> std::io::Result<NamedPipeServer> {
        // Safety: `security_attributes.as_ptr()` points at a live, fully-initialized
        // `SECURITY_ATTRIBUTES` whose `lpSecurityDescriptor` remains valid for at least as long
        // as `security_attributes` (owned by `ConnectionListenerImpl` for its whole lifetime).
        // The kernel copies the descriptor's contents during `CreateNamedPipeW`, so nothing
        // borrows past this call.
        unsafe {
            ServerOptions::new()
                .create_with_security_attributes_raw(pipe_path, security_attributes.as_ptr())
        }
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
