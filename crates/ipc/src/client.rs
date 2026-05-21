use std::{collections::HashMap, sync::Arc};

use futures::{channel::oneshot, future::FutureExt, io::BufReader, AsyncRead, AsyncWrite};
use warpui::r#async::executor::Background;

use crate::{platform::client::connect_client, protocol::Request};

use super::{
    protocol::{
        receive_message, send_message, ConnectionAddress, ProtocolError, RequestId, Response,
    },
    service::service_id,
    Service,
};

#[derive(Debug)]
pub enum InitializationError {
    Io(std::io::Error),
    UnsupportedPlatform,
}

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("Failed to initialize client: {0:?}")]
    Initialization(InitializationError),

    #[error("Connection was dropped.")]
    Disconnected,

    #[error("Internal error occurred: {0:?}")]
    InternalProtocol(#[from] ProtocolError),

    #[error("The channel for receiving the response from the inbound message task is closed.")]
    ResponseChannelClosed,

    #[error(
        "The channel for transmitting pending request info to the inbound message task is closed."
    )]
    PendingRequestInfoChannelClosed,
}

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug)]
struct PendingRequestInfo {
    /// The ID of the in-flight request.
    request_id: RequestId,

    /// A sender for relaying the response bytes back to the caller of `send_request()`.
    response_result_tx: oneshot::Sender<Result<Vec<u8>>>,
}

#[derive(Debug)]
struct OutboundRequest {
    // The request to be sent to the server.
    request: Request,

    // A sender for relaying any error that occurs when sending the request.
    //
    // If the request is sent successfully, this sender is moved to the new `PendingRequestInfo`
    // created for the request, where it is eventually used to relay response bytes back to the
    // caller.
    response_result_tx: oneshot::Sender<Result<Vec<u8>>>,
}

pub struct Client {
    /// A sender for relaying requests from `Self::send_request()` to the background task
    /// responsible for actually writing requests to the socket.
    outbound_message_tx: async_channel::Sender<OutboundRequest>,

    /// A receiver for a single-message bounded channel that emits an event when the server
    /// connection is dropped.
    disconnect_rx: async_channel::Receiver<()>,

    /// A reference to the background executor so that we don't drop it while waiting on tasks
    /// that use it to run to completion. Otherwise, it can hang when all references are dropped.
    _background_executor: Arc<Background>,
}

impl Client {
    /// Creates a client connected to a server corresponding to the given `connection_address`.
    ///
    /// If successful, spawns background tasks to send requests and receive responses.
    pub async fn connect(
        connection_address: ConnectionAddress,
        background_executor: Arc<Background>,
    ) -> Result<Self> {
        let (reader, writer) = connect_client(connection_address).await?;
        let (disconnect_tx, disconnect_rx) = async_channel::bounded(1);
        let (pending_request_info_tx, pending_request_info_rx) = async_channel::unbounded();
        let disconnect_tx_clone = disconnect_tx.clone();
        background_executor
            .spawn(async move {
                Self::handle_incoming_responses(reader, pending_request_info_rx).await;
                let _ = disconnect_tx_clone.try_send(());
            })
            .detach();

        let (outbound_message_tx, outbound_message_rx) = async_channel::unbounded();
        background_executor
            .spawn(async move {
                Self::handle_outgoing_requests(
                    writer,
                    outbound_message_rx,
                    pending_request_info_tx,
                )
                .await;
                let _ = disconnect_tx.try_send(());
            })
            .detach();

        Ok(Self {
            outbound_message_tx,
            disconnect_rx,
            _background_executor: background_executor,
        })
    }

    pub async fn wait_for_disconnect(&self) {
        let _ = self.disconnect_rx.recv().await;
    }

    /// Schedules the given message to be written to the underlying transport.
    pub(super) async fn send_request<S: Service>(&self, request_bytes: Vec<u8>) -> Result<Vec<u8>> {
        let request = Request::new(service_id::<S>(), request_bytes);

        // Create a channel for the response result. The sending end is sent to the outbound
        // message task. The outbound message task uses it to relay any error that might occur
        // when sending the message. If the message is sent successfully, the sending end is
        // forwarded to the _inbound_ message task, which will eventually use it to relay the
        // response bytes.
        let (response_result_tx, response_result_rx) = oneshot::channel();

        if self
            .outbound_message_tx
            .send(OutboundRequest {
                request,
                response_result_tx,
            })
            .await
            .is_err()
        {
            // The background inbound traffic processing task exited, so we must be disconnected.
            return Err(ClientError::Disconnected);
        }

        match response_result_rx.await {
            Ok(response_result) => response_result,
            Err(_) => Err(ClientError::ResponseChannelClosed),
        }
    }

