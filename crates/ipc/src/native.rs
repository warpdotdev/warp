//! This module implements IPC transport on top of the `interprocess` crate, which uses Unix Domain
//! Sockets on Unix platforms and named pipes on Windows under the hood.
use async_compat::CompatExt as _;
use futures::{AsyncRead, AsyncWrite};

use crate::ConnectionAddress;
#[cfg(windows)]
mod windows_pipe {
    use std::io;
    use std::sync::{Arc, mpsc};

    use warpui_core::r#async::executor::Background;

    use crate::ConnectionAddress;

    /// Client-side access requested when opening a Warp IPC named pipe.
    ///
    /// This is `FILE_GENERIC_READ | (FILE_GENERIC_WRITE - FILE_APPEND_DATA)`, expanded as a
    /// specific mask instead of using `GENERIC_WRITE`. On named pipes, `FILE_APPEND_DATA` aliases
    /// `FILE_CREATE_PIPE_INSTANCE`; requesting/granting it would allow a client to create an
    /// additional server instance for the same pipe name and race/capture traffic.
    pub(super) const NAMED_PIPE_CLIENT_ACCESS_MASK: u32 = 0x0012_019B;

    /// Returns the full `\\.\pipe\<name>` path for `connection_address`, matching the path
    /// `interprocess` derives internally for the same connection address.
    pub(super) fn pipe_path(connection_address: &ConnectionAddress) -> String {
        format!(r"\\.\pipe\{connection_address}")
    }

    pub(super) fn run_on_background<T>(
        background_executor: Arc<Background>,
        work: impl FnOnce() -> io::Result<T> + Send + 'static,
    ) -> io::Result<T>
    where
        T: Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        background_executor
            .spawn(async move {
                let _ = tx.send(work());
            })
            .detach();
        rx.recv().map_err(io::Error::other)?
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn client_access_mask_omits_pipe_instance_creation_bit() {
            const FILE_APPEND_DATA_AND_CREATE_PIPE_INSTANCE: u32 = 0x0000_0004;

            assert_eq!(
                NAMED_PIPE_CLIENT_ACCESS_MASK & FILE_APPEND_DATA_AND_CREATE_PIPE_INSTANCE,
                0
            );
            assert_ne!(NAMED_PIPE_CLIENT_ACCESS_MASK & 0x0000_0001, 0);
            assert_ne!(NAMED_PIPE_CLIENT_ACCESS_MASK & 0x0000_0002, 0);
        }
    }
}

pub(crate) mod client {
    use std::sync::Arc;

    #[cfg(not(windows))]
    use interprocess::local_socket::tokio::LocalSocketStream;
    use warpui_core::r#async::executor::Background;

    use super::*;
    use crate::client::{ClientError, InitializationError, Result};

