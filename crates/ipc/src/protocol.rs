use std::{
    fmt::{Debug, Display},
    marker::Unpin,
};

use futures::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    AsyncRead, AsyncWrite,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use uuid::Uuid;

use super::service::ServiceId;

/// The size of a usize, in bytes.
const USIZE_SIZE: usize = std::mem::size_of::<usize>();

/// Unique "address" for a server/client connection.
///
/// In the case of this local socket implementation, this is a socket address (path on the
/// filesystem). Conceptually, this somewhat similar to an IP address + port.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct ConnectionAddress(pub(super) String);

impl ConnectionAddress {
    /// Returns a `ConnectionAddress` containing a path for a socket address.
    pub(super) fn new() -> Self {
        Self(format!("/tmp/warp-ipc-{}.sock", rand::random::<i64>()))
    }
}

impl Display for ConnectionAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ConnectionAddress {
    fn from(value: String) -> Self {
        ConnectionAddress(value)
    }
}

/// A unique ID for each request message.
///
/// The corresponding response for the request should contain the same ID.
pub(super) type RequestId = Uuid;

/// Trait for arbitrary messages that may be sent across the 'wire' (the socket).
pub trait Message: 'static + Send + Sync + Debug + Clone + DeserializeOwned + Serialize {}
impl<T> Message for T where T: 'static + Send + Sync + Debug + Clone + DeserializeOwned + Serialize {}

/// Request message sent by clients and received by servers.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct Request {
    /// A unique ID for the request.
    pub(super) id: RequestId,

    /// The ID of the service to which this request belongs.
    pub(super) service_id: ServiceId,

    /// The actual request payload.
    pub(super) bytes: Vec<u8>,
}

impl Request {
    /// Constructs a `Request`, generating a unique request ID in the process.
    pub(super) fn new(service_id: ServiceId, bytes: Vec<u8>) -> Self {
        Self {
            id: Uuid::new_v4(),
            service_id,
            bytes,
        }
    }

    pub(super) fn id(&self) -> &RequestId {
        &self.id
    }
}

/// Response message sent by servers and received by clients.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) enum Response {
    /// For responses produced "successfully". "Successful" only pertains to the frameworks ability
    /// to successfully execute the `Service` handler and produce a response. `Service`s may
    /// internally implement their own error types/response schemas.
    Success {
        /// The ID of the request for which this is a response.
        request_id: RequestId,

        /// The ID of the service to which this response belongs.
        service_id: ServiceId,

        /// The actual response payload.
        bytes: Vec<u8>,
    },

    /// For responses that failed due to a framework-level issue. For example, the client attempted
    /// to call a service that wasn't registered in the server.
    Failure {
        /// The ID of the request for which this is a response.
        request_id: RequestId,

        error_message: String,
    },
}

impl Response {
    /// Constructs a "success" response for the request with the given `request_id`.
    pub(super) fn success(request_id: RequestId, service_id: ServiceId, bytes: Vec<u8>) -> Self {
        Self::Success {
            request_id,
            service_id,
            bytes,
        }
    }