    /// Handles incoming response messages and relays them  back to the caller via a
    /// request-specific async channel.
    async fn handle_incoming_responses(
        reader: impl AsyncRead + Unpin,
        pending_request_info_rx: async_channel::Receiver<PendingRequestInfo>,
    ) {
        let mut reader = BufReader::new(reader);

        // Map from request ID to async channel sender, through which we should relay the
        // corresponding response bytes.
        let mut response_senders = HashMap::<RequestId, oneshot::Sender<Result<Vec<u8>>>>::new();

        loop {
            futures::select! {
                pending_request_info = pending_request_info_rx.recv().fuse() => {
                    // TODO(zachbai): Because we're asynchronously receiving `PendingRequestInfo`
                    // from the outbound request task, it's possible that the response is actually
                    // received before the pending_request_info is received and handled by this
                    // block. We should hold onto unmatched responses for some small amount of time
                    // and check if new `PendingRequestInfo`s match the recently received responses.
                    // Similarly, its possible the server never responds to a request with a
                    // `PendingRequestInfo` -- we should implement timed cleanups of
                    // `PendingRequestInfo` (a request timeout) to address the possible memory leak.
                    match pending_request_info {
                        Ok(PendingRequestInfo {
                            request_id, response_result_tx
                        }) => {
                            // We've just sent a request, so update the `response_senders` map
                            // so we can relay the response back.
                            response_senders.insert(request_id, response_result_tx);
                        }
                        Err(_) => {
                            // This happens when the channel is closed, which implies the client
                            // was `Drop`ped, so break and exit.
                            break;
                        }

                    }
                }
                response = receive_message(&mut reader).fuse() => {
                    match response {
                        Ok(response) => {
                            let (request_id, response_result) = match response {
                                Response::Success {
                                    request_id,
                                    bytes: response_bytes,
                                    ..
                                } =>  {
                                    (request_id, Ok(response_bytes))
                                }
                                Response::Failure {
                                    request_id,
                                    error_message,
                                } => {
                                    (request_id, Err(ClientError::InternalProtocol(ProtocolError::Other(error_message))))
                                }
                            };

                            if let Some(response_result_tx) = response_senders.remove(&request_id) {
                                // The channel might be closed if the task that called
                                // `send_message` has been dropped, but that's ok.
                                let _ = response_result_tx.send(response_result);
                            } else {
                                // When there is no corresponding response_senders
                                // entry for the message's request ID, we weren't
                                // expecting it.
                                log::warn!("Received unexpected message with id {request_id}.");
                            }
                        }
                        Err(e) => {
                            match e {
                                ProtocolError::Disconnected(_)=> {
                                    // The server was disconnected, so break and exit.
                                    break;
                                }
                                e => {
                                    log::warn!("Error occurred while receiving message: {e:?}");
                                }
                            }
                        }
                    }
                }
                complete => break,
            }
        }
    }

    /// Polls `outbound_message_rx` for request messages and sends them over the IPC transport.
    ///
    /// If a request is sent successfully, the `response_result_tx` from the corresponding
    /// `OutboundRequest` is sent to the _inbound_ response task, which sends the response through
    /// it once received.
    async fn handle_outgoing_requests(
        mut writer: impl AsyncWrite + Unpin,
        outbound_request_rx: async_channel::Receiver<OutboundRequest>,
        pending_request_info_tx: async_channel::Sender<PendingRequestInfo>,
    ) {
        while let Ok(OutboundRequest {
            request,
            response_result_tx,
        }) = outbound_request_rx.recv().await
        {
            let request_id = *request.id();
            match send_message(&mut writer, request).await {
                Ok(()) => {
                    if pending_request_info_tx.is_closed() {
                        // The channel might be closed if the task that called
                        // `send_message` has been dropped, but that's ok.
                        let _ = response_result_tx
                            .send(Err(ClientError::PendingRequestInfoChannelClosed));
                    } else {
                        // Let the inbound traffic task know that we successfully sent a
                        // request, so it can relay the response back to the caller.
                        //
                        // We pass on the `response_result_tx` from the `OutboundRequest`
                        // object, which will eventually be used to relay the response.
                        let pending_request_info = PendingRequestInfo {
                            request_id,
                            response_result_tx,
                        };
                        let _ = pending_request_info_tx.send(pending_request_info).await;
                    }
                }
                Err(ProtocolError::Disconnected(_)) => {
                    // The channel might be closed if the task that called
                    // `send_message` has been dropped, but that's ok.
                    let _ = response_result_tx.send(Err(ClientError::Disconnected));
                    break;
                }
                Err(e) => {
                    // The channel might be closed if the task that called
                    // `send_message` has been dropped, but that's ok.
                    let _ = response_result_tx.send(Err(ClientError::InternalProtocol(e)));
                }
            }
        }
    }
}
