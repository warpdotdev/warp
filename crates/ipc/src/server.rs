use std::collections::HashMap;
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use futures::io::BufReader;
use futures::{AsyncRead, AsyncWrite};
use warpui_core::r#async::executor::{Background, BackgroundTask};

use super::protocol::{
    receive_message, send_message, ConnectionAddress, ProtocolError, Request, Response,
};
use super::service::{service_id, Service, ServiceId};
use crate::platform::server::{ConnectionImpl, ConnectionListenerImpl};
use crate::service::ServiceImpl;

/// Helper trait to enable storing a polymorphic collection of `ServiceImpl` implementions in
/// `Server`.
///
/// This is akin to the `AnyView` and `AnyModel` traits used by the UI framework to similarly store
/// `View` callbacks that are actually parameterized by the type of the actual `View`
/// implementation.
#[async_trait]
pub(super) trait AnyServiceImpl: Send + Sync {
    async fn handle_request(&self, request: &[u8]) -> Vec<u8>;

    fn clone_service(&self) -> Box<dyn AnyServiceImpl>;
}

#[async_trait]
impl<I, S> AnyServiceImpl for I
where
    S: Service,
    I: ServiceImpl<Service = S> + Clone + Sized,
{
    async fn handle_request(&self, request_bytes: &[u8]) -> Vec<u8> {
        let request: S::Request =
            bincode::deserialize(request_bytes).expect("Failed to deserialize request bytes.");
        bincode::serialize::<S::Response>(&I::handle_request(self, request).await)
            .expect("Should be able to serialize response.")
    }

    fn clone_service(&self) -> Box<dyn AnyServiceImpl> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn AnyServiceImpl> {
    fn clone(&self) -> Self {
        self.clone_service()
    }
}

#[derive(Debug)]
pub enum InitializationError {
    Io(std::io::Error),
    UnsupportedPlatform,
}

#[derive(thiserror::Error, Debug)]
pub enum ServerError {
    #[error("Failed to initialize server: {0:?}")]
    Initialization(InitializationError),

    #[error("Failed to accept connection: {0:?}")]
    AcceptConnection(std::io::Error),
}

pub type Result<T> = std::result::Result<T, ServerError>;

/// A wrapper struct for abstracting-away platform-specific implementations for the server
/// functionality that listens for and accepts new connections.
struct ConnectionListener(ConnectionListenerImpl);

impl ConnectionListener {
    fn new(connection_address: ConnectionAddress) -> Result<Self> {
        ConnectionListenerImpl::new(connection_address).map(Self)
    }

    /// Waits until a client connects and returns the connection.
    async fn accept_connection(&self) -> Result<Connection> {
        self.0.accept_connection().await.map(Connection)
    }
}

/// A wrapper struct for abstracting-away platform-specific implementations of the underlying
/// transport for the IPC connection.
///
/// The main property of a [`Connection`] is that it can be consumed to create read and write
/// 'halves' which can be used to asynchronously read/write bytes to/from the transport.
struct Connection(ConnectionImpl);

impl Connection {
    /// Returns an `AsyncRead` impl to read bytes from the transport and `AsyncWrite` to write
    /// bytes to the transport.
    fn into_split(self) -> (impl AsyncRead + Unpin, impl AsyncWrite + Unpin) {
        self.0.into_split()
    }
}

/// Helper struct for building and running a server.
///
/// Usage:
///
/// ```ignore
/// let (server, connection_address) = ServerBuilder::default()
///     // Implements `ServiceImpl<MyService>`.
///     .with_service(MyServiceImpl::new())
///     .build_and_run()
///     .expect("Failed to run server.");
/// ```
#[derive(Default)]
pub struct ServerBuilder {
    services: HashMap<ServiceId, Box<dyn AnyServiceImpl>>,
    fixed_connection_address: Option<ConnectionAddress>,
}

impl ServerBuilder {
    pub fn with_service<S: ServiceImpl + Sized>(mut self, service_impl: S) -> Self {
        self.services
            .insert(service_id::<S::Service>(), Box::new(service_impl));
        self
    }

    /// Use a fixed address name instead of a randomly generated one.
    pub fn with_fixed_address(mut self, fixed_address: String) -> Self {
        self.fixed_connection_address = Some(ConnectionAddress::from(fixed_address));
        self
    }

    /// Instantiates a `Server` which listens for incoming client connections.
    ///
    /// If the server instantiation fails, returns an error.
    pub fn build_and_run(
        self,
        background_executor: Arc<Background>,
    ) -> Result<(Server, ConnectionAddress)> {
        let connection_address =
            if let Some(fixed_connection_address) = self.fixed_connection_address {
                fixed_connection_address
            } else {
                ConnectionAddress::new()
            };
        Server::run(
            connection_address.clone(),
            self.services,
            background_executor,
        )
        .map(|server| (server, connection_address))
    }
}

/// Serves registered `Service` implementations over platform-specific IPC transport.
///
/// Two background tasks are spawned for each client connection -- one for processing incoming
/// requests and one for sending outgoing responses.
pub struct Server {
    _tasks: Vec<BackgroundTask>,
}

impl Server {
    /// Runs the main server tasks.
    ///
    /// Two main tasks are spawned immediately -- one for listening for incoming client connections
    /// and one for "accepting" connections that were found. When "accepting" a connection, two
    /// additional connection-specific tasks are spawned -- one for processing incoming requests
    /// and one for sending outbound responses.
    fn run(
        connection_address: ConnectionAddress,
        services: HashMap<ServiceId, Box<dyn AnyServiceImpl>>,
        background_executor: Arc<Background>,
    ) -> Result<Self> {
        let listener = ConnectionListener::new(connection_address)?;

        // Spawn two separate background tasks. The first is responsible for listening for new
        // client connections and passing them to the second, which itself spawns tasks to process
        // inbound requests and outbound responses from each connection.
        //
        // A channel is used to pass connections between the two tasks.
        let (new_connection_tx, new_connection_rx) = async_channel::unbounded();
        let tasks = vec![
            background_executor.spawn(Self::listen_for_new_connections(
                listener,
                new_connection_tx,
            )),
            background_executor.spawn(Self::accept_new_connections(
                services,
                new_connection_rx,
                background_executor.clone(),
            )),
        ];
        Ok(Self { _tasks: tasks })
    }

    /// Listens for new connections on `listener`, relaying them through the given sender.
    async fn listen_for_new_connections(
        listener: ConnectionListener,
        new_connection_tx: Sender<Connection>,
    ) {
        loop {
            match listener.accept_connection().await {
                Ok(stream) => {
                    if new_connection_tx.send(stream).await.is_err() {
                        // The task responsible for handling new connections has
                        // exited, so break and exit too.
                        return;
                    }
                }
                Err(e) => {
                    log::warn!("Could not establish connection with client: {e:?}");
                }
            }
        }
    }

    /// Receives new connections from the given `Receiver` and spawns dedicated background tasks
    /// for processing incoming request messages and outgoing response messages.
    async fn accept_new_connections(
        services: HashMap<ServiceId, Box<dyn AnyServiceImpl>>,
        new_connection_rx: Receiver<Connection>,
        background_executor: Arc<Background>,
    ) {
        // Maintain references to the task handles so they're cancelled when this is dropped.
        let mut tasks = vec![];

        loop {
            let Ok(connection) = new_connection_rx.recv().await else {
                // The task responsible for listening for new connections has exited, so
                // break and exit too.
                return;
            };

            let (reader, writer) = connection.into_split();
            let (response_tx, response_rx) = async_channel::unbounded::<Response>();

            tasks.push(background_executor.spawn(Self::handle_incoming_requests(
                reader,
                services.clone(),
                response_tx,
            )));
            tasks.push(
                background_executor.spawn(Self::handle_outgoing_responses(writer, response_rx)),
            );
        }
    }

    /// Processes incoming request messages.
    ///
    /// This includes deserializing the request message into a `Service`-specific request type,
    /// dispatching the request to the `Service` itself, and sending the resulting response thru
    /// the given `response_tx`.
    ///
    /// The receiving end of the `response_tx` channel is processed in a separate task dedicated to
    /// sending outbound messages back to the client.
    async fn handle_incoming_requests(
        reader: impl AsyncRead + Unpin,
        services: HashMap<ServiceId, Box<dyn AnyServiceImpl>>,
        response_tx: Sender<Response>,
    ) {
        let mut reader = BufReader::new(reader);
        loop {
            match receive_message(&mut reader).await {
                Ok(Request {
                    id,
                    service_id,
                    bytes,
                }) => {
                    let response_message = match services.get(&service_id) {
                        Some(service) => {
                            let response_bytes = service.handle_request(&bytes[..]).await;
                            Response::success(id, service_id, response_bytes)
                        }
                        None => {
                            Response::failure(id, format!("No such service (ID: {service_id})"))
                        }
                    };

                    if response_tx.send(response_message).await.is_err() {
                        // This means the response_tx channel is closed, which probably
                        // means the outgoing messages task has exited. So this task should
                        // exit too.
                        break;
                    }
                }
                Err(e) => {
                    match e {
                        ProtocolError::Serialization(e) => {
                            log::warn!("Failed to deserialize request: {e:?}");
                        }
                        ProtocolError::Disconnected(_) => {
                            // The socket is disconnected, so exit.
                            log::warn!("IPC server disconnected unexpectedly.");
                            break;
                        }
                        e => {
                            log::warn!("Unknown error occurred when receiving request: {e:?}");
                        }
                    }
                }
            }
        }
    }

    /// Process outgoing response messages, received from the given `response_rx` receiver.
    async fn handle_outgoing_responses(
        mut writer: impl AsyncWrite + Unpin,
        response_rx: Receiver<Response>,
    ) {
        while let Ok(message) = response_rx.recv().await {
            if let Err(e) = send_message(&mut writer, message).await {
                match e {
                    ProtocolError::Serialization(e) => {
                        log::warn!("Failed to serialize response: {e:?}");
                    }
                    ProtocolError::Disconnected(_) => {
                        // The socket is disconnected, so exit.
                        break;
                    }
                    e => {
                        log::warn!("Unknown error occurred when sending response: {e:?}");
                    }
                }
            }
        }
    }
}
