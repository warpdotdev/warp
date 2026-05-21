use prost::Message;

use super::*;
use crate::proto::{
    client_message, server_message, ClientMessage, Initialize, InitializeResponse, ServerMessage,
};

#[tokio::test]
async fn round_trip_client_message() {
    let msg = ClientMessage {
        request_id: "test-123".to_string(),
        message: Some(client_message::Message::Initialize(Initialize {
            auth_token: String::new(),
            user_id: String::new(),
            user_email: String::new(),
            crash_reporting_enabled: true,
            codebase_index_limits: None,
        })),
    };

    let mut buf = Vec::new();
    write_client_message(&mut buf, &msg).await.unwrap();

    let mut cursor = &buf[..];
    let decoded: ClientMessage = read_client_message(&mut cursor).await.unwrap();

    assert_eq!(decoded.request_id, "test-123");
    match decoded.message {
        Some(client_message::Message::Initialize(_)) => {}
        other => panic!("unexpected message variant: {other:?}"),
    }
}

#[tokio::test]
async fn round_trip_server_message() {
    let msg = ServerMessage {
        request_id: "resp-456".to_string(),
        message: Some(server_message::Message::InitializeResponse(
            InitializeResponse {
                server_version: "0.1.0".to_string(),
                host_id: "test-host".to_string(),
            },
        )),
    };

    let mut buf = Vec::new();
    write_server_message(&mut buf, &msg).await.unwrap();

    let mut cursor = &buf[..];
    let decoded: ServerMessage = read_server_message(&mut cursor).await.unwrap();

    assert_eq!(decoded.request_id, "resp-456");
    match decoded.message {
        Some(server_message::Message::InitializeResponse(resp)) => {
            assert_eq!(resp.server_version, "0.1.0");
        }
        other => panic!("unexpected message variant: {other:?}"),
    }
}

#[tokio::test]
async fn read_unexpected_eof_on_empty_input() {
    let mut cursor: &[u8] = &[];
    let result = read_client_message(&mut cursor).await;
    assert!(matches!(result, Err(ProtocolError::UnexpectedEof)));
}

#[tokio::test]
async fn read_truncated_payload() {
    // Write a length prefix claiming 100 bytes, but only provide 4.
    let mut buf = Vec::new();
    buf.extend_from_slice(&100u32.to_le_bytes());
    buf.extend_from_slice(&[0u8; 4]);

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    assert!(matches!(result, Err(ProtocolError::UnexpectedEof)));
}

#[tokio::test]
async fn round_trip_zero_length_message() {
    // A default ClientMessage with no fields set encodes to zero bytes.
    let msg = ClientMessage::default();

    let mut buf = Vec::new();
    write_client_message(&mut buf, &msg).await.unwrap();

    // The first 4 bytes should be the length (0).
    assert_eq!(&buf[..4], &0u32.to_le_bytes());

    let mut cursor = &buf[..];
    let decoded: ClientMessage = read_client_message(&mut cursor).await.unwrap();
    assert_eq!(decoded.request_id, "");
    assert!(decoded.message.is_none());
}

#[tokio::test]
async fn read_message_too_large() {
    // Write a length prefix exceeding MAX_MESSAGE_SIZE.
    let oversized_len = (MAX_MESSAGE_SIZE as u32) + 1;
    let buf = oversized_len.to_le_bytes();

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    assert!(matches!(result, Err(ProtocolError::MessageTooLarge { .. })));
}

#[tokio::test]
async fn write_message_too_large() {
    // Build a ClientMessage whose encoded size exceeds MAX_MESSAGE_SIZE.
    let msg = ClientMessage {
        request_id: "x".repeat(MAX_MESSAGE_SIZE + 1),
        message: None,
    };

    let mut buf = Vec::new();
    let result = write_client_message(&mut buf, &msg).await;
    assert!(matches!(result, Err(ProtocolError::MessageTooLarge { .. })));
    // Nothing should have been written to the stream.
    assert!(buf.is_empty());
}

#[test]
fn try_extract_request_id_from_valid_message() {
    let msg = ClientMessage {
        request_id: "abc-123".to_string(),
        message: Some(client_message::Message::Initialize(Initialize {
            auth_token: String::new(),
            user_id: String::new(),
            user_email: String::new(),
            crash_reporting_enabled: true,
            codebase_index_limits: None,
        })),
    };
    let buf = msg.encode_to_vec();
    assert_eq!(try_extract_request_id(&buf), Some("abc-123".to_string()));
}

#[test]
fn try_extract_request_id_from_corrupted_payload_with_valid_id() {
    // Manually construct bytes: valid request_id field followed by
    // corrupt trailing bytes (unterminated varint that would crash
    // a full prost decode but doesn't affect our field-1 extraction).
    let mut buf = Vec::new();
    // Field 1 (string): tag=0x0a, length=7, "req-456"
    buf.push(0x0a);
    buf.push(7);
    buf.extend_from_slice(b"req-456");
    // Corrupt trailing bytes: unterminated varint (all continuation bits set).
    buf.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);

    // request_id should still be extractable despite trailing corruption.
    assert_eq!(try_extract_request_id(&buf), Some("req-456".to_string()));
}
#[test]
fn try_extract_request_id_from_empty_bytes() {
    assert_eq!(try_extract_request_id(&[]), None);
}

#[test]
fn try_extract_request_id_from_garbage_bytes() {
    // Completely random bytes that don't form a valid protobuf.
    // This may or may not decode depending on what prost makes of it,
    // but should not panic. If it decodes to an empty request_id, we
    // return None.
    let result = try_extract_request_id(&[0xFF, 0xFF, 0xFF, 0xFF]);
    // We don't assert a specific value — just that it doesn't panic.
    // If prost happens to decode something, it'll be empty or garbage.
    let _ = result;
}

#[tokio::test]
async fn decode_error_extracts_request_id() {
    // Construct a corrupted message with a valid request_id field.
    let mut payload = Vec::new();
    // Field 1 (string): tag=0x0a, length=6, "req-42"
    payload.push(0x0a);
    payload.push(6);
    payload.extend_from_slice(b"req-42");
    // Invalid trailing bytes that cause prost decode failure.
    payload.extend_from_slice(&[0x0F, 0x01]);

    let mut buf = Vec::new();
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&payload);

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    match result {
        Err(ProtocolError::Decode(_, Some(id))) => {
            assert_eq!(id.to_string(), "req-42");
        }
        other => panic!("expected Decode error with request_id, got: {other:?}"),
    }
}

#[tokio::test]
async fn decode_error_none_when_no_request_id() {
    // Completely invalid protobuf bytes with no valid field 1.
    let garbage = vec![0xFF, 0xFE, 0xFD, 0xFC];
    let mut buf = Vec::new();
    buf.extend_from_slice(&(garbage.len() as u32).to_le_bytes());
    buf.extend_from_slice(&garbage);

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    match result {
        Err(ProtocolError::Decode(_, None)) => {}
        other => panic!("expected Decode error with None request_id, got: {other:?}"),
    }
}
