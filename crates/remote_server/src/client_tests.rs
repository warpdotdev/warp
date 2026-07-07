use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use warp_core::SessionId;
use warpui_core::r#async::executor;

use super::*;
use crate::proto::{
    client_message, host_scoped_request, notification, run_command_response, server_message,
    session_scoped_request, ClientMessage, CodebaseIndexStatus, CodebaseIndexStatusState,
    CodebaseIndexStatusUpdated, CodebaseIndexStatusesSnapshot, ErrorCode, GetDiffStateResponse,
    InitializeResponse, OpenBufferResponse, RemoteAgentContextSnapshot, RemoteContextFileProto,
    RunCommandResponse, RunCommandSuccess, ServerMessage, WriteFile,
};
use crate::protocol;

/// Extract the session-scoped inner message from a ClientMessage wrapper.
fn unwrap_session_scoped(msg: &ClientMessage) -> &session_scoped_request::Message {
    match &msg.message {
        Some(client_message::Message::SessionScoped(w)) => w.message.as_ref().unwrap(),
        other => panic!("Expected SessionScoped, got {other:?}"),
    }
}

#[tokio::test]
async fn remote_agent_context_snapshot_push_becomes_client_event() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    drop(server_read);

    let executor = executor::Background::default();
    let (_client, event_rx, _failure_rx, _host_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
    let mut writer = server_write.compat_write();

    protocol::write_server_message(
        &mut writer,
        &ServerMessage {
            request_id: String::new(),
            message: Some(server_message::Message::RemoteAgentContextSnapshot(
                RemoteAgentContextSnapshot {
                    revision: 7,
                    home_dir: "/home/user".to_string(),
                    skills: vec![crate::proto::RemoteSkillProto {
                        path: "/home/user/.agents/skills/test/SKILL.md".to_string(),
                        content: "skill content".to_string(),
                        source: Some(crate::proto::remote_skill_proto::Source::Home(
                            crate::proto::HomeSkillMetadata {},
                        )),
                    }],
                    global_rules: vec![RemoteContextFileProto {
                        path: "/home/user/.agents/AGENTS.md".to_string(),
                        content: "rule content".to_string(),
                    }],
                },
            )),
        },
    )
    .await
    .unwrap();
    writer.flush().await.unwrap();

    match event_rx.recv().await.unwrap() {
        ClientEvent::RemoteAgentContextSnapshotReceived { snapshot } => {
            assert_eq!(snapshot.revision, 7);
            assert_eq!(snapshot.skills[0].content, "skill content");
            assert_eq!(snapshot.global_rules[0].content, "rule content");
        }
        other => panic!("Expected RemoteAgentContextSnapshotReceived, got {other:?}"),
    }
}

/// Extract the host-scoped inner message from a ClientMessage wrapper.
fn unwrap_host_scoped(msg: &ClientMessage) -> &host_scoped_request::Message {
    match &msg.message {
        Some(client_message::Message::HostScoped(w)) => w.message.as_ref().unwrap(),
        other => panic!("Expected HostScoped, got {other:?}"),
    }
}

/// Extract the notification inner message from a ClientMessage wrapper.
fn unwrap_notification(msg: &ClientMessage) -> &notification::Message {
    match &msg.message {
        Some(client_message::Message::Notification(w)) => w.message.as_ref().unwrap(),
        other => panic!("Expected Notification, got {other:?}"),
    }
}

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
    let (_client, event_rx, _failure_rx, _host_rx) =
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
    let (client, event_rx, _failure_rx, _host_rx) =
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
                codebase_index_limits: None,
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
        let session_scoped_request::Message::Initialize(init) = unwrap_session_scoped(msg) else {
            panic!("Expected Initialize");
        };
        assert!(init.auth_token.is_empty());
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
                codebase_index_limits: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn initialize_sends_auth_token_when_provided() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        let session_scoped_request::Message::Initialize(init) = unwrap_session_scoped(msg) else {
            panic!("Expected Initialize");
        };
        assert_eq!(init.auth_token, "secret-token");
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
                codebase_index_limits: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn authenticate_sends_fire_and_forget_message() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, _server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, _event_rx, _failure_rx, _host_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    client.authenticate("rotated-secret");

    let msg = protocol::read_client_message(&mut server_read.compat())
        .await
        .unwrap();
    let notification::Message::Authenticate(auth) = unwrap_notification(&msg) else {
        panic!("Expected Authenticate");
    };
    assert_eq!(auth.auth_token, "rotated-secret");
}

