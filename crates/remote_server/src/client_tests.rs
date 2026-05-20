use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::proto::{
    client_message, get_fragment_metadata_from_hash_response, run_command_response, server_message,
    ClientMessage, CodebaseIndexStatus, CodebaseIndexStatusState, CodebaseIndexStatusUpdated,
    CodebaseIndexStatusesSnapshot, ErrorCode, FileOperationError, FragmentMetadata,
    FragmentMetadataLookupError, FragmentMetadataLookupErrorCode,
    GetFragmentMetadataFromHashResponse, GetFragmentMetadataFromHashSuccess, InitializeResponse,
    MissingFragmentMetadata, RunCommandResponse, RunCommandSuccess, ServerMessage,
};
use crate::protocol;
use warp_core::SessionId;
use warpui::r#async::executor;

use super::*;

/// Generic mock server: loops reading ClientMessages and responds using the
/// provided closure. Exits cleanly on EOF.
async fn mock_server_with<F>(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    responder: F,
) where
    F: Fn(&ClientMessage) -> server_message::Message,
{
    loop {
        match protocol::read_client_message(&mut reader).await {
            Ok(msg) => {
                let response = ServerMessage {
                    request_id: msg.request_id.clone(),
                    message: Some(responder(&msg)),
                };
                protocol::write_server_message(&mut writer, &response)
                    .await
                    .unwrap();
            }
            Err(protocol::ProtocolError::UnexpectedEof) => break,
            Err(e) => panic!("mock server error: {e}"),
        }
    }
}

fn not_enabled_codebase_status(repo_path: &str) -> CodebaseIndexStatus {
    CodebaseIndexStatus {
        repo_path: repo_path.to_string(),
        state: CodebaseIndexStatusState::NotEnabled.into(),
        last_updated_epoch_millis: Some(123),
        progress_completed: None,
        progress_total: None,
        failure_message: None,
        root_hash: None,
    }
}

#[tokio::test]
async fn codebase_index_push_messages_become_client_events() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    drop(server_read);

    let executor = executor::Background::default();
    let (_client, event_rx, _failure_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
    let mut writer = server_write.compat_write();

    protocol::write_server_message(
        &mut writer,
        &ServerMessage {
            request_id: String::new(),
            message: Some(server_message::Message::CodebaseIndexStatusesSnapshot(
                CodebaseIndexStatusesSnapshot {
                    statuses: vec![not_enabled_codebase_status("/repo")],
                },
            )),
        },
    )
    .await
    .unwrap();
    protocol::write_server_message(
        &mut writer,
        &ServerMessage {
            request_id: String::new(),
            message: Some(server_message::Message::CodebaseIndexStatusUpdated(
                CodebaseIndexStatusUpdated {
                    status: Some(not_enabled_codebase_status("/repo")),
                },
            )),
        },
    )
    .await
    .unwrap();
    writer.flush().await.unwrap();

    match event_rx.recv().await.unwrap() {
        ClientEvent::CodebaseIndexStatusesSnapshotReceived { statuses } => {
            assert_eq!(statuses.len(), 1);
            assert_eq!(statuses[0].repo_path, "/repo");
        }
        other => panic!("Expected CodebaseIndexStatusesSnapshotReceived, got {other:?}"),
    }
    match event_rx.recv().await.unwrap() {
        ClientEvent::CodebaseIndexStatusUpdated { status } => {
            assert_eq!(status.repo_path, "/repo");
        }
        other => panic!("Expected CodebaseIndexStatusUpdated, got {other:?}"),
    }
}

/// Sets up a duplex stream, spawns `mock_server_with` with the given responder,
/// and returns a connected `RemoteServerClient`, its event receiver, and the
/// background executor (which must be kept alive for the test duration).
fn setup_mock_client<F>(
    responder: F,
) -> (
    RemoteServerClient,
    async_channel::Receiver<ClientEvent>,
    executor::Background,
)
where
    F: Fn(&ClientMessage) -> server_message::Message + Send + 'static,
{
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);

    tokio::spawn(mock_server_with(
        server_read.compat(),
        server_write.compat_write(),
        responder,
    ));

    let executor = executor::Background::default();
    let (client, event_rx, _failure_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
    (client, event_rx, executor)
}