    /// Returns a tuple containing structs for reading and writing to a local socket, which is the
    /// underlying IPC transport for native (non-wasm) platforms.
    pub async fn connect_client(
        connection_address: ConnectionAddress,
        background_executor: Arc<Background>,
    ) -> Result<(
        Box<dyn AsyncRead + Send + Unpin>,
        Box<dyn AsyncWrite + Send + Unpin>,
    )> {
        #[cfg(windows)]
        {
            windows_pipe_client::connect_client(connection_address, background_executor)
                .await
                .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))
        }

        #[cfg(not(windows))]
        drop(background_executor);
        #[cfg(not(windows))]
        let stream = LocalSocketStream::connect(connection_address.0.as_str())
            .compat()
            .await
            .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))?;
        #[cfg(not(windows))]
        {
            let (reader, writer) = stream.into_split();
            Ok((Box::new(reader), Box::new(writer)))
        }
    }

    #[cfg(windows)]
    mod windows_pipe_client {
        use std::io;
        use std::sync::Arc;

        use futures::{AsyncRead, AsyncWrite};
        use tokio::net::windows::named_pipe::NamedPipeClient;
        use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};
        use warpui_core::r#async::executor::Background;
        use windows::Win32::Foundation::ERROR_PIPE_BUSY;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
        };
        use windows::Win32::System::Pipes::WaitNamedPipeW;
        use windows::core::PCWSTR;

        use crate::ConnectionAddress;
        use crate::platform::windows_pipe::{
            NAMED_PIPE_CLIENT_ACCESS_MASK, pipe_path, run_on_background,
        };

        pub async fn connect_client(
            connection_address: ConnectionAddress,
            background_executor: Arc<Background>,
        ) -> io::Result<(
            Box<dyn AsyncRead + Send + Unpin>,
            Box<dyn AsyncWrite + Send + Unpin>,
        )> {
            let path = pipe_path(&connection_address);
            let client = run_on_background(background_executor, move || open_client(&path))?;
            let (reader, writer) = tokio::io::split(client);
            Ok((Box::new(reader.compat()), Box::new(writer.compat_write())))
        }

        fn open_client(path: &str) -> io::Result<NamedPipeClient> {
            let path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            loop {
                match open_client_once(&path_wide) {
                    Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {
                        wait_for_server(&path_wide)?;
                    }
                    result => return result,
                }
            }
        }

        fn open_client_once(path_wide: &[u16]) -> io::Result<NamedPipeClient> {
            let handle = unsafe {
                CreateFileW(
                    PCWSTR(path_wide.as_ptr()),
                    NAMED_PIPE_CLIENT_ACCESS_MASK,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(
                        FILE_FLAG_OVERLAPPED.0
                            | SECURITY_IDENTIFICATION.0
                            | SECURITY_SQOS_PRESENT.0,
                    ),
                    None,
                )
            }
            .map_err(io::Error::from)?;

            // SAFETY: `CreateFileW` returned a valid, owned, overlapped named-pipe handle. Tokio's
            // `NamedPipeClient` assumes ownership of it from here.
            unsafe { NamedPipeClient::from_raw_handle(handle.0 as _) }
        }

        fn wait_for_server(path_wide: &[u16]) -> io::Result<()> {
            unsafe { WaitNamedPipeW(PCWSTR(path_wide.as_ptr()), 0) }
                .ok()
                .map_err(io::Error::from)
        }
    }
}

pub(crate) mod server {
    use std::sync::Arc;

    use interprocess::local_socket::tokio::{LocalSocketListener, LocalSocketStream};
    use warpui_core::r#async::executor::Background;

    use super::*;
    use crate::server::{InitializationError, Result, ServerError};

    /// Server-side connection stream. On Windows, this may be backed by either the
    /// `interprocess`-managed transport or, when a security descriptor is requested (see
    /// [`ConnectionListenerImpl::new`]), by a named pipe created directly via
    /// [`windows_pipe`] since `interprocess` does not expose a way to customize named pipe
    /// security attributes.
    enum ConnectionStream {
        Standard(LocalSocketStream),
        #[cfg(windows)]
        WindowsPipe(windows_pipe::PipeStream),
    }

    pub struct ConnectionImpl {
        stream: ConnectionStream,
    }

    impl ConnectionImpl {
        pub fn into_split(
            self,
        ) -> (
            Box<dyn AsyncRead + Send + Unpin>,
            Box<dyn AsyncWrite + Send + Unpin>,
        ) {
            match self.stream {
                ConnectionStream::Standard(stream) => {
                    let (reader, writer) = stream.into_split();
                    (Box::new(reader), Box::new(writer))
                }
                #[cfg(windows)]
                ConnectionStream::WindowsPipe(stream) => {
                    let (reader, writer) = stream.into_split();
                    (Box::new(reader), Box::new(writer))
                }
            }
        }
    }

    enum ListenerImpl {
        Standard(LocalSocketListener),
        #[cfg(windows)]
        WindowsPipe(windows_pipe::PipeListener),
    }

    pub struct ConnectionListenerImpl {
        listener: ListenerImpl,
    }

