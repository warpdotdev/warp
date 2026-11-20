use super::*;

fn sample_id() -> PersistentSessionId {
    PersistentSessionId::from(uuid::Uuid::from_u128(
        0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
    ))
}

fn sample_metadata() -> SessionMetadata {
    SessionMetadata {
        hook_session_id: 42,
        shell_type: "zsh".to_owned(),
    }
}

#[test]
fn persistent_session_id_round_trips_through_uuid() {
    let uuid = uuid::Uuid::new_v4();
    let id = PersistentSessionId::from(uuid);
    assert_eq!(id.0, *uuid.as_bytes());
    assert_eq!(uuid::Uuid::from(id), uuid);
}

#[test]
fn client_message_round_trips_through_bincode() {
    let message = ClientMessage::Request {
        request_id: 7,
        request: DaemonRequest::Prune {
            keep: vec![sample_id()],
        },
    };
    let bytes = bincode::serialize(&message).unwrap();
    let decoded: ClientMessage = bincode::deserialize(&bytes).unwrap();
    match decoded {
        ClientMessage::Request {
            request_id: 7,
            request: DaemonRequest::Prune { keep },
        } => assert_eq!(keep, vec![sample_id()]),
        other => panic!("unexpected message: {other:?}"),
    }
}

#[test]
fn daemon_message_round_trips_through_bincode() {
    let message = DaemonMessage::Response {
        request_id: 3,
        response: DaemonResponse::Attached(AttachInfo {
            session_id: sample_id(),
            pid: 1234,
            metadata: sample_metadata(),
            replay_len: 512,
            has_bootstrap_preamble: true,
            exit_status: Some(ObservedExitStatus::Code(0)),
        }),
    };
    let bytes = bincode::serialize(&message).unwrap();
    let decoded: DaemonMessage = bincode::deserialize(&bytes).unwrap();
    match decoded {
        DaemonMessage::Response {
            request_id: 3,
            response: DaemonResponse::Attached(info),
        } => {
            assert_eq!(info.session_id, sample_id());
            assert_eq!(info.pid, 1234);
            assert_eq!(info.metadata.hook_session_id, 42);
            assert_eq!(info.metadata.shell_type, "zsh");
            assert_eq!(info.replay_len, 512);
            assert!(info.has_bootstrap_preamble);
            assert!(matches!(
                info.exit_status,
                Some(ObservedExitStatus::Code(0))
            ));
        }
        other => panic!("unexpected message: {other:?}"),
    }
}

#[test]
fn daemon_event_round_trips_through_bincode() {
    let message = DaemonMessage::Event(DaemonEvent::SessionExited {
        session_id: sample_id(),
        pid: 99,
        status: ObservedExitStatus::Signal(9),
    });
    let bytes = bincode::serialize(&message).unwrap();
    let decoded: DaemonMessage = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(
        decoded,
        DaemonMessage::Event(DaemonEvent::SessionExited {
            pid: 99,
            status: ObservedExitStatus::Signal(9),
            ..
        })
    ));
}

#[test]
fn spawn_environment_round_trips_non_utf8_values() {
    use std::os::unix::ffi::OsStringExt;

    let environment = SpawnEnvironment {
        vars: vec![
            (OsString::from("EXAMPLE_PLAIN"), OsString::from("value")),
            (
                OsString::from("EXAMPLE_RAW"),
                OsString::from_vec(vec![0x66, 0xff, 0xfe, 0x6f]),
            ),
        ],
    };
    let bytes = bincode::serialize(&environment).unwrap();
    let decoded: SpawnEnvironment = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded, environment);
}

#[test]
fn captured_environment_matches_the_process() {
    let environment = SpawnEnvironment::capture();
    let expected: Vec<_> = std::env::vars_os().collect();
    assert_eq!(environment.vars, expected);
}