#[tokio::test]
async fn initialize_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|_| {
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
        })
    });

    let resp = client
        .initialize(
            None,
            InitializeParams {
                user_id: String::new(),
                user_email: String::new(),
                crash_reporting_enabled: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(resp.server_version, "test-0.1.0");
    assert_eq!(resp.host_id, "test-host-id");
}

#[tokio::test]
async fn initialize_sends_empty_auth_token_when_none() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::Initialize(init)) => {
                assert!(init.auth_token.is_empty());
            }
            other => panic!("Expected Initialize, got {other:?}"),
        }
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
        })
    });

    client
        .initialize(
            None,
            InitializeParams {
                user_id: String::new(),
                user_email: String::new(),
                crash_reporting_enabled: true,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn initialize_sends_auth_token_when_provided() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::Initialize(init)) => {
                assert_eq!(init.auth_token, "secret-token");
            }
            other => panic!("Expected Initialize, got {other:?}"),
        }
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
        })
    });

    client
        .initialize(
            Some("secret-token"),
            InitializeParams {
                user_id: String::new(),
                user_email: String::new(),
                crash_reporting_enabled: true,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn get_fragment_metadata_from_hash_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::GetFragmentMetadataFromHash(request)) => {
                assert_eq!(request.repo_path, "/repo");
                assert_eq!(request.root_hash, "root-hash");
                assert_eq!(request.content_hashes, vec!["found-hash", "missing-hash"]);
            }
            other => panic!("Expected GetFragmentMetadataFromHash, got {other:?}"),
        }
        server_message::Message::GetFragmentMetadataFromHashResponse(
            GetFragmentMetadataFromHashResponse {
                result: Some(get_fragment_metadata_from_hash_response::Result::Success(
                    GetFragmentMetadataFromHashSuccess {
                        fragments: vec![FragmentMetadata {
                            content_hash: "found-hash".to_string(),
                            path: "/repo/src/lib.rs".to_string(),
                            start_line: 1,
                            end_line: 3,
                            byte_start: 0,
                            byte_end: 42,
                        }],
                        missing_hashes: vec![MissingFragmentMetadata {
                            content_hash: "missing-hash".to_string(),
                            error: Some(FileOperationError {
                                message: "missing".to_string(),
                            }),
                        }],
                    },
                )),
            },
        )
    });

    let response = client
        .get_fragment_metadata_from_hash(
            "/repo".to_string(),
            "root-hash".to_string(),
            vec!["found-hash".to_string(), "missing-hash".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(response.fragments.len(), 1);
    assert_eq!(response.fragments[0].content_hash, "found-hash");
    assert_eq!(response.missing_hashes.len(), 1);
    assert_eq!(response.missing_hashes[0].content_hash, "missing-hash");
}

#[tokio::test]
async fn resync_codebase_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::ResyncCodebase(request)) => {
                assert_eq!(request.repo_path, "/repo");
                assert_eq!(request.auth_token, "auth-token");
            }
            other => panic!("Expected ResyncCodebase, got {other:?}"),
        }
        server_message::Message::CodebaseIndexStatusUpdated(CodebaseIndexStatusUpdated {
            status: Some(not_enabled_codebase_status("/repo")),
        })
    });

    let status = client
        .resync_codebase("/repo".to_string(), "auth-token".to_string())
        .await
        .unwrap();

    assert_eq!(status.repo_path, "/repo");
}

#[tokio::test]
async fn get_fragment_metadata_from_hash_error_maps_to_typed_client_error() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::GetFragmentMetadataFromHash(request)) => {
                assert_eq!(request.repo_path, "/repo");
            }
            other => panic!("Expected GetFragmentMetadataFromHash, got {other:?}"),
        }
        server_message::Message::GetFragmentMetadataFromHashResponse(
            GetFragmentMetadataFromHashResponse {
                result: Some(get_fragment_metadata_from_hash_response::Result::Error(
                    FragmentMetadataLookupError {
                        code: FragmentMetadataLookupErrorCode::IndexNotSynced.into(),
                        message: "Codebase index has no synced root hash".to_string(),
                        current_root_hash: None,
                    },
                )),
            },
        )
    });

    let error = client
        .get_fragment_metadata_from_hash(
            "/repo".to_string(),
            "root-hash".to_string(),
            vec!["hash".to_string()],
        )
        .await
        .unwrap_err();

    match error {
        ClientError::FragmentMetadataLookup { code, message } => {
            assert_eq!(code, FragmentMetadataLookupErrorCode::IndexNotSynced);
            assert_eq!(message, "Codebase index has no synced root hash");
        }
        other => panic!("Expected FragmentMetadataLookup error, got {other:?}"),
    }
}

#[tokio::test]
async fn authenticate_sends_fire_and_forget_message() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, _server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, _event_rx, _failure_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    client.authenticate("rotated-secret");

    let msg = protocol::read_client_message(&mut server_read.compat())
        .await
        .unwrap();
    match msg.message {
        Some(client_message::Message::Authenticate(auth)) => {
            assert_eq!(auth.auth_token, "rotated-secret");
        }
        other => panic!("Expected Authenticate, got {other:?}"),
    }
}

#[tokio::test]
async fn disconnected_on_closed_stream() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    // Drop the server side immediately.
    drop(server_stream);

    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, disconnect_rx, _failure_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    // An initialize call on a dead stream must complete with an error rather than hang.
    let result = client
        .initialize(
            None,
            InitializeParams {
                user_id: String::new(),
                user_email: String::new(),
                crash_reporting_enabled: true,
            },
        )
        .await;
    assert!(result.is_err());

    // The reader task should detect EOF and emit a Disconnected event.
    let event = disconnect_rx.recv().await.unwrap();
    assert!(matches!(event, ClientEvent::Disconnected));
}