    impl ConnectionListenerImpl {
        /// Creates a listener for `connection_address`.
        ///
        /// `windows_pipe_security_descriptor`, when set, requests that the underlying named pipe
        /// be created with the given SDDL security descriptor instead of the OS default. This is
        /// ignored outside of Windows, where local sockets are Unix Domain Sockets rather than
        /// named pipes and thus have no equivalent concept of a security descriptor.
        pub fn new(
            connection_address: ConnectionAddress,
            windows_pipe_security_descriptor: Option<&str>,
            background_executor: Arc<Background>,
        ) -> Result<Self> {
            #[cfg(windows)]
            if let Some(sddl) = windows_pipe_security_descriptor {
                let listener = windows_pipe::PipeListener::bind(
                    &connection_address,
                    sddl,
                    background_executor,
                )
                .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;
                return Ok(Self {
                    listener: ListenerImpl::WindowsPipe(listener),
                });
            }
            #[cfg(not(windows))]
            let _ = windows_pipe_security_descriptor;
            #[cfg(not(windows))]
            let _ = background_executor;

            let listener = warpui_core::r#async::block_on(
                async move { LocalSocketListener::bind(connection_address.to_string()) }.compat(),
            )
            .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;
            Ok(Self {
                listener: ListenerImpl::Standard(listener),
            })
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            match &self.listener {
                ListenerImpl::Standard(listener) => listener
                    .accept()
                    .compat()
                    .await
                    .map(|stream| ConnectionImpl {
                        stream: ConnectionStream::Standard(stream),
                    })
                    .map_err(ServerError::AcceptConnection),
                #[cfg(windows)]
                ListenerImpl::WindowsPipe(listener) => listener
                    .accept()
                    .await
                    .map(|stream| ConnectionImpl {
                        stream: ConnectionStream::WindowsPipe(stream),
                    })
                    .map_err(ServerError::AcceptConnection),
            }
        }
    }

    /// Windows named pipe transport that bypasses `interprocess`'s pipe creation so that an
    /// explicit security descriptor can be attached. `interprocess` (as of 1.2.1) always creates
    /// named pipes with `lpSecurityAttributes = NULL`, which grants the default security
    /// descriptor (full control to the creator, read-only to Everyone). That default is
    /// insufficient for servers that must accept connections from a client running at a
    /// different elevation level than the server (see the single-instance URI channel in
    /// `app_services::windows`, which is the sole caller of this path).
    #[cfg(windows)]
    mod windows_pipe {
        use std::ffi::c_void;
        use std::io;
        use std::sync::Arc;

        use tokio::net::windows::named_pipe::{self, NamedPipeServer};
        use tokio::sync::Mutex;
        use tokio_util::compat::{
            Compat, TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _,
        };
        use warpui_core::r#async::executor::Background;
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows::core::PCWSTR;

        use crate::ConnectionAddress;
        use crate::platform::windows_pipe::{pipe_path, run_on_background};

        /// RAII wrapper around a security descriptor allocated by
        /// `ConvertStringSecurityDescriptorToSecurityDescriptorW`, which the caller is
        /// responsible for freeing with `LocalFree`.
        struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

        // SAFETY: the security descriptor is only ever read (not mutated) after construction, so
        // it's safe to share a reference to it across threads/tasks.
        unsafe impl Send for OwnedSecurityDescriptor {}
        unsafe impl Sync for OwnedSecurityDescriptor {}

        impl Drop for OwnedSecurityDescriptor {
            fn drop(&mut self) {
                if !self.0.0.is_null() {
                    unsafe {
                        let _ = LocalFree(Some(HLOCAL(self.0.0)));
                    }
                }
            }
        }

        /// Parses `sddl` into a Windows security descriptor.
        fn parse_security_descriptor(sddl: &str) -> io::Result<OwnedSecurityDescriptor> {
            let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl_wide.as_ptr()),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )
            }
            .map_err(|e| io::Error::other(format!("Failed to parse SDDL {sddl:?}: {e:#}")))?;
            Ok(OwnedSecurityDescriptor(descriptor))
        }

        pub struct PipeStream(NamedPipeServer);

        impl PipeStream {
            pub fn into_split(
                self,
            ) -> (
                Compat<tokio::io::ReadHalf<NamedPipeServer>>,
                Compat<tokio::io::WriteHalf<NamedPipeServer>>,
            ) {
                let (reader, writer) = tokio::io::split(self.0);
                (reader.compat(), writer.compat_write())
            }
        }

        pub struct PipeListener {
            path: String,
            security_descriptor: OwnedSecurityDescriptor,
            // Per Windows' named pipe semantics, only the very first instance created for a given
            // pipe name establishes its security descriptor; subsequent instances (created here
            // after each `accept`) ignore whatever security attributes they're given. We keep
            // building fresh `SECURITY_ATTRIBUTES` from the same descriptor for every instance
            // anyway, since `CreateNamedPipeW` requires *some* value to be supplied.
            stored_instance: Mutex<NamedPipeServer>,
        }

        impl PipeListener {
            pub fn bind(
                connection_address: &ConnectionAddress,
                sddl: &str,
                background_executor: Arc<Background>,
            ) -> io::Result<Self> {
                let path = pipe_path(connection_address);
                let security_descriptor = parse_security_descriptor(sddl)?;
                // `first_pipe_instance` guards against "named pipe squatting": without it, a
                // malicious process could pre-create a pipe with this name before we do, and we'd
                // silently become an additional instance of that attacker-controlled pipe instead
                // of failing loudly.
                let first_instance = {
                    let path = path.clone();
                    let sddl = sddl.to_owned();
                    run_on_background(background_executor, move || {
                        let security_descriptor = parse_security_descriptor(&sddl)?;
                        Self::create_pipe_instance(&path, &security_descriptor, true)
                    })?
                };
                Ok(Self {
                    path,
                    security_descriptor,
                    stored_instance: Mutex::new(first_instance),
                })
            }

            fn create_pipe_instance(
                path: &str,
                security_descriptor: &OwnedSecurityDescriptor,
                first_instance: bool,
            ) -> io::Result<NamedPipeServer> {
                let mut attributes = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: security_descriptor.0.0,
                    bInheritHandle: false.into(),
                };
                // SAFETY: `attributes` is a validly-initialized `SECURITY_ATTRIBUTES` whose
                // `lpSecurityDescriptor` points at a security descriptor that outlives this call
                // (owned by `self` for the lifetime of the listener). The OS only reads through
                // this pointer for the duration of the `CreateNamedPipeW` call underlying
                // `create_with_security_attributes_raw`.
                unsafe {
                    named_pipe::ServerOptions::new()
                        .first_pipe_instance(first_instance)
                        .create_with_security_attributes_raw(
                            path,
                            &mut attributes as *mut SECURITY_ATTRIBUTES as *mut c_void,
                        )
                }
            }

            pub async fn accept(&self) -> io::Result<PipeStream> {
                let mut stored_instance = self.stored_instance.lock().await;
                stored_instance.connect().await?;
                let next_instance =
                    Self::create_pipe_instance(&self.path, &self.security_descriptor, false)?;
                let connected_instance = std::mem::replace(&mut *stored_instance, next_instance);
                Ok(PipeStream(connected_instance))
            }
        }

        #[cfg(test)]
        mod tests {
            use std::sync::Arc;

            use uuid::Uuid;
            use warpui_core::r#async::executor::Background;

            use super::*;

            #[test]
            fn bind_creates_initial_instance_on_background_runtime() {
                let background_executor =
                    Arc::new(Background::new(1, |_| "ipc-test-background".to_owned()));
                let connection_address =
                    ConnectionAddress::from(format!("WarpTest{}_URI_CHANNEL", Uuid::new_v4()));

                let listener = PipeListener::bind(
                    &connection_address,
                    "D:(A;;GA;;;SY)(A;;GA;;;OW)(A;;0x12019B;;;IU)",
                    background_executor,
                );

                assert!(listener.is_ok(), "{:?}", listener.err());
            }
        }
    }
}
