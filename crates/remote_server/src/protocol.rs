use std::fmt;

use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use prost::Message;

use crate::proto::{ClientMessage, ServerMessage};

/// Maximum allowed message payload size (64 MB).
///
/// `read_message` rejects payloads exceeding this limit after decoding the
/// length prefix but before allocating the payload buffer, preventing OOM from
/// corrupted or adversarial length prefixes.
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// Errors that can occur during protocol-level read/write operations.
#[derive(thiserror::Error, Debug)]
pub enum ProtocolError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// When full protobuf decode fails, the protocol layer attempts to extract
    /// the `request_id` from the raw bytes so callers can correlate the error.
    #[error("Failed to decode protobuf message: {0}")]
    Decode(prost::DecodeError, Option<RequestId>),

    #[error("Unexpected EOF while reading message")]
    UnexpectedEof,

    #[error("Message too large: {size} bytes exceeds limit of {max} bytes")]
    MessageTooLarge { size: usize, max: usize },
}

/// A typed wrapper around the proto `string request_id` field.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(String);

impl RequestId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Returns true if this is an empty request ID, indicating a push message
    /// from the server (not correlated to any client request).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<RequestId> for String {
    fn from(id: RequestId) -> Self {
        id.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reads a length-delimited protobuf message from `reader`.
///
/// Wire format: `[4-byte little-endian length][protobuf bytes]`.
pub async fn read_message<M: Message + Default>(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<M, ProtocolError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ProtocolError::UnexpectedEof);
        }
        Err(e) => return Err(ProtocolError::Io(e)),
    }
    let len = u32::from_le_bytes(len_buf) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: len,
            max: MAX_MESSAGE_SIZE,
        });
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtocolError::UnexpectedEof
        } else {
            ProtocolError::Io(e)
        }
    })?;

    M::decode(&buf[..]).map_err(|e| {
        let request_id = try_extract_request_id(&buf).map(RequestId::from);
        ProtocolError::Decode(e, request_id)
    })
}

/// Writes a length-delimited protobuf message to `writer`.
///
/// Wire format: `[4-byte little-endian length][protobuf bytes]`.
pub async fn write_message<M: Message>(
    writer: &mut (impl AsyncWrite + Unpin),
    msg: &M,
) -> Result<(), ProtocolError> {
    let encoded = msg.encode_to_vec();
    if encoded.len() > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: encoded.len(),
            max: MAX_MESSAGE_SIZE,
        });
    }
    let len = encoded.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&encoded).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads a `ClientMessage` from the given reader.
pub async fn read_client_message(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<ClientMessage, ProtocolError> {
    read_message(reader).await
}

/// Writes a `ClientMessage` to the given writer.
pub async fn write_client_message(
    writer: &mut (impl AsyncWrite + Unpin),
    msg: &ClientMessage,
) -> Result<(), ProtocolError> {
    write_message(writer, msg).await
}

/// Reads a `ServerMessage` from the given reader.
pub async fn read_server_message(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<ServerMessage, ProtocolError> {
    read_message(reader).await
}

/// Writes a `ServerMessage` to the given writer.
pub async fn write_server_message(
    writer: &mut (impl AsyncWrite + Unpin),
    msg: &ServerMessage,
) -> Result<(), ProtocolError> {
    write_message(writer, msg).await
}

impl ProtocolError {
    /// Whether a read loop can safely continue after this error.
    ///
    /// True only when the payload was fully consumed, keeping the stream aligned
    /// at the next length prefix.
    pub fn is_read_recoverable(&self) -> bool {
        match self {
            ProtocolError::Decode(..) => true,
            ProtocolError::Io(_) => false,
            ProtocolError::UnexpectedEof => false,
            ProtocolError::MessageTooLarge { .. } => false,
        }
    }

    /// Whether a write loop can safely continue after this error.
    ///
    /// True only when nothing was written to the stream, keeping it aligned.
    pub fn is_write_recoverable(&self) -> bool {
        match self {
            ProtocolError::MessageTooLarge { .. } => true,
            ProtocolError::Io(_) => false,
            ProtocolError::Decode(..) => false,
            ProtocolError::UnexpectedEof => false,
        }
    }
}

/// Attempts to extract the `request_id` from raw protobuf bytes by parsing
/// only field 1 (string) and ignoring the rest of the buffer.
///
/// This uses manual wire-format parsing: field 1 of type string has tag byte
/// `0x0a` (field_number=1, wire_type=2) followed by a varint length and UTF-8
/// bytes. We stop as soon as field 1 is extracted, so corruption in later
/// bytes does not affect extraction.
///
/// **Note**: This assumes `request_id` is always field 1 in the message schema.
/// If the protobuf schema changes, update this accordingly.
///
/// Returns `None` if the buffer doesn't start with a valid field 1 string,
/// or if the extracted string is empty.
fn try_extract_request_id(buf: &[u8]) -> Option<String> {
    // Field 1 (string) wire tag: field_number=1, wire_type=2 (length-delimited).
    if buf.first() != Some(&0x0a) {
        return None;
    }
    let buf = &buf[1..];

    // Decode varint-encoded string length.
    let (len, consumed) = decode_varint(buf)?;
    let buf = &buf[consumed..];

    if buf.len() < len {
        return None;
    }

    let s = std::str::from_utf8(&buf[..len]).ok()?;
    if s.is_empty() {
        return None;
    }
    Some(s.to_string())
}

/// Decodes a protobuf varint from the start of `buf`.
/// Returns `(value, bytes_consumed)` or `None` if the varint is malformed.
fn decode_varint(buf: &[u8]) -> Option<(usize, usize)> {
    let mut result: u64 = 0;
    for (i, &byte) in buf.iter().enumerate() {
        if i >= 10 {
            // Varint too long.
            return None;
        }
        result |= ((byte & 0x7F) as u64) << (i * 7);
        if byte & 0x80 == 0 {
            return Some((result as usize, i + 1));
        }
    }
    None
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
