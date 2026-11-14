#[cfg(unix)]
use std::io::{Read as _, Write as _};

#[cfg(any(unix, windows))]
use chrono::Duration;
use chrono::Utc;
use uuid::Uuid;

use super::*;
#[cfg(any(unix, windows))]
use crate::auth::CredentialGrant;
use crate::discovery::{ControlEndpoint, CredentialBrokerReference, InstanceId};
#[cfg(unix)]
#[test]
fn credential_client_exchanges_request_over_broker_socket() {
    let dir = tempfile::tempdir().expect("temp dir");
    let socket_path = dir.path().join("broker.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("broker binds");
    let grant = CredentialGrant::new(
        InstanceId("inst_expected".to_owned()),
        ActionKind::AppPing,
        Duration::minutes(5),
    );
    let credential = ScopedCredential {
        bearer_token: "scoped-token".to_owned(),
        grant,
    };
    let expected_request = CredentialRequest::new(ActionKind::AppPing);
    let server_request = expected_request.clone();
    let server_credential = credential.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("broker accepts");
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .expect("broker reads request");
        let request = serde_json::from_slice::<CredentialRequest>(&bytes).expect("request decodes");
        assert_eq!(request, server_request);
        serde_json::to_writer(&mut stream, &server_credential).expect("broker writes credential");
        stream.flush().expect("broker flushes credential");
    });

    let response = request_credential_over_socket(&socket_path, &expected_request)
        .expect("credential exchange succeeds");
    server.join().expect("broker server completes");
    assert_eq!(
        serde_json::from_str::<ScopedCredential>(&response).expect("response decodes"),
        credential
    );
}

/// Serves one broker connection on a fresh named pipe from a background thread.
///
/// `respond` receives the delimited request bytes and returns the response to
/// write before the server closes its end.
#[cfg(windows)]
fn serve_one_pipe_connection(
    respond: impl FnOnce(Vec<u8>) -> Vec<u8> + Send + 'static,
) -> (String, std::thread::JoinHandle<()>) {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let pipe_path = format!(
        r"\\.\pipe\warp-local-control-test-{}",
        Uuid::new_v4().simple()
    );
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let server_path = pipe_path.clone();
    let server = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .expect("runtime builds");
        runtime.block_on(async move {
            let mut pipe = tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .create(&server_path)
                .expect("broker pipe is created");
            ready_tx.send(()).expect("ready signal is sent");
            pipe.connect().await.expect("broker accepts");
            let mut request = Vec::new();
            let mut reader = tokio::io::BufReader::new(&mut pipe);
            // A client that rejects the server disconnects without a request.
            let read = reader
                .read_until(CREDENTIAL_REQUEST_DELIMITER, &mut request)
                .await;
            if read.is_err() || request.last() != Some(&CREDENTIAL_REQUEST_DELIMITER) {
                return;
            }
            let response = respond(request);
            let _ = pipe.write_all(&response).await;
        });
    });
    ready_rx.recv().expect("broker pipe is ready");
    (pipe_path, server)
}

#[cfg(windows)]
#[test]
fn credential_client_exchanges_request_over_broker_pipe() {
    let grant = CredentialGrant::new(
        InstanceId("inst_expected".to_owned()),
        ActionKind::AppPing,
        Duration::minutes(5),
    );
    let credential = ScopedCredential {
        bearer_token: "scoped-token".to_owned(),
        grant,
    };
    let expected_request = CredentialRequest::new(ActionKind::AppPing);
    let server_request = expected_request.clone();
    let server_credential = credential.clone();
    let (pipe_path, server) = serve_one_pipe_connection(move |bytes| {
        assert_eq!(bytes.last(), Some(&CREDENTIAL_REQUEST_DELIMITER));
        let request = serde_json::from_slice::<CredentialRequest>(&bytes[..bytes.len() - 1])
            .expect("request decodes");
        assert_eq!(request, server_request);
        serde_json::to_vec(&server_credential).expect("credential encodes")
    });

    let response =
        request_credential_over_pipe(Path::new(&pipe_path), std::process::id(), &expected_request)
            .expect("credential exchange succeeds");
    server.join().expect("broker server completes");
    assert_eq!(
        serde_json::from_str::<ScopedCredential>(&response).expect("response decodes"),
        credential
    );
}

#[cfg(windows)]
#[test]
fn credential_client_rejects_pipe_served_by_unexpected_process() {
    let (pipe_path, server) = serve_one_pipe_connection(|_| {
        panic!("a rejected broker must not receive a credential request")
    });

    let err = request_credential_over_pipe(
        Path::new(&pipe_path),
        std::process::id().wrapping_add(1),
        &CredentialRequest::new(ActionKind::AppPing),
    )
    .expect_err("unexpected server process is rejected");
    server.join().expect("broker server completes");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[cfg(windows)]
#[test]
fn credential_client_reports_missing_broker_pipe() {
    let pipe_path = format!(
        r"\\.\pipe\warp-local-control-test-missing-{}",
        Uuid::new_v4().simple()
    );

    let err = request_credential_over_pipe(
        Path::new(&pipe_path),
        std::process::id(),
        &CredentialRequest::new(ActionKind::AppPing),
    )
    .expect_err("missing broker is reported");
    assert_eq!(err.code, ErrorCode::TransportUnavailable);
}

#[test]
fn probe_rejects_mismatched_instance_identity() {
    let instance = InstanceRecord {
        protocol_version: crate::PROTOCOL_VERSION,
        instance_id: InstanceId("inst_expected".to_owned()),
        pid: std::process::id(),
        channel: "local".to_owned(),
        app_id: "dev.warp.WarpLocal".to_owned(),
        app_version: None,
        started_at: Utc::now(),
        executable_path: None,
        endpoint: Some(ControlEndpoint::localhost(4000)),
        credential_broker: Some(CredentialBrokerReference {
            socket_path: "inst_expected.broker.sock".into(),
        }),
        actions: vec![ActionKind::AppPing.metadata()],
    };
    let err = validate_probe_response(
        &instance,
        ResponseEnvelope::ok(
            Uuid::new_v4(),
            serde_json::json!({ "instance_id": "inst_other" }),
        ),
    )
    .expect_err("mismatched live identity is rejected");
    assert_eq!(err.code, ErrorCode::TransportUnavailable);
}