    /// Constructs a "failure" response for the request with the given `request_id`.
    pub(super) fn failure(request_id: RequestId, error_message: String) -> Self {
        Self::Failure {
            request_id,
            error_message,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ProtocolError {
    /// An error occurred when serializing the request or response.
    #[error(transparent)]
    Serialization(#[from] bincode::Error),

    /// The connection was dropped.
    #[error(transparent)]
    Disconnected(#[from] std::io::Error),

    #[error("Unknown error occurred: {0}")]
    Other(String),
}

/// Writes the given message to the given `writer`.
pub(super) async fn send_message<M, W>(writer: &mut W, message: M) -> Result<(), ProtocolError>
where
    M: Message,
    W: AsyncWrite + Unpin,
{
    let serialized_msg = bincode::serialize(&message)?;

    // Create a buffer to hold the data to be written.
    let mut buf = Vec::with_capacity(serialized_msg.len() + USIZE_SIZE);

    // First, add a message "header" - a usize representing the length of the
    // serialized payload, in bytes.
    buf.extend_from_slice(&serialized_msg.len().to_be_bytes());

    // Next, add the serialized payload itself.
    buf.extend(serialized_msg);

    // Finally, write the buffer to the underlying transport.
    Ok(writer.write_all(&buf[..]).await?)
}

/// Reads the next message from the given `reader`.
pub(super) async fn receive_message<M, R>(reader: &mut BufReader<R>) -> Result<M, ProtocolError>
where
    M: Message,
    R: AsyncRead + Unpin,
{
    // Start by allocating a buffer that is only large enough to receive the
    // message header, to ensure we don't accidentally receive multiple messages
    // in a single read.
    let mut header_buf = [0; USIZE_SIZE];

    // Read the message "header" from the socket.
    reader.read_exact(&mut header_buf[..]).await?;

    // Parse the message header - we convert the bytes back into a usize, which
    // tells us the size of the serialized message, in bytes.  We add the size
    // of a usize to get the total number of bytes we expect to read off the
    // wire.
    let payload_len = usize::from_be_bytes(header_buf);

    // Grow the initial buffer to a sufficient size and read the rest of the
    // message from the socket.
    let mut payload_buf = vec![0; payload_len];
    reader.read_exact(&mut payload_buf).await?;

    // Deserialize the message.
    let message: M = bincode::deserialize(&payload_buf[..])?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_new_assigns_service_id_bytes_and_unique_ids() {
        let service_id = "test-service".to_string();
        let bytes = vec![1, 2, 3];

        let request = Request::new(service_id.clone(), bytes.clone());
        let other_request = Request::new(service_id.clone(), bytes.clone());

        assert_eq!(request.service_id, service_id);
        assert_eq!(request.bytes, bytes);
        assert_eq!(request.id(), &request.id);
        assert_ne!(request.id, other_request.id);
    }

    #[test]
    fn response_constructors_preserve_payloads() {
        let request_id = Uuid::from_u128(0x1234567890abcdef1234567890abcdef);
        let service_id = "test-service".to_string();
        let bytes = vec![4, 5, 6];

        let success = Response::success(request_id, service_id.clone(), bytes.clone());
        match success {
            Response::Success {
                request_id: actual_request_id,
                service_id: actual_service_id,
                bytes: actual_bytes,
            } => {
                assert_eq!(actual_request_id, request_id);
                assert_eq!(actual_service_id, service_id);
                assert_eq!(actual_bytes, bytes);
            }
            Response::Failure { .. } => panic!("expected success response"),
        }

        let error_message = "service not registered".to_string();
        let failure = Response::failure(request_id, error_message.clone());
        match failure {
            Response::Failure {
                request_id: actual_request_id,
                error_message: actual_error_message,
            } => {
                assert_eq!(actual_request_id, request_id);
                assert_eq!(actual_error_message, error_message);
            }
            Response::Success { .. } => panic!("expected failure response"),
        }
    }

    #[test]
    fn request_serializes_and_deserializes_round_trip() {
        let request_id = Uuid::from_u128(0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa);
        let request = Request {
            id: request_id,
            service_id: "test-service".to_string(),
            bytes: vec![7, 8, 9],
        };

        let serialized = bincode::serialize(&request).expect("request should serialize");
        let deserialized: Request =
            bincode::deserialize(&serialized).expect("request should deserialize");

        assert_eq!(deserialized.id, request_id);
        assert_eq!(deserialized.service_id, request.service_id);
        assert_eq!(deserialized.bytes, request.bytes);
    }

    #[test]
    fn response_serializes_and_deserializes_round_trip() {
        let request_id = Uuid::from_u128(0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
        let success = Response::success(request_id, "test-service".to_string(), vec![10, 11]);
        let failure = Response::failure(request_id, "boom".to_string());

        let serialized_success = bincode::serialize(&success).expect("success should serialize");
        let deserialized_success: Response =
            bincode::deserialize(&serialized_success).expect("success should deserialize");
        match deserialized_success {
            Response::Success {
                request_id: actual_request_id,
                service_id,
                bytes,
            } => {
                assert_eq!(actual_request_id, request_id);
                assert_eq!(service_id, "test-service");
                assert_eq!(bytes, vec![10, 11]);
            }
            Response::Failure { .. } => panic!("expected success response"),
        }

        let serialized_failure = bincode::serialize(&failure).expect("failure should serialize");
        let deserialized_failure: Response =
            bincode::deserialize(&serialized_failure).expect("failure should deserialize");
        match deserialized_failure {
            Response::Failure {
                request_id: actual_request_id,
                error_message,
            } => {
                assert_eq!(actual_request_id, request_id);
                assert_eq!(error_message, "boom");
            }
            Response::Success { .. } => panic!("expected failure response"),
        }
    }

    #[test]
    fn connection_address_from_string_displays_and_serializes_round_trip() {
        let path = "/tmp/warp-ipc-test.sock".to_string();
        let address = ConnectionAddress::from(path.clone());

        assert_eq!(address.to_string(), path);

        let serialized = bincode::serialize(&address).expect("address should serialize");
        let deserialized: ConnectionAddress =
            bincode::deserialize(&serialized).expect("address should deserialize");

        assert_eq!(deserialized, address);
        assert_eq!(deserialized.to_string(), path);
    }

    #[test]
    fn connection_address_new_uses_tmp_warp_ipc_socket_path() {
        let address = ConnectionAddress::new();
        let address = address.to_string();

        assert!(address.starts_with("/tmp/warp-ipc-"));
        assert!(address.ends_with(".sock"));
    }
}