#[tokio::test]
async fn run_command_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        let command = match &msg.message {
            Some(client_message::Message::RunCommand(req)) => req.command.clone(),
            other => panic!("Expected RunCommand, got {other:?}"),
        };
        server_message::Message::RunCommandResponse(RunCommandResponse {
            result: Some(run_command_response::Result::Success(RunCommandSuccess {
                stdout: format!("output of: {command}").into_bytes(),
                stderr: Vec::new(),
                exit_code: Some(0),
            })),
        })
    });

    let resp = client
        .run_command(
            SessionId::from(42u64),
            "echo hello".to_string(),
            None,
            Default::default(),
        )
        .await
        .unwrap();
    let success = match resp.result {
        Some(run_command_response::Result::Success(s)) => s,
        other => panic!("Expected RunCommandSuccess, got {other:?}"),
    };
    assert_eq!(success.stdout, b"output of: echo hello");
    assert!(success.stderr.is_empty());
    assert_eq!(success.exit_code, Some(0));
}

#[tokio::test]
async fn concurrent_in_flight_requests() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|_| {
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
        })
    });
    let client = std::sync::Arc::new(client);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let c = std::sync::Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.initialize(
                None,
                InitializeParams {
                    user_id: String::new(),
                    user_email: String::new(),
                    crash_reporting_enabled: true,
                },
            )
            .await
            .expect("concurrent initialize failed")
        }));
    }

    for h in handles {
        let resp = h.await.unwrap();
        assert_eq!(resp.server_version, "test-0.1.0");
        assert_eq!(resp.host_id, "test-host-id");
    }
}

/// Simulates a server that reads raw bytes, sends an error response for
/// malformed messages where the request_id is parseable, then continues
/// processing valid messages.
async fn mock_server_with_error_handling(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
) {
    loop {
        match protocol::read_client_message(&mut reader).await {
            Ok(msg) => {
                let response = ServerMessage {
                    request_id: msg.request_id,
                    message: Some(server_message::Message::InitializeResponse(
                        InitializeResponse {
                            server_version: "test-0.1.0".to_string(),
                            host_id: "test-host-id".to_string(),
                        },
                    )),
                };
                protocol::write_server_message(&mut writer, &response)
                    .await
                    .unwrap();
            }
            Err(protocol::ProtocolError::Decode(_, Some(ref id))) => {
                let error_response = ServerMessage {
                    request_id: id.to_string(),
                    message: Some(server_message::Message::Error(
                        crate::proto::ErrorResponse {
                            code: ErrorCode::InvalidRequest.into(),
                            message: "malformed message".to_string(),
                        },
                    )),
                };
                protocol::write_server_message(&mut writer, &error_response)
                    .await
                    .unwrap();
            }
            Err(protocol::ProtocolError::Decode(_, None)) => {}
            Err(protocol::ProtocolError::UnexpectedEof) => break,
            Err(e) => panic!("mock server error: {e}"),
        }
    }
}

/// Sends a corrupted protobuf with a valid request_id to the server,
/// verifying the server responds with an ErrorResponse for that request_id.
#[tokio::test]
async fn server_returns_error_for_malformed_message_with_parseable_id() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);

    tokio::spawn(mock_server_with_error_handling(
        server_read.compat(),
        server_write.compat_write(),
    ));

    // Manually construct a corrupted message with a valid request_id field
    // followed by bytes that cause a prost decode failure.
    let mut payload = Vec::new();
    // Field 1 (string): tag=0x0a, length=15, "malformed-req-1"
    payload.push(0x0a);
    payload.push(15);
    payload.extend_from_slice(b"malformed-req-1");
    // Invalid trailing bytes: field tag with reserved wire type 7 causes
    // prost to fail, but our try_extract_request_id stops after field 1.
    payload.extend_from_slice(&[0x0F, 0x01]); // field 1, wire type 7 (invalid)

    // Write the corrupted message with length prefix.
    let mut client_write = client_write.compat_write();
    let len = payload.len() as u32;
    client_write.write_all(&len.to_le_bytes()).await.unwrap();
    client_write.write_all(&payload).await.unwrap();
    client_write.flush().await.unwrap();

    // Read the error response from the server.
    let mut client_reader = futures::io::BufReader::new(client_read.compat());
    let response: ServerMessage = protocol::read_server_message(&mut client_reader)
        .await
        .unwrap();

    assert_eq!(response.request_id, "malformed-req-1");
    match response.message {
        Some(server_message::Message::Error(e)) => {
            assert_eq!(e.code(), ErrorCode::InvalidRequest);
        }
        other => panic!("expected ErrorResponse, got: {other:?}"),
    }
}