#[tokio::test]
async fn send_host_scoped_returns_ok_when_connected() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, _server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, _event_rx, _failure_rx, _host_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    let msg = ClientMessage::host_scoped(
        "req-host-1".to_string(),
        host_scoped_request::Message::WriteFile(WriteFile {
            path: "/tmp/foo.txt".to_string(),
            content: "hello".to_string(),
        }),
    );

    // On a healthy connection, dispatch succeeds (the message is queued).
    assert!(client.send_host_scoped(msg).is_ok());

    // The queued message is written to the server with the host-scoped envelope.
    let received = protocol::read_client_message(&mut server_read.compat())
        .await
        .unwrap();
    assert_eq!(received.request_id, "req-host-1");
    let host_scoped_request::Message::WriteFile(write) = unwrap_host_scoped(&received) else {
        panic!("Expected WriteFile host-scoped request");
    };
    assert_eq!(write.path, "/tmp/foo.txt");
    assert_eq!(write.content, "hello");
}

#[tokio::test]
async fn disconnected_on_closed_stream() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    // Drop the server side immediately.
    drop(server_stream);

    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, disconnect_rx, _failure_rx, _host_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    // An initialize call on a dead stream must complete with an error rather than hang.
    let result = client
        .initialize(
            None,
            InitializeParams {
                user_id: String::new(),
                user_email: String::new(),
                crash_reporting_enabled: true,
                codebase_index_limits: None,
            },
        )
        .await;
    assert!(result.is_err());

    // The reader task should detect EOF and emit a Disconnected event.
    let event = disconnect_rx.recv().await.unwrap();
    assert!(matches!(event, ClientEvent::Disconnected));

    // After the Disconnected event has been observed, the reader task has
    // already stored `true` into the atomic flag (it does the store before
    // sending the event), so callers can rely on `is_disconnected()` to
    // short-circuit further requests.
    assert!(client.is_disconnected());
}

#[tokio::test]
async fn is_disconnected_starts_false() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|_| {
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
        })
    });

    assert!(!client.is_disconnected());
}

#[tokio::test]
async fn run_command_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        let session_scoped_request::Message::RunCommand(req) = unwrap_session_scoped(msg) else {
            panic!("Expected RunCommand");
        };
        let command = req.command.clone();
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
                    codebase_index_limits: None,
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

/// A malformed *server* response carrying a parseable request_id that doesn't
/// match a session-scoped pending request must surface as
/// `HostScopedDecodeFailed` so the manager can fail the host request promptly
/// instead of letting it hang until the request timeout.
#[tokio::test]
async fn malformed_host_scoped_response_emits_decode_failed_event() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    drop(server_read);

    let executor = executor::Background::default();
    let (_client, event_rx, _failure_rx, _host_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
    let mut server_write = server_write.compat_write();

    // Field 1 (string): tag=0x0a, length=15, "host-req-decode", then invalid
    // trailing bytes (field 1, reserved wire type 7) so prost decode fails
    // while `try_extract_request_id` still recovers the request_id.
    let mut payload = Vec::new();
    payload.push(0x0a);
    payload.push(15);
    payload.extend_from_slice(b"host-req-decode");
    payload.extend_from_slice(&[0x0F, 0x01]);

    let len = payload.len() as u32;
    server_write.write_all(&len.to_le_bytes()).await.unwrap();
    server_write.write_all(&payload).await.unwrap();
    server_write.flush().await.unwrap();

    match event_rx.recv().await.unwrap() {
        ClientEvent::HostScopedDecodeFailed { request_id } => {
            assert_eq!(request_id.to_string(), "host-req-decode");
        }
        other => panic!("Expected HostScopedDecodeFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn get_diff_state_round_trips_as_session_scoped() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match unwrap_session_scoped(msg) {
            session_scoped_request::Message::GetDiffState(req) => {
                assert_eq!(req.repo_path, "/repo");
            }
            other => panic!("Expected GetDiffState, got {other:?}"),
        }
        server_message::Message::GetDiffStateResponse(GetDiffStateResponse { result: None })
    });

    let resp = client
        .get_diff_state("/repo".to_string(), crate::proto::DiffMode::default())
        .await
        .expect("get_diff_state should succeed");
    assert!(resp.result.is_none());
}

#[tokio::test]
async fn open_buffer_round_trips_as_session_scoped() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match unwrap_session_scoped(msg) {
            session_scoped_request::Message::OpenBuffer(req) => {
                assert_eq!(req.path, "/tmp/f.txt");
                assert!(!req.force_reload);
            }
            other => panic!("Expected OpenBuffer, got {other:?}"),
        }
        server_message::Message::OpenBufferResponse(OpenBufferResponse { result: None })
    });

    let resp = client
        .open_buffer("/tmp/f.txt".to_string(), false)
        .await
        .expect("open_buffer should succeed");
    assert!(resp.result.is_none());
}

/// A session-scoped request on a connection that has already dropped resolves
/// promptly with a transport error (no hang), because `pending_requests` is
/// cleared on disconnect.
#[tokio::test]
async fn get_diff_state_on_dead_connection_errors_promptly() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    drop(server_stream);

    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, disconnect_rx, _failure_rx, _host_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    // Drain the Disconnected event so the reader-task teardown is observed.
    let _ = disconnect_rx.recv().await;

    let result = client
        .get_diff_state("/repo".to_string(), crate::proto::DiffMode::default())
        .await;
    assert!(result.is_err());
}
