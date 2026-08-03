#[cfg(unix)]
use std::io::{Read as _, Write as _};

#[cfg(unix)]
use chrono::Duration;
use chrono::Utc;
use uuid::Uuid;

use super::*;
#[cfg(unix)]
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
